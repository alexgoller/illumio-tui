//! A batch of writes whose later steps name what earlier ones create. A
//! step that needs such an href carries the placeholder `{step:N}`; the
//! executor puts the href in just before sending. A placeholder never
//! passes `checked_href`, so it cannot reach the wire.
//!
//! [`Writer::plan_batch`] turns a generator proposal into the batch: one
//! fresh draft ruleset read (dedupe again, create-if-missing for every
//! ruleset), one fresh IP list read when it creates IP lists, one GET per
//! rule update. [`Writer::execute_batch`] sends the steps one at a time
//! through [`Writer::execute`] and stops at the first failure.
use super::ip_lists::IpListDraft;
use super::rule_sets::RuleSetDraft;
use super::rules::RulePatch;
use super::{
    ObjectKind, Plan, WriteError, WriteRequest, Writer, Written, checked_href, checked_policy_href,
};
use crate::CacheMode;
use crate::client::WriteMethod;
use crate::generate::{GenActor, Proposal, RuleAction, Target};
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

/// Every step `s` names, in order of appearance.
fn placeholders_in(s: &str) -> Vec<usize> {
    let mut out = Vec::new();
    let mut rest = s;
    while let Some(i) = rest.find("{step:") {
        let tail = &rest[i..];
        match tail.find('}') {
            Some(j) => {
                if let Some(n) = parse_placeholder(&tail[..=j]) {
                    out.push(n);
                }
                rest = &tail[j + 1..];
            }
            None => break,
        }
    }
    out
}

/// One write of a batch: its plan, its kind, the earlier steps whose
/// hrefs it carries.
#[derive(Debug, Clone)]
pub struct BatchStep {
    pub plan: Plan,
    pub kind: ObjectKind,
    pub needs: Vec<usize>,
}

/// The steps in sending order (IP lists, rulesets, rule updates, rule
/// creates) and the reads planning took.
#[derive(Debug, Clone, Default)]
pub struct Batch {
    pub steps: Vec<BatchStep>,
    pub reads: u32,
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
    /// An earlier step failed.
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

impl Batch {
    /// The steps that send a request (not the `Noop`s).
    pub fn sends(&self) -> usize {
        self.steps
            .iter()
            .filter(|s| s.plan.request().is_some())
            .count()
    }

    /// `7 requests: 1 ip_list, 1 rule_set, 1 rule update, 4 rules ·
    /// planned with 3 reads`, and what is reused.
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
        s
    }

    /// `text` with each placeholder spelled for a person: `<href of step
    /// 2: create rule_set RF-shop-prod>` (steps counted from 1), or the
    /// existing href a `Noop` found.
    pub fn spell(&self, text: &str) -> String {
        let mut out = text.to_string();
        for n in placeholders_in(text) {
            let what = match self.steps.get(n).map(|s| &s.plan) {
                Some(Plan::Send { request, .. }) => {
                    format!("<href of step {}: {}>", n + 1, request.summary)
                }
                Some(Plan::Noop { existing_href, .. }) => existing_href.clone(),
                None => format!("<href of missing step {}>", n + 1),
            };
            out = out.replace(&step_placeholder(n), &what);
        }
        out
    }

    /// The confirm's text: one block per step, `N/M METHOD path`, the
    /// summary and the body, placeholders spelled.
    pub fn lines(&self) -> Vec<String> {
        let n = self.steps.len();
        let mut out = Vec::new();
        for (i, s) in self.steps.iter().enumerate() {
            match &s.plan {
                Plan::Send { request, .. } => {
                    out.push(format!(
                        "{}/{n} {} {}",
                        i + 1,
                        request.method.as_str(),
                        self.spell(&request.path)
                    ));
                    out.push(format!("  {}", self.spell(&request.summary)));
                    match &request.body {
                        Some(b) => out.extend(
                            serde_json::to_string_pretty(b)
                                .unwrap_or_default()
                                .lines()
                                .map(|l| format!("  {}", self.spell(l))),
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

/// A string with every placeholder replaced by `href_of`'s answer.
fn substitute_str(
    s: &str,
    href_of: &impl Fn(usize) -> Result<String, WriteError>,
) -> Result<String, WriteError> {
    let mut out = s.to_string();
    for n in placeholders_in(s) {
        out = out.replace(&step_placeholder(n), &href_of(n)?);
    }
    Ok(out)
}

fn substitute_value(
    v: &mut Value,
    href_of: &impl Fn(usize) -> Result<String, WriteError>,
) -> Result<(), WriteError> {
    match v {
        Value::String(s) => *s = substitute_str(s, href_of)?,
        Value::Array(a) => {
            for x in a {
                substitute_value(x, href_of)?;
            }
        }
        Value::Object(o) => {
            for x in o.values_mut() {
                substitute_value(x, href_of)?;
            }
        }
        _ => {}
    }
    Ok(())
}

/// The href a finished step left: written, or found by a `Noop`.
fn result_href(r: &StepResult) -> Option<&str> {
    match r {
        StepResult::Written(w) => Some(&w.href),
        StepResult::Skipped { existing_href } => Some(existing_href),
        StepResult::Failed(_) | StepResult::NotSent => None,
    }
}

/// Step `i`'s plan with the hrefs of the steps it needs put in, each
/// checked as the kind it stands for; refused when a placeholder names a
/// step it does not wait for, one without an href, or survives.
fn resolve_step(
    i: usize,
    step: BatchStep,
    done: &[StepResult],
    kinds: &[ObjectKind],
    org_id: u64,
) -> Result<Plan, WriteError> {
    let BatchStep { plan, kind, needs } = step;
    let Plan::Send {
        mut request,
        kind: plan_kind,
    } = plan
    else {
        return Ok(plan);
    };
    let href_of = |n: usize| -> Result<String, WriteError> {
        if n >= i || !needs.contains(&n) {
            return Err(WriteError::invalid(format!(
                "step {} names step {}, which it does not wait for: nothing more sent",
                i + 1,
                n + 1
            )));
        }
        let href = done.get(n).and_then(result_href).ok_or_else(|| {
            WriteError::invalid(format!(
                "step {} needs the href of step {}, which has none: nothing more sent",
                i + 1,
                n + 1
            ))
        })?;
        match kinds[n] {
            ObjectKind::RuleSet => checked_policy_href(href, "rule_sets", org_id),
            ObjectKind::IpList => checked_href(href, "ip_lists", org_id),
            other => Err(WriteError::invalid(format!(
                "step {} names a {} href: only rulesets and IP lists are named",
                i + 1,
                other.as_str()
            ))),
        }
    };
    request.path = substitute_str(&request.path, &href_of)?;
    if let Some(b) = request.body.as_mut() {
        substitute_value(b, &href_of)?;
    }
    let left = request.path.contains("{step:")
        || request
            .body
            .as_ref()
            .is_some_and(|b| b.to_string().contains("{step:"));
    if left {
        return Err(WriteError::invalid(format!(
            "step {} still holds a placeholder: nothing more sent",
            i + 1
        )));
    }
    debug_assert_eq!(plan_kind, kind);
    Ok(Plan::Send {
        request,
        kind: plan_kind,
    })
}

/// An IP list found by name serves the rules only when it holds exactly
/// the draft's ranges and FQDNs: a list of that name with other addresses
/// would let the rules reach them.
fn reused_list_holds(draft: &IpListDraft, href: &str, lists: &[IpList]) -> Result<(), WriteError> {
    let Some(found) = lists.iter().find(|l| l.href == href) else {
        return Ok(());
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
    /// order. Refused, with nothing sent, when the fresh read is
    /// truncated, a rule's target is ambiguous, nothing is left to write,
    /// or a ruleset to create has the name of one that does not cover its
    /// scope.
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
                steps.push(BatchStep {
                    plan,
                    kind: ObjectKind::IpList,
                    needs: Vec::new(),
                });
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
            if let Plan::Noop { existing_href, .. } = &plan {
                return Err(WriteError::invalid(format!(
                    "ruleset {} exists ({existing_href}) but its scope does not cover {}: rename it or set generator.ruleset_name; nothing planned",
                    d.name,
                    proposal.scopes[*t].text()
                )));
            }
            if proposal.ruleset_step(*t) != Some(steps.len()) {
                return Err(WriteError::invalid(
                    "the batch's ruleset steps are out of order; nothing planned",
                ));
            }
            steps.push(BatchStep {
                plan,
                kind: ObjectKind::RuleSet,
                needs: Vec::new(),
            });
        }
        for r in &proposal.rules {
            if let RuleAction::AddServices { rule_href, add } = &r.action {
                let patch = RulePatch {
                    add_services: add.clone(),
                    ..RulePatch::default()
                };
                let plan = self.plan_update_rule(rule_href, &patch).await?;
                reads += 1;
                steps.push(BatchStep {
                    plan,
                    kind: ObjectKind::Rule,
                    needs: Vec::new(),
                });
            }
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
            steps.push(BatchStep {
                plan,
                kind: ObjectKind::Rule,
                needs,
            });
        }
        Ok(Batch { steps, reads })
    }

    /// Sends the steps in order, one [`Writer::execute`] each (audit,
    /// cache invalidation, re-fetch on a 204); the first failure stops the
    /// batch and the rest are `NotSent`. Never retried, nothing rolled
    /// back: the steps are draft objects.
    pub async fn execute_batch(&self, batch: Batch) -> Vec<StepResult> {
        let org = self.pce().client().org_id();
        let kinds: Vec<ObjectKind> = batch.steps.iter().map(|s| s.kind).collect();
        let mut out: Vec<StepResult> = Vec::with_capacity(batch.steps.len());
        for (i, step) in batch.steps.into_iter().enumerate() {
            if out.iter().any(|r| matches!(r, StepResult::Failed(_))) {
                out.push(StepResult::NotSent);
                continue;
            }
            let result = match resolve_step(i, step, &out, &kinds, org) {
                Err(e) => StepResult::Failed(e),
                Ok(plan) => match self.execute(plan).await {
                    Ok(super::Outcome::Written(w)) => StepResult::Written(w),
                    Ok(super::Outcome::Skipped { existing_href, .. }) => {
                        StepResult::Skipped { existing_href }
                    }
                    Err(e) => StepResult::Failed(e),
                },
            };
            out.push(result);
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        assert_eq!(
            placeholders_in("{step:1}/sec_rules and {step:0}"),
            vec![1, 0]
        );
    }
}
