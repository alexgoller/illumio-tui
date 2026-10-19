use super::*;
use crate::models::Label;
use crate::resolve::{NameIndex, ResolvedLabel};
use crate::traffic::ResolvedEndpoint;
use crate::write::bulk::{BulkTarget, ItemResult};
use serde_json::{Value, json};

fn origin() -> RecordOrigin {
    RecordOrigin {
        profile: "default".into(),
        pce: "pce.example".into(),
        org_id: 5,
    }
}

fn workload(n: u32, mode: &str, vis: &str) -> Workload {
    serde_json::from_value(json!({
        "href": format!("/orgs/5/workloads/w{n}"),
        "hostname": format!("web-{n}"),
        "managed": true,
        "enforcement_mode": mode,
        "visibility_level": vis,
        "labels": [],
    }))
    .unwrap()
}

fn item(n: u32, mode: &str, vis: &str) -> BulkItem {
    let w = workload(n, mode, vis);
    BulkItem {
        href: w.href.clone(),
        name: w.display_name(),
        change: format!("{mode} → full"),
        body: Value::Null,
        base: BulkTarget::of(&w, &NameIndex::default()),
    }
}

fn report(results: &[(u32, ItemStatus)]) -> BulkReport {
    BulkReport {
        target: "default (pce.example, org 5)".into(),
        change: "full".into(),
        refused: None,
        reads: 0,
        reread_skipped: None,
        steps: Vec::new(),
        items: results
            .iter()
            .map(|(n, s)| ItemResult {
                href: format!("/orgs/5/workloads/w{n}"),
                name: format!("web-{n}"),
                change: String::new(),
                status: *s,
                error: None,
            })
            .collect(),
        labels_changed: false,
        labels_created: false,
        failure: None,
    }
}

fn full() -> ModeState {
    ModeState::target(EnforcementMode::Full, None)
}

fn record() -> ChangeRecord {
    let items = vec![
        (item(1, "visibility_only", "flow_summary"), full()),
        (item(2, "idle", "flow_off"), full()),
        (item(3, "visibility_only", "flow_summary"), full()),
    ];
    let mut r = ChangeRecord::of_move(
        "shop/prod",
        &origin(),
        "2026-10-19T10:00:00.789Z".parse().unwrap(),
        Some(full()),
        &items,
        &report(&[
            (1, ItemStatus::Updated),
            (2, ItemStatus::Updated),
            (3, ItemStatus::Failed),
        ]),
    );
    r.readiness = Some(RecordReadiness {
        verdict: "ready".into(),
        coverage: Some(1.0),
        window: "7d".into(),
        flows: 412,
        forced: false,
    });
    r
}

#[test]
fn a_record_has_the_specs_shape_and_round_trips() {
    let r = record();
    let v = serde_json::to_value(&r).unwrap();
    assert_eq!(v["app"], "shop/prod");
    assert_eq!(v["profile"], "default");
    assert_eq!(v["pce"], "pce.example");
    assert_eq!(v["org_id"], 5);
    assert_eq!(v["at"], "2026-10-19T10:00:00Z", "to the second");
    assert_eq!(v["to"], json!({"mode": "full", "visibility": null}));
    assert_eq!(
        v["readiness"],
        json!({"verdict": "ready", "coverage": 1.0, "window": "7d", "flows": 412})
    );
    assert!(v.get("rollback_of").is_none());
    assert_eq!(
        v["workloads"][0],
        json!({
            "href": "/orgs/5/workloads/w1", "hostname": "web-1",
            "before": {"mode": "visibility_only", "visibility": "flow_summary"},
            "after": {"mode": "full", "visibility": null},
            "result": "updated"
        })
    );
    assert_eq!(v["workloads"][2]["result"], "failed");
    let text = serde_json::to_string(&r).unwrap();
    assert!(!text.contains("secret") && !text.contains("api_key"));
    let back: ChangeRecord = serde_json::from_str(&text).unwrap();
    assert_eq!(back, r);
    assert!(r.any_moved());
}

#[test]
fn file_names_are_safe_and_sort_by_time() {
    let r = record();
    let now: Timestamp = "2026-10-19T10:00:00.789Z".parse().unwrap();
    assert_eq!(r.file_name(now), "shop_prod-20261019T100000.789Z.json");
    let mut rb = r.clone();
    rb.rollback_of = Some("x".into());
    assert_eq!(
        rb.file_name(now),
        "shop_prod-20261019T100000.789Z-rollback.json"
    );
    assert_eq!(file_safe("a b/c:d"), "a_b_c_d");
    assert_eq!(
        records_dir_from(None, Some(Path::new("/home/u"))).unwrap(),
        Path::new("/home/u/.local/state/illumio-tui/enforcements")
    );
    assert_eq!(
        records_dir_from(Some(Path::new("/x")), Some(Path::new("/home/u"))).unwrap(),
        Path::new("/x/illumio-tui/enforcements")
    );
    assert!(records_dir_from(None, None).is_none());
}

#[test]
fn records_are_written_atomically_never_over_another_and_the_newest_is_found() {
    let dir = tempfile::tempdir().unwrap();
    let d = dir.path().join("enforcements");
    assert!(newest_record(&d, "shop/prod").unwrap().is_none(), "no dir");
    let t = |s: &str| -> Timestamp { s.parse().unwrap() };
    let mut old = record();
    old.at = "2026-10-18T10:00:00Z".into();
    let p_old = write_record(&d, &old, t("2026-10-18T10:00:00.100Z")).unwrap();
    let new = record();
    let p_new = write_record(&d, &new, t("2026-10-19T10:00:00.100Z")).unwrap();
    // The same millisecond again: a new file beside it.
    let p_same = write_record(&d, &new, t("2026-10-19T10:00:00.100Z")).unwrap();
    assert_ne!(p_new, p_same);
    assert!(p_same.to_string_lossy().ends_with("-2.json"));
    // A rollback, later, and another app's record, and junk: passed over.
    let mut rb = record();
    rb.at = "2026-10-20T10:00:00Z".into();
    rb.rollback_of = Some(p_new.display().to_string());
    write_record(&d, &rb, t("2026-10-20T10:00:00.100Z")).unwrap();
    let mut other = record();
    other.app = "crm/prod".into();
    other.at = "2026-10-21T10:00:00Z".into();
    write_record(&d, &other, t("2026-10-21T10:00:00.100Z")).unwrap();
    std::fs::write(d.join("shop_prod-junk.json"), "{").unwrap();
    let names: Vec<String> = std::fs::read_dir(&d)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().to_string())
        .collect();
    assert!(names.iter().all(|n| !n.contains(".tmp")), "{names:?}");
    let (p, r) = newest_record(&d, "shop/prod").unwrap().unwrap();
    assert_eq!(r, new);
    assert!(p == p_new || p == p_same);
    assert_eq!(read_record(&p_old).unwrap(), old);
    assert!(read_record(&d.join("shop_prod-junk.json")).is_err());
}

#[test]
fn rollback_restores_moved_workloads_grouped_and_skips_changed_ones() {
    let mut r = record();
    // w4: moved from selective · flow_drops; w5: gone; w6: no mode before.
    for (n, before, result) in [
        (4, ("selective", "flow_drops"), "updated"),
        (5, ("idle", "flow_off"), "updated"),
        (6, ("", ""), "unanswered"),
        (7, ("visibility_only", "flow_summary"), "updated"),
    ] {
        r.workloads.push(RecordWorkload {
            href: format!("/orgs/5/workloads/w{n}"),
            hostname: format!("web-{n}"),
            before: ModeState {
                mode: Some(before.0.to_string()).filter(|s| !s.is_empty()),
                visibility: Some(before.1.to_string()).filter(|s| !s.is_empty()),
            },
            after: full(),
            result: result.into(),
        });
    }
    let current = vec![
        workload(1, "full", "flow_summary"),
        // Someone set w2 to selective since.
        workload(2, "selective", "flow_off"),
        workload(3, "visibility_only", "flow_summary"),
        workload(4, "full", "flow_drops"),
        workload(6, "full", "flow_summary"),
        workload(7, "full", "flow_summary"),
    ];
    let plan = plan_rollback(&r, &current);
    let groups: Vec<(EnforcementMode, Option<VisibilityLevel>, Vec<String>)> = plan
        .groups
        .iter()
        .map(|g| {
            (
                g.mode,
                g.visibility,
                g.workloads.iter().map(|w| w.href.clone()).collect(),
            )
        })
        .collect();
    assert_eq!(
        groups,
        vec![
            (
                EnforcementMode::VisibilityOnly,
                Some(VisibilityLevel::FlowSummary),
                vec!["/orgs/5/workloads/w1".into(), "/orgs/5/workloads/w7".into()]
            ),
            (
                EnforcementMode::Selective,
                Some(VisibilityLevel::FlowDrops),
                vec!["/orgs/5/workloads/w4".into()]
            ),
        ]
    );
    assert_eq!(plan.count(), 3);
    let skipped: Vec<(&str, &str)> = plan
        .skipped
        .iter()
        .map(|s| (s.hostname.as_str(), s.reason.as_str()))
        .collect();
    assert_eq!(
        skipped,
        vec![
            (
                "web-2",
                "changed since (selective · flow_off now, full set): left as it is"
            ),
            ("web-3", "not moved by that change (failed)"),
            ("web-5", "not found among the app's workloads"),
            ("web-6", "previous mode unknown (none recorded)"),
        ]
    );
}

#[test]
fn a_target_with_a_level_holds_only_with_that_level() {
    let t = ModeState::target(EnforcementMode::Full, Some(VisibilityLevel::FlowDrops));
    let now = |m: &str, v: &str| ModeState {
        mode: Some(m.into()),
        visibility: Some(v.into()),
    };
    assert!(t.holds_in(&now("full", "flow_drops")));
    assert!(!t.holds_in(&now("full", "flow_summary")));
    assert!(full().holds_in(&now("full", "flow_summary")));
    assert!(!ModeState::default().holds_in(&ModeState::default()));
}

fn ep(app: &str, host: &str) -> ResolvedEndpoint {
    let labels: Vec<Label> = serde_json::from_value(json!([
        {"href": "/orgs/5/labels/3", "key": "app", "value": app},
        {"href": "/orgs/5/labels/4", "key": "env", "value": "prod"}
    ]))
    .unwrap();
    ResolvedEndpoint {
        ip: Some("10.0.0.1".into()),
        hostname: Some(host.into()),
        workload_href: Some(format!("/orgs/5/workloads/{host}")),
        labels: labels
            .into_iter()
            .map(|l| ResolvedLabel {
                href: l.href,
                key: l.key,
                value: l.value,
            })
            .collect(),
        ..ResolvedEndpoint::default()
    }
}

fn flow(
    src: ResolvedEndpoint,
    dst: ResolvedEndpoint,
    port: u16,
    conns: u64,
    d: PolicyDecision,
) -> ResolvedFlow {
    ResolvedFlow {
        src,
        dst,
        port: Some(port),
        proto: "tcp".into(),
        process_name: None,
        user_name: None,
        num_connections: conns,
        policy_decision: d,
        draft_policy_decision: None,
        flow_direction: None,
        first_detected: None,
        last_detected: None,
        rules: Vec::new(),
        bytes_in: None,
        bytes_out: None,
    }
}

#[test]
fn watch_groups_blocked_flows_involving_the_app() {
    use PolicyDecision as D;
    let shop: GroupKey = vec!["shop".into(), "prod".into()];
    let keys = vec!["app".to_string(), "env".to_string()];
    let flows = vec![
        flow(ep("crm", "c1"), ep("shop", "s1"), 5432, 3, D::Blocked),
        flow(ep("crm", "c2"), ep("shop", "s2"), 5432, 2, D::Blocked),
        flow(ep("shop", "s1"), ep("hr", "h1"), 443, 1, D::Blocked),
        flow(ep("shop", "s1"), ep("shop", "s2"), 80, 4, D::Blocked),
        // Allowed, potentially blocked, or another app's: not reported.
        flow(ep("crm", "c1"), ep("shop", "s1"), 22, 9, D::Allowed),
        flow(
            ep("crm", "c1"),
            ep("shop", "s1"),
            23,
            9,
            D::PotentiallyBlocked,
        ),
        flow(ep("crm", "c1"), ep("hr", "h1"), 24, 9, D::Blocked),
    ];
    let b = blocked_since(&flows, &shop, &keys, None);
    assert_eq!(b.flows, 4);
    assert_eq!(b.connections, 10);
    assert_eq!(b.accepted, 0);
    let t: Vec<(&str, &str, &str, u64, u64)> = b
        .tuples
        .iter()
        .map(|g| {
            (
                g.direction.as_str(),
                g.peer_text.as_str(),
                g.service.as_str(),
                g.flows,
                g.connections,
            )
        })
        .collect();
    assert_eq!(
        t,
        vec![
            ("in", "crm/prod", "5432/tcp", 2, 5),
            ("in", "shop/prod", "80/tcp", 1, 4),
            ("out", "hr/prod", "443/tcp", 1, 1),
        ]
    );
    assert!(blocked_since(&[], &shop, &keys, None).tuples.is_empty());
}
