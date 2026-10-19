use super::*;
use crate::marks::{Mark, MarkSet, Side};
use crate::models::{DraftDecision, Label};
use crate::resolve::ResolvedLabel;
use crate::traffic::ResolvedEndpoint;
use serde_json::json;

const DAY: Duration = Duration::from_secs(24 * 3600);

fn labels() -> Vec<Label> {
    serde_json::from_value(json!([
        {"href": "/orgs/5/labels/1", "key": "role", "value": "web"},
        {"href": "/orgs/5/labels/2", "key": "role", "value": "db"},
        {"href": "/orgs/5/labels/3", "key": "app", "value": "shop"},
        {"href": "/orgs/5/labels/4", "key": "env", "value": "prod"},
        {"href": "/orgs/5/labels/50", "key": "app", "value": "crm"},
        {"href": "/orgs/5/labels/51", "key": "app", "value": "hr"}
    ]))
    .unwrap()
}

fn index() -> NameIndex {
    NameIndex::new(&labels(), &[], &[], &[])
}

fn keys() -> Vec<String> {
    vec!["app".into(), "env".into()]
}

fn shop() -> GroupKey {
    vec!["shop".into(), "prod".into()]
}

fn crm() -> GroupKey {
    vec!["crm".into(), "prod".into()]
}

/// A labelled workload endpoint: `app` and `env` (and `role` when given).
fn wl_ep(app: &str, role: Option<&str>, host: &str) -> ResolvedEndpoint {
    let all = labels();
    let label = |k: &str, v: &str| {
        let l = all.iter().find(|l| l.key == k && l.value == v).unwrap();
        ResolvedLabel {
            href: l.href.clone(),
            key: k.into(),
            value: v.into(),
        }
    };
    let mut ls = vec![label("app", app), label("env", "prod")];
    if let Some(r) = role {
        ls.push(label("role", r));
    }
    ResolvedEndpoint {
        ip: Some(format!("10.0.0.{}", host.len())),
        hostname: Some(host.into()),
        workload_href: Some(format!("/orgs/5/workloads/{host}")),
        labels: ls,
        ..ResolvedEndpoint::default()
    }
}

fn ip_ep(ip: &str) -> ResolvedEndpoint {
    ResolvedEndpoint {
        ip: Some(ip.into()),
        ..ResolvedEndpoint::default()
    }
}

fn flow(
    src: ResolvedEndpoint,
    dst: ResolvedEndpoint,
    port: u16,
    conns: u64,
    draft: Option<&str>,
) -> ResolvedFlow {
    ResolvedFlow {
        src,
        dst,
        port: Some(port),
        proto: "tcp".into(),
        process_name: None,
        user_name: None,
        num_connections: conns,
        policy_decision: PolicyDecision::PotentiallyBlocked,
        draft_policy_decision: draft.map(DraftDecision::parse),
        flow_direction: None,
        first_detected: None,
        last_detected: None,
        rules: Vec::new(),
        bytes_in: None,
        bytes_out: None,
    }
}

fn now() -> Timestamp {
    "2026-10-19T08:00:00Z".parse().unwrap()
}

/// A workload with label hrefs; managed ones report a healthy VEN unless
/// `online` is false.
fn workload(host: &str, hrefs: &[&str], mode: &str, managed: bool, online: bool) -> Workload {
    let mut w = json!({
        "href": format!("/orgs/5/workloads/{host}"),
        "hostname": host,
        "labels": hrefs.iter().map(|h| json!({"href": h})).collect::<Vec<_>>(),
        "enforcement_mode": mode,
        "managed": managed,
        "online": online,
    });
    if managed {
        w["agent"] = json!({"status": {
            "status": "active",
            "last_heartbeat_on": "2026-10-19T07:59:00Z",
            "security_policy_sync_state": "applied"
        }});
    }
    serde_json::from_value(w).unwrap()
}

const SHOP_WEB: [&str; 3] = ["/orgs/5/labels/3", "/orgs/5/labels/4", "/orgs/5/labels/1"];
const SHOP_DB: [&str; 3] = ["/orgs/5/labels/3", "/orgs/5/labels/4", "/orgs/5/labels/2"];

fn shop_workloads(mode: &str) -> Vec<Workload> {
    vec![
        workload("web-1", &SHOP_WEB, mode, true, true),
        workload("db-1", &SHOP_DB, mode, true, true),
    ]
}

/// 20 allowed flows into shop from crm, so evidence is not little.
fn plenty() -> Vec<ResolvedFlow> {
    (0..20)
        .map(|i| {
            flow(
                wl_ep("crm", None, &format!("crm-{i}")),
                wl_ep("shop", Some("web"), "web-1"),
                443,
                1,
                Some("allowed"),
            )
        })
        .collect()
}

struct Fixture {
    flows: Vec<ResolvedFlow>,
    draft: bool,
    window: Duration,
    workloads: Vec<Workload>,
    pending: Vec<PendingChange>,
    rule_sets: Vec<RuleSet>,
    idx: NameIndex,
    keys: Vec<String>,
}

impl Fixture {
    fn new(flows: Vec<ResolvedFlow>) -> Self {
        Self {
            flows,
            draft: true,
            window: 7 * DAY,
            workloads: shop_workloads("visibility_only"),
            pending: Vec::new(),
            rule_sets: Vec::new(),
            idx: index(),
            keys: keys(),
        }
    }

    fn input<'a>(&'a self, marks: Option<&'a ActiveMarks<'a>>) -> ReadinessInput<'a> {
        ReadinessInput {
            flows: &self.flows,
            draft: self.draft,
            truncated: false,
            window: self.window,
            workloads: &self.workloads,
            pending: &self.pending,
            rule_sets: &self.rule_sets,
            marks,
            idx: &self.idx,
            scope_keys: &self.keys,
            tier_key: "role",
            now: now(),
        }
    }

    fn shop(&self) -> AppReadiness {
        app_readiness(&self.input(None), &shop())
    }
}

#[test]
fn without_draft_decisions_the_app_is_not_evaluated_but_counted() {
    let mut fx = Fixture::new(plenty());
    fx.draft = false;
    for f in &mut fx.flows {
        f.draft_policy_decision = None;
    }
    let r = fx.shop();
    assert_eq!(r.verdict, Verdict::NotEvaluated);
    assert_eq!((r.evidence.flows, r.evidence.connections), (20, 20));
    assert_eq!(r.coverage.ratio, None);
    assert_eq!(r.coverage.percent_text(), "-");
    assert_eq!(r.coverage.total.would_block, 0);
    assert!(r.gaps.is_empty());
}

#[test]
fn would_block_flows_need_rules_and_become_gaps_largest_first() {
    let mut f = plenty();
    let db = || wl_ep("shop", Some("db"), "db-1");
    // Two crm hosts on 5432: one tuple of 2 flows, 7 connections.
    f.push(flow(
        wl_ep("crm", None, "a"),
        db(),
        5432,
        3,
        Some("blocked"),
    ));
    f.push(flow(
        wl_ep("crm", None, "b"),
        db(),
        5432,
        4,
        Some("potentially_blocked"),
    ));
    // An IP on 22: 9 connections, the largest gap.
    f.push(flow(ip_ep("10.9.9.9"), db(), 22, 9, Some("blocked")));
    // Out to hr on 8080, a value of no known class: would block.
    f.push(flow(
        db(),
        wl_ep("hr", None, "hr-1"),
        8080,
        2,
        Some("weird_new_value"),
    ));
    let r = Fixture::new(f).shop();
    assert_eq!(r.verdict, Verdict::NeedsRules);
    let gaps: Vec<(Direction, String, String, u64, u64)> = r
        .gaps
        .iter()
        .map(|g| {
            (
                g.direction,
                g.peer_text(),
                g.service.clone(),
                g.flows,
                g.connections,
            )
        })
        .collect();
    assert_eq!(
        gaps,
        vec![
            (Direction::In, "10.9.9.9/-".into(), "22/tcp".into(), 1, 9),
            (Direction::In, "crm/prod".into(), "5432/tcp".into(), 2, 7),
            (Direction::Out, "hr/prod".into(), "8080/tcp".into(), 1, 2),
        ]
    );
    let c = &r.coverage;
    assert_eq!(
        (c.inbound.allowed, c.inbound.would_block, c.inbound.flows),
        (20, 16, 23)
    );
    assert_eq!((c.outbound.would_block, c.outbound.flows), (2, 1));
    assert_eq!(c.total.would_block_flows, 4);
    assert_eq!(c.ratio, Some(20.0 / 38.0));
    assert_eq!(c.percent_text(), "52.6%");
}

#[test]
fn a_flow_without_a_draft_decision_on_an_evaluated_result_would_block() {
    let mut f = plenty();
    f.push(flow(
        ip_ep("10.9.9.9"),
        wl_ep("shop", None, "web-1"),
        80,
        1,
        None,
    ));
    assert_eq!(Fixture::new(f).shop().verdict, Verdict::NeedsRules);
}

fn mark(action: Action, port: u16) -> Mark {
    Mark {
        id: format!("m-{}-{port}", action.as_str()),
        action,
        src: Side::Any,
        dst: Side::Any,
        services: vec![crate::marks::ServiceSpec::parse(&format!("{port}/tcp")).unwrap()],
        decisions: Vec::new(),
        draft_decisions: Vec::new(),
        scope: None,
        expires: None,
        comment: "test".into(),
        created: None,
        created_on: None,
    }
}

#[test]
fn accepted_marks_take_flows_out_of_the_gaps_unless_a_highlight_outranks_them() {
    let mut f = plenty();
    let web = || wl_ep("shop", Some("web"), "web-1");
    f.push(flow(ip_ep("10.9.9.1"), web(), 22, 5, Some("blocked")));
    f.push(flow(ip_ep("10.9.9.2"), web(), 23, 6, Some("blocked")));
    f.push(flow(ip_ep("10.9.9.3"), web(), 24, 7, Some("blocked")));
    let fx = Fixture::new(f);
    let mut expired = mark(Action::Squelch, 24);
    expired.expires = Some("2026-10-01T00:00:00Z".parse().unwrap());
    let set = MarkSet {
        marks: vec![
            mark(Action::Squelch, 22),
            mark(Action::AcceptBlocked, 23),
            mark(Action::Critical, 23),
            expired,
        ],
    };
    // Without marks: three gaps.
    assert_eq!(fx.shop().gaps.len(), 3);
    let active = set.active(None, now(), true);
    let r = app_readiness(&fx.input(Some(&active)), &shop());
    let ports: Vec<&str> = r.gaps.iter().map(|g| g.service.as_str()).collect();
    assert_eq!(
        ports,
        ["24/tcp", "23/tcp"],
        "22 squelched; 23 critical outranks accept-blocked; 24's mark expired"
    );
    assert_eq!(r.coverage.total.accepted, 5);
    assert_eq!(r.coverage.total.would_block, 13);
    assert_eq!(r.coverage.ratio, Some(25.0 / 38.0));
    assert_eq!(r.verdict, Verdict::NeedsRules);
    // Accepting all three leaves no gap: the app is ready.
    let all = MarkSet {
        marks: vec![
            mark(Action::Squelch, 22),
            mark(Action::AcceptBlocked, 23),
            mark(Action::AcceptBlocked, 24),
        ],
    };
    let active = all.active(None, now(), true);
    let r = app_readiness(&fx.input(Some(&active)), &shop());
    assert!(r.gaps.is_empty());
    assert_eq!(r.coverage.ratio, Some(1.0));
    assert_eq!(r.coverage.percent_text(), "100%");
    assert_eq!(r.verdict, Verdict::Ready);
}

#[test]
fn a_flow_inside_the_app_counts_once_as_inbound_and_one_between_apps_on_both() {
    let web = || wl_ep("shop", Some("web"), "web-1");
    let db = || wl_ep("shop", Some("db"), "db-1");
    let f = vec![
        flow(web(), db(), 5432, 10, Some("blocked")),
        flow(wl_ep("crm", None, "crm-1"), web(), 443, 3, Some("blocked")),
    ];
    let fx = Fixture::new(f);
    let all = readiness(&fx.input(None), &[]);
    let names: Vec<&str> = all.iter().map(|r| r.app.as_str()).collect();
    assert_eq!(names, ["crm/prod", "shop/prod"], "every app of the result");
    let s = &all[1];
    assert_eq!(
        (s.coverage.inbound.flows, s.coverage.outbound.flows),
        (2, 0)
    );
    assert_eq!(s.evidence.connections, 13);
    let inside = s.gaps.iter().find(|g| g.service == "5432/tcp").unwrap();
    assert_eq!(
        (inside.direction, inside.peer_text()),
        (Direction::In, "shop/prod".into())
    );
    let c = &all[0];
    assert_eq!(
        (c.coverage.inbound.flows, c.coverage.outbound.flows),
        (0, 1)
    );
    assert_eq!(c.gaps[0].direction, Direction::Out);
    assert_eq!(c.gaps[0].peer_text(), "shop/prod");
    assert_eq!(c.verdict, Verdict::NeedsRules);
    assert!(
        c.warnings.contains(&Warning::NoManagedWorkloads),
        "no crm workload in the list"
    );
}

#[test]
fn only_unicast_flows_count() {
    let mut f = plenty();
    let web = || wl_ep("shop", Some("web"), "web-1");
    f.push(flow(web(), ip_ep("224.0.0.251"), 5353, 4, Some("blocked")));
    f.push(flow(
        web(),
        ip_ep("255.255.255.255"),
        67,
        4,
        Some("blocked"),
    ));
    f.push(flow(web(), ip_ep("ff02::1"), 546, 4, Some("blocked")));
    let r = Fixture::new(f).shop();
    assert_eq!(r.evidence.flows, 20);
    assert_eq!(r.verdict, Verdict::Ready);
    assert!(is_unicast(&flow(web(), ip_ep("10.0.0.255"), 1, 1, None)));
}

#[test]
fn little_evidence_is_a_short_window_or_few_flows_and_only_a_warning() {
    let little = |r: &AppReadiness| {
        r.warnings
            .iter()
            .any(|w| matches!(w, Warning::LittleEvidence { .. }))
    };
    let fx = Fixture::new(plenty());
    let r = fx.shop();
    assert!(!r.evidence.little && !little(&r), "7d and 20 flows");
    assert_eq!(r.evidence.window, "7d");
    let mut short = Fixture::new(plenty());
    short.window = 6 * DAY;
    let r = short.shop();
    assert!(r.evidence.little && little(&r));
    assert_eq!(r.verdict, Verdict::Ready, "a warning does not change it");
    let mut few = plenty();
    few.pop();
    let r = Fixture::new(few).shop();
    assert!(r.evidence.little);
    assert_eq!(
        r.warnings[0].text(),
        "little evidence (7d, 19 flows)",
        "{:?}",
        r.warnings
    );
}

fn rule_sets() -> Vec<RuleSet> {
    serde_json::from_value(json!([
        {"href": "/orgs/5/sec_policy/draft/rule_sets/100", "name": "Shop-Prod", "enabled": true,
         "scopes": [[{"label": {"href": "/orgs/5/labels/3"}}, {"label": {"href": "/orgs/5/labels/4"}}]],
         "rules": [{"href": "/orgs/5/sec_policy/draft/rule_sets/100/sec_rules/1", "enabled": true,
                    "consumers": [{"ip_list": {"href": "/orgs/5/sec_policy/draft/ip_lists/21"}}],
                    "providers": [{"label": {"href": "/orgs/5/labels/1"}}],
                    "ingress_services": [{"href": "/orgs/5/sec_policy/draft/services/31"}]}]},
        {"href": "/orgs/5/sec_policy/draft/rule_sets/200", "name": "CRM", "enabled": true,
         "scopes": [[{"label": {"href": "/orgs/5/labels/50"}}]],
         "rules": [{"href": "/orgs/5/sec_policy/draft/rule_sets/200/sec_rules/9", "enabled": true,
                    "consumers": [{"ip_list": {"href": "/orgs/5/sec_policy/draft/ip_lists/22"}}],
                    "providers": [{"actors": "ams"}], "ingress_services": []}]},
        {"href": "/orgs/5/sec_policy/draft/rule_sets/300", "name": "Global", "enabled": true,
         "scopes": [[]], "rules": []}
    ]))
    .unwrap()
}

fn pending(href: &str) -> PendingChange {
    PendingChange {
        href: href.into(),
        change_type: "update".into(),
        name: None,
        object_type: crate::models::Ref::new(href).object_type().to_string(),
    }
}

#[test]
fn pending_changes_are_matched_to_the_app_by_scope_and_by_the_objects_its_rules_use() {
    let mut fx = Fixture::new(plenty());
    fx.rule_sets = rule_sets();
    fx.pending = vec![
        pending("/orgs/5/sec_policy/draft/rule_sets/100/sec_rules/1"),
        pending("/orgs/5/sec_policy/draft/rule_sets/200"),
        pending("/orgs/5/sec_policy/draft/ip_lists/21"),
        pending("/orgs/5/sec_policy/draft/ip_lists/22"),
        pending("/orgs/5/sec_policy/draft/services/31"),
        pending("/orgs/5/sec_policy/draft/rule_sets/300"),
    ];
    let r = fx.shop();
    let hrefs: Vec<&str> = r.pending.changes.iter().map(|p| p.href.as_str()).collect();
    assert_eq!(
        hrefs,
        [
            "/orgs/5/sec_policy/draft/rule_sets/100/sec_rules/1",
            "/orgs/5/sec_policy/draft/ip_lists/21",
            "/orgs/5/sec_policy/draft/services/31",
            "/orgs/5/sec_policy/draft/rule_sets/300",
        ],
        "CRM's ruleset and its IP list are not shop's"
    );
    assert_eq!(r.pending.count, 4);
    assert_eq!(r.verdict, Verdict::NeedsProvision);
    let crm = app_readiness(&fx.input(None), &crm());
    assert_eq!(
        crm.pending.count, 3,
        "its ruleset, its IP list, the global one"
    );
}

#[test]
fn ready_needs_coverage_and_nothing_pending() {
    let mut fx = Fixture::new(plenty());
    fx.rule_sets = rule_sets();
    fx.pending = vec![pending("/orgs/5/sec_policy/draft/rule_sets/200")];
    let r = fx.shop();
    assert_eq!(r.verdict, Verdict::Ready);
    assert!(r.verdict.allows_enforcement());
    assert!(r.warnings.is_empty(), "{:?}", r.warnings);
    assert_eq!(r.workloads.managed, 2);
    assert_eq!(r.workloads.enforcement.visibility, 2);
}

#[test]
fn needs_rules_comes_before_needs_provision() {
    let mut f = plenty();
    f.push(flow(
        ip_ep("10.9.9.9"),
        wl_ep("shop", None, "web-1"),
        22,
        1,
        Some("blocked"),
    ));
    let mut fx = Fixture::new(f);
    fx.rule_sets = rule_sets();
    fx.pending = vec![pending("/orgs/5/sec_policy/draft/rule_sets/100")];
    let r = fx.shop();
    assert_eq!(r.verdict, Verdict::NeedsRules);
    assert_eq!(r.pending.count, 1);
    assert!(!r.verdict.allows_enforcement());
}

#[test]
fn an_app_in_full_enforcement_is_enforced_with_its_numbers() {
    let mut f = plenty();
    f.push(flow(
        ip_ep("10.9.9.9"),
        wl_ep("shop", None, "web-1"),
        22,
        1,
        Some("blocked"),
    ));
    let mut fx = Fixture::new(f);
    fx.workloads = shop_workloads("full");
    fx.workloads
        .push(workload("nas", &SHOP_WEB, "visibility_only", false, true));
    let r = fx.shop();
    assert_eq!(
        r.verdict,
        Verdict::Enforced,
        "unmanaged ones are never moved"
    );
    assert_eq!(r.gaps.len(), 1, "the numbers stay as a check");
    fx.draft = false;
    assert_eq!(fx.shop().verdict, Verdict::Enforced);
    fx.workloads[0] = workload("web-1", &SHOP_WEB, "selective", true, true);
    assert_eq!(fx.shop().verdict, Verdict::NotEvaluated);
}

#[test]
fn workload_warnings_tier_label_offline_vens_and_unmanaged() {
    let mut fx = Fixture::new(plenty());
    fx.workloads = vec![
        workload("web-1", &SHOP_WEB, "visibility_only", true, true),
        workload(
            "nolabel-2",
            &["/orgs/5/labels/3", "/orgs/5/labels/4"],
            "idle",
            true,
            false,
        ),
        workload(
            "nolabel-1",
            &["/orgs/5/labels/3", "/orgs/5/labels/4"],
            "visibility_only",
            true,
            true,
        ),
        workload(
            "nas",
            &["/orgs/5/labels/3", "/orgs/5/labels/4"],
            "visibility_only",
            false,
            true,
        ),
        workload(
            "crm-1",
            &["/orgs/5/labels/50", "/orgs/5/labels/4"],
            "full",
            true,
            true,
        ),
    ];
    let r = fx.shop();
    let w = &r.workloads;
    assert_eq!((w.total, w.managed, w.unmanaged), (4, 3, 1));
    assert_eq!(w.missing_tier, 2, "an unmanaged one is not counted");
    assert_eq!(w.missing_tier_names, ["nolabel-1", "nolabel-2"]);
    assert_eq!(w.offline, 1);
    assert_eq!(
        (
            w.enforcement.visibility,
            w.enforcement.idle,
            w.enforcement.unmanaged
        ),
        (2, 1, 1)
    );
    assert_eq!(
        r.warnings,
        vec![
            Warning::MissingTier {
                key: "role".into(),
                workloads: 2
            },
            Warning::OfflineVens { workloads: 1 },
            Warning::Unmanaged { workloads: 1 },
        ]
    );
    assert_eq!(
        r.warnings_text(),
        "2 without role · 1 VEN offline · 1 unmanaged"
    );
    assert_eq!(r.verdict, Verdict::Ready, "warnings do not change it");
}

#[test]
fn an_app_with_a_missing_key_groups_its_workloads_by_dash() {
    let mut fx = Fixture::new(vec![flow(
        ip_ep("10.9.9.9"),
        ResolvedEndpoint {
            workload_href: Some("/orgs/5/workloads/x".into()),
            hostname: Some("x".into()),
            labels: vec![ResolvedLabel {
                href: "/orgs/5/labels/3".into(),
                key: "app".into(),
                value: "shop".into(),
            }],
            ..ResolvedEndpoint::default()
        },
        80,
        1,
        Some("allowed"),
    )]);
    fx.workloads = vec![workload(
        "x",
        &["/orgs/5/labels/3", "/orgs/5/labels/1"],
        "idle",
        true,
        true,
    )];
    let all = readiness(&fx.input(None), &[]);
    assert_eq!(all.len(), 1);
    assert_eq!(all[0].app, "shop/-");
    assert_eq!(all[0].workloads.total, 1);
    assert_eq!(
        workload_group(&fx.workloads[0], &keys(), &fx.idx),
        Some(vec!["shop".to_string(), "-".to_string()])
    );
}

#[test]
fn apps_of_lists_labelled_workload_groups_only() {
    let f = vec![
        flow(ip_ep("10.9.9.9"), wl_ep("shop", None, "web-1"), 80, 1, None),
        flow(wl_ep("crm", None, "c"), ip_ep("224.0.0.1"), 80, 1, None),
    ];
    assert_eq!(
        apps_of(&f, &keys()),
        vec![shop()],
        "crm sent multicast only, the IP is no workload"
    );
}

#[test]
fn verdict_words_and_json() {
    assert_eq!(Verdict::NeedsProvision.text(), "needs provision");
    assert_eq!(Verdict::NeedsProvision.as_str(), "needs_provision");
    let r = Fixture::new(plenty()).shop();
    let v = serde_json::to_value(&r).unwrap();
    assert_eq!(v["app"], "shop/prod");
    assert_eq!(v["verdict"], "ready");
    assert!(v.get("key").is_none());
    assert_eq!(v["coverage"]["ratio"], 1.0);
    assert_eq!(v["evidence"]["window"], "7d");
    let w = serde_json::to_value(Warning::OfflineVens { workloads: 2 }).unwrap();
    assert_eq!(w, json!({"kind": "offline_vens", "workloads": 2}));
}
