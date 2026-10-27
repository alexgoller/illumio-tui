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
use wiremock::matchers::{method, path};
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
    // Once per run: the file adds none.
    expect(
        &server,
        WORKLOADS,
        include_str!("../../../fixtures/workloads.json"),
        2,
    )
    .await;
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
    expect(
        &server,
        WORKLOADS,
        include_str!("../../../fixtures/workloads.json"),
        2,
    )
    .await;
    expect(
        &server,
        LABELS,
        include_str!("../../../fixtures/labels.json"),
        1,
    )
    .await;
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
    expect(
        &server,
        WORKLOADS,
        include_str!("../../../fixtures/workloads.json"),
        1,
    )
    .await;
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
