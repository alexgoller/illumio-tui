//! Enforcement readiness per app (#54): whether an app's workloads can be
//! moved to enforcement under the draft policy without blocking traffic
//! it was seen to need. Pure: computed from data the caller already holds
//! (the traffic result with its draft decisions, the workloads, the
//! pending changes, the draft rulesets, the active marks); no requests.
//!
//! Terms (design `2026-10-19-app-enforcement-design.md`):
//! - **App**: a group of the scope keys' label values (`shop/prod`).
//! - **App flows**: the unicast flows with the app on either side; a flow
//!   inside the app counts once, as inbound.
//! - **Would block**: an app flow whose draft decision is not of the
//!   allowed class (`potentially_blocked`, `blocked`, an unknown value, or
//!   none at all on an evaluated result).
//! - **Accepted**: a would-block flow with a live `squelch` or
//!   `accept-blocked` mark (and no stronger highlight mark): never a gap.
use crate::app::{EnforcementCounts, PolicyReason, app_policy};
use crate::impact::rule_set_href_of;
use crate::marks::{Action, ActiveMarks};
use crate::models::{
    Actor, IngressService, PendingChange, PolicyDecision, RuleSet, VenHealth, Workload,
};
use crate::resolve::NameIndex;
use crate::traffic::{
    Direction, GroupKey, ResolvedFlow, compact_duration, group_by_service_refs, group_key,
    keys_or_default,
};
use jiff::Timestamp;
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::net::IpAddr;
use std::time::Duration;

/// A window shorter than this is little evidence.
pub const LITTLE_EVIDENCE_WINDOW: Duration = Duration::from_secs(7 * 24 * 3600);
/// Fewer app flows than this is little evidence.
pub const LITTLE_EVIDENCE_FLOWS: u64 = 20;

/// Everything readiness is computed from. `flows` is the traffic result
/// (any apps; only the unicast flows count); `draft` says whether it was
/// re-evaluated against the draft policy (`update_rules`).
#[derive(Clone, Copy)]
pub struct ReadinessInput<'a> {
    pub flows: &'a [ResolvedFlow],
    /// The flows carry draft decisions: without them every app is
    /// `not_evaluated` (or `enforced`).
    pub draft: bool,
    /// The traffic result hit its cap: the evidence is partial.
    pub truncated: bool,
    /// The query window's length.
    pub window: Duration,
    /// The workloads to judge (all of them, or a superset of the apps').
    pub workloads: &'a [Workload],
    /// `sec_policy/pending`.
    pub pending: &'a [PendingChange],
    /// The draft rulesets: whose scope covers the app, what they use.
    pub rule_sets: &'a [RuleSet],
    /// The active marks; `None` without a marks file or with marks off.
    pub marks: Option<&'a ActiveMarks<'a>>,
    pub idx: &'a NameIndex,
    /// The keys an app is a group of (`app`, `env`; empty: those two).
    pub scope_keys: &'a [String],
    /// The tier key managed workloads should carry (`role`).
    pub tier_key: &'a str,
    /// For VEN health (offline heartbeats).
    pub now: Timestamp,
}

/// The judgement, first that applies: `enforced` (every managed workload
/// is in full enforcement already), `not_evaluated` (no draft decision),
/// `needs_rules` (would-block flows), `needs_provision` (pending changes
/// for the app), `ready`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    NotEvaluated,
    NeedsRules,
    NeedsProvision,
    Ready,
    Enforced,
}

impl Verdict {
    /// `needs_rules`: the JSON word.
    pub fn as_str(self) -> &'static str {
        match self {
            Verdict::NotEvaluated => "not_evaluated",
            Verdict::NeedsRules => "needs_rules",
            Verdict::NeedsProvision => "needs_provision",
            Verdict::Ready => "ready",
            Verdict::Enforced => "enforced",
        }
    }

    /// `needs rules`: the table's and the TUI's words.
    pub fn text(self) -> &'static str {
        match self {
            Verdict::NotEvaluated => "not evaluated",
            Verdict::NeedsRules => "needs rules",
            Verdict::NeedsProvision => "needs provision",
            Verdict::Ready => "ready",
            Verdict::Enforced => "enforced",
        }
    }

    /// A move to enforcement may go ahead without `--force`.
    pub fn allows_enforcement(self) -> bool {
        matches!(self, Verdict::Ready | Verdict::Enforced)
    }
}

/// The app's workloads.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct ReadinessWorkloads {
    pub total: usize,
    pub managed: usize,
    /// Without a VEN: never moved.
    pub unmanaged: usize,
    /// By effective enforcement (`unmanaged` for those without a VEN).
    pub enforcement: EnforcementCounts,
    /// Managed workloads whose VEN is offline or in error.
    pub offline: usize,
    /// Managed workloads without a label of the tier key.
    pub missing_tier: usize,
    /// Their names, sorted.
    pub missing_tier_names: Vec<String>,
}

/// How much traffic the judgement rests on.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Evidence {
    /// The query window, `7d`.
    pub window: String,
    /// App flow records (unicast; one inside the app counted once).
    pub flows: u64,
    /// Their lifetime connection counts, summed.
    pub connections: u64,
    /// The window is under 7 days or there are under 20 app flows.
    pub little: bool,
    /// The traffic result hit its cap.
    pub truncated: bool,
}

/// One side's app flows: connections allowed by the draft, would-block,
/// and accepted (would-block, but marked squelch or accept-blocked).
/// `allowed + would_block + accepted == connections` once evaluated; all
/// three are 0 when not.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct SideCounts {
    pub flows: u64,
    pub connections: u64,
    pub allowed: u64,
    pub would_block: u64,
    pub accepted: u64,
    /// Flow records that would block (not accepted).
    pub would_block_flows: u64,
}

impl SideCounts {
    fn add(&mut self, other: &SideCounts) {
        self.flows += other.flows;
        self.connections += other.connections;
        self.allowed += other.allowed;
        self.would_block += other.would_block;
        self.accepted += other.accepted;
        self.would_block_flows += other.would_block_flows;
    }
}

/// Draft coverage of the app's connections, in and out.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize)]
pub struct Coverage {
    pub inbound: SideCounts,
    pub outbound: SideCounts,
    pub total: SideCounts,
    /// `(allowed + accepted) / connections`, 0..=1; `None` when not
    /// evaluated or without connections.
    pub ratio: Option<f64>,
}

impl Coverage {
    /// `97.6%`, `100%`, or `-`.
    pub fn percent_text(&self) -> String {
        match self.ratio {
            None => "-".into(),
            Some(r) if r >= 1.0 => "100%".into(),
            Some(r) => format!("{:.1}%", (r * 1000.0).floor() / 10.0),
        }
    }
}

/// One policy tuple the draft does not allow: the peer group on the other
/// side and the service, the app's would-block flows on it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Gap {
    /// `in`: the peer is the source; `out`: the destination.
    pub direction: Direction,
    pub peer: GroupKey,
    /// `443/tcp`, or `icmp`.
    pub service: String,
    pub flows: u64,
    pub connections: u64,
}

impl Gap {
    /// `web/prod`.
    pub fn peer_text(&self) -> String {
        self.peer.join("/")
    }
}

/// The pending changes that hold the app back: in rulesets whose scope
/// covers it, and the objects those rulesets use.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct PendingForApp {
    pub count: usize,
    pub changes: Vec<PendingChange>,
}

/// What does not change the verdict but should be read before a move.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Warning {
    /// The window is under 7 days or the app has under 20 flows.
    LittleEvidence { window: String, flows: u64 },
    /// The traffic result hit its cap: flows may be missing.
    Truncated,
    /// Managed workloads without a tier label.
    MissingTier { key: String, workloads: usize },
    /// Managed workloads whose VEN is offline or in error.
    OfflineVens { workloads: usize },
    /// Workloads without a VEN: a move leaves them as they are.
    Unmanaged { workloads: usize },
    /// No managed workload in the list: nothing to move.
    NoManagedWorkloads,
}

impl Warning {
    /// `little evidence (24h, 4 flows)`.
    pub fn text(&self) -> String {
        match self {
            Warning::LittleEvidence { window, flows } => {
                format!("little evidence ({window}, {flows} flows)")
            }
            Warning::Truncated => "traffic truncated".into(),
            Warning::MissingTier { key, workloads } => format!("{workloads} without {key}"),
            Warning::OfflineVens { workloads } => format!("{workloads} VEN offline"),
            Warning::Unmanaged { workloads } => format!("{workloads} unmanaged"),
            Warning::NoManagedWorkloads => "no managed workloads".into(),
        }
    }
}

/// One app's readiness.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct AppReadiness {
    /// `shop/prod`.
    pub app: String,
    /// The group values, in scope-key order.
    #[serde(skip)]
    pub key: GroupKey,
    pub verdict: Verdict,
    pub workloads: ReadinessWorkloads,
    pub evidence: Evidence,
    pub coverage: Coverage,
    /// Largest first (connections, then flows).
    pub gaps: Vec<Gap>,
    pub pending: PendingForApp,
    pub warnings: Vec<Warning>,
}

impl AppReadiness {
    /// `little evidence (24h, 4 flows) · 1 unmanaged`, or empty.
    pub fn warnings_text(&self) -> String {
        self.warnings
            .iter()
            .map(Warning::text)
            .collect::<Vec<_>>()
            .join(" · ")
    }
}

/// Every app in the traffic result: the labelled workload groups on
/// either side of a unicast flow, sorted.
pub fn apps_of(flows: &[ResolvedFlow], scope_keys: &[String]) -> Vec<GroupKey> {
    let keys = keys_or_default(scope_keys);
    let mut apps = BTreeSet::new();
    for f in flows.iter().filter(|f| is_unicast(f)) {
        for ep in [&f.src, &f.dst] {
            if ep.is_workload() && !ep.labels.is_empty() {
                apps.insert(group_key(ep, &keys));
            }
        }
    }
    apps.into_iter().collect()
}

/// Readiness of `apps`, in that order; every app of the traffic result
/// (`apps_of`) when `apps` is empty. One pass over the flows and one over
/// the workloads, whatever the number of apps.
pub fn readiness(input: &ReadinessInput, apps: &[GroupKey]) -> Vec<AppReadiness> {
    let keys = keys_or_default(input.scope_keys);
    let apps: Vec<GroupKey> = if apps.is_empty() {
        apps_of(input.flows, &keys)
    } else {
        apps.to_vec()
    };
    let slot: HashMap<&GroupKey, usize> = apps.iter().enumerate().map(|(i, a)| (a, i)).collect();
    let mut sides: Vec<[Vec<&ResolvedFlow>; 2]> = vec![Default::default(); apps.len()];
    for f in input.flows.iter().filter(|f| is_unicast(f)) {
        let dst = group_key(&f.dst, &keys);
        let src = group_key(&f.src, &keys);
        if let Some(&i) = slot.get(&dst) {
            sides[i][0].push(f);
        }
        // Inside the app: counted once, as inbound.
        if src != dst
            && let Some(&i) = slot.get(&src)
        {
            sides[i][1].push(f);
        }
    }
    let mut members: Vec<Vec<&Workload>> = vec![Vec::new(); apps.len()];
    for w in input.workloads {
        if w.deleted == Some(true) {
            continue;
        }
        if let Some(k) = workload_group(w, &keys, input.idx)
            && let Some(&i) = slot.get(&k)
        {
            members[i].push(w);
        }
    }
    apps.iter()
        .zip(sides)
        .zip(members)
        .map(|((app, [inbound, outbound]), wls)| {
            judge(input, &keys, app, &inbound, &outbound, &wls)
        })
        .collect()
}

/// One app's readiness (`readiness` for a single app).
pub fn app_readiness(input: &ReadinessInput, app: &[String]) -> AppReadiness {
    readiness(input, &[app.to_vec()])
        .pop()
        .expect("one app in, one out")
}

fn judge(
    input: &ReadinessInput,
    keys: &[String],
    app: &GroupKey,
    inbound: &[&ResolvedFlow],
    outbound: &[&ResolvedFlow],
    wls: &[&Workload],
) -> AppReadiness {
    let workloads = workload_counts(wls, input.idx, input.tier_key, input.now);
    let (cin, gaps_in) = side(input, inbound);
    let (cout, gaps_out) = side(input, outbound);
    let mut total = SideCounts::default();
    total.add(&cin);
    total.add(&cout);
    let ratio = (input.draft && total.connections > 0)
        .then(|| (total.allowed + total.accepted) as f64 / total.connections as f64);
    let coverage = Coverage {
        inbound: cin,
        outbound: cout,
        total,
        ratio,
    };
    let window = compact_duration(input.window);
    let evidence = Evidence {
        window: window.clone(),
        flows: total.flows,
        connections: total.connections,
        little: input.window < LITTLE_EVIDENCE_WINDOW || total.flows < LITTLE_EVIDENCE_FLOWS,
        truncated: input.truncated,
    };
    let mut gaps = Vec::new();
    if input.draft {
        gaps.extend(tuples(&gaps_in, keys, Direction::In));
        gaps.extend(tuples(&gaps_out, keys, Direction::Out));
        gaps.sort_by(|a, b| {
            b.connections
                .cmp(&a.connections)
                .then_with(|| b.flows.cmp(&a.flows))
                .then_with(|| a.direction.cmp(&b.direction))
                .then_with(|| a.peer.cmp(&b.peer))
                .then_with(|| a.service.cmp(&b.service))
        });
    }
    let pending = pending_for(input, keys, app);
    let enforced = workloads.managed > 0 && workloads.enforcement.enforced == workloads.managed;
    let verdict = if enforced {
        Verdict::Enforced
    } else if !input.draft {
        Verdict::NotEvaluated
    } else if total.would_block_flows > 0 {
        Verdict::NeedsRules
    } else if pending.count > 0 {
        Verdict::NeedsProvision
    } else {
        Verdict::Ready
    };
    let mut warnings = Vec::new();
    if evidence.little {
        warnings.push(Warning::LittleEvidence {
            window,
            flows: total.flows,
        });
    }
    if input.truncated {
        warnings.push(Warning::Truncated);
    }
    if workloads.managed == 0 {
        warnings.push(Warning::NoManagedWorkloads);
    }
    if workloads.missing_tier > 0 {
        warnings.push(Warning::MissingTier {
            key: input.tier_key.to_string(),
            workloads: workloads.missing_tier,
        });
    }
    if workloads.offline > 0 {
        warnings.push(Warning::OfflineVens {
            workloads: workloads.offline,
        });
    }
    if workloads.unmanaged > 0 {
        warnings.push(Warning::Unmanaged {
            workloads: workloads.unmanaged,
        });
    }
    AppReadiness {
        app: app.join("/"),
        key: app.clone(),
        verdict,
        workloads,
        evidence,
        coverage,
        gaps,
        pending,
        warnings,
    }
}

/// One side's counts, and its would-block flows that are not accepted.
fn side<'a>(
    input: &ReadinessInput,
    flows: &[&'a ResolvedFlow],
) -> (SideCounts, Vec<&'a ResolvedFlow>) {
    let mut c = SideCounts::default();
    let mut gaps = Vec::new();
    for &f in flows {
        c.flows += 1;
        c.connections += f.num_connections;
        if !input.draft {
            continue;
        }
        if draft_allows(f) {
            c.allowed += f.num_connections;
        } else if accepted(input.marks, f) {
            c.accepted += f.num_connections;
        } else {
            c.would_block += f.num_connections;
            c.would_block_flows += 1;
            gaps.push(f);
        }
    }
    (c, gaps)
}

/// The draft decision is of the allowed class. No draft decision on an
/// evaluated result is not an allow.
pub fn draft_allows(f: &ResolvedFlow) -> bool {
    f.draft_policy_decision
        .as_ref()
        .is_some_and(|d| d.class() == PolicyDecision::Allowed)
}

/// A live `squelch` or `accept-blocked` mark matches the flow, and no
/// highlight (`critical`, `important`, `revisit`), which outranks them.
pub fn accepted(marks: Option<&ActiveMarks>, f: &ResolvedFlow) -> bool {
    let Some(m) = marks else {
        return false;
    };
    let hits = m.matching(f);
    let actions = || hits.iter().map(|&i| m.mark(i).action);
    !actions().any(Action::is_highlight)
        && actions().any(|a| matches!(a, Action::Squelch | Action::AcceptBlocked))
}

/// The would-block flows as tuples: peer group and service.
fn tuples(flows: &[&ResolvedFlow], keys: &[String], dir: Direction) -> Vec<Gap> {
    group_by_service_refs(flows.iter().copied(), keys)
        .into_iter()
        .map(|g| Gap {
            direction: dir,
            peer: match dir {
                Direction::In => g.src,
                Direction::Out => g.dst,
            },
            service: g.service.unwrap_or_default(),
            flows: g.flows,
            connections: g.connections,
        })
        .collect()
}

/// Unicast: Explorer marks broadcast and multicast flows, but the resolved
/// flow does not keep the mark, so the destination address decides: a
/// multicast address or the limited broadcast is not unicast. (The CLI
/// also asks Explorer for unicast only.)
pub fn is_unicast(f: &ResolvedFlow) -> bool {
    match f.dst.ip.as_deref().and_then(|s| s.parse::<IpAddr>().ok()) {
        Some(IpAddr::V4(v4)) => !(v4.is_multicast() || v4.is_broadcast()),
        Some(IpAddr::V6(v6)) => !v6.is_multicast(),
        None => true,
    }
}

/// A label's key and value: the workload's own copy, else the index's.
fn label_kv<'a>(
    l: &'a crate::models::WorkloadLabel,
    idx: &'a NameIndex,
) -> Option<(&'a str, &'a str)> {
    match (l.key.as_deref(), l.value.as_deref()) {
        (Some(k), Some(v)) => Some((k, v)),
        _ => idx
            .label(&l.href)
            .map(|x| (x.key.as_str(), x.value.as_str())),
    }
}

/// The workload's app under `keys` (`-` for a key it has no label of), as
/// a flow endpoint groups; `None` for a workload without labels.
pub fn workload_group(w: &Workload, keys: &[String], idx: &NameIndex) -> Option<GroupKey> {
    if w.labels.is_empty() {
        return None;
    }
    let kv: Vec<(&str, &str)> = w.labels.iter().filter_map(|l| label_kv(l, idx)).collect();
    Some(
        keys.iter()
            .map(|k| {
                kv.iter()
                    .find(|(key, _)| key == k)
                    .map_or("-", |(_, v)| v)
                    .to_string()
            })
            .collect(),
    )
}

fn workload_counts(
    wls: &[&Workload],
    idx: &NameIndex,
    tier_key: &str,
    now: Timestamp,
) -> ReadinessWorkloads {
    let mut r = ReadinessWorkloads::default();
    for w in wls {
        r.total += 1;
        r.enforcement.count(w.enforcement().unwrap_or_default());
        if w.is_unmanaged() {
            r.unmanaged += 1;
            continue;
        }
        r.managed += 1;
        if matches!(w.ven_health(now), VenHealth::Offline | VenHealth::Error) {
            r.offline += 1;
        }
        let has_tier = w
            .labels
            .iter()
            .any(|l| label_kv(l, idx).is_some_and(|(k, _)| k == tier_key));
        if !has_tier {
            r.missing_tier += 1;
            r.missing_tier_names.push(w.display_name());
        }
    }
    r.missing_tier_names.sort();
    r
}

/// `/sec_policy/active/…` and `/sec_policy/draft/…` name one object.
fn draft_href(href: &str) -> String {
    href.replacen("/sec_policy/active/", "/sec_policy/draft/", 1)
}

/// The objects an actor list uses: label groups, IP lists, virtual
/// services.
fn actor_refs(actors: &[Actor], out: &mut HashSet<String>) {
    for a in actors {
        for r in [&a.label_group, &a.ip_list, &a.virtual_service]
            .into_iter()
            .flatten()
        {
            out.insert(draft_href(&r.href));
        }
    }
}

/// Pending changes for the app: a covering ruleset itself (or one of its
/// rules), or an object (service, IP list, label group) a covering
/// ruleset's scopes or rules use. A ruleset covers the app when one of its
/// scopes does or it applies to every workload (`app::app_policy`).
fn pending_for(input: &ReadinessInput, keys: &[String], app: &[String]) -> PendingForApp {
    if input.pending.is_empty() {
        return PendingForApp::default();
    }
    let hrefs: Vec<String> = keys
        .iter()
        .zip(app)
        .filter(|(_, v)| v.as_str() != "-")
        .filter_map(|(k, v)| input.idx.label_href(k, v).map(str::to_string))
        .collect();
    let covering: BTreeMap<String, &RuleSet> = app_policy(input.rule_sets, &hrefs, input.idx)
        .into_iter()
        .filter(|r| matches!(r.reason, PolicyReason::Scope | PolicyReason::AllWorkloads))
        .filter_map(|r| {
            input
                .rule_sets
                .iter()
                .find(|rs| rs.href == r.href)
                .map(|rs| (draft_href(&rs.href), rs))
        })
        .collect();
    let mut used = HashSet::new();
    for rs in covering.values() {
        for s in &rs.scopes {
            actor_refs(s, &mut used);
        }
        for r in &rs.rules {
            actor_refs(&r.consumers, &mut used);
            actor_refs(&r.providers, &mut used);
            for s in &r.ingress_services {
                if let IngressService::Ref(x) = s {
                    used.insert(draft_href(&x.href));
                }
            }
        }
        for r in &rs.deny_rules {
            actor_refs(&r.consumers, &mut used);
            actor_refs(&r.providers, &mut used);
            for s in &r.ingress_services {
                if let IngressService::Ref(x) = s {
                    used.insert(draft_href(&x.href));
                }
            }
        }
    }
    let changes: Vec<PendingChange> = input
        .pending
        .iter()
        .filter(|p| {
            let h = draft_href(&p.href);
            let in_set = rule_set_href_of(&h).unwrap_or(&h);
            covering.contains_key(in_set) || used.contains(&h)
        })
        .cloned()
        .collect();
    PendingForApp {
        count: changes.len(),
        changes,
    }
}

#[cfg(test)]
mod tests;
