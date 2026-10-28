//! A snapshot as a baseline (#85) against a mock PCE: `workloads
//! --baseline-snapshot` (the one workloads read pinned with `.expect`, the
//! file read only; rows' `change` and `was`, `meta.compare`, `--changes`,
//! the filters on both sides, another PCE's snapshot refused with exit 2
//! and no request) and `readiness --trend` (the snapshots' readiness
//! sections, no request beyond readiness's own). Files under a temp
//! `XDG_DATA_HOME`; JSON checked against its schema.
#[path = "common/schema_check.rs"]
mod schema_check;
use assert_cmd::Command;
use illumio_tui::snapshot::{self, Sections, Snapshot, VERSION};
use pce::models::{Workload, WorkloadLabel};
use serde_json::{Value, json};
use std::path::PathBuf;
use wiremock::matchers::{method, path, query_param_is_missing};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// Temp config, cache and data homes for one test.
struct Env {
    home: tempfile::TempDir,
}

impl Env {
    fn new() -> Self {
        Self {
            home: tempfile::tempdir().unwrap(),
        }
    }

    /// The profile's snapshot directory: env-only connections are `default`.
    fn dir(&self) -> PathBuf {
        self.home.path().join("data/illumio-tui/snapshots/default")
    }

    fn cmd(&self, server: &MockServer) -> Command {
        let url = url::Url::parse(&server.uri()).unwrap();
        let mut c = Command::cargo_bin("illumio-tui").unwrap();
        c.env_clear()
            .env("PATH", std::env::var("PATH").unwrap_or_default())
            .env("PCE_SCHEME", "http")
            .env("PCE_HOST", url.host_str().unwrap())
            .env("PCE_PORT", url.port().unwrap().to_string())
            .env("PCE_ORG_ID", "5")
            .env("API_KEY", "key")
            .env("API_SECRET", "s3cr3t-compare-test")
            .env("XDG_CONFIG_HOME", self.home.path().join("config"))
            .env("XDG_CACHE_HOME", self.home.path().join("cache"))
            .env("XDG_STATE_HOME", self.home.path().join("state"))
            .env("XDG_DATA_HOME", self.home.path().join("data"));
        c
    }
}

fn answer(body: &str) -> ResponseTemplate {
    ResponseTemplate::new(200)
        .set_body_string(body.to_string())
        .insert_header("Content-Type", "application/json")
}

async fn expect(server: &MockServer, p: &str, body: &str, times: u64) {
    Mock::given(method("GET"))
        .and(path(p))
        .respond_with(answer(body))
        .expect(times)
        .mount(server)
        .await;
}

const WORKLOADS: &str = "/api/v2/orgs/5/workloads";

/// The workloads read against a snapshot: unfiltered (every filter is
/// applied locally, to both sides), answered with `body`, `times` times. A
/// read carrying a filter matches nothing here and fails the test.
async fn expect_unfiltered(server: &MockServer, body: &str, times: u64) {
    let mut m = Mock::given(method("GET")).and(path(WORKLOADS));
    for p in [
        "labels",
        "hostname",
        "name",
        "ip_address",
        "enforcement_mode",
        "managed",
        "online",
    ] {
        m = m.and(query_param_is_missing(p));
    }
    m.respond_with(answer(body))
        .expect(times)
        .mount(server)
        .await;
}
const LABELS: &str = "/api/v2/orgs/5/labels";

fn fixture_workloads() -> Vec<Workload> {
    serde_json::from_str(include_str!("../../../fixtures/workloads.json")).unwrap()
}

fn host(server: &MockServer) -> String {
    let url = url::Url::parse(&server.uri()).unwrap();
    format!("{}:{}", url.host_str().unwrap(), url.port().unwrap())
}

/// A workloads snapshot of this PCE taken an hour ago (the fixtures'
/// heartbeats are as stale then as now: the health is the same).
fn snap(server: &MockServer, workloads: Vec<Workload>) -> Snapshot {
    Snapshot {
        version: VERSION,
        profile: "default".into(),
        pce: host(server),
        org: 5,
        taken_at: jiff::Timestamp::now() - jiff::SignedDuration::from_hours(1),
        label: Some("before".into()),
        truncated: Vec::new(),
        sections: Sections {
            workloads: Some(workloads.into_iter().map(snapshot::trim_workload).collect()),
            ..Sections::default()
        },
    }
}

/// The baseline: host-1 then in app crm and visibility_only, host-4 not
/// there yet, and two workloads since gone (one in shop, one in crm).
fn baseline(server: &MockServer) -> Snapshot {
    let mut ws = fixture_workloads();
    ws.pop();
    let w = &mut ws[0];
    for l in &mut w.labels {
        if l.key.as_deref() == Some("app") {
            l.value = Some("crm".into());
            l.href = "/orgs/5/labels/30".into();
        }
    }
    w.enforcement_mode = Some("visibility_only".into());
    for (i, app) in [(1, "shop"), (2, "crm")] {
        let mut g = ws[1].clone();
        g.href = format!("/orgs/5/workloads/gone-{i}");
        g.hostname = Some(format!("gone-{i}.example"));
        g.labels = vec![WorkloadLabel {
            href: format!("/orgs/5/labels/{app}"),
            key: Some("app".into()),
            value: Some(app.into()),
        }];
        ws.push(g);
    }
    snap(server, ws)
}

fn stderr(a: &assert_cmd::assert::Assert) -> String {
    String::from_utf8_lossy(&a.get_output().stderr).to_string()
}

fn stdout(a: &assert_cmd::assert::Assert) -> String {
    String::from_utf8_lossy(&a.get_output().stdout).to_string()
}

fn by_host<'a>(rows: &'a Value, host: &str) -> &'a Value {
    rows.as_array()
        .unwrap()
        .iter()
        .find(|r| r["hostname"] == host)
        .unwrap_or_else(|| panic!("no {host} in {rows:#}"))
}

#[tokio::test]
async fn workloads_against_a_snapshot_read_the_list_once_and_the_file() {
    let server = MockServer::start().await;
    // Once per run, unfiltered: the file adds none.
    expect_unfiltered(&server, include_str!("../../../fixtures/workloads.json"), 2).await;
    let env = Env::new();
    let b = baseline(&server);
    snapshot::save(&env.dir(), &b).unwrap();
    let a = env
        .cmd(&server)
        .args([
            "--json",
            "workloads",
            "--baseline-snapshot",
            "latest",
            "--changes",
            "new,changed,gone,same",
        ])
        .assert()
        .success();
    let v = schema_check::envelope(&a.get_output().stdout);
    assert_eq!(v["meta"]["schema"], "workloads");
    assert_eq!(v["meta"]["api_calls"], 1);
    let c = &v["meta"]["compare"];
    assert_eq!(c["snapshot"], snapshot::id_for(b.taken_at));
    assert_eq!(c["label"], "before");
    assert_eq!(c["changes"], "new,changed,gone,same");
    assert_eq!(
        c["counts"],
        json!({"new": 1, "changed": 1, "gone": 2, "same": 2})
    );
    let rows = &v["data"];
    let h1 = by_host(rows, "host-1.example");
    assert_eq!(h1["change"], "changed");
    assert_eq!(h1["was"], "app: crm→shop · mode: visibility→full");
    assert_eq!(by_host(rows, "host-4.example")["change"], "new");
    assert_eq!(by_host(rows, "host-4.example")["was"], Value::Null);
    assert_eq!(by_host(rows, "host-2.example")["change"], "same");
    let gone = by_host(rows, "gone-1.example");
    assert_eq!(gone["change"], "gone");
    assert_eq!(gone["labels"][0]["value"], "shop");
    // The table's meta line names the snapshot and the counts.
    let t = env
        .cmd(&server)
        .args(["--table", "workloads", "--baseline-snapshot", "latest"])
        .assert()
        .success();
    let line = format!(
        "vs snapshot {} · 1 new · 1 changed · 2 gone",
        b.taken_at.strftime("%m-%d %H:%M")
    );
    assert!(stderr(&t).contains(&line), "{}", stderr(&t));
}

#[tokio::test]
async fn changes_default_to_the_differences_and_the_filters_apply_to_both_sides() {
    let server = MockServer::start().await;
    // `--label` too is applied locally: no label read, no labels parameter.
    expect_unfiltered(&server, include_str!("../../../fixtures/workloads.json"), 2).await;
    expect(&server, LABELS, "[]", 0).await;
    let env = Env::new();
    snapshot::save(&env.dir(), &baseline(&server)).unwrap();
    let a = env
        .cmd(&server)
        .args(["--json", "workloads", "--baseline-snapshot", "latest"])
        .assert()
        .success();
    let v = schema_check::envelope(&a.get_output().stdout);
    let changes: Vec<&str> = v["data"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["change"].as_str().unwrap())
        .collect();
    assert_eq!(changes, ["changed", "new", "gone", "gone"]);
    // `app=shop`: the gone workload in crm is not listed; host-1 (shop
    // now, crm then) is changed, not gone.
    let a = env
        .cmd(&server)
        .args([
            "--csv",
            "workloads",
            "--baseline-snapshot",
            "latest",
            "--label",
            "app=shop",
        ])
        .assert()
        .success();
    let out = stdout(&a);
    let mut lines = out.lines();
    let head = lines.next().unwrap();
    assert!(head.ends_with(",change,was,enforcement_mode"), "{head}");
    let body: Vec<&str> = lines.collect();
    assert_eq!(body.len(), 2, "{out}");
    assert!(body[0].starts_with("host-1.example,"), "{out}");
    assert!(body[0].contains(",changed,app: crm→shop · mode: visibility→full,"));
    assert!(body[1].starts_with("gone-1.example,"), "{out}");
    assert!(body[1].contains(",gone,,"), "{out}");
}

/// A workload moved from app=shop to app=crm since the snapshot is never
/// gone: with `--label app=shop` it matches neither side now (excluded, as
/// the TUI's `f app=shop` hides it); with `--label app=crm` it is changed.
#[tokio::test]
async fn a_workload_moved_out_of_the_filter_is_not_gone() {
    let server = MockServer::start().await;
    let mut live = fixture_workloads();
    for l in &mut live[0].labels {
        if l.key.as_deref() == Some("app") {
            l.value = Some("crm".into());
            l.href = "/orgs/5/labels/30".into();
        }
    }
    expect_unfiltered(&server, &serde_json::to_string(&live).unwrap(), 2).await;
    let env = Env::new();
    let b = snap(&server, fixture_workloads());
    snapshot::save(&env.dir(), &b).unwrap();
    let run = |label: &'static str| {
        env.cmd(&server)
            .args([
                "--json",
                "workloads",
                "--baseline-snapshot",
                "latest",
                "--label",
                label,
                "--changes",
                "new,changed,gone,same",
            ])
            .assert()
            .success()
    };
    let v = schema_check::envelope(&run("app=shop").get_output().stdout);
    let rows = v["data"].as_array().unwrap();
    assert!(
        rows.iter().all(|r| r["hostname"] != "host-1.example"),
        "{rows:#?}"
    );
    assert!(rows.iter().all(|r| r["change"] != "gone"), "{rows:#?}");
    let v = schema_check::envelope(&run("app=crm").get_output().stdout);
    let h1 = by_host(&v["data"], "host-1.example");
    assert_eq!(h1["change"], "changed");
    assert_eq!(h1["was"], "app: shop→crm");
}

#[tokio::test]
async fn another_pces_or_an_unknown_snapshot_is_refused_before_any_request() {
    let server = MockServer::start().await;
    expect(&server, WORKLOADS, "[]", 0).await;
    let env = Env::new();
    let mut other = baseline(&server);
    other.pce = "other.example:8443".into();
    snapshot::save(&env.dir(), &other).unwrap();
    let e = env
        .cmd(&server)
        .args(["workloads", "--baseline-snapshot", "latest"])
        .assert()
        .code(2);
    assert!(stderr(&e).contains("different PCEs"), "{}", stderr(&e));
    let e = env
        .cmd(&server)
        .args(["workloads", "--baseline-snapshot", "2025"])
        .assert()
        .code(2);
    assert!(stderr(&e).contains("no snapshot"), "{}", stderr(&e));
    let e = env
        .cmd(&server)
        .args(["workloads", "--changes", "new"])
        .assert()
        .code(2);
    assert!(stderr(&e).contains("--baseline-snapshot"), "{}", stderr(&e));
}

#[tokio::test]
async fn fields_pick_the_change_columns() {
    let server = MockServer::start().await;
    expect_unfiltered(&server, include_str!("../../../fixtures/workloads.json"), 1).await;
    let env = Env::new();
    snapshot::save(&env.dir(), &baseline(&server)).unwrap();
    let a = env
        .cmd(&server)
        .args([
            "--json",
            "--fields",
            "hostname,change,was",
            "workloads",
            "--baseline-snapshot",
            "latest",
            "--changes",
            "changed",
        ])
        .assert()
        .success();
    let v = schema_check::picked(&a.get_output().stdout);
    assert_eq!(
        v["data"],
        json!([{
            "hostname": "host-1.example",
            "change": "changed",
            "was": "app: crm→shop · mode: visibility→full"
        }])
    );
}

const QHREF: &str = "/orgs/5/traffic_flows/async_queries/q-1";

/// A completed status whose `updated_at` moves on every call, as the
/// PCE's does: a draft `completed` after the PUT is then new.
struct Ticking(std::sync::atomic::AtomicU64);

impl wiremock::Respond for Ticking {
    fn respond(&self, _: &wiremock::Request) -> ResponseTemplate {
        let n = self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        ResponseTemplate::new(200).set_body_json(json!({
            "status": "completed", "rules": "completed",
            "result": {"href": format!("{QHREF}/download")}, "flows_count": 4,
            "updated_at": format!("2026-10-14T00:00:{:02}Z", n % 60)
        }))
    }
}

/// Readiness's requests, each pinned to `n` runs: the name index's lists,
/// one query (POST, status, `update_rules`, download), the workloads, the
/// pending changes and the draft rulesets. Nothing else is mounted: a
/// request `--trend` added would fail.
async fn mount_readiness(server: &MockServer, n: u64) {
    for (p, body) in [
        (LABELS, include_str!("../../../fixtures/labels.json")),
        (
            "/api/v2/orgs/5/sec_policy/draft/label_groups",
            include_str!("../../../fixtures/label_groups.json"),
        ),
        (
            "/api/v2/orgs/5/sec_policy/draft/ip_lists",
            include_str!("../../../fixtures/ip_lists.json"),
        ),
        (
            "/api/v2/orgs/5/sec_policy/draft/services",
            include_str!("../../../fixtures/services.json"),
        ),
        (WORKLOADS, include_str!("../../../fixtures/workloads.json")),
        (
            "/api/v2/orgs/5/sec_policy/pending",
            include_str!("../../../fixtures/pending_list.json"),
        ),
        (
            "/api/v2/orgs/5/sec_policy/draft/rule_sets",
            include_str!("../../../fixtures/rule_sets.json"),
        ),
    ] {
        expect(server, p, body, n).await;
    }
    Mock::given(method("POST"))
        .and(path("/api/v2/orgs/5/traffic_flows/async_queries"))
        .respond_with(
            ResponseTemplate::new(202).set_body_json(json!({"href": QHREF, "status": "queued"})),
        )
        .expect(n)
        .mount(server)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("/api/v2{QHREF}")))
        .respond_with(Ticking(std::sync::atomic::AtomicU64::new(0)))
        .mount(server)
        .await;
    Mock::given(method("PUT"))
        .and(path(format!("/api/v2{QHREF}/update_rules")))
        .respond_with(ResponseTemplate::new(204))
        .expect(n)
        .mount(server)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("/api/v2{QHREF}/download")))
        .respond_with(answer(include_str!("../../../fixtures/traffic_flows.json")))
        .expect(n)
        .mount(server)
        .await;
}

/// A snapshot holding only readiness: `apps` as (app, verdict, gaps).
fn readiness_snap(server: &MockServer, at: &str, apps: &[(&str, &str, u64)]) -> Snapshot {
    Snapshot {
        version: VERSION,
        profile: "default".into(),
        pce: host(server),
        org: 5,
        taken_at: at.parse().unwrap(),
        label: None,
        truncated: Vec::new(),
        sections: Sections {
            readiness: Some(json!({
                "window": "7d",
                "draft": true,
                "apps": apps
                    .iter()
                    .map(|(a, v, g)| json!({"app": a, "verdict": v, "gap_count": g}))
                    .collect::<Vec<_>>()
            })),
            ..Sections::default()
        },
    }
}

#[tokio::test]
async fn readiness_trend_reads_the_snapshots_and_adds_the_live_point() {
    let server = MockServer::start().await;
    // Three runs, readiness's requests each; the files add none.
    mount_readiness(&server, 3).await;
    let env = Env::new();
    for (at, gaps) in [("2026-10-01T03:00:00Z", 9), ("2026-10-08T03:00:00Z", 4)] {
        snapshot::save(
            &env.dir(),
            &readiness_snap(&server, at, &[("shop/prod", "needs_rules", gaps)]),
        )
        .unwrap();
    }
    let mut other = readiness_snap(
        &server,
        "2026-10-09T03:00:00Z",
        &[("shop/prod", "ready", 0)],
    );
    other.pce = "other.example:8443".into();
    snapshot::save(&env.dir(), &other).unwrap();
    let a = env
        .cmd(&server)
        .args(["--json", "readiness", "--trend"])
        .assert()
        .success();
    let v = schema_check::envelope(&a.get_output().stdout);
    assert_eq!(v["meta"]["schema"], "readiness");
    let warnings = v["meta"]["warnings"].to_string();
    assert!(warnings.contains("other.example:8443"), "{warnings}");
    let apps = v["data"]["apps"].as_array().unwrap();
    let shop = apps.iter().find(|a| a["app"] == "shop/prod").unwrap();
    let t = shop["trend"].as_array().unwrap();
    assert_eq!(t.len(), 3, "{t:#?}");
    assert_eq!(t[0]["gaps"], 9);
    assert_eq!(t[0]["verdict"], "needs_rules");
    assert_eq!(t[1]["taken_at"], "2026-10-08T03:00:00Z");
    assert_eq!(t[2]["verdict"], shop["verdict"]);
    assert_eq!(t[2]["gaps"], shop["gap_count"]);
    // The table: a sparkline of the gaps, last.
    let a = env
        .cmd(&server)
        .args(["--table", "readiness", "--trend"])
        .assert()
        .success();
    let out = stdout(&a);
    let head = out.lines().find(|l| l.contains("verdict")).unwrap();
    assert!(head.trim_end().ends_with("trend"), "{out}");
    let row = out.lines().find(|l| l.starts_with("shop/prod")).unwrap();
    let live = shop["gap_count"].as_u64().unwrap() as usize;
    let spark: String = snapshot::baseline::sparkline(&[9, 4, live])
        .into_iter()
        .collect();
    assert!(row.trim_end().ends_with(&spark), "{spark} in {out}");
    // Another window than the snapshots' 7d: no live point, said.
    let a = env
        .cmd(&server)
        .args(["--json", "readiness", "--trend", "--window", "24h"])
        .assert()
        .success();
    let v = schema_check::envelope(&a.get_output().stdout);
    let warnings = v["meta"]["warnings"].to_string();
    assert!(
        warnings.contains("live point left out: window 1d differs from the snapshots' 7d"),
        "{warnings}"
    );
    let shop = v["data"]["apps"]
        .as_array()
        .unwrap()
        .iter()
        .find(|a| a["app"] == "shop/prod")
        .unwrap();
    assert_eq!(shop["trend"].as_array().unwrap().len(), 2);
}

/// A live list cut by its row cap cannot say what is gone: a workload
/// past the cap would read as gone. No gone row, `counts.gone` null, and
/// the cut named.
#[tokio::test]
async fn a_cut_list_judges_no_workload_gone() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(WORKLOADS))
        .and(query_param_is_missing("labels"))
        .respond_with(
            answer(include_str!("../../../fixtures/workloads.json"))
                .insert_header("X-Total-Count", "5000"),
        )
        .expect(2)
        .mount(&server)
        .await;
    let env = Env::new();
    snapshot::save(&env.dir(), &baseline(&server)).unwrap();
    let a = env
        .cmd(&server)
        .args(["--json", "workloads", "--baseline-snapshot", "latest"])
        .assert()
        .success();
    let v = schema_check::envelope(&a.get_output().stdout);
    let c = &v["meta"]["compare"];
    assert_eq!(c["counts"]["gone"], Value::Null);
    assert_eq!(c["list_cut_at"], 4);
    assert!(
        v["data"]
            .as_array()
            .unwrap()
            .iter()
            .all(|r| r["change"] != "gone"),
        "{v:#}"
    );
    assert!(
        v["meta"]["warnings"]
            .to_string()
            .contains("gone not judged: list cut at 4"),
        "{v:#}"
    );
    let t = env
        .cmd(&server)
        .args(["--table", "workloads", "--baseline-snapshot", "latest"])
        .assert()
        .success();
    assert!(
        stderr(&t).contains("1 new · 1 changed · gone not judged: list cut at 4"),
        "{}",
        stderr(&t)
    );
}
