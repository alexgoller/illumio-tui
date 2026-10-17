//! One app's profile, for the app overlay and `illumio-tui app`: its peers
//! by service and decision, its workloads' enforcement and VEN health, and
//! the policy that covers it. Pure; no requests.
use crate::models::{
    Actor, ActorMatch, PolicyDecision, RuleSet, VenHealth, Workload, WorkloadFilter,
};
use crate::resolve::NameIndex;
use crate::traffic::{Direction, Focus, GroupKey, ResolvedFlow, group_by_service_refs, group_key};
use jiff::Timestamp;
use serde::Serialize;

/// One peer group, service and decision on one side of the app.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AppPeer {
    pub peer: GroupKey,
    /// `443/tcp`, or `icmp`.
    pub service: String,
    pub decision: PolicyDecision,
    pub flows: u64,
    pub connections: u64,
    pub bytes: u64,
    /// What the draft policy would change, with draft decisions loaded.
    #[serde(flatten)]
    pub draft: Option<crate::traffic::DraftCounts>,
    /// What the marks say about the row (`pce::marks::annotate_*`);
    /// flattened into JSON as `mark`, `marks` and `squelched`.
    #[serde(flatten)]
    pub marks: Option<crate::marks::RowMarks>,
}

impl AppPeer {
    /// `web/prod`.
    pub fn peer_text(&self) -> String {
        self.peer.join("/")
    }
}

/// The app's peers on `side` (`In`: the sources of the flows into it,
/// `Out`: the destinations of its flows out), one row per peer group,
/// service and decision: `group_by_service` on each decision's flows. By
/// connections, then peer, service and decision. A flow from the app to
/// itself is a peer row on both sides.
pub fn app_peers(flows: &[ResolvedFlow], focus: &Focus, side: Direction) -> Vec<AppPeer> {
    let on_side: Vec<&ResolvedFlow> = flows
        .iter()
        .filter(|f| {
            let ep = match side {
                Direction::In => &f.dst,
                Direction::Out => &f.src,
            };
            group_key(ep, &focus.keys) == focus.app
        })
        .collect();
    // One pass splits the side by decision; the groups borrow the flows.
    const DECISIONS: [PolicyDecision; 4] = [
        PolicyDecision::Blocked,
        PolicyDecision::PotentiallyBlocked,
        PolicyDecision::Allowed,
        PolicyDecision::Unknown,
    ];
    let mut parts: [Vec<&ResolvedFlow>; 4] = Default::default();
    for f in on_side {
        let i = DECISIONS
            .iter()
            .position(|d| *d == f.policy_decision)
            .unwrap_or(3);
        parts[i].push(f);
    }
    let mut out = Vec::new();
    for (d, part) in DECISIONS.into_iter().zip(parts) {
        for g in group_by_service_refs(part, &focus.keys) {
            out.push(AppPeer {
                peer: match side {
                    Direction::In => g.src,
                    Direction::Out => g.dst,
                },
                service: g.service.unwrap_or_default(),
                decision: d,
                flows: g.flows,
                connections: g.connections,
                bytes: g.bytes,
                draft: g.draft,
                marks: None,
            });
        }
    }
    out.sort_by(|a, b| {
        b.connections
            .cmp(&a.connections)
            .then_with(|| a.peer.cmp(&b.peer))
            .then_with(|| a.service.cmp(&b.service))
            .then_with(|| a.decision.as_str().cmp(b.decision.as_str()))
    });
    out
}

/// `full` → `enforced`, `visibility_only` → `visibility`; the panel's words.
/// Takes the effective enforcement ([`Workload::enforcement`]), so a
/// workload without a VEN is `unmanaged`.
pub fn enforcement_word(mode: &str) -> &'static str {
    match mode {
        "full" => "enforced",
        "selective" => "selective",
        "visibility_only" => "visibility",
        "idle" => "idle",
        "unmanaged" => "unmanaged",
        _ => "other",
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct EnforcementCounts {
    pub enforced: usize,
    pub selective: usize,
    pub visibility: usize,
    pub idle: usize,
    /// Workloads without a VEN, whatever `enforcement_mode` the PCE reports.
    pub unmanaged: usize,
    pub other: usize,
}

impl EnforcementCounts {
    pub fn count(&mut self, mode: &str) {
        match enforcement_word(mode) {
            "enforced" => self.enforced += 1,
            "selective" => self.selective += 1,
            "visibility" => self.visibility += 1,
            "idle" => self.idle += 1,
            "unmanaged" => self.unmanaged += 1,
            _ => self.other += 1,
        }
    }

    /// `enforced 9 · selective 2 · visibility 3`: the modes present, in
    /// that order; `none` without workloads.
    pub fn text(&self) -> String {
        let parts: Vec<String> = [
            ("enforced", self.enforced),
            ("selective", self.selective),
            ("visibility", self.visibility),
            ("idle", self.idle),
            ("unmanaged", self.unmanaged),
            ("other", self.other),
        ]
        .into_iter()
        .filter(|(_, n)| *n > 0)
        .map(|(w, n)| format!("{w} {n}"))
        .collect();
        if parts.is_empty() {
            "none".into()
        } else {
            parts.join(" · ")
        }
    }
}

/// One workload of the app: name, role (the tier key's label), effective
/// enforcement, the PCE's raw `enforcement_mode`, VEN health.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AppWorkload {
    pub href: String,
    pub name: String,
    pub role: Option<String>,
    /// The effective enforcement: the PCE's mode, or `unmanaged` without a VEN.
    pub enforcement: String,
    /// The PCE's `enforcement_mode` as reported (`visibility_only` for most
    /// unmanaged workloads).
    pub enforcement_mode: Option<String>,
    pub ven: VenHealth,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct WorkloadSummary {
    pub workloads: usize,
    pub enforcement: EnforcementCounts,
    /// VENs healthy or with a warning.
    pub ven_active: usize,
    /// VENs offline or in error.
    pub ven_stale: usize,
    /// Workloads without a VEN.
    pub unmanaged: usize,
}

impl WorkloadSummary {
    /// `13 active · 1 stale`, with `· 2 unmanaged` when some have no VEN.
    pub fn ven_text(&self) -> String {
        let mut s = format!("{} active · {} stale", self.ven_active, self.ven_stale);
        if self.unmanaged > 0 {
            s.push_str(&format!(" · {} unmanaged", self.unmanaged));
        }
        s
    }
}

/// The label hrefs of `Focus::label_matches`.
pub fn label_hrefs(matches: &[ActorMatch]) -> Vec<String> {
    matches
        .iter()
        .filter_map(|m| match m {
            ActorMatch::Label { label } => Some(label.href.clone()),
            _ => None,
        })
        .collect()
}

/// `GET /workloads?labels=[[...]]`: one AND group of the app's labels.
pub fn app_workload_filter(hrefs: &[String]) -> WorkloadFilter {
    WorkloadFilter {
        labels: vec![hrefs.to_vec()],
        ..WorkloadFilter::default()
    }
}

/// The workload carries every one of the app's labels.
pub fn workload_in_app(w: &Workload, hrefs: &[String]) -> bool {
    hrefs.iter().all(|h| w.labels.iter().any(|l| &l.href == h))
}

/// One row per workload (by name) and the summary: enforcement modes, VEN
/// health at `now`, the role from the `tier_key` label.
pub fn app_workloads(
    workloads: &[Workload],
    idx: &NameIndex,
    tier_key: &str,
    now: Timestamp,
) -> (Vec<AppWorkload>, WorkloadSummary) {
    let mut s = WorkloadSummary::default();
    let mut rows: Vec<AppWorkload> = workloads
        .iter()
        .map(|w| {
            let ven = w.ven_health(now);
            let enforcement = w.enforcement().unwrap_or_default().to_string();
            s.workloads += 1;
            s.enforcement.count(&enforcement);
            match ven {
                VenHealth::Healthy | VenHealth::Warning => s.ven_active += 1,
                VenHealth::Offline | VenHealth::Error => s.ven_stale += 1,
                VenHealth::Unmanaged => s.unmanaged += 1,
            }
            AppWorkload {
                href: w.href.clone(),
                name: w.display_name(),
                role: w.labels.iter().find_map(|l| {
                    idx.label(&l.href)
                        .filter(|x| x.key == tier_key)
                        .map(|x| x.value.clone())
                }),
                enforcement,
                enforcement_mode: w.enforcement_mode.clone(),
                ven,
            }
        })
        .collect();
    rows.sort_by(|a, b| a.name.cmp(&b.name));
    (rows, s)
}

/// Why a ruleset is listed: a scope covers the app, its scope is every
/// workload, or only some of its rules name the app's labels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PolicyReason {
    Scope,
    AllWorkloads,
    Rules,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AppPolicyRow {
    pub href: String,
    pub name: String,
    pub enabled: bool,
    pub reason: PolicyReason,
    /// Every rule (allow and deny) of a covering ruleset; the rules naming
    /// the app otherwise.
    pub rules: usize,
}

/// The actor is one of the app's labels, or a label group holding one
/// (its sub-groups included, recursively).
fn names_app(a: &Actor, hrefs: &[String], idx: &NameIndex) -> bool {
    if let Some(l) = &a.label {
        return hrefs.contains(&l.href);
    }
    if let Some(g) = &a.label_group {
        return crate::impact::group_members(&g.href, idx)
            .iter()
            .any(|l| hrefs.contains(l));
    }
    false
}

/// One scope (an AND of label and label-group actors) covers the app:
/// every actor names one of its labels and no exclusion does. An empty
/// scope is every workload.
pub fn scope_covers(scope: &[Actor], hrefs: &[String], idx: &NameIndex) -> bool {
    scope.iter().all(|a| {
        let n = names_app(a, hrefs, idx);
        if a.exclusion == Some(true) { !n } else { n }
    })
}

/// The rulesets for the app: those a scope covers (by name), then those
/// scoped to every workload, then those with rules whose consumers or
/// providers name its labels.
pub fn app_policy(rule_sets: &[RuleSet], hrefs: &[String], idx: &NameIndex) -> Vec<AppPolicyRow> {
    let names = |consumers: &[Actor], providers: &[Actor]| {
        consumers
            .iter()
            .chain(providers)
            .any(|a| a.exclusion != Some(true) && names_app(a, hrefs, idx))
    };
    let mut rows: Vec<AppPolicyRow> = rule_sets
        .iter()
        .filter_map(|rs| {
            let all = rs.rules.len() + rs.deny_rules.len();
            let named = rs
                .rules
                .iter()
                .filter(|r| names(&r.consumers, &r.providers))
                .count()
                + rs.deny_rules
                    .iter()
                    .filter(|r| names(&r.consumers, &r.providers))
                    .count();
            let covering: Vec<&Vec<Actor>> = rs
                .scopes
                .iter()
                .filter(|s| scope_covers(s, hrefs, idx))
                .collect();
            // A covering scope with actors (exclusions only included) is
            // a scope; only an empty one (`[[]]`, or none) is every workload.
            let reason = if covering.iter().any(|s| !s.is_empty()) {
                PolicyReason::Scope
            } else if rs.scopes.is_empty() || !covering.is_empty() {
                PolicyReason::AllWorkloads
            } else if named > 0 {
                PolicyReason::Rules
            } else {
                return None;
            };
            Some(AppPolicyRow {
                href: rs.href.clone(),
                name: rs.name.clone(),
                enabled: rs.enabled,
                reason,
                rules: if reason == PolicyReason::Rules {
                    named
                } else {
                    all
                },
            })
        })
        .collect();
    rows.sort_by(|a, b| a.reason.cmp(&b.reason).then_with(|| a.name.cmp(&b.name)));
    rows
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{Label, LabelGroup, Ref};
    use crate::traffic::resolved::tests::{flows, index};
    use crate::traffic::{FocusDirection, parse_app};

    fn shop() -> Focus {
        Focus::new(parse_app("shop/prod"), vec!["app".into(), "env".into()])
    }

    fn shop_hrefs() -> Vec<String> {
        vec!["/orgs/5/labels/3".into(), "/orgs/5/labels/4".into()]
    }

    fn rows(p: &[AppPeer]) -> Vec<(String, String, PolicyDecision, u64)> {
        p.iter()
            .map(|r| (r.peer_text(), r.service.clone(), r.decision, r.connections))
            .collect()
    }

    #[test]
    fn peers_are_one_row_per_group_service_and_decision() {
        let f = flows();
        assert_eq!(
            rows(&app_peers(&f, &shop(), Direction::In)),
            vec![
                (
                    "shop/prod".into(),
                    "5432/tcp".into(),
                    PolicyDecision::Allowed,
                    310
                ),
                (
                    "ipl:Corp/-".into(),
                    "8443/tcp".into(),
                    PolicyDecision::PotentiallyBlocked,
                    12
                ),
                (
                    "unmanaged-nas/-".into(),
                    "icmp".into(),
                    PolicyDecision::Blocked,
                    3
                ),
            ]
        );
        assert_eq!(
            rows(&app_peers(&f, &shop(), Direction::Out)),
            vec![
                (
                    "shop/prod".into(),
                    "5432/tcp".into(),
                    PolicyDecision::Allowed,
                    310
                ),
                (
                    "fqdn-1.example/-".into(),
                    "443/tcp".into(),
                    PolicyDecision::Allowed,
                    5
                ),
            ],
            "the flow to itself is a peer on both sides"
        );
        let mut split = f[0].clone();
        split.policy_decision = PolicyDecision::Blocked;
        let both = app_peers(&[f[0].clone(), split], &shop(), Direction::In);
        assert_eq!(
            both.len(),
            2,
            "same peer and service, two decisions: two rows"
        );
    }

    #[test]
    fn workloads_count_enforcement_and_ven_health() {
        let all: Vec<Workload> =
            serde_json::from_str(include_str!("../../../fixtures/workloads.json")).unwrap();
        let mine: Vec<Workload> = all
            .iter()
            .filter(|w| workload_in_app(w, &shop_hrefs()))
            .cloned()
            .collect();
        assert_eq!(mine.len(), 2, "aaaa-1 and aaaa-2 carry app=shop env=prod");
        let now: Timestamp = "2026-10-10T08:00:00Z".parse().unwrap();
        let (rows, s) = app_workloads(&mine, &index(), "role", now);
        let view: Vec<(String, Option<String>, String, VenHealth)> = rows
            .iter()
            .map(|w| (w.name.clone(), w.role.clone(), w.enforcement.clone(), w.ven))
            .collect();
        assert_eq!(
            view,
            vec![
                (
                    "host-1.example".into(),
                    Some("web".into()),
                    "full".into(),
                    VenHealth::Healthy
                ),
                (
                    "host-2.example".into(),
                    Some("db".into()),
                    "visibility_only".into(),
                    VenHealth::Offline
                ),
            ]
        );
        assert_eq!(s.workloads, 2);
        assert_eq!(s.enforcement.text(), "enforced 1 · visibility 1");
        assert_eq!(s.ven_text(), "1 active · 1 stale");
        let (_, all_s) = app_workloads(&all, &index(), "role", now);
        assert_eq!(all_s.unmanaged, 1, "aaaa-3 has no VEN");
        assert_eq!(all_s.ven_text(), "2 active · 1 stale · 1 unmanaged");
        // aaaa-3 reports `idle`; without a VEN it counts as unmanaged.
        assert_eq!(all_s.enforcement.unmanaged, 1);
        assert_eq!(all_s.enforcement.idle, 0);
        assert_eq!(
            all_s.enforcement.text(),
            "enforced 1 · selective 1 · visibility 1 · unmanaged 1"
        );
        let (all_rows, _) = app_workloads(&all, &index(), "role", now);
        let nas = all_rows.iter().find(|w| w.name == "unmanaged-nas").unwrap();
        assert_eq!(nas.enforcement, "unmanaged");
        assert_eq!(
            nas.enforcement_mode.as_deref(),
            Some("idle"),
            "raw API value"
        );
        assert_eq!(enforcement_word("unmanaged"), "unmanaged");
        assert_eq!(enforcement_word("visibility_only"), "visibility");
        assert_eq!(EnforcementCounts::default().text(), "none");
    }

    #[test]
    fn the_workload_filter_is_one_and_group_of_the_app_labels() {
        let f = app_workload_filter(&shop_hrefs());
        assert_eq!(f.labels, vec![shop_hrefs()]);
        assert!(f.to_query().contains(&(
            "labels",
            r#"[["/orgs/5/labels/3","/orgs/5/labels/4"]]"#.to_string()
        )));
        let m = shop().label_matches(&index()).unwrap();
        assert_eq!(label_hrefs(&m), shop_hrefs());
    }

    #[test]
    fn policy_lists_covering_scopes_first_then_global_ones_then_named_rules() {
        let sets: Vec<RuleSet> =
            serde_json::from_str(include_str!("../../../fixtures/rule_sets.json")).unwrap();
        let idx = index();
        let rows: Vec<(String, PolicyReason, usize)> = app_policy(&sets, &shop_hrefs(), &idx)
            .into_iter()
            .map(|r| (r.name, r.reason, r.rules))
            .collect();
        assert_eq!(
            rows,
            vec![
                ("Shop-Prod".into(), PolicyReason::Scope, 4),
                ("Global-Infra".into(), PolicyReason::AllWorkloads, 1),
            ],
            "Shop-Prod: 2 allow + 2 deny rules; Global-Infra's scope is all workloads"
        );
        let narrow: RuleSet = serde_json::from_value(serde_json::json!({
            "href": "/orgs/5/sec_policy/draft/rule_sets/200",
            "name": "Shop-Dev",
            "enabled": true,
            "scopes": [[{"label": {"href": "/orgs/5/labels/3"}}, {"label": {"href": "/orgs/5/labels/104"}}]],
            "rules": [
                {"href": "/r/1", "enabled": true, "consumers": [{"label": {"href": "/orgs/5/labels/4"}}], "providers": [{"actors": "ams"}], "ingress_services": []},
                {"href": "/r/2", "enabled": true, "consumers": [{"actors": "ams"}], "providers": [{"actors": "ams"}], "ingress_services": []}
            ]
        }))
        .unwrap();
        let rows = app_policy(&[narrow], &shop_hrefs(), &idx);
        assert_eq!(rows.len(), 1);
        assert_eq!(
            (rows[0].reason, rows[0].rules),
            (PolicyReason::Rules, 1),
            "env=dev scope: only the rule naming env=prod"
        );
        assert!(app_policy(&[], &shop_hrefs(), &idx).is_empty());
    }

    #[test]
    fn a_scope_covers_only_when_every_actor_names_the_app_and_no_exclusion_does() {
        let idx = index();
        let label = |h: &str| Actor {
            label: Some(Ref::new(h)),
            ..Actor::default()
        };
        let h = shop_hrefs();
        assert!(
            scope_covers(&[label("/orgs/5/labels/3")], &h, &idx),
            "app=shop: wider than the app"
        );
        assert!(
            !scope_covers(
                &[label("/orgs/5/labels/3"), label("/orgs/5/labels/5")],
                &h,
                &idx
            ),
            "loc=eu is not the app's"
        );
        let mut ex = label("/orgs/5/labels/4");
        ex.exclusion = Some(true);
        assert!(
            !scope_covers(&[label("/orgs/5/labels/3"), ex], &h, &idx),
            "env=prod excluded"
        );
        assert!(
            scope_covers(&[], &h, &idx),
            "an empty scope is every workload"
        );
    }

    #[test]
    fn a_label_group_names_the_app_through_its_sub_groups() {
        let labels: Vec<Label> =
            serde_json::from_str(include_str!("../../../fixtures/labels.json")).unwrap();
        // outer -> inner -> env=prod; outer and loop point at each other.
        let groups: Vec<LabelGroup> = serde_json::from_value(serde_json::json!([
            {"href": "/orgs/5/sec_policy/draft/label_groups/20", "name": "outer", "key": "env",
             "labels": [], "sub_groups": [{"href": "/orgs/5/sec_policy/active/label_groups/21"}, {"href": "/orgs/5/sec_policy/draft/label_groups/22"}]},
            {"href": "/orgs/5/sec_policy/draft/label_groups/21", "name": "inner", "key": "env",
             "labels": [{"href": "/orgs/5/labels/4"}], "sub_groups": []},
            {"href": "/orgs/5/sec_policy/draft/label_groups/22", "name": "loop", "key": "env",
             "labels": [], "sub_groups": [{"href": "/orgs/5/sec_policy/draft/label_groups/20"}]}
        ]))
        .unwrap();
        let idx = NameIndex::new(&labels, &groups, &[], &[]);
        let group = |h: &str| Actor {
            label_group: Some(Ref::new(h)),
            ..Actor::default()
        };
        let shop = Actor {
            label: Some(Ref::new("/orgs/5/labels/3")),
            ..Actor::default()
        };
        let h = shop_hrefs();
        let outer = group("/orgs/5/sec_policy/draft/label_groups/20");
        assert!(
            scope_covers(&[shop.clone(), outer.clone()], &h, &idx),
            "env=prod two groups down"
        );
        assert!(
            scope_covers(
                &[
                    shop.clone(),
                    group("/orgs/5/sec_policy/draft/label_groups/22")
                ],
                &h,
                &idx
            ),
            "a cycle ends"
        );
        let set: RuleSet = serde_json::from_value(serde_json::json!({
            "href": "/orgs/5/sec_policy/draft/rule_sets/300", "name": "Nested", "enabled": true,
            "scopes": [[{"label": {"href": "/orgs/5/labels/3"}}, {"label_group": {"href": "/orgs/5/sec_policy/draft/label_groups/20"}}]],
            "rules": []
        }))
        .unwrap();
        assert_eq!(app_policy(&[set], &h, &idx)[0].reason, PolicyReason::Scope);
    }

    #[test]
    fn a_scope_of_exclusions_only_is_a_scope_not_every_workload() {
        let idx = index();
        let set: RuleSet = serde_json::from_value(serde_json::json!({
            "href": "/orgs/5/sec_policy/draft/rule_sets/400", "name": "Not-EU", "enabled": true,
            "scopes": [[{"label": {"href": "/orgs/5/labels/5"}, "exclusion": true}]],
            "rules": [{"href": "/r/9", "enabled": true, "consumers": [{"actors": "ams"}], "providers": [{"actors": "ams"}], "ingress_services": []}]
        }))
        .unwrap();
        let rows = app_policy(&[set], &shop_hrefs(), &idx);
        assert_eq!((rows[0].reason, rows[0].rules), (PolicyReason::Scope, 1));
    }

    #[test]
    fn direction_does_not_change_the_peers_of_a_side() {
        let f = flows();
        let inbound = shop().with_direction(FocusDirection::In);
        assert_eq!(
            app_peers(&f, &inbound, Direction::In),
            app_peers(&f, &shop(), Direction::In)
        );
    }
}
