use super::baseline::*;
use super::tests::fixture;
use super::*;
use pce::models::WorkloadLabel;
use pce::readiness::Verdict;
use pce::traffic::Change;
use serde_json::json;

fn me() -> Identity {
    Identity {
        profile: "demo".into(),
        pce: "pce.example".into(),
        org: 5,
    }
}

fn workloads(s: &mut Snapshot) -> &mut Vec<Workload> {
    s.sections.workloads.as_mut().unwrap()
}

#[test]
fn the_compare_names_new_gone_changed_and_same_with_their_transitions() {
    let base = fixture("2026-10-10T07:59:00Z");
    let mut now = fixture("2026-10-10T08:00:00Z");
    let ws = workloads(&mut now);
    let gone = ws.remove(ws.len() - 1);
    let w = &mut ws[0];
    w.labels.retain(|l| l.key.as_deref() != Some("app"));
    w.labels.push(WorkloadLabel {
        href: "/orgs/5/labels/9".into(),
        key: Some("app".into()),
        value: Some("cart".into()),
    });
    w.enforcement_mode = Some("selective".into());
    let mut new = ws[1].clone();
    new.href = "/orgs/5/workloads/new-1".into();
    ws.push(new);
    let (live, base_ws) = (
        now.sections.workloads.clone().unwrap(),
        base.sections.workloads.clone().unwrap(),
    );
    let c = compare_workloads(&live, now.taken_at, &base_ws, base.taken_at);
    assert_eq!(
        (c.counts.new, c.counts.changed, c.counts.gone, c.counts.same),
        (1, 1, 1, 2)
    );
    assert_eq!(c.gone.len(), 1);
    assert_eq!(c.gone[0].href, gone.href);
    let chg = c.of("/orgs/5/workloads/aaaa-1").unwrap();
    assert_eq!(chg.change, Change::Changed);
    assert_eq!(chg.was_text(), "app: shop→cart · mode: full→selective");
    assert_eq!(c.of("/orgs/5/workloads/new-1").unwrap().change, Change::New);
    assert_eq!(
        c.of("/orgs/5/workloads/aaaa-2").unwrap().change,
        Change::Same
    );
}

#[test]
fn the_ven_and_its_health_are_transitions_each_side_at_its_own_time() {
    let base = fixture("2026-10-10T08:00:00Z");
    let mut now = fixture("2026-10-10T08:00:00Z");
    // The VEN stopped and moved on a version, and its heartbeat is now a
    // day old: offline now, healthy then.
    now.taken_at = "2026-10-11T08:00:00Z".parse().unwrap();
    let w = &mut workloads(&mut now)[0];
    w.agent = Some(json!({"status": {
        "status": "stopped",
        "agent_version": "23.5.0",
        "last_heartbeat_on": "2026-10-10T07:58:00Z"
    }}));
    let base_ws = base.sections.workloads.clone().unwrap();
    let live = now.sections.workloads.clone().unwrap();
    let c = compare_workloads(&live, now.taken_at, &base_ws, base.taken_at);
    let chg = c.of("/orgs/5/workloads/aaaa-1").unwrap();
    assert_eq!(
        chg.was,
        [
            "ven: active→stopped",
            "ven 23.2.31-3113→23.5.0",
            "health: healthy→offline"
        ]
    );
}

#[test]
fn a_label_added_or_removed_reads_with_a_dash() {
    let base = fixture("2026-10-10T08:00:00Z");
    let mut now = base.clone();
    let ws = workloads(&mut now);
    ws[0].labels.retain(|l| l.key.as_deref() != Some("env"));
    ws[3].labels.push(WorkloadLabel {
        href: "/orgs/5/labels/3".into(),
        key: Some("app".into()),
        value: Some("shop".into()),
    });
    let live = now.sections.workloads.clone().unwrap();
    let c = compare_workloads(
        &live,
        now.taken_at,
        base.sections.workloads.as_deref().unwrap(),
        base.taken_at,
    );
    assert_eq!(
        c.of("/orgs/5/workloads/aaaa-1").unwrap().was,
        ["env: prod→-"]
    );
    assert_eq!(
        c.of("/orgs/5/workloads/aaaa-4").unwrap().was,
        ["app: -→shop"]
    );
}

#[test]
fn a_baseline_is_found_among_the_snapshots_with_workloads_of_this_pce() {
    let dir = tempfile::tempdir().unwrap();
    let mut a = fixture("2026-10-01T00:00:00Z");
    save(dir.path(), &a).unwrap();
    // The newest holds no workloads: `latest` is the one before.
    let mut b = fixture("2026-10-02T00:00:00Z");
    b.sections.workloads = None;
    save(dir.path(), &b).unwrap();
    let (m, s) = load_baseline(dir.path(), "latest", &me()).unwrap();
    assert_eq!(m.id, "2026-10-01T000000Z");
    assert_eq!(s.taken_at, a.taken_at);
    assert_eq!(
        load_baseline(dir.path(), "2026-10-02", &me()).unwrap_err(),
        BaselineError::Usage(
            "snapshot 2026-10-02T000000Z has no workloads section: nothing to compare".into()
        )
    );
    assert!(matches!(
        load_baseline(dir.path(), "2027", &me()),
        Err(BaselineError::Usage(_))
    ));
    // Another PCE's snapshot is refused, naming both.
    a.pce = "other.example:8443".into();
    a.taken_at = "2026-10-03T00:00:00Z".parse().unwrap();
    save(dir.path(), &a).unwrap();
    let e = load_baseline(dir.path(), "latest", &me()).unwrap_err();
    assert!(
        matches!(&e, BaselineError::Usage(t) if t.contains("different PCEs") && t.contains("other.example:8443")),
        "{e}"
    );
}

fn readiness(apps: &[(&str, &str, u64)]) -> serde_json::Value {
    json!({
        "window": "7d",
        "draft": true,
        "apps": apps
            .iter()
            .map(|(app, verdict, gaps)| json!({"app": app, "verdict": verdict, "gap_count": gaps}))
            .collect::<Vec<_>>()
    })
}

#[test]
fn the_trend_reads_the_newest_readiness_sections_per_app() {
    let dir = tempfile::tempdir().unwrap();
    // Without readiness: not read.
    save(dir.path(), &fixture("2026-09-01T00:00:00Z")).unwrap();
    for day in 1..=16u32 {
        let mut s = fixture(&format!("2026-10-{day:02}T00:00:00Z"));
        s.sections.readiness = Some(readiness(&[
            ("shop/prod", "needs_rules", u64::from(20 - day)),
            (
                "crm/prod",
                if day > 10 { "ready" } else { "needs_provision" },
                0,
            ),
        ]));
        save(dir.path(), &s).unwrap();
    }
    // Another PCE's: skipped and named.
    let mut other = fixture("2026-10-17T00:00:00Z");
    other.pce = "other.example:443".into();
    other.sections.readiness = Some(readiness(&[("shop/prod", "ready", 0)]));
    save(dir.path(), &other).unwrap();
    let t = load_trend(dir.path(), &me()).unwrap();
    // The newest 14 holding readiness: the other PCE's was one of them.
    assert_eq!(t.snapshots, TREND_MAX - 1);
    assert_eq!(t.skipped.len(), 1, "{:?}", t.skipped);
    let shop = t.of("shop/prod").unwrap();
    assert_eq!(shop.len(), 13);
    assert_eq!(shop[0].taken_at.to_string(), "2026-10-04T00:00:00Z");
    assert_eq!(shop[12].gaps, 4);
    assert_eq!(spark_text(shop).chars().count(), 13);
    assert!(spark_text(shop).starts_with('█'));
    assert!(spark_text(shop).ends_with('▃'), "{}", spark_text(shop));
    let crm = t.of("crm/prod").unwrap();
    assert_eq!(crm.last().unwrap().verdict, Verdict::Ready);
    assert_eq!(spark_text(crm), "▁".repeat(13));
    assert!(t.of("nope").is_none());
}

#[test]
fn fewer_than_two_snapshots_show_no_trend() {
    let mut s = fixture("2026-10-01T00:00:00Z");
    s.sections.readiness = Some(readiness(&[("shop/prod", "ready", 0)]));
    let t = trend_of(std::slice::from_ref(&s));
    assert_eq!(t.snapshots, 1);
    assert!(t.of("shop/prod").is_none());
    let t = trend_of(&[s.clone(), s]);
    assert_eq!(t.of("shop/prod").unwrap().len(), 2);
}

#[test]
fn the_sparkline_scales_to_the_largest() {
    assert_eq!(sparkline(&[0, 7, 14]), ['▁', '▅', '█']);
    assert_eq!(sparkline(&[0, 0]), ['▁', '▁']);
    assert!(sparkline(&[]).is_empty());
}
