//! A batch of writes whose later steps name what earlier ones create. A
//! step that needs such an href carries the placeholder `{step:N}`, and
//! only where an href goes: the ruleset prefix of its path, or the href of
//! an IP list actor. The executor puts the href in just before sending;
//! free text (names, descriptions) is never touched. A placeholder never
//! passes `checked_href`, so it cannot reach the wire.
//!
//! [`Writer::plan_batch`] turns a generator proposal into the batch: one
//! fresh draft ruleset read (dedupe again, create-if-missing for every
//! ruleset), one fresh IP list read when it creates IP lists, one GET per
//! rule to update. [`Writer::execute_batch`] sends the steps one at a time
//! through [`Writer::execute`] and stops at the first failure.
use super::actors::ServiceRef;
use super::guard::WriteContext;
use super::ip_lists::IpListDraft;
use super::rule_sets::RuleSetDraft;
use super::rules::{RulePatch, apply_rule_patch};
use super::{
    ObjectKind, Plan, WriteError, WriteRequest, Writer, Written, checked_nested_href,
    checked_policy_href,
};
use crate::CacheMode;
use crate::client::WriteMethod;
use crate::generate::{GenActor, GenRule, Proposal, RuleAction, Target};
use crate::models::{IpList, PolicyScope, RuleSet};
use crate::resolve::NameIndex;
use serde_json::Value;

/// `{step:3}`: the href step 3 creates (or finds).
pub fn step_placeholder(step: usize) -> String {
    format!("{{step:{step}}}")
}

/// The step a whole placeholder names: `{step:3}` → 3.
pub fn parse_placeholder(s: &str) -> Option<usize> {
    s.strip_prefix("{step:")?.strip_suffix('}')?.parse().ok()
}

/// The profile and PCE a batch was planned on; it is sent only there.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BatchOrigin {
    pub profile: String,
    pub host: String,
    pub org_id: u64,
}

impl BatchOrigin {
    pub fn of(c: &WriteContext) -> Self {
        Self {
            profile: c.profile.clone(),
            host: c.host.clone(),
            org_id: c.org_id,
        }
    }

    /// `lab (pce.example, org 5)`.
    pub fn text(&self) -> String {
        format!("{} ({}, org {})", self.profile, self.host, self.org_id)
    }
}

/// One write of a batch: its plan, its kind, the earlier steps whose
/// hrefs it carries, the override denies that block its rules' flows,
/// and for an update the fields it replaces as planning read them.
#[derive(Debug, Clone)]
pub struct BatchStep {
    pub plan: Plan,
    pub kind: ObjectKind,
    pub needs: Vec<usize>,
    /// `blocked: 3 of 5 flows by override deny <href>`, one per rule.
    pub deny_notes: Vec<String>,
    /// `ipl:Corp: 7 IPs seen: 10.0.0.1, ...`: the IP peers its rules name
    /// an existing list for.
    pub notes: Vec<String>,
    /// An update's replaced fields as the plan-time GET returned them;
    /// the step is sent only while the rule still holds exactly these.
    pub expect: Option<Value>,
}

impl BatchStep {
    pub fn new(plan: Plan, kind: ObjectKind, needs: Vec<usize>) -> Self {
        Self {
            plan,
            kind,
            needs,
            deny_notes: Vec::new(),
            notes: Vec::new(),
            expect: None,
        }
    }
}

/// The steps in sending order (IP lists, rulesets, rule updates, rule
/// creates), the reads planning took and where it was planned.
#[derive(Debug, Clone, Default)]
pub struct Batch {
    pub steps: Vec<BatchStep>,
    pub reads: u32,
    /// `None`: never sent.
    pub origin: Option<BatchOrigin>,
    /// The proposal as the fresh read left it (targets, actions and
    /// override denies again): what the steps write. `None` outside
    /// `plan_batch`.
    pub refreshed: Option<Proposal>,
    /// The fresh draft ruleset read the steps were planned against.
    pub drafts: Option<Vec<RuleSet>>,
    /// A warning the confirm shows first (the caller's: what the fresh
    /// read changed against what was previewed).
    pub note: Option<String>,
}

/// What became of one step.
#[derive(Debug)]
pub enum StepResult {
    Written(Written),
    /// A `Noop`: the object existed; its href serves later steps.
    Skipped {
        existing_href: String,
    },
    Failed(WriteError),
    /// An earlier step failed (or was not audited).
    NotSent,
}

/// What `Cmd::PlanBatch` carries: the proposal on screen and what its
/// targets need to be resolved again against the fresh read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BatchInput {
    pub proposal: Proposal,
    /// `generator.ruleset_name`.
    pub ruleset_name: String,
    /// The app ruleset `R` picked among several.
    pub picked: Option<String>,
}

fn plural(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

/// The path's ruleset prefix: `{step:1}/sec_rules` → (1, `/sec_rules`).
fn path_slot(path: &str) -> Option<(usize, &str)> {
    let rest = path.strip_prefix("{step:")?;
    let j = rest.find('}')?;
    Some((rest[..j].parse().ok()?, &rest[j + 1..]))
}

/// Fills every placeholder slot of `req` with `f(step, kind it stands
/// for)`: the path's ruleset prefix and the href of each IP list actor.
/// Nothing else is read or changed.
fn fill(
    req: &mut WriteRequest,
    f: &mut dyn FnMut(usize, ObjectKind) -> Result<String, WriteError>,
) -> Result<(), WriteError> {
    if let Some((n, rest)) = path_slot(&req.path) {
        let rest = rest.to_string();
        req.path = format!("{}{rest}", f(n, ObjectKind::RuleSet)?);
    }
    if let Some(body) = req.body.as_mut() {
        for side in ["providers", "consumers"] {
            let Some(Value::Array(actors)) = body.get_mut(side) else {
                continue;
            };
            for a in actors {
                if let Some(Value::String(h)) = a.get_mut("ip_list").and_then(|l| l.get_mut("href"))
                    && let Some(n) = parse_placeholder(h)
                {
                    *h = f(n, ObjectKind::IpList)?;
                }
            }
        }
    }
    Ok(())
}

/// Whether a placeholder stands in the path or in any `href` of the body:
/// after [`fill`], one that is not in a slot it may fill.
fn holds_placeholder(req: &WriteRequest) -> bool {
    fn any_href(v: &Value) -> bool {
        match v {
            Value::Array(a) => a.iter().any(any_href),
            Value::Object(o) => o.iter().any(|(k, x)| match x {
                Value::String(s) => k == "href" && s.contains("{step:"),
                other => any_href(other),
            }),
            _ => false,
        }
    }
    req.path.contains("{step:") || req.body.as_ref().is_some_and(any_href)
}

/// Every step names exactly the steps it waits for, each earlier and of
/// the kind its slot takes, and no placeholder stands where none may.
/// Checked at plan time and again before the first request.
fn check_placeholders(steps: &[BatchStep]) -> Result<(), WriteError> {
    for (i, s) in steps.iter().enumerate() {
        let Plan::Send { request, .. } = &s.plan else {
            if s.needs.is_empty() {
                continue;
            }
            return Err(WriteError::invalid(format!(
                "step {} sends nothing but waits for other steps: nothing sent",
                i + 1
            )));
        };
        let mut named: Vec<(usize, ObjectKind)> = Vec::new();
        let mut probe = request.clone();
        fill(&mut probe, &mut |n, kind| {
            named.push((n, kind));
            Ok(String::new())
        })?;
        if holds_placeholder(&probe) {
            return Err(WriteError::invalid(format!(
                "step {} holds a placeholder outside the hrefs it may fill: nothing sent",
                i + 1
            )));
        }
        for (n, kind) in &named {
            if *n >= i || !s.needs.contains(n) {
                return Err(WriteError::invalid(format!(
                    "step {} names step {}, which it does not wait for: nothing sent",
                    i + 1,
                    n + 1
                )));
            }
            if steps[*n].kind != *kind {
                return Err(WriteError::invalid(format!(
                    "step {} takes a {} href from step {}, which writes a {}: nothing sent",
                    i + 1,
                    kind.as_str(),
                    n + 1,
                    steps[*n].kind.as_str()
                )));
            }
        }
        if let Some(n) = s.needs.iter().find(|n| !named.iter().any(|(m, _)| m == *n)) {
            return Err(WriteError::invalid(format!(
                "step {} waits for step {} but names it nowhere: nothing sent",
                i + 1,
                n + 1
            )));
        }
    }
    Ok(())
}

impl Batch {
    /// The steps that send a request (not the `Noop`s).
    pub fn sends(&self) -> usize {
        self.steps
            .iter()
            .filter(|s| s.plan.request().is_some())
            .count()
    }

    /// Rules whose flows an override deny blocks.
    pub fn denied_rules(&self) -> usize {
        self.steps.iter().map(|s| s.deny_notes.len()).sum()
    }

    /// `7 requests: 1 ip_list, 1 rule_set, 1 rule update, 4 rules ·
    /// planned with 3 reads`, what is reused and how many rules an
    /// override deny blocks.
    pub fn summary(&self) -> String {
        let count = |kind: ObjectKind, method: WriteMethod| {
            self.steps
                .iter()
                .filter(|s| s.kind == kind && s.plan.request().is_some_and(|r| r.method == method))
                .count()
        };
        let parts: Vec<String> = [
            (
                count(ObjectKind::IpList, WriteMethod::Post),
                "ip_list",
                "ip_lists",
            ),
            (
                count(ObjectKind::RuleSet, WriteMethod::Post),
                "rule_set",
                "rule_sets",
            ),
            (
                count(ObjectKind::Rule, WriteMethod::Put),
                "rule update",
                "rule updates",
            ),
            (count(ObjectKind::Rule, WriteMethod::Post), "rule", "rules"),
        ]
        .into_iter()
        .filter(|(n, _, _)| *n > 0)
        .map(|(n, one, many)| plural(n, one, many))
        .collect();
        let mut s = format!(
            "{}: {} · planned with {}",
            plural(self.sends(), "request", "requests"),
            parts.join(", "),
            plural(self.reads as usize, "read", "reads")
        );
        let reused = self.steps.len() - self.sends();
        if reused > 0 {
            s.push_str(&format!(
                " · reuses {}",
                plural(reused, "object", "objects")
            ));
        }
        let checks = self.steps.iter().filter(|s| s.expect.is_some()).count();
        if checks > 0 {
            s.push_str(&format!(
                " · +{} to check rules before updating",
                plural(checks, "GET", "GETs")
            ));
        }
        let denied = self.denied_rules();
        if denied > 0 {
            s.push_str(&format!(
                " · {} blocked by override denies",
                plural(denied, "rule", "rules")
            ));
        }
        s
    }

    /// How step `n` (from 0) stands in another's request for a person:
    /// `<href of step 2: create rule_set RF-shop-prod>`, or the existing
    /// href a `Noop` found.
    fn spelled(&self, n: usize) -> String {
        match self.steps.get(n).map(|s| &s.plan) {
            Some(Plan::Send { request, .. }) => {
                format!("<href of step {}: {}>", n + 1, request.summary)
            }
            Some(Plan::Noop { existing_href, .. }) => existing_href.clone(),
            None => format!("<href of missing step {}>", n + 1),
        }
    }

    /// The confirm's text: one block per step, `N/M METHOD path`, the
    /// summary, the override denies that block its rules, the body;
    /// placeholders spelled where they stand (free text stays as it is).
    pub fn lines(&self) -> Vec<String> {
        let n = self.steps.len();
        let mut out = Vec::new();
        for (i, s) in self.steps.iter().enumerate() {
            match &s.plan {
                Plan::Send { request, .. } => {
                    let mut shown = request.clone();
                    let _ = fill(&mut shown, &mut |m, _| Ok(self.spelled(m)));
                    out.push(format!(
                        "{}/{n} {} {}",
                        i + 1,
                        shown.method.as_str(),
                        shown.path
                    ));
                    out.push(format!("  {}", shown.summary));
                    out.extend(s.deny_notes.iter().map(|d| format!("  ! {d}")));
                    out.extend(s.notes.iter().map(|d| format!("  · {d}")));
                    match &shown.body {
                        Some(b) => out.extend(
                            serde_json::to_string_pretty(b)
                                .unwrap_or_default()
                                .lines()
                                .map(|l| format!("  {l}")),
                        ),
                        None => out.push("  (no body)".into()),
                    }
                }
                Plan::Noop {
                    existing_href,
                    reason,
                } => out.push(format!("{}/{n} reuse {existing_href}: {reason}", i + 1)),
            }
            out.push(String::new());
        }
        out.pop();
        out
    }
}

/// The href a finished step left: written, or found by a `Noop`.
fn result_href(r: &StepResult) -> Option<String> {
    match r {
        StepResult::Written(w) => Some(w.href.clone()),
        StepResult::Skipped { existing_href } => Some(existing_href.clone()),
        StepResult::Failed(_) | StepResult::NotSent => None,
    }
}

/// Whether the batch stops after this step: it failed, or it was
/// written without an audit entry.
fn stops(r: &StepResult) -> bool {
    match r {
        StepResult::Failed(_) => true,
        StepResult::Written(w) => w.audit_failed.is_some(),
        StepResult::Skipped { .. } | StepResult::NotSent => false,
    }
}

/// Step `i`'s plan with the hrefs of the steps it needs put in, each from
/// this batch's own results and checked as the kind it stands for;
/// refused when a placeholder names a step it does not wait for, one
/// without an href, or survives.
fn resolve_step(
    i: usize,
    step: &BatchStep,
    done: &[Option<String>],
    org_id: u64,
) -> Result<Plan, WriteError> {
    let Plan::Send { request, kind } = &step.plan else {
        return Ok(step.plan.clone());
    };
    let mut request = request.clone();
    fill(&mut request, &mut |n, slot| {
        if n >= i || !step.needs.contains(&n) {
            return Err(WriteError::invalid(format!(
                "step {} names step {}, which it does not wait for: nothing more sent",
                i + 1,
                n + 1
            )));
        }
        let href = done.get(n).cloned().flatten().ok_or_else(|| {
            WriteError::invalid(format!(
                "step {} needs the href of step {}, which has none: nothing more sent",
                i + 1,
                n + 1
            ))
        })?;
        match slot {
            ObjectKind::RuleSet => checked_policy_href(&href, "rule_sets", org_id),
            _ => checked_policy_href(&href, "ip_lists", org_id),
        }
    })?;
    if holds_placeholder(&request) {
        return Err(WriteError::invalid(format!(
            "step {} still holds a placeholder: nothing more sent",
            i + 1
        )));
    }
    Ok(Plan::Send {
        request,
        kind: *kind,
    })
}

/// An IP list found by name serves the rules only when it holds exactly
/// the draft's ranges and FQDNs: a list of that name with other addresses
/// would let the rules reach them.
fn reused_list_holds(draft: &IpListDraft, href: &str, lists: &[IpList]) -> Result<(), WriteError> {
    let Some(found) = lists.iter().find(|l| l.href == href) else {
        return Err(WriteError::invalid(format!(
            "ip_list {} was found as {href} but is not in the read: nothing planned",
            draft.name.trim()
        )));
    };
    let have = IpListDraft::from_ip_list(found);
    let same = have.ranges.len() == draft.ranges.len()
        && draft
            .ranges
            .iter()
            .all(|r| have.ranges.iter().any(|x| x.same_range(r)))
        && have.fqdns.len() == draft.fqdns.len()
        && draft
            .fqdns
            .iter()
            .all(|f| have.fqdns.iter().any(|x| x.fqdn == f.fqdn));
    if same {
        Ok(())
    } else {
        Err(WriteError::invalid(format!(
            "ip_list {} exists ({href}) but holds other addresses: rename or fix it; nothing planned",
            draft.name.trim()
        )))
    }
}

/// A ruleset to create that the fresh read already has by name: it is
/// not the scope's (`refresh` would have made it the target), so the
/// batch must not write into it.
fn refuse_named_elsewhere(plan: &Plan, name: &str, scope: &str) -> Result<(), WriteError> {
    match plan {
        Plan::Noop { existing_href, .. } => Err(WriteError::invalid(format!(
            "ruleset {name} exists ({existing_href}) but its scope does not cover {scope}: rename it or set generator.ruleset_name; nothing planned"
        ))),
        Plan::Send { .. } => Ok(()),
    }
}

/// `ipl:Corp: 2 IPs seen: 10.1.4.9, 10.1.4.10`, one per existing list
/// the rule names for IP peers.
fn ip_notes(r: &GenRule) -> Vec<String> {
    r.ips
        .iter()
        .map(|(list, ips)| crate::generate::seen_text(list, ips))
        .collect()
}

/// `blocked: 3 of 5 flows by override deny <href>` for a rule whose flows
/// an override deny blocks (as the fresh read says).
fn deny_note(r: &GenRule) -> Option<String> {
    (r.denied_flows > 0).then(|| {
        format!(
            "blocked: {} of {} flows by override deny {}",
            r.denied_flows.min(r.flows),
            r.flows,
            r.denied_by.join(", ")
        )
    })
}

impl Writer<'_> {
    /// The create plan of each ruleset against `sets` (one read, already
    /// made): a live ruleset with the same name (any case) is a `Noop`
    /// with its href, several are an error. No request.
    pub fn plan_rule_set_creates(
        &self,
        drafts: &[RuleSetDraft],
        sets: &[RuleSet],
    ) -> Result<Vec<Plan>, WriteError> {
        drafts
            .iter()
            .map(|d| self.rule_set_create(d, Some(sets)))
            .collect()
    }

    /// [`Writer::plan_rule_set_creates`] after one fresh draft ruleset
    /// read for all of them; refused when the read is truncated.
    pub async fn plan_create_rule_sets_if_missing(
        &self,
        drafts: &[RuleSetDraft],
    ) -> Result<Vec<Plan>, WriteError> {
        let sets = self
            .pce()
            .rule_sets(PolicyScope::Draft, CacheMode::Bypass)
            .await?;
        if sets.truncated {
            return Err(WriteError::invalid(
                "too many rulesets to check which exist; nothing planned",
            ));
        }
        self.plan_rule_set_creates(drafts, &sets.value)
    }

    /// The create plan of each IP list against `lists` (one read, already
    /// made): a live list of exactly that name is a `Noop` with its href.
    pub fn plan_ip_list_creates(
        &self,
        drafts: &[IpListDraft],
        lists: &[IpList],
    ) -> Result<Vec<Plan>, WriteError> {
        drafts
            .iter()
            .map(|d| self.ip_list_create(d, Some(lists)))
            .collect()
    }

    /// The batch for `input`: the reads above, then the steps in sending
    /// order, bound to this writer's profile and PCE. Refused, with
    /// nothing sent, when a fresh read is truncated, a rule's target is
    /// ambiguous, nothing is left to write, a ruleset to create has the
    /// name of one that does not cover its scope, or an IP list of the
    /// name holds other addresses.
    pub async fn plan_batch(
        &self,
        input: BatchInput,
        idx: &NameIndex,
    ) -> Result<Batch, WriteError> {
        let BatchInput {
            mut proposal,
            ruleset_name,
            picked,
        } = input;
        let org = self.pce().client().org_id();
        let fresh = self
            .pce()
            .rule_sets(PolicyScope::Draft, CacheMode::Bypass)
            .await?;
        let mut reads = 1;
        if fresh.truncated {
            return Err(WriteError::invalid(
                "too many draft rulesets to prove the rules are not there yet; nothing planned",
            ));
        }
        proposal.refresh(&fresh.value, idx, &ruleset_name, picked.as_deref());
        if let Some(why) = proposal.blocker() {
            return Err(WriteError::invalid(why));
        }
        if !proposal.writes() {
            return Err(WriteError::invalid(
                "every proposed rule is in the draft already; nothing to write",
            ));
        }
        let mut steps = Vec::new();
        if !proposal.ip_lists.is_empty() {
            let lists = self.pce().ip_lists(CacheMode::Bypass).await?;
            reads += 1;
            if lists.truncated {
                return Err(WriteError::invalid(
                    "too many IP lists to check which exist; nothing planned",
                ));
            }
            let plans = self.plan_ip_list_creates(&proposal.ip_lists, &lists.value)?;
            for (i, (d, plan)) in proposal.ip_lists.iter().zip(plans).enumerate() {
                if let Plan::Noop { existing_href, .. } = &plan {
                    reused_list_holds(d, existing_href, &lists.value)?;
                }
                if proposal.ip_list_step(i) != steps.len() {
                    return Err(WriteError::invalid(
                        "the batch's IP list steps are out of order; nothing planned",
                    ));
                }
                steps.push(BatchStep::new(plan, ObjectKind::IpList, Vec::new()));
            }
        }
        let creates: Vec<(usize, &RuleSetDraft)> = proposal
            .targets
            .iter()
            .enumerate()
            .filter_map(|(t, x)| match x {
                Target::Create(d) => Some((t, d)),
                _ => None,
            })
            .collect();
        let drafts: Vec<RuleSetDraft> = creates.iter().map(|(_, d)| (*d).clone()).collect();
        for ((t, d), plan) in creates
            .iter()
            .zip(self.plan_rule_set_creates(&drafts, &fresh.value)?)
        {
            // `refresh` already gives such a scope no target; kept so a
            // ruleset scoped elsewhere is never written into.
            refuse_named_elsewhere(&plan, &d.name, &proposal.scopes[*t].text())?;
            if proposal.ruleset_step(*t) != Some(steps.len()) {
                return Err(WriteError::invalid(
                    "the batch's ruleset steps are out of order; nothing planned",
                ));
            }
            steps.push(BatchStep::new(plan, ObjectKind::RuleSet, Vec::new()));
        }
        // Services added to one rule by several proposed rules go in one
        // update: two PUTs of the list would drop the first one's.
        // (rule href, services to add, deny notes, IP notes).
        type Update = (String, Vec<ServiceRef>, Vec<String>, Vec<String>);
        let mut updates: Vec<Update> = Vec::new();
        for r in &proposal.rules {
            let RuleAction::AddServices { rule_href, add } = &r.action else {
                continue;
            };
            let href = checked_nested_href(rule_href, "rule_sets", "sec_rules", org)?;
            let note = deny_note(r);
            match updates.iter_mut().find(|u| u.0 == href) {
                Some((_, adds, notes, ips)) => {
                    for s in add {
                        if !adds.contains(s) {
                            adds.push(s.clone());
                        }
                    }
                    notes.extend(note);
                    ips.extend(ip_notes(r));
                }
                None => updates.push((href, add.clone(), note.into_iter().collect(), ip_notes(r))),
            }
        }
        for (href, add, notes, ips) in updates {
            let patch = RulePatch {
                add_services: add,
                ..RulePatch::default()
            };
            patch.check().map_err(WriteError::Invalid)?;
            let current = self.fetch_rule(&href).await?;
            reads += 1;
            let body = apply_rule_patch(&current, &patch).map_err(WriteError::Invalid)?;
            // What the PUT replaces, as read: the step waits for it unchanged.
            let expect = body.as_object().map(|o| {
                Value::Object(
                    o.keys()
                        .map(|k| (k.clone(), current.get(k).cloned().unwrap_or(Value::Null)))
                        .collect(),
                )
            });
            let mut step = BatchStep::new(
                super::rules::update(href, "rule", body, ObjectKind::Rule),
                ObjectKind::Rule,
                Vec::new(),
            );
            step.expect = expect.filter(|_| step.plan.request().is_some());
            step.deny_notes = notes;
            step.notes = ips;
            steps.push(step);
        }
        for r in &proposal.rules {
            if r.action != RuleAction::Create {
                continue;
            }
            let draft = r.to_draft(&proposal);
            let mut needs: Vec<usize> = r
                .providers
                .iter()
                .chain(&r.consumers)
                .filter_map(|a| match a {
                    GenActor::NewIpList(i) => Some(proposal.ip_list_step(*i)),
                    GenActor::Ref(_) => None,
                })
                .collect();
            let plan = match (
                proposal.targets.get(r.target),
                proposal.ruleset_step(r.target),
            ) {
                (Some(Target::Existing { href, .. }), _) => self.plan_create_rule(href, &draft)?,
                (Some(Target::Create(d)), Some(step)) => {
                    needs.push(step);
                    draft.validate().map_err(WriteError::Invalid)?;
                    Plan::Send {
                        request: WriteRequest {
                            method: WriteMethod::Post,
                            path: format!("{}/sec_rules", step_placeholder(step)),
                            body: Some(draft.to_body()),
                            summary: format!("create rule in rule_set {}", d.name.trim()),
                            objects: Vec::new(),
                        },
                        kind: ObjectKind::Rule,
                    }
                }
                _ => return Err(WriteError::invalid("a rule has no ruleset to go into")),
            };
            needs.sort_unstable();
            needs.dedup();
            let mut step = BatchStep::new(plan, ObjectKind::Rule, needs);
            step.deny_notes.extend(deny_note(r));
            step.notes = ip_notes(r);
            steps.push(step);
        }
        check_placeholders(&steps)?;
        Ok(Batch {
            steps,
            reads,
            origin: Some(BatchOrigin::of(self.guard().context())),
            refreshed: Some(proposal),
            drafts: Some(fresh.value),
            note: None,
        })
    }

    /// Sends the steps in order, one [`Writer::execute`] each (audit,
    /// cache invalidation, re-fetch on a 204); the first failure, or a
    /// write whose audit entry could not be written, stops the batch and
    /// the rest are `NotSent`. Refused whole, with nothing sent, when it
    /// was planned on another profile or PCE or its placeholders do not
    /// add up. An update is sent only while its rule still holds what
    /// planning read (one GET before its PUT). Never retried, nothing
    /// rolled back: the steps are draft objects.
    pub async fn execute_batch(&self, batch: Batch) -> Vec<StepResult> {
        let mut out = Vec::with_capacity(batch.steps.len());
        self.execute_batch_each(batch, &mut |_, r| out.push(r))
            .await;
        out
    }

    /// [`Writer::execute_batch`], handing each step's result to `each` as
    /// soon as it is known (in step order, every step once), so a caller
    /// keeps an exact record even if this future is dropped or panics.
    pub async fn execute_batch_each(
        &self,
        batch: Batch,
        each: &mut (dyn FnMut(usize, StepResult) + Send),
    ) {
        let n = batch.steps.len();
        let mut refuse = |e: WriteError| {
            each(0, StepResult::Failed(e));
            for i in 1..n {
                each(i, StepResult::NotSent);
            }
        };
        if n == 0 {
            return;
        }
        let here = BatchOrigin::of(self.guard().context());
        match &batch.origin {
            None => {
                return refuse(WriteError::invalid(
                    "the batch names no profile it was planned on: nothing sent",
                ));
            }
            Some(o) if *o != here => {
                return refuse(WriteError::invalid(format!(
                    "planned on {}, this session writes to {}: nothing sent; plan again",
                    o.text(),
                    here.text()
                )));
            }
            Some(_) => {}
        }
        if let Err(e) = check_placeholders(&batch.steps) {
            return refuse(e);
        }
        let org = self.pce().client().org_id();
        // Each finished step's href (for the steps after it), and whether
        // the batch has stopped.
        let mut hrefs: Vec<Option<String>> = Vec::with_capacity(n);
        let mut stopped = false;
        for (i, step) in batch.steps.iter().enumerate() {
            let result = if stopped {
                StepResult::NotSent
            } else {
                self.execute_step(i, step, &hrefs, org).await
            };
            hrefs.push(result_href(&result));
            stopped = stopped || stops(&result);
            each(i, result);
        }
    }

    /// Step `i` with its hrefs put in, its rule checked unchanged (an
    /// update), sent once.
    async fn execute_step(
        &self,
        i: usize,
        step: &BatchStep,
        hrefs: &[Option<String>],
        org: u64,
    ) -> StepResult {
        let plan = match resolve_step(i, step, hrefs, org) {
            Ok(plan) => plan,
            Err(e) => return StepResult::Failed(e),
        };
        if let (Some(expect), Some(req)) = (&step.expect, plan.request())
            && let Err(e) = self.unchanged(i, &req.path, expect).await
        {
            return StepResult::Failed(e);
        }
        match self.execute(plan).await {
            Ok(super::Outcome::Written(w)) => StepResult::Written(w),
            Ok(super::Outcome::Skipped { existing_href, .. }) => {
                StepResult::Skipped { existing_href }
            }
            Err(e) => StepResult::Failed(e),
        }
    }

    /// One GET of the rule step `i` updates: refused when a field the PUT
    /// replaces differs from what planning read (someone changed it).
    async fn unchanged(&self, i: usize, href: &str, expect: &Value) -> Result<(), WriteError> {
        let now = self.fetch_rule(href).await?;
        let changed: Vec<&str> = expect
            .as_object()
            .into_iter()
            .flatten()
            .filter(|(k, v)| now.get(k.as_str()).unwrap_or(&Value::Null) != *v)
            .map(|(k, _)| k.as_str())
            .collect();
        if changed.is_empty() {
            Ok(())
        } else {
            Err(WriteError::invalid(format!(
                "step {}: rule {href} changed since planning ({}): nothing more sent; plan again",
                i + 1,
                changed.join(", ")
            )))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::write::checked_href;
    use serde_json::json;

    fn post(path: &str, body: Value, kind: ObjectKind, needs: Vec<usize>) -> BatchStep {
        BatchStep::new(
            Plan::Send {
                request: WriteRequest {
                    method: WriteMethod::Post,
                    path: path.into(),
                    body: Some(body),
                    summary: "test".into(),
                    objects: Vec::new(),
                },
                kind,
            },
            kind,
            needs,
        )
    }

    #[test]
    fn placeholders_round_trip_and_never_pass_as_hrefs() {
        assert_eq!(step_placeholder(3), "{step:3}");
        assert_eq!(parse_placeholder("{step:3}"), Some(3));
        assert_eq!(parse_placeholder("{step:x}"), None);
        assert_eq!(
            parse_placeholder("/orgs/5/sec_policy/draft/rule_sets/1"),
            None
        );
        assert!(checked_policy_href(&step_placeholder(1), "rule_sets", 5).is_err());
        assert!(checked_href(&step_placeholder(0), "ip_lists", 5).is_err());
        assert_eq!(path_slot("{step:1}/sec_rules"), Some((1, "/sec_rules")));
    }

    #[test]
    fn placeholders_are_filled_in_href_slots_only() {
        let mut s = post(
            "{step:1}/sec_rules",
            json!({"description": "app {step:0}", "providers": [{"ip_list": {"href": "{step:0}"}}]}),
            ObjectKind::Rule,
            vec![0, 1],
        );
        let Plan::Send { request, .. } = &mut s.plan else {
            unreachable!()
        };
        fill(request, &mut |n, k| Ok(format!("<{n} {}>", k.as_str()))).unwrap();
        assert_eq!(request.path, "<1 rule_set>/sec_rules");
        let body = request.body.as_ref().unwrap();
        assert_eq!(body["providers"][0]["ip_list"]["href"], "<0 ip_list>");
        assert_eq!(body["description"], "app {step:0}", "free text untouched");
        assert!(!holds_placeholder(request));
    }

    #[test]
    fn the_plan_time_check_refuses_what_does_not_add_up() {
        let ipl = post(
            "/orgs/5/sec_policy/draft/ip_lists",
            json!({}),
            ObjectKind::IpList,
            vec![],
        );
        let rs = post(
            "/orgs/5/sec_policy/draft/rule_sets",
            json!({}),
            ObjectKind::RuleSet,
            vec![],
        );
        let ok = post(
            "{step:1}/sec_rules",
            json!({"providers": [{"ip_list": {"href": "{step:0}"}}], "description": "{step:7}"}),
            ObjectKind::Rule,
            vec![0, 1],
        );
        let steps = vec![ipl.clone(), rs.clone(), ok];
        assert!(
            check_placeholders(&steps).is_ok(),
            "free text may say anything"
        );
        let err = |steps: Vec<BatchStep>| check_placeholders(&steps).unwrap_err().to_string();
        // Waits for a step it never names.
        let idle = post("/orgs/5/x", json!({}), ObjectKind::Rule, vec![0]);
        assert!(err(vec![ipl.clone(), idle]).contains("waits for step 1 but names it nowhere"));
        // An IP list slot naming the ruleset step.
        let wrong = post(
            "/orgs/5/sec_policy/draft/rule_sets/1/sec_rules",
            json!({"providers": [{"ip_list": {"href": "{step:1}"}}]}),
            ObjectKind::Rule,
            vec![1],
        );
        assert!(
            err(vec![ipl.clone(), rs.clone(), wrong])
                .contains("takes a ip_list href from step 2, which writes a rule_set")
        );
        // A placeholder in an href that is not a slot.
        let stray = post(
            "/orgs/5/sec_policy/draft/rule_sets/1/sec_rules",
            json!({"consumers": [{"label": {"href": "{step:0}"}}]}),
            ObjectKind::Rule,
            vec![],
        );
        assert!(err(vec![ipl, stray]).contains("outside the hrefs it may fill"));
    }

    #[test]
    fn a_ruleset_found_by_name_is_refused() {
        let found = Plan::Noop {
            existing_href: "/orgs/5/sec_policy/draft/rule_sets/300".into(),
            reason: "rule_set RF-crm-prod exists".into(),
        };
        let e = refuse_named_elsewhere(&found, "RF-crm-prod", "crm/prod").unwrap_err();
        assert!(
            e.to_string().contains(
                "ruleset RF-crm-prod exists (/orgs/5/sec_policy/draft/rule_sets/300) but its scope does not cover crm/prod"
            ),
            "{e}"
        );
        let send = post(
            "/orgs/5/sec_policy/draft/rule_sets",
            json!({}),
            ObjectKind::RuleSet,
            vec![],
        );
        assert!(refuse_named_elsewhere(&send.plan, "RF-crm-prod", "crm/prod").is_ok());
    }
}
