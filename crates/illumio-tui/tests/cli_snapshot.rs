//! `snapshot save|list|diff|rm|prune` (#84) against a mock PCE: every read
//! pinned with `.expect` (save's eight lists, `--what`, a live `now` side
//! reading only what the file side has, `--with-readiness`'s query), the
//! files under a temp `XDG_DATA_HOME`, the JSON checked against its schema,
//! the refusal of another PCE's snapshot (exit 2), `--fail-on-change`
//! (exit 9), and corrupt or newer files.
#[path = "common/schema_check.rs"]
mod schema_check;
use assert_cmd::Command;
use illumio_tui::snapshot::{self, PolicySection, Sections, Snapshot, VERSION};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use wiremock::matchers::{method, path, query_param};
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

    fn data(&self) -> PathBuf {
        self.home.path().join("data")
    }

    /// The profile's snapshot directory: env-only connections are `default`.
    fn dir(&self) -> PathBuf {
        self.data().join("illumio-tui/snapshots/default")
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
            .env("API_SECRET", "s3cr3t-snapshot-test")
            .env("XDG_CONFIG_HOME", self.home.path().join("config"))
            .env("XDG_CACHE_HOME", self.home.path().join("cache"))
            .env("XDG_STATE_HOME", self.home.path().join("state"))
            .env("XDG_DATA_HOME", self.data());
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

const LABELS: &str = "/api/v2/orgs/5/labels";
const GROUPS: &str = "/api/v2/orgs/5/sec_policy/draft/label_groups";
const IP_LISTS: &str = "/api/v2/orgs/5/sec_policy/draft/ip_lists";
const SERVICES: &str = "/api/v2/orgs/5/sec_policy/draft/services";
const DRAFT_RS: &str = "/api/v2/orgs/5/sec_policy/draft/rule_sets";
const ACTIVE_RS: &str = "/api/v2/orgs/5/sec_policy/active/rule_sets";
const BOUNDARIES: &str = "/api/v2/orgs/5/sec_policy/draft/enforcement_boundaries";
const WORKLOADS: &str = "/api/v2/orgs/5/workloads";

/// The eight reads of a full save: policy `p` times, labels `l`,
/// workloads `w`.
async fn mount(server: &MockServer, p: u64, l: u64, w: u64) {
    expect(
        server,
        LABELS,
        include_str!("../../../fixtures/labels.json"),
        l,
    )
    .await;
    for (path_, body) in [
        (GROUPS, include_str!("../../../fixtures/label_groups.json")),
        (IP_LISTS, include_str!("../../../fixtures/ip_lists.json")),
        (SERVICES, include_str!("../../../fixtures/services.json")),
        (DRAFT_RS, include_str!("../../../fixtures/rule_sets.json")),
        (ACTIVE_RS, "[]"),
        (
            BOUNDARIES,
            include_str!("../../../fixtures/enforcement_boundaries.json"),
        ),
    ] {
        expect(server, path_, body, p).await;
    }
    expect(
        server,
        WORKLOADS,
        include_str!("../../../fixtures/workloads.json"),
        w,
    )
    .await;
}

fn parse<T: serde::de::DeserializeOwned>(text: &str) -> T {
    serde_json::from_str(text).unwrap()
}

/// A snapshot of the fixtures as the CLI's env connection would take it.
fn fixture(server: &MockServer, at: &str) -> Snapshot {
    let url = url::Url::parse(&server.uri()).unwrap();
    let host = format!("{}:{}", url.host_str().unwrap(), url.port().unwrap());
    let workloads: Vec<pce::models::Workload> =
        parse(include_str!("../../../fixtures/workloads.json"));
    Snapshot {
        version: VERSION,
        profile: "default".into(),
        pce: host,
        org: 5,
        taken_at: at.parse().unwrap(),
        label: None,
        truncated: Vec::new(),
        sections: Sections {
            policy: Some(PolicySection {
                draft_rule_sets: parse(include_str!("../../../fixtures/rule_sets.json")),
                active_rule_sets: Vec::new(),
                label_groups: parse(include_str!("../../../fixtures/label_groups.json")),
                ip_lists: parse(include_str!("../../../fixtures/ip_lists.json")),
                services: parse(include_str!("../../../fixtures/services.json")),
                enforcement_boundaries: parse(include_str!(
                    "../../../fixtures/enforcement_boundaries.json"
                )),
            }),
            labels: Some(parse(include_str!("../../../fixtures/labels.json"))),
            workloads: Some(workloads.into_iter().map(snapshot::trim_workload).collect()),
            readiness: None,
        },
    }
}

/// An older and a newer snapshot: one workload relabelled and moved to
/// full enforcement, one IP list range added.
fn two_files(env: &Env, server: &MockServer) -> (Snapshot, Snapshot) {
    let a = fixture(server, "2026-10-01T03:00:00Z");
    let mut b = fixture(server, "2026-10-08T03:00:00Z");
    b.label = Some("after".into());
    let w = &mut b.sections.workloads.as_mut().unwrap()[1];
    w.enforcement_mode = Some("full".into());
    w.labels.retain(|l| l.key.as_deref() != Some("env"));
    let p = b.sections.policy.as_mut().unwrap();
    p.ip_lists[1]
        .ip_ranges
        .push(parse(r#"{"from_ip": "10.9.0.0/16"}"#));
    snapshot::save(&env.dir(), &a).unwrap();
    snapshot::save(&env.dir(), &b).unwrap();
    (a, b)
}

fn stdout(a: &assert_cmd::assert::Assert) -> String {
    String::from_utf8_lossy(&a.get_output().stdout).to_string()
}

fn stderr(a: &assert_cmd::assert::Assert) -> String {
    String::from_utf8_lossy(&a.get_output().stderr).to_string()
}

fn files(dir: &Path) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(dir)
        .map(|r| r.map(|e| e.unwrap().path()).collect())
        .unwrap_or_default();
    v.sort();
    v
}

#[tokio::test]
async fn save_reads_each_list_once_and_writes_one_file() {
    let server = MockServer::start().await;
    mount(&server, 1, 1, 1).await;
    let env = Env::new();
    let a = env
        .cmd(&server)
        .args(["--json", "snapshot", "save", "--label", "before Q4"])
        .assert()
        .success();
    let v = schema_check::envelope(&a.get_output().stdout);
    assert_eq!(v["meta"]["schema"], "snapshot-save");
    assert_eq!(v["meta"]["api_calls"], 8);
    let d = &v["data"];
    let written = files(&env.dir());
    assert_eq!(written.len(), 1);
    assert_eq!(d["path"], written[0].display().to_string());
    assert_eq!(d["label"], "before Q4");
    assert_eq!(d["profile"], "default");
    assert_eq!(d["org"], 5);
    assert_eq!(d["bytes"], std::fs::metadata(&written[0]).unwrap().len());
    let sections: Vec<&str> = d["sections"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["section"].as_str().unwrap())
        .collect();
    assert_eq!(sections, ["policy", "labels", "workloads"]);
    assert_eq!(d["sections"][2]["items"], 4);
    let s = snapshot::load(&written[0]).unwrap();
    assert_eq!(
        s.sections
            .policy
            .unwrap()
            .enforcement_boundaries
            .unwrap()
            .len(),
        1
    );
    assert!(s.sections.readiness.is_none());
    // The file holds no credentials.
    let text = std::fs::read_to_string(&written[0]).unwrap();
    assert!(!text.contains("s3cr3t"));
}

#[tokio::test]
async fn save_what_reads_only_those_sections() {
    let server = MockServer::start().await;
    mount(&server, 0, 1, 1).await;
    let env = Env::new();
    let a = env
        .cmd(&server)
        .args(["--table", "snapshot", "save", "--what", "labels,workloads"])
        .assert()
        .success();
    let out = stdout(&a);
    assert!(out.contains("snapshot 20"), "{out}");
    assert!(out.contains("labels") && !out.contains("policy"), "{out}");
    let metas = snapshot::list(&env.dir()).unwrap();
    assert_eq!(metas[0].sections, ["labels", "workloads"]);
}

#[tokio::test]
async fn list_rm_and_prune_read_only_the_files() {
    let server = MockServer::start().await;
    mount(&server, 0, 0, 0).await;
    let env = Env::new();
    two_files(&env, &server);
    snapshot::save(&env.dir(), &fixture(&server, "2026-10-15T03:00:00Z")).unwrap();
    let a = env
        .cmd(&server)
        .args(["--json", "snapshot", "list"])
        .assert()
        .success();
    let v = schema_check::envelope(&a.get_output().stdout);
    assert_eq!(v["meta"]["schema"], "snapshot-list");
    assert_eq!(v["meta"]["api_calls"], 0);
    let ids: Vec<&str> = v["data"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["id"].as_str().unwrap())
        .collect();
    assert_eq!(
        ids,
        [
            "2026-10-01T030000Z",
            "2026-10-08T030000Z",
            "2026-10-15T030000Z"
        ]
    );
    assert_eq!(v["data"][1]["label"], "after");
    let csv = env
        .cmd(&server)
        .args(["--csv", "snapshot", "list"])
        .assert()
        .success();
    assert!(stdout(&csv).starts_with("id,taken_at,label,sections,bytes,problem,path\r\n"));

    let rm = env
        .cmd(&server)
        .args(["--json", "snapshot", "rm", "2026-10-08"])
        .assert()
        .success();
    let v = schema_check::envelope(&rm.get_output().stdout);
    assert_eq!(v["data"]["removed"][0]["id"], "2026-10-08T030000Z");
    assert_eq!(v["data"]["kept"], 2);
    let pr = env
        .cmd(&server)
        .args(["--json", "snapshot", "prune", "--keep", "1"])
        .assert()
        .success();
    let v = schema_check::envelope(&pr.get_output().stdout);
    assert_eq!(v["meta"]["schema"], "snapshot-rm");
    assert_eq!(v["data"]["removed"][0]["id"], "2026-10-01T030000Z");
    assert_eq!(v["data"]["kept"], 1);
    assert_eq!(files(&env.dir()).len(), 1);
    // An unknown one: exit 2, nothing removed.
    let e = env
        .cmd(&server)
        .args(["snapshot", "rm", "2025"])
        .assert()
        .code(2);
    assert!(stderr(&e).contains("no snapshot"), "{}", stderr(&e));
}

#[tokio::test]
async fn diff_of_two_files_reports_each_section_and_reads_nothing() {
    let server = MockServer::start().await;
    mount(&server, 0, 0, 0).await;
    let env = Env::new();
    two_files(&env, &server);
    // Default: previous latest.
    let a = env
        .cmd(&server)
        .args(["--json", "snapshot", "diff"])
        .assert()
        .success();
    let v = schema_check::envelope(&a.get_output().stdout);
    assert_eq!(v["meta"]["schema"], "snapshot-diff");
    assert_eq!(v["meta"]["api_calls"], 0);
    let d = &v["data"];
    assert_eq!(d["from"]["id"], "2026-10-01T030000Z");
    assert_eq!(d["to"]["id"], "2026-10-08T030000Z");
    assert_eq!(d["to"]["label"], "after");
    assert_eq!(d["changed"], true);
    assert_eq!(d["sections"].as_array().unwrap().len(), 7);
    let changes = d["changes"].as_array().unwrap();
    assert_eq!(changes.len(), 2, "{changes:#?}");
    let ip = &changes[0];
    assert_eq!(
        (ip["section"].as_str(), ip["change"].as_str()),
        (Some("iplists"), Some("changed"))
    );
    assert_eq!(ip["fields"], json!(["ranges"]));
    assert_eq!(ip["details"], json!(["+range 10.9.0.0/16"]));
    let wl = &changes[1];
    assert_eq!(wl["section"], "workloads");
    assert_eq!(wl["name"], "host-2.example");
    assert_eq!(wl["fields"], json!(["labels", "enforcement_mode"]));
    assert_eq!(
        wl["details"],
        json!([
            "-label env=prod",
            "-enforcement_mode: visibility_only",
            "+enforcement_mode: full"
        ])
    );
    // One section, by id prefixes; the table names the change.
    let t = env
        .cmd(&server)
        .args([
            "--table",
            "snapshot",
            "diff",
            "2026-10-01",
            "latest",
            "--section",
            "workloads",
        ])
        .assert()
        .success();
    let out = stdout(&t);
    assert!(out.contains("host-2.example"), "{out}");
    assert!(out.contains("+enforcement_mode: full"), "{out}");
    assert!(!out.contains("10.9.0.0"), "{out}");
    // CSV: the changes, one row each.
    let c = env
        .cmd(&server)
        .args(["--csv", "snapshot", "diff", "--section", "iplists"])
        .assert()
        .success();
    let csv = stdout(&c);
    assert!(csv.starts_with("section,change,scope,name,fields,details,href\r\n"));
    assert_eq!(csv.lines().count(), 2, "{csv}");
    // --fields picks from data.
    let f = env
        .cmd(&server)
        .args([
            "--json",
            "--fields",
            "changed,changes.name",
            "snapshot",
            "diff",
        ])
        .assert()
        .success();
    let v = schema_check::picked(&f.get_output().stdout);
    assert_eq!(v["data"]["changes"][1], json!({"name": "host-2.example"}));
}

#[tokio::test]
async fn fail_on_change_exits_9_after_the_report_and_0_without_a_change() {
    let server = MockServer::start().await;
    mount(&server, 0, 0, 0).await;
    let env = Env::new();
    two_files(&env, &server);
    let a = env
        .cmd(&server)
        .args(["--json", "snapshot", "diff", "--fail-on-change"])
        .assert()
        .code(9);
    let v = schema_check::envelope(&a.get_output().stdout);
    assert_eq!(v["data"]["changed"], true);
    let err: Value = serde_json::from_str(stderr(&a).trim()).unwrap();
    assert_eq!(err["error"]["kind"], "snapshots_differ");
    assert_eq!(err["error"]["exit_code"], 9);
    env.cmd(&server)
        .args([
            "--json",
            "snapshot",
            "diff",
            "--fail-on-change",
            "--section",
            "labels,rulesets",
        ])
        .assert()
        .success();
}

#[tokio::test]
async fn now_reads_only_the_sections_the_file_side_has() {
    let server = MockServer::start().await;
    // The file has workloads; only they are read live.
    mount(&server, 0, 0, 1).await;
    let env = Env::new();
    two_files(&env, &server);
    let a = env
        .cmd(&server)
        .args([
            "--json",
            "snapshot",
            "diff",
            "previous",
            "now",
            "--section",
            "workloads",
        ])
        .assert()
        .success();
    let v = schema_check::envelope(&a.get_output().stdout);
    assert_eq!(v["meta"]["api_calls"], 1);
    assert_eq!(v["meta"]["cache"], "miss");
    assert_eq!(v["data"]["to"]["id"], "now");
    // The fixtures now are the older file: nothing changed.
    assert_eq!(v["data"]["changed"], false);
    env.cmd(&server)
        .args(["snapshot", "diff", "now", "now"])
        .assert()
        .code(2);
}

#[tokio::test]
async fn another_pces_snapshot_is_refused_with_both_named() {
    let server = MockServer::start().await;
    mount(&server, 0, 0, 0).await;
    let env = Env::new();
    let a = fixture(&server, "2026-10-01T03:00:00Z");
    let mut b = fixture(&server, "2026-10-08T03:00:00Z");
    b.pce = "other.example".into();
    snapshot::save(&env.dir(), &a).unwrap();
    snapshot::save(&env.dir(), &b).unwrap();
    let e = env.cmd(&server).args(["snapshot", "diff"]).assert().code(2);
    let msg = stderr(&e);
    assert!(
        msg.contains("other.example") && msg.contains("127.0.0.1"),
        "{msg}"
    );
    // Against now: the file's PCE is not this one.
    let e = env
        .cmd(&server)
        .args(["snapshot", "diff", "latest", "now"])
        .assert()
        .code(2);
    assert!(stderr(&e).contains("other.example"), "{}", stderr(&e));
}

#[tokio::test]
async fn a_corrupt_or_newer_file_is_a_clear_error() {
    let server = MockServer::start().await;
    mount(&server, 0, 0, 0).await;
    let env = Env::new();
    two_files(&env, &server);
    let newest = env.dir().join("2026-10-20T000000Z.json");
    std::fs::write(&newest, "{\"version\": 1, \"profile\": ").unwrap();
    // Named by its time it is picked, and refused with why.
    let e = env
        .cmd(&server)
        .args(["snapshot", "diff", "previous", "2026-10-20"])
        .assert()
        .code(1);
    assert!(stderr(&e).contains("not a snapshot file"), "{}", stderr(&e));
    let mut v = serde_json::to_value(fixture(&server, "2026-10-20T00:00:00Z")).unwrap();
    v["version"] = json!(VERSION + 1);
    std::fs::write(&newest, v.to_string()).unwrap();
    let e = env
        .cmd(&server)
        .args(["snapshot", "diff", "previous", "2026-10-20"])
        .assert()
        .code(1);
    assert!(stderr(&e).contains("newer than this illumio-tui reads"));
    // list still lists it, with the problem.
    let l = env
        .cmd(&server)
        .args(["--json", "snapshot", "list"])
        .assert()
        .success();
    let v = schema_check::envelope(&l.get_output().stdout);
    assert!(
        v["data"][2]["problem"]
            .as_str()
            .unwrap()
            .contains("upgrade")
    );
}

const QHREF: &str = "/orgs/5/traffic_flows/async_queries/q-1";

/// The readiness query: one POST, its status, one `update_rules`, one
/// download; the pending changes once.
async fn mount_readiness(server: &MockServer) {
    Mock::given(method("POST"))
        .and(path("/api/v2/orgs/5/traffic_flows/async_queries"))
        .respond_with(
            ResponseTemplate::new(202).set_body_json(json!({"href": QHREF, "status": "queued"})),
        )
        .expect(1)
        .mount(server)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("/api/v2{QHREF}")))
        .respond_with(Ticking(std::sync::atomic::AtomicU64::new(0)))
        .mount(server)
        .await;
    Mock::given(method("PUT"))
        .and(path(format!("/api/v2{QHREF}/update_rules")))
        .and(query_param("label_based_rules", "false"))
        .respond_with(ResponseTemplate::new(204))
        .expect(1)
        .mount(server)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("/api/v2{QHREF}/download")))
        .respond_with(answer(include_str!("../../../fixtures/traffic_flows.json")))
        .expect(1)
        .mount(server)
        .await;
    expect(
        server,
        "/api/v2/orgs/5/sec_policy/pending",
        include_str!("../../../fixtures/pending_list.json"),
        1,
    )
    .await;
}

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

#[tokio::test]
async fn with_readiness_adds_the_readiness_report_in_one_query() {
    let server = MockServer::start().await;
    // Readiness reuses the snapshot's workloads and draft rulesets.
    mount(&server, 1, 1, 1).await;
    mount_readiness(&server).await;
    let env = Env::new();
    let a = env
        .cmd(&server)
        .args(["--json", "snapshot", "save", "--with-readiness"])
        .assert()
        .success();
    let v = schema_check::envelope(&a.get_output().stdout);
    let sections: Vec<&str> = v["data"]["sections"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["section"].as_str().unwrap())
        .collect();
    assert_eq!(sections, ["policy", "labels", "workloads", "readiness"]);
    let s = snapshot::load(&files(&env.dir())[0]).unwrap();
    let r = s.sections.readiness.unwrap();
    assert_eq!(r["window"], "7d");
    assert!(!r["apps"].as_array().unwrap().is_empty());
}

#[test]
fn schema_and_help_name_the_snapshot_commands() {
    for words in [
        "snapshot save",
        "snapshot list",
        "snapshot diff",
        "snapshot rm",
        "snapshot prune",
    ] {
        let mut args = vec!["schema"];
        args.extend(words.split(' '));
        let a = Command::cargo_bin("illumio-tui")
            .unwrap()
            .env_clear()
            .args(&args)
            .assert()
            .success();
        let v: Value = serde_json::from_slice(&a.get_output().stdout).unwrap();
        assert!(
            v["$comment"]
                .as_str()
                .unwrap()
                .starts_with("--csv columns: "),
            "{words}"
        );
    }
    let z = Command::cargo_bin("illumio-tui")
        .unwrap()
        .env_clear()
        .args(["completions", "zsh"])
        .assert()
        .success();
    let script = stdout(&z);
    for sub in ["snapshot", "prune", "--fail-on-change", "--with-readiness"] {
        assert!(script.contains(sub), "{sub}");
    }
}

/// Every list but the boundaries, which answer `status`.
/// `later`: the labels and workloads reads, after the policy (0 when the
/// boundaries read fails the save first).
async fn mount_boundaries_answering(server: &MockServer, status: u16, later: u64) {
    expect(
        server,
        LABELS,
        include_str!("../../../fixtures/labels.json"),
        later,
    )
    .await;
    for (path_, body) in [
        (GROUPS, include_str!("../../../fixtures/label_groups.json")),
        (IP_LISTS, include_str!("../../../fixtures/ip_lists.json")),
        (SERVICES, include_str!("../../../fixtures/services.json")),
        (DRAFT_RS, include_str!("../../../fixtures/rule_sets.json")),
        (ACTIVE_RS, "[]"),
    ] {
        expect(server, path_, body, 1).await;
    }
    expect(
        server,
        WORKLOADS,
        include_str!("../../../fixtures/workloads.json"),
        later,
    )
    .await;
    Mock::given(method("GET"))
        .and(path(BOUNDARIES))
        .respond_with(ResponseTemplate::new(status).set_body_string("{}"))
        .mount(server)
        .await;
}

#[tokio::test]
async fn a_pce_without_boundaries_still_saves_and_says_so() {
    for status in [404u16, 403] {
        let server = MockServer::start().await;
        mount_boundaries_answering(&server, status, 1).await;
        let env = Env::new();
        let a = env
            .cmd(&server)
            .args(["--json", "snapshot", "save"])
            .assert()
            .success();
        let v = schema_check::envelope(&a.get_output().stdout);
        let w = v["meta"]["warnings"].to_string();
        assert!(
            w.contains("enforcement boundaries not readable"),
            "{status}: {w}"
        );
        let f = &files(&env.dir())[0];
        let s = snapshot::load(f).unwrap();
        assert!(s.sections.policy.unwrap().enforcement_boundaries.is_none());
        // Against a snapshot with boundaries: not compared, not removed.
        snapshot::save(&env.dir(), &fixture(&server, "2020-01-01T00:00:00Z")).unwrap();
        let d = env
            .cmd(&server)
            .args(["--json", "snapshot", "diff", "--section", "boundaries"])
            .assert()
            .success();
        let v = schema_check::envelope(&d.get_output().stdout);
        assert_eq!(v["data"]["changes"], json!([]));
        assert_eq!(v["data"]["not_compared"][0]["section"], "boundaries");
    }
    // Any other failure still fails the save, and nothing is written.
    let server = MockServer::start().await;
    mount_boundaries_answering(&server, 500, 0).await;
    let env = Env::new();
    env.cmd(&server).args(["snapshot", "save"]).assert().code(4);
    assert!(files(&env.dir()).is_empty());
}

#[tokio::test]
async fn unreadable_and_foreign_files_are_never_latest_nor_pruned() {
    let server = MockServer::start().await;
    mount(&server, 0, 0, 0).await;
    let env = Env::new();
    two_files(&env, &server);
    let d = env.dir();
    std::fs::write(d.join("2026-10-20T000000Z.json"), "{broken").unwrap();
    let mut v = serde_json::to_value(fixture(&server, "2026-10-21T00:00:00Z")).unwrap();
    v["version"] = json!(VERSION + 1);
    std::fs::write(d.join("2026-10-21T000000Z.json"), v.to_string()).unwrap();
    std::fs::write(d.join("notes.json"), "{}").unwrap();
    std::fs::write(d.join("backup.json"), "{}").unwrap();
    // The default diff still compares the two readable ones.
    let a = env
        .cmd(&server)
        .args(["--json", "snapshot", "diff"])
        .assert()
        .success();
    let v = schema_check::envelope(&a.get_output().stdout);
    assert_eq!(v["data"]["to"]["id"], "2026-10-08T030000Z");
    let l = env
        .cmd(&server)
        .args(["--json", "snapshot", "list"])
        .assert()
        .success();
    let v = schema_check::envelope(&l.get_output().stdout);
    assert_eq!(
        v["data"].as_array().unwrap().len(),
        4,
        "notes, backup unlisted"
    );
    let p = env
        .cmd(&server)
        .args(["--json", "snapshot", "prune", "--keep", "1"])
        .assert()
        .success();
    let v = schema_check::envelope(&p.get_output().stdout);
    assert_eq!(v["data"]["removed"][0]["id"], "2026-10-01T030000Z");
    assert_eq!(v["data"]["removed"].as_array().unwrap().len(), 1);
    assert!(
        v["meta"]["warnings"]
            .to_string()
            .contains("2 unreadable snapshot files left alone")
    );
    for f in [
        "2026-10-08T030000Z.json",
        "2026-10-20T000000Z.json",
        "2026-10-21T000000Z.json",
        "notes.json",
        "backup.json",
    ] {
        assert!(d.join(f).exists(), "{f}");
    }
    let e = env
        .cmd(&server)
        .args(["snapshot", "prune", "--keep", "0"])
        .assert()
        .code(2);
    assert!(stderr(&e).contains("--keep"), "{}", stderr(&e));
}

#[tokio::test]
async fn file_commands_resolve_only_the_profile_name() {
    let server = MockServer::start().await;
    mount(&server, 0, 0, 0).await;
    let env = Env::new();
    // A config profile whose secret command fails and would run on a resolve.
    let cfg = env.home.path().join("config/illumio-tui");
    std::fs::create_dir_all(&cfg).unwrap();
    std::fs::write(
        cfg.join("config.toml"),
        "default_profile = \"lab\"\n[profiles.lab]\nhost = \"pce.example\"\norg_id = 5\napi_key = \"k\"\napi_secret_cmd = \"exit 3\"\n",
    )
    .unwrap();
    let dir = env.data().join("illumio-tui/snapshots/lab");
    let mut a = fixture(&server, "2026-10-01T03:00:00Z");
    a.profile = "lab".into();
    let mut b = a.clone();
    b.taken_at = "2026-10-02T03:00:00Z".parse().unwrap();
    snapshot::save(&dir, &a).unwrap();
    snapshot::save(&dir, &b).unwrap();
    let run = |args: &[&str]| {
        let mut c = Command::cargo_bin("illumio-tui").unwrap();
        c.env_clear()
            .env("PATH", std::env::var("PATH").unwrap_or_default())
            .env("XDG_CONFIG_HOME", env.home.path().join("config"))
            .env("XDG_DATA_HOME", env.data())
            .arg("--json")
            .args(args)
            .assert()
            .success()
    };
    let l = run(&["snapshot", "list"]);
    let v = schema_check::envelope(&l.get_output().stdout);
    assert_eq!(v["meta"]["profile"], "lab");
    assert_eq!(v["data"].as_array().unwrap().len(), 2);
    run(&["snapshot", "diff"]);
    run(&["snapshot", "prune", "--keep", "1"]);
    run(&["snapshot", "rm", "latest"]);
    // A live read does resolve the profile: its secret command fails.
    env.cmd(&server)
        .env_remove("PCE_HOST")
        .env_remove("API_SECRET")
        .env_remove("API_KEY")
        .env_remove("PCE_ORG_ID")
        .env_remove("PCE_PORT")
        .args(["snapshot", "save"])
        .assert()
        .code(2);
}
