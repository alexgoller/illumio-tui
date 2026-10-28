//! The provision impact (#86): what provisioning the draft would change,
//! before it is provisioned. Three sections: the pending objects counted
//! by type and change, the rules of each pending ruleset added, removed and
//! changed with their workload coverage active → draft, and one traffic
//! query re-evaluated against the draft (which flows move from allowed to
//! blocked and back). The PCE evaluates against the whole draft, so the
//! summary always covers everything pending. `policy impact`, `policy
//! provision --impact` and the TUI's `I` / `:impact` share [`load`]; the
//! sections themselves are pure.
use crate::tui::diff::{DenySide, draft_href, inlines_deny, rule_summary};
use pce::impact::{ImpactInputs, RuleActors, rule_impact_with, rule_set_href_of, workload_labels};
use pce::models::{PendingChange, PolicyScope, RuleSet, TrafficQuery, Workload, WorkloadFilter};
use pce::traffic::{DraftPair, Transitions, draft_pairs, resolve_flows, transitions};
use pce::{CacheMode, Evaluation, Impact, NameIndex, Pce, RuleKind};
use serde::Serialize;
use serde_json::Value;
use std::collections::BTreeSet;
use tokio_util::sync::CancellationToken;

/// The pair rows the table (and the TUI) show per direction unless told
/// otherwise; `--json` carries them all.
pub const DEFAULT_PAIRS: usize = 20;

/// Pending objects of one type and change type.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ChangeCount {
    pub object_type: String,
    pub change_type: String,
    pub count: usize,
    /// `policy provision` with targets: how many of them the request holds
    /// (a rule through its ruleset); absent otherwise.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub in_request: Option<usize>,
}

/// How a rule differs between the active and the draft policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum RuleChangeKind {
    Added,
    Removed,
    Changed,
}

impl RuleChangeKind {
    pub fn as_str(self) -> &'static str {
        match self {
            RuleChangeKind::Added => "added",
            RuleChangeKind::Removed => "removed",
            RuleChangeKind::Changed => "changed",
        }
    }
}

/// One rule provisioning would add, remove or change.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RuleChange {
    pub change: RuleChangeKind,
    pub kind: RuleKind,
    /// The rule's draft href (an active one in draft form).
    pub href: String,
    /// `allow role=db ← role=web 5432/tcp`: the draft rule, or the active
    /// one when removed; ` (disabled)` when it is off.
    pub rule: String,
    /// A changed rule's active text when it differs; `null` when only its
    /// coverage moved (the ruleset's scopes or state, a label group).
    pub was: Option<String>,
    /// Workload coverage under the active policy; `null` when the rule is
    /// not there or not in force (it or its ruleset disabled).
    pub active: Option<Impact>,
    /// The same under the draft.
    pub draft: Option<Impact>,
}

/// One pending ruleset's rules, draft against active.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RulesetImpact {
    pub name: String,
    /// The draft href.
    pub href: String,
    /// The ruleset's own pending change (`create`, `update`, `delete`), or
    /// `update` when only its rules are pending.
    pub change: String,
    pub added: usize,
    pub removed: usize,
    pub changed: usize,
    /// False when a side's deny rules were not in its ruleset list (this
    /// PCE keeps them apart): only allow rules were compared.
    pub deny_compared: bool,
    pub rules: Vec<RuleChange>,
}

/// The traffic section: one query over `window`, re-evaluated against the
/// draft.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TrafficImpact {
    /// The window as given (`24h`, `7d`).
    pub window: String,
    /// Flow records loaded.
    pub flows: u64,
    /// The query reached its cap: the counts are lower bounds.
    pub truncated: bool,
    /// A PCE region did not answer.
    pub incomplete: bool,
    pub transitions: Transitions,
    /// Group pairs with flows the draft newly blocks, busiest first.
    pub blocked_pairs: Vec<DraftPair>,
    /// How many there are (`blocked_pairs` may hold fewer, `--limit`).
    pub blocked_pairs_total: usize,
    /// Group pairs with flows the draft newly allows.
    pub allowed_pairs: Vec<DraftPair>,
    pub allowed_pairs_total: usize,
}

/// What provisioning the draft would change.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ProvisionImpact {
    /// Pending changes (rules counted apart from their rulesets).
    pub pending: usize,
    pub changes: Vec<ChangeCount>,
    pub rulesets: Vec<RulesetImpact>,
    /// What the summary does not count.
    pub not_counted: Vec<String>,
    /// `null` with nothing pending: no query is sent.
    pub traffic: Option<TrafficImpact>,
}

/// What the summary never counts, whatever is pending.
pub const NOT_COUNTED: [&str; 3] = [
    "workload enforcement modes and labels: not pending, never provisioned",
    "rule coverage: virtual services, virtual servers and enforcement boundaries are not counted (the traffic section shows their effect)",
    "a partial provision: the PCE evaluates the whole draft",
];

/// The pending objects by type and change type, in that order; with
/// `requested` (the draft hrefs a provision request names) how many of
/// each it holds.
pub fn changes_section(
    pending: &[PendingChange],
    requested: Option<&BTreeSet<String>>,
) -> Vec<ChangeCount> {
    let mut out: Vec<ChangeCount> = Vec::new();
    for p in pending {
        let i = match out
            .iter()
            .position(|c| c.object_type == p.object_type && c.change_type == p.change_type)
        {
            Some(i) => i,
            None => {
                out.push(ChangeCount {
                    object_type: p.object_type.clone(),
                    change_type: p.change_type.clone(),
                    count: 0,
                    in_request: requested.map(|_| 0),
                });
                out.len() - 1
            }
        };
        out[i].count += 1;
        if let (Some(n), Some(set)) = (out[i].in_request.as_mut(), requested) {
            *n += usize::from(in_request(&p.href, set));
        }
    }
    out.sort_by(|a, b| (&a.object_type, &a.change_type).cmp(&(&b.object_type, &b.change_type)));
    out
}

/// Whether a provision request naming `set` holds the pending `href`: its
/// own draft href, or a rule's ruleset.
fn in_request(href: &str, set: &BTreeSet<String>) -> bool {
    let h = draft_href(href);
    set.contains(&h) || rule_set_href_of(&h).is_some_and(|rs| set.contains(rs))
}

/// The draft hrefs of a provision request body's `change_subset`.
pub fn request_hrefs(body: &Value) -> BTreeSet<String> {
    body.get("change_subset")
        .and_then(Value::as_object)
        .into_iter()
        .flat_map(|m| m.values())
        .filter_map(Value::as_array)
        .flatten()
        .filter_map(|o| o.get("href").and_then(Value::as_str))
        .map(draft_href)
        .collect()
}

/// What the rules section is computed over.
pub struct RuleInputs<'a> {
    pub pending: &'a [PendingChange],
    pub draft: &'a [RuleSet],
    pub active: &'a [RuleSet],
    /// Built from the draft label groups and IP lists.
    pub draft_index: &'a NameIndex,
    /// The same labels with the active label groups and IP lists.
    pub active_index: &'a NameIndex,
    pub workloads: &'a [Workload],
    /// The workload read was cut short: coverage counts are lower bounds.
    pub truncated: bool,
}

/// One rule of one side, ready to compare.
struct Entry {
    href: String,
    kind: RuleKind,
    summary: String,
    actors: RuleActors,
    /// It and its ruleset enabled.
    on: bool,
}

/// The mark of an extra-scope rule's text (`unscoped_consumers`: its
/// consumers may come from outside the ruleset's scopes, so its coverage
/// counts them anywhere).
const EXTRA_SCOPE: &str = " (extra-scope)";

/// A deny rule carrying `unscoped_consumers` (a field the model does not
/// name): marked, and said to be counted in scope, as deny coverage keeps
/// consumers to the scopes; empty otherwise.
fn deny_scope_note(d: &pce::models::DenyRule) -> &'static str {
    if d.extra.get("unscoped_consumers") == Some(&Value::Bool(true)) {
        " (extra-scope; coverage counts consumers in scope)"
    } else {
        ""
    }
}

/// The rules of `rs` (allow, then deny when `with_deny`), those pending
/// deletion left out.
fn entries(rs: &RuleSet, deny: DenySide, with_deny: bool, idx: &NameIndex) -> Vec<Entry> {
    let live = |u: &Option<String>| u.as_deref() != Some("delete");
    let mut out: Vec<Entry> = rs
        .rules
        .iter()
        .filter(|r| live(&r.update_type))
        .map(|r| Entry {
            href: draft_href(&r.href),
            kind: RuleKind::Allow,
            summary: rule_summary(
                RuleKind::Allow,
                &r.providers,
                &r.consumers,
                &r.ingress_services,
                r.enabled,
                idx,
            ) + if r.unscoped_consumers == Some(true) {
                EXTRA_SCOPE
            } else {
                ""
            },
            actors: RuleActors::of_rule(r),
            on: r.enabled && rs.enabled,
        })
        .collect();
    if let (true, DenySide::Known(deny)) = (with_deny, deny) {
        out.extend(deny.iter().filter(|d| live(&d.update_type)).map(|d| {
            let kind = if d.override_ {
                RuleKind::OverrideDeny
            } else {
                RuleKind::Deny
            };
            Entry {
                href: draft_href(&d.href),
                kind,
                summary: rule_summary(
                    kind,
                    &d.providers,
                    &d.consumers,
                    &d.ingress_services,
                    d.enabled,
                    idx,
                ) + deny_scope_note(d),
                actors: RuleActors::of_deny(d),
                on: d.enabled && rs.enabled,
            }
        }));
    }
    out
}

/// The rulesets a pending change concerns (a rule's through its ruleset),
/// in pending order, as draft hrefs.
fn pending_rulesets(pending: &[PendingChange]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for p in pending {
        if let Some(rs) = rule_set_href_of(&draft_href(&p.href))
            && !out.iter().any(|h| h == rs)
        {
            out.push(rs.to_string());
        }
    }
    out
}

/// Whether any pending change concerns a ruleset (else the rules section
/// needs no read).
pub fn any_ruleset_pending(pending: &[PendingChange]) -> bool {
    !pending_rulesets(pending).is_empty()
}

/// The ruleset of `list` whose draft href is `href`.
fn find<'a>(list: &'a [RuleSet], href: &str) -> Option<&'a RuleSet> {
    list.iter().find(|r| draft_href(&r.href) == href)
}

/// Per pending ruleset, its rules added, removed and changed, each with
/// its coverage under the active policy (active label groups and IP lists)
/// and under the draft. A ruleset pending deletion counts as absent from
/// the draft: its rules are removed. A rule is changed when its text, its
/// state or its coverage differs.
pub fn rules_section(i: &RuleInputs) -> Vec<RulesetImpact> {
    let labels = workload_labels(i.workloads);
    let side_inputs = |index| ImpactInputs {
        workloads: i.workloads,
        index,
        truncated: i.truncated,
    };
    let (a_in, d_in) = (side_inputs(i.active_index), side_inputs(i.draft_index));
    // One PCE answers both lists: when either carries deny rules inline,
    // an empty list on the other side means none there.
    let inline = inlines_deny(i.active) || inlines_deny(i.draft);
    let (a_inl, d_inl) = (inline, inline);
    let mut out = Vec::new();
    for href in pending_rulesets(i.pending) {
        let d = find(i.draft, &href).filter(|r| r.update_type.as_deref() != Some("delete"));
        let a = find(i.active, &href);
        let a_deny = a.map_or(DenySide::Known(&[]), |r| DenySide::of(r, a_inl, None));
        let d_deny = d.map_or(DenySide::Known(&[]), |r| DenySide::of(r, d_inl, None));
        let deny_compared =
            !matches!(a_deny, DenySide::Unfetched) && !matches!(d_deny, DenySide::Unfetched);
        let a_rules = a.map_or_else(Vec::new, |r| {
            entries(r, a_deny, deny_compared, i.draft_index)
        });
        let d_rules = d.map_or_else(Vec::new, |r| {
            entries(r, d_deny, deny_compared, i.draft_index)
        });
        let coverage = |rs: Option<&RuleSet>, e: &Entry, inputs: &ImpactInputs| {
            rs.filter(|_| e.on)
                .map(|rs| rule_impact_with(&rs.scopes, &e.actors, inputs, &labels))
        };
        let mut rules = Vec::new();
        for e in &d_rules {
            let draft = coverage(d, e, &d_in);
            match a_rules.iter().find(|x| x.href == e.href) {
                None => rules.push(RuleChange {
                    change: RuleChangeKind::Added,
                    kind: e.kind,
                    href: e.href.clone(),
                    rule: e.summary.clone(),
                    was: None,
                    active: None,
                    draft,
                }),
                Some(x) => {
                    let active = coverage(a, x, &a_in);
                    let counts =
                        |c: &Option<Impact>| c.as_ref().map(|c| (c.providers, c.consumers));
                    if x.summary != e.summary || counts(&active) != counts(&draft) {
                        rules.push(RuleChange {
                            change: RuleChangeKind::Changed,
                            kind: e.kind,
                            href: e.href.clone(),
                            rule: e.summary.clone(),
                            was: (x.summary != e.summary).then(|| x.summary.clone()),
                            active,
                            draft,
                        });
                    }
                }
            }
        }
        for x in a_rules
            .iter()
            .filter(|x| !d_rules.iter().any(|e| e.href == x.href))
        {
            rules.push(RuleChange {
                change: RuleChangeKind::Removed,
                kind: x.kind,
                href: x.href.clone(),
                rule: x.summary.clone(),
                was: None,
                active: coverage(a, x, &a_in),
                draft: None,
            });
        }
        let count = |k: RuleChangeKind| rules.iter().filter(|r| r.change == k).count();
        let own = i.pending.iter().find(|p| draft_href(&p.href) == href);
        out.push(RulesetImpact {
            name: d
                .or(a)
                .map(|r| r.name.clone())
                .or_else(|| own.and_then(|p| p.name.clone()))
                .or_else(|| find(i.draft, &href).map(|r| r.name.clone()))
                .unwrap_or_else(|| href.clone()),
            change: own.map_or_else(|| "update".to_string(), |p| p.change_type.clone()),
            added: count(RuleChangeKind::Added),
            removed: count(RuleChangeKind::Removed),
            changed: count(RuleChangeKind::Changed),
            deny_compared,
            href,
            rules,
        });
    }
    out
}

/// `12→40`, `-→3` (not in force on that side), `≥12` from a truncated
/// workload list; one side of a rule's coverage.
pub fn coverage_text(c: &RuleChange, side: fn(&Impact) -> usize) -> String {
    let one = |i: &Option<Impact>| match i {
        None => "-".to_string(),
        Some(i) if i.truncated => format!("≥{}", side(i)),
        Some(i) => side(i).to_string(),
    };
    format!("{}→{}", one(&c.active), one(&c.draft))
}

/// `providers 12→40 · consumers 3→3`.
pub fn coverage_line(c: &RuleChange) -> String {
    format!(
        "providers {} · consumers {}",
        coverage_text(c, |i| i.providers),
        coverage_text(c, |i| i.consumers)
    )
}

/// What a rule's coverage could not count (`--impact`'s unsupported
/// notes, either side, once each).
pub fn unsupported(c: &RuleChange) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for u in [&c.active, &c.draft]
        .into_iter()
        .flatten()
        .flat_map(|i| &i.unsupported)
    {
        if !out.contains(u) {
            out.push(u.clone());
        }
    }
    out
}

/// The traffic section of one draft-evaluated result.
pub fn traffic_section(
    flows: &[pce::traffic::ResolvedFlow],
    keys: &[String],
    window: &str,
    truncated: bool,
    incomplete: bool,
) -> TrafficImpact {
    let pairs = draft_pairs(flows, keys);
    TrafficImpact {
        window: window.to_string(),
        flows: flows.len() as u64,
        truncated,
        incomplete,
        transitions: transitions(flows),
        blocked_pairs_total: pairs.newly_blocked.len(),
        blocked_pairs: pairs.newly_blocked,
        allowed_pairs_total: pairs.newly_allowed.len(),
        allowed_pairs: pairs.newly_allowed,
    }
}

impl ProvisionImpact {
    /// Nothing pending: an empty summary, no query.
    pub fn nothing_pending() -> Self {
        Self {
            pending: 0,
            changes: Vec::new(),
            rulesets: Vec::new(),
            not_counted: Vec::new(),
            traffic: None,
        }
    }

    /// Flow records the draft would newly block (allowed now, blocked or
    /// potentially blocked under the draft): what `--fail-on-blocked`
    /// gates on.
    pub fn newly_blocked(&self) -> u64 {
        self.traffic
            .as_ref()
            .map_or(0, |t| t.transitions.newly_blocked)
    }

    /// Keeps the first `n` pairs of each direction (the totals stay).
    pub fn limit_pairs(&mut self, n: usize) {
        if let Some(t) = self.traffic.as_mut() {
            t.blocked_pairs.truncate(n);
            t.allowed_pairs.truncate(n);
        }
    }
}

/// What [`load`] needs besides the PCE.
#[derive(Debug, Clone, PartialEq)]
pub struct LoadSpec {
    /// The traffic query (window, cap), checked by the caller.
    pub query: TrafficQuery,
    /// Its window as the user gave it (`24h`), echoed in the summary.
    pub window: String,
    /// The label keys a pair is grouped by (`app,env` when empty).
    pub keys: Vec<String>,
    pub mode: CacheMode,
    /// `policy provision` with targets: the draft hrefs of the request.
    pub requested: Option<BTreeSet<String>>,
}

/// [`load`]'s answer: the summary and what its reads say about it.
#[derive(Debug, Clone)]
pub struct Loaded {
    pub report: ProvisionImpact,
    /// The pending list came from the cache.
    pub cache_hit: bool,
    /// A ruleset list or the workload list was cut short.
    pub rules_truncated: bool,
}

/// The query over the last `window` at `cap`; an error for a window that
/// does not parse or needs confirming (> 7d), before any request.
pub fn impact_query(window: &str, cap: u32) -> Result<TrafficQuery, String> {
    let w = pce::traffic::parse_window(window).map_err(|e| format!("window: {e}"))?;
    let q = pce::traffic::TrafficQueryBuilder::last(w)
        .and_then(|b| b.max_results(cap).build())
        .map_err(|e| e.to_string())?;
    match q.needs_confirmation() {
        Some(reason) => Err(format!(
            "expensive query: {reason}; policy impact --window W --yes runs it"
        )),
        None => Ok(q),
    }
}

/// The summary from the PCE: the pending list (cached); nothing more when
/// nothing is pending. Then the name index (cached), and when a ruleset is
/// pending the draft and active rulesets, the active label groups and IP
/// lists and one workloads read (all cached); then one Explorer query over
/// `spec.query`, run and re-evaluated against the draft (POST, polls, PUT
/// update_rules, polls, download) in its own slot. `spec.mode` bypasses
/// the cache for each list once (the draft label groups and IP lists come
/// with the index).
pub async fn load(pce: &Pce, spec: &LoadSpec, cancel: &CancellationToken) -> pce::Result<Loaded> {
    let mode = spec.mode;
    let pending = pce.pending_changes(mode).await?;
    let cache_hit = pending.hit;
    let pending = pending.value;
    if pending.is_empty() {
        return Ok(Loaded {
            report: ProvisionImpact::nothing_pending(),
            cache_hit,
            rules_truncated: false,
        });
    }
    let idx = pce.name_index(mode).await?;
    let mut rules_truncated = false;
    let rulesets = if any_ruleset_pending(&pending) {
        let draft = pce.rule_sets(PolicyScope::Draft, mode).await?;
        let active = pce.rule_sets(PolicyScope::Active, mode).await?;
        let groups = pce.active_label_groups(mode).await?.value;
        let lists = pce.active_ip_lists(mode).await?.value;
        let w = pce.workloads(&WorkloadFilter::default(), mode).await?;
        rules_truncated = draft.truncated || active.truncated || w.truncated;
        let active_index = idx.with_label_groups(&groups).with_ip_lists(&lists);
        rules_section(&RuleInputs {
            pending: &pending,
            draft: &draft.value,
            active: &active.value,
            draft_index: &idx,
            active_index: &active_index,
            workloads: &w.value,
            truncated: w.truncated,
        })
    } else {
        Vec::new()
    };
    let r = pce
        .traffic_eval(
            pce::IMPACT_SLOT,
            &spec.query,
            Evaluation::Draft(None),
            mode,
            cancel,
            None,
        )
        .await?;
    let flows = resolve_flows(&r.value.flows, &idx);
    let traffic = traffic_section(
        &flows,
        &spec.keys,
        &spec.window,
        r.truncated,
        r.value.incomplete,
    );
    let mut not_counted: Vec<String> = NOT_COUNTED.iter().map(|s| s.to_string()).collect();
    if pending
        .iter()
        .any(|p| matches!(p.object_type.as_str(), "label_groups" | "ip_lists"))
    {
        not_counted.push(
            "rules of rulesets with no pending change whose label groups or IP lists are pending: not listed (the traffic section shows their effect)".into(),
        );
    }
    let skipped: Vec<&str> = rulesets
        .iter()
        .filter(|r| !r.deny_compared)
        .map(|r| r.name.as_str())
        .collect();
    if !skipped.is_empty() {
        not_counted.push(format!(
            "deny rules of {}: not in the ruleset list (this PCE keeps them apart), not compared",
            skipped.join(", ")
        ));
    }
    Ok(Loaded {
        report: ProvisionImpact {
            pending: pending.len(),
            changes: changes_section(&pending, spec.requested.as_ref()),
            rulesets,
            not_counted,
            traffic: Some(traffic),
        },
        cache_hit,
        rules_truncated,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use pce::models::{IpList, Label, LabelGroup, Service};
    use serde_json::json;

    fn index() -> NameIndex {
        let labels: Vec<Label> =
            serde_json::from_str(include_str!("../../../fixtures/labels.json")).unwrap();
        let groups: Vec<LabelGroup> =
            serde_json::from_str(include_str!("../../../fixtures/label_groups.json")).unwrap();
        let lists: Vec<IpList> =
            serde_json::from_str(include_str!("../../../fixtures/ip_lists.json")).unwrap();
        let services: Vec<Service> =
            serde_json::from_str(include_str!("../../../fixtures/services.json")).unwrap();
        NameIndex::new(&labels, &groups, &lists, &services)
    }

    fn workloads() -> Vec<Workload> {
        serde_json::from_str(include_str!("../../../fixtures/workloads.json")).unwrap()
    }

    fn change(href: &str, change_type: &str) -> PendingChange {
        PendingChange {
            href: href.into(),
            change_type: change_type.into(),
            name: None,
            object_type: pce::models::Ref::new(href).object_type().to_string(),
        }
    }

    /// Rule 4 of Shop-Prod: role=web ← the label group `All prod envs`,
    /// the same text on both sides.
    fn rule4(scope: &str) -> serde_json::Value {
        json!({"href": format!("/orgs/5/sec_policy/{scope}/rule_sets/100/sec_rules/4"),
               "enabled": true, "providers": [{"label": {"href": "/orgs/5/labels/1"}}],
               "consumers": [{"label_group": {"href": format!("/orgs/5/sec_policy/{scope}/label_groups/10")}}],
               "ingress_services": [{"port": 8080, "proto": 6}]})
    }

    /// The fixture's draft, plus rule 4 and the Legacy ruleset pending
    /// deletion.
    fn draft() -> Vec<RuleSet> {
        let mut v: Vec<serde_json::Value> =
            serde_json::from_str(include_str!("../../../fixtures/rule_sets.json")).unwrap();
        v[0]["rules"].as_array_mut().unwrap().push(rule4("draft"));
        v.push(json!({"href": "/orgs/5/sec_policy/draft/rule_sets/102", "name": "Legacy",
                      "enabled": true, "scopes": [[]], "update_type": "delete",
                      "rules": [{"href": "/orgs/5/sec_policy/draft/rule_sets/102/sec_rules/9",
                                 "enabled": true, "providers": [{"label": {"href": "/orgs/5/labels/1"}}],
                                 "consumers": [{"actors": "ams"}], "ingress_services": [{"port": 22, "proto": 6}]}]}));
        serde_json::from_value(serde_json::Value::Array(v)).unwrap()
    }

    /// Shop-Prod as provisioned (rule 2 from role=db instead of Corp, no
    /// Global-Infra yet) and Legacy.
    fn active() -> Vec<RuleSet> {
        let text = include_str!("../../../fixtures/rule_sets.json")
            .replace("/sec_policy/draft/", "/sec_policy/active/");
        let mut v: Vec<serde_json::Value> = serde_json::from_str(&text).unwrap();
        v.truncate(1);
        v[0]["update_type"] = json!(null);
        v[0]["rules"][1]["consumers"] = json!([{"label": {"href": "/orgs/5/labels/2"}}]);
        v[0]["rules"].as_array_mut().unwrap().push(rule4("active"));
        v.push(json!({"href": "/orgs/5/sec_policy/active/rule_sets/102", "name": "Legacy",
                      "enabled": true, "scopes": [[]],
                      "rules": [{"href": "/orgs/5/sec_policy/active/rule_sets/102/sec_rules/9",
                                 "enabled": true, "providers": [{"label": {"href": "/orgs/5/labels/1"}}],
                                 "consumers": [{"actors": "ams"}], "ingress_services": [{"port": 22, "proto": 6}]}],
                      "deny_rules": []}));
        serde_json::from_value(serde_json::Value::Array(v)).unwrap()
    }

    fn pending() -> Vec<PendingChange> {
        vec![
            change("/orgs/5/sec_policy/draft/rule_sets/101", "create"),
            change(
                "/orgs/5/sec_policy/draft/rule_sets/100/sec_rules/2",
                "update",
            ),
            change("/orgs/5/sec_policy/draft/rule_sets/102", "delete"),
            change("/orgs/5/sec_policy/draft/label_groups/10", "update"),
            change("/orgs/5/sec_policy/draft/ip_lists/21", "create"),
        ]
    }

    fn section() -> Vec<RulesetImpact> {
        let idx = index();
        // `All prod envs` holds no label yet in the active policy.
        let empty: LabelGroup = serde_json::from_value(json!({
            "href": "/orgs/5/sec_policy/active/label_groups/10", "name": "All prod envs",
            "key": "env", "labels": [], "sub_groups": []
        }))
        .unwrap();
        let active_index = idx.with_label_groups(&[empty]);
        let (d, a, w, p) = (draft(), active(), workloads(), pending());
        rules_section(&RuleInputs {
            pending: &p,
            draft: &d,
            active: &a,
            draft_index: &idx,
            active_index: &active_index,
            workloads: &w,
            truncated: false,
        })
    }

    #[test]
    fn changes_count_by_type_and_change_and_mark_the_request() {
        let p = pending();
        let all = changes_section(&p, None);
        assert_eq!(all.len(), 5);
        assert_eq!(
            (all[0].object_type.as_str(), all[0].change_type.as_str()),
            ("ip_lists", "create")
        );
        assert!(all.iter().all(|c| c.count == 1 && c.in_request.is_none()));
        let body = json!({"change_subset": {
            "rule_sets": [{"href": "/orgs/5/sec_policy/draft/rule_sets/100"}],
            "ip_lists": [{"href": "/orgs/5/sec_policy/draft/ip_lists/21"}]
        }});
        let req = request_hrefs(&body);
        let some = changes_section(&p, Some(&req));
        let marked: Vec<(&str, usize)> = some
            .iter()
            .map(|c| (c.object_type.as_str(), c.in_request.unwrap()))
            .collect();
        assert_eq!(
            marked,
            [
                ("ip_lists", 1),
                ("label_groups", 0),
                ("rule_sets", 0),
                ("rule_sets", 0),
                ("sec_rules", 1)
            ],
            "a rule is in the request through its ruleset"
        );
    }

    #[test]
    fn rules_are_added_removed_and_changed_with_coverage_on_each_side() {
        let s = section();
        let names: Vec<(&str, &str)> = s
            .iter()
            .map(|r| (r.name.as_str(), r.change.as_str()))
            .collect();
        assert_eq!(
            names,
            [
                ("Global-Infra", "create"),
                ("Shop-Prod", "update"),
                ("Legacy", "delete")
            ]
        );
        // A new ruleset, disabled: its rule is added, in force nowhere.
        let infra = &s[0];
        assert_eq!((infra.added, infra.removed, infra.changed), (1, 0, 0));
        assert_eq!(infra.rules[0].change, RuleChangeKind::Added);
        assert_eq!(infra.rules[0].draft, None, "its ruleset is disabled");
        assert_eq!(
            coverage_line(&infra.rules[0]),
            "providers -→- · consumers -→-"
        );
        // Rule 2's consumers changed; rule 4 kept its text, but its label
        // group gained members; rule 1 and the deny rules are unchanged.
        let shop = &s[1];
        assert!(shop.deny_compared);
        assert_eq!((shop.added, shop.removed, shop.changed), (0, 0, 2));
        let r2 = &shop.rules[0];
        assert!(r2.href.ends_with("/sec_rules/2"));
        assert!(r2.was.as_deref().unwrap().contains("role=db"), "{r2:?}");
        assert!(r2.rule.contains("Corp"), "{}", r2.rule);
        assert_eq!(coverage_line(r2), "providers 1→1 · consumers 1→0");
        let r4 = &shop.rules[1];
        assert!(r4.href.ends_with("/sec_rules/4"));
        assert_eq!(r4.was, None, "the text is the same");
        assert_eq!(coverage_line(r4), "providers 1→1 · consumers 0→2");
        // A deleted ruleset: its rules are removed.
        let legacy = &s[2];
        assert_eq!((legacy.added, legacy.removed, legacy.changed), (0, 1, 0));
        assert_eq!(legacy.rules[0].change, RuleChangeKind::Removed);
        assert_eq!(legacy.rules[0].draft, None);
        assert_eq!(
            coverage_text(&legacy.rules[0], |i| i.providers),
            "1→-",
            "only host-1 is web"
        );
    }

    #[test]
    fn deny_rules_neither_list_carries_are_not_compared() {
        let idx = index();
        let (mut a, mut d) = (active(), draft());
        for rs in a.iter_mut().chain(d.iter_mut()) {
            rs.deny_rules.clear();
        }
        let (w, p) = (workloads(), pending());
        let s = rules_section(&RuleInputs {
            pending: &p,
            draft: &d,
            active: &a,
            draft_index: &idx,
            active_index: &idx,
            workloads: &w,
            truncated: true,
        });
        let shop = s.iter().find(|r| r.name == "Shop-Prod").unwrap();
        assert!(!shop.deny_compared);
        assert!(shop.rules.iter().all(|r| r.kind == RuleKind::Allow));
        assert_eq!(coverage_text(&shop.rules[0], |i| i.providers), "≥1→≥1");
    }

    /// The PCE inlines deny rules but has none active: the first draft
    /// deny rule is still compared (and added), not skipped as "kept
    /// apart".
    #[test]
    fn a_first_deny_rule_is_added_when_the_active_list_holds_none() {
        let idx = index();
        let mut a = active();
        for rs in &mut a {
            rs.deny_rules.clear();
        }
        let (d, w, p) = (draft(), workloads(), pending());
        let s = rules_section(&RuleInputs {
            pending: &p,
            draft: &d,
            active: &a,
            draft_index: &idx,
            active_index: &idx,
            workloads: &w,
            truncated: false,
        });
        let shop = s.iter().find(|r| r.name == "Shop-Prod").unwrap();
        assert!(shop.deny_compared);
        let added: Vec<RuleKind> = shop
            .rules
            .iter()
            .filter(|r| r.change == RuleChangeKind::Added)
            .map(|r| r.kind)
            .collect();
        assert_eq!(added, [RuleKind::Deny, RuleKind::OverrideDeny]);
    }

    /// Rule 2 of Shop-Prod is extra-scope (`unscoped_consumers`): its text
    /// says so, rule 1's (intra-scope) does not.
    #[test]
    fn extra_scope_rules_say_so_in_their_text() {
        let s = section();
        let shop = s.iter().find(|r| r.name == "Shop-Prod").unwrap();
        let r2 = shop.rules.iter().find(|r| r.href.ends_with("/2")).unwrap();
        assert!(r2.rule.ends_with(" (extra-scope)"), "{}", r2.rule);
        assert!(r2.was.as_deref().unwrap().ends_with(" (extra-scope)"));
        let r4 = shop.rules.iter().find(|r| r.href.ends_with("/4")).unwrap();
        assert!(!r4.rule.contains("extra-scope"), "{}", r4.rule);
        // A deny rule carrying the flag: marked, and said to be counted in
        // scope (deny coverage keeps consumers to the scopes).
        let mut d: pce::models::DenyRule = serde_json::from_value(json!({
            "href": "/orgs/5/sec_policy/draft/rule_sets/100/deny_rules/9", "enabled": true,
            "providers": [], "consumers": [], "ingress_services": [], "unscoped_consumers": true
        }))
        .unwrap();
        assert!(deny_scope_note(&d).contains("extra-scope"));
        d.extra.clear();
        assert_eq!(deny_scope_note(&d), "");
    }

    #[test]
    fn nothing_pending_has_no_section_and_no_newly_blocked() {
        let n = ProvisionImpact::nothing_pending();
        assert_eq!(n.newly_blocked(), 0);
        assert!(n.traffic.is_none());
        assert!(!any_ruleset_pending(&[]));
        assert!(!any_ruleset_pending(&[change(
            "/orgs/5/sec_policy/draft/ip_lists/21",
            "update"
        )]));
    }
}
