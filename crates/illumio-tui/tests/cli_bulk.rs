//! `workloads enforce` and `workloads relabel` against a mock PCE: the
//! guard and flags before any request, the typed count, dry runs, chunks,
//! the re-read, the audit and exit codes.
#[path = "common/schema_check.rs"]
mod schema_check;
use assert_cmd::Command;
use serde_json::{Value, json};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, Request, ResponseTemplate};

const TEST_SECRET: &str = "s3cr3t-cli-bulk-test-77";
const LIST: &str = "/api/v2/orgs/5/workloads";
const BULK: &str = "/api/v2/orgs/5/workloads/bulk_update";

fn no_config_dir() -> std::path::PathBuf {
    static N: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = N.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    std::env::temp_dir().join(format!(
        "illumio-tui-bulk-no-config-{}-{n}",
        std::process::id()
    ))
}

fn cmd(server: &MockServer) -> Command {
    let url = url::Url::parse(&server.uri()).unwrap();
    let mut c = Command::cargo_bin("illumio-tui").unwrap();
    c.env_clear()
        .env("PATH", std::env::var("PATH").unwrap_or_default())
        .env("PCE_SCHEME", "http")
        .env("PCE_HOST", url.host_str().unwrap())
        .env("PCE_PORT", url.port().unwrap().to_string())
        .env("PCE_ORG_ID", "5")
        .env("API_KEY", "key")
        .env("API_SECRET", TEST_SECRET)
        .env("XDG_CONFIG_HOME", no_config_dir());
    c
}

fn writer(server: &MockServer, state: &std::path::Path) -> Command {
    let mut c = cmd(server);
    c.env("ILLUMIO_WRITES", "1")
        .env("XDG_STATE_HOME", state)
        .arg("--json");
    c
}

fn stdout_json(a: &assert_cmd::assert::Assert) -> Value {
    schema_check::envelope(&a.get_output().stdout)
}

fn audit_lines(state: &std::path::Path) -> Vec<Value> {
    let p = state.join("illumio-tui").join("writes.jsonl");
    match std::fs::read_to_string(p) {
        Ok(t) => t
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect(),
        Err(_) => Vec::new(),
    }
}

async fn mount_index(server: &MockServer) {
    for (p, body) in [
        (
            "/api/v2/orgs/5/labels",
            include_str!("../../../fixtures/labels.json"),
        ),
        ("/api/v2/orgs/5/sec_policy/draft/label_groups", "[]"),
        ("/api/v2/orgs/5/sec_policy/draft/ip_lists", "[]"),
        ("/api/v2/orgs/5/sec_policy/draft/services", "[]"),
    ] {
        Mock::given(method("GET"))
            .and(path(p))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_string(body)
                    .insert_header("Content-Type", "application/json"),
            )
            .mount(server)
            .await;
    }
}

fn wl(n: usize, mode: &str, managed: bool, labels: Value) -> Value {
    json!({
        "href": format!("/orgs/5/workloads/w{n}"),
        "hostname": format!("host-{n}"),
        "managed": managed,
        "enforcement_mode": mode,
        "visibility_level": "flow_summary",
        "labels": labels,
    })
}

fn shop() -> Value {
    json!([{"href": "/orgs/5/labels/3", "key": "app", "value": "shop"}])
}

async fn mount_list(server: &MockServer, list: Value) {
    Mock::given(method("GET"))
        .and(path(LIST))
        .respond_with(ResponseTemplate::new(200).set_body_json(list))
        .mount(server)
        .await;
}

fn all_updated(req: &Request) -> ResponseTemplate {
    let sent: Value = serde_json::from_slice(&req.body).unwrap();
    let answer: Vec<Value> = sent
        .as_array()
        .unwrap()
        .iter()
        .map(|i| json!({"href": i["href"], "status": "updated"}))
        .collect();
    ResponseTemplate::new(200).set_body_json(answer)
}

async fn count(server: &MockServer, m: &str, p: &str) -> usize {
    server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .filter(|r| r.method.as_str() == m && r.url.path() == p)
        .count()
}

async fn bodies(server: &MockServer, p: &str) -> Vec<Value> {
    server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .filter(|r| r.method.as_str() == "PUT" && r.url.path() == p)
        .map(|r| serde_json::from_slice(&r.body).unwrap())
        .collect()
}

fn three() -> Value {
    json!([
        wl(1, "visibility_only", true, shop()),
        wl(2, "visibility_only", true, shop()),
        wl(3, "visibility_only", false, shop()),
    ])
}

#[tokio::test(flavor = "multi_thread")]
async fn writes_off_or_no_flag_refuse_before_any_request() {
    let server = MockServer::start().await;
    let a = cmd(&server)
        .args(["--json", "workloads", "enforce", "--all", "full", "--yes"])
        .assert()
        .code(6);
    let out = String::from_utf8_lossy(&a.get_output().stderr).to_string();
    assert!(out.contains("write_refused"), "{out}");
    let state = tempfile::tempdir().unwrap();
    writer(&server, state.path())
        .args(["workloads", "relabel", "--all", "--set", "app=shop"])
        .assert()
        .code(2);
    assert!(server.received_requests().await.unwrap().is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn enforce_dry_run_prints_the_plan_and_the_chunks() {
    let server = MockServer::start().await;
    mount_index(&server).await;
    mount_list(&server, three()).await;
    let state = tempfile::tempdir().unwrap();
    let a = writer(&server, state.path())
        .args([
            "workloads",
            "enforce",
            "full",
            "--label",
            "app=shop",
            "--dry-run",
        ])
        .assert()
        .success();
    let out = stdout_json(&a);
    let d = &out["data"];
    assert_eq!(d["dry_run"], true);
    assert_eq!(d["count"], 2);
    assert_eq!(d["needs_confirm_count"], true);
    assert_eq!(d["workloads"][0]["change"], "visibility_only → full");
    assert_eq!(d["skipped"][0]["reason"], "unmanaged");
    // `--yes` would send right after its own selection read: no re-read.
    assert_eq!(d["reads"], 0);
    assert!(
        d["reread"]
            .as_str()
            .unwrap()
            .starts_with("no re-read (the selection read ran"),
        "{}",
        d["reread"]
    );
    assert_eq!(d["requests"][0]["method"], "PUT");
    assert_eq!(d["requests"][0]["path"], "/orgs/5/workloads/bulk_update");
    assert_eq!(
        d["requests"][0]["body"],
        json!([
            {"href": "/orgs/5/workloads/w1", "enforcement_mode": "full"},
            {"href": "/orgs/5/workloads/w2", "enforcement_mode": "full"},
        ])
    );
    assert_eq!(
        d["summary"],
        "3 workloads: 2 visibility_only → full · 1 unmanaged (skipped)"
    );
    assert_eq!(count(&server, "GET", LIST).await, 1, "one list read");
    assert_eq!(count(&server, "PUT", BULK).await, 0);
    assert!(audit_lines(state.path()).is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn full_on_several_needs_the_matching_count() {
    let server = MockServer::start().await;
    mount_index(&server).await;
    mount_list(&server, three()).await;
    let state = tempfile::tempdir().unwrap();
    let a = writer(&server, state.path())
        .args(["workloads", "enforce", "--all", "full", "--yes"])
        .assert()
        .code(2);
    let err = String::from_utf8_lossy(&a.get_output().stderr).to_string();
    assert!(err.contains("--confirm-count"), "{err}");
    let a = writer(&server, state.path())
        .args([
            "workloads",
            "enforce",
            "--all",
            "selective",
            "--yes",
            "--confirm-count",
            "3",
        ])
        .assert()
        .code(2);
    let err = String::from_utf8_lossy(&a.get_output().stderr).to_string();
    assert!(err.contains("does not match: 2 workloads"), "{err}");
    assert_eq!(count(&server, "PUT", BULK).await, 0);
    assert!(audit_lines(state.path()).is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn enforce_yes_re_reads_once_sends_and_audits() {
    let server = MockServer::start().await;
    mount_list(&server, three()).await;
    Mock::given(method("PUT"))
        .and(path(BULK))
        .respond_with(all_updated)
        .expect(1)
        .mount(&server)
        .await;
    let state = tempfile::tempdir().unwrap();
    let a = writer(&server, state.path())
        .args([
            "workloads",
            "enforce",
            "--all",
            "full",
            "--visibility",
            "flow_drops",
            "--yes",
            "--confirm-count",
            "2",
        ])
        .assert()
        .success();
    let out = stdout_json(&a);
    assert_eq!(
        out["data"]["reads"], 0,
        "the selection was read moments before"
    );
    assert!(out["data"]["reread_skipped"].as_str().is_some());
    assert_eq!(out["data"]["steps"][0]["status"], "written");
    let items = out["data"]["items"].as_array().unwrap();
    assert_eq!(items.iter().filter(|i| i["status"] == "updated").count(), 2);
    assert_eq!(
        out["meta"]["write"]["path"],
        "/orgs/5/workloads/bulk_update"
    );
    assert_eq!(
        bodies(&server, BULK).await[0][0],
        json!({"href": "/orgs/5/workloads/w1", "enforcement_mode": "full", "visibility_level": "flow_drops"})
    );
    // The selection only: it was read moments before the send.
    assert_eq!(count(&server, "GET", LIST).await, 1);
    let audit = audit_lines(state.path());
    assert_eq!(audit.len(), 1);
    assert_eq!(audit[0]["source"], "cli");
    assert_eq!(audit[0]["items"].as_array().unwrap().len(), 2);
    server.verify().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn two_hundred_fifty_workloads_go_in_three_requests() {
    let server = MockServer::start().await;
    let list = Value::Array(
        (0..250)
            .map(|n| wl(n, "visibility_only", true, json!([])))
            .collect(),
    );
    mount_list(&server, list).await;
    Mock::given(method("PUT"))
        .and(path(BULK))
        .respond_with(all_updated)
        .expect(3)
        .mount(&server)
        .await;
    let state = tempfile::tempdir().unwrap();
    let a = writer(&server, state.path())
        .args([
            "workloads",
            "enforce",
            "--all",
            "idle",
            "--yes",
            "--confirm-count",
            "250",
        ])
        .assert()
        .success();
    let out = stdout_json(&a);
    assert_eq!(out["data"]["steps"].as_array().unwrap().len(), 3);
    let sizes: Vec<usize> = bodies(&server, BULK)
        .await
        .iter()
        .map(|b| b.as_array().unwrap().len())
        .collect();
    assert_eq!(sizes, vec![100, 100, 50]);
    assert_eq!(audit_lines(state.path()).len(), 3);
    server.verify().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn relabel_dry_run_and_yes() {
    let server = MockServer::start().await;
    mount_index(&server).await;
    let list = json!([
        wl(
            1,
            "full",
            true,
            json!([{"href": "/orgs/5/labels/1", "key": "role", "value": "web"}, {"href": "/orgs/5/labels/4", "key": "env", "value": "prod"}])
        ),
        wl(2, "full", false, json!([])),
    ]);
    mount_list(&server, list).await;
    Mock::given(method("PUT"))
        .and(path(BULK))
        .respond_with(all_updated)
        .expect(1)
        .mount(&server)
        .await;
    let state = tempfile::tempdir().unwrap();
    let a = writer(&server, state.path())
        .args([
            "workloads",
            "relabel",
            "--all",
            "--set",
            "app=shop,env=-",
            "--dry-run",
        ])
        .assert()
        .success();
    let out = stdout_json(&a);
    let d = &out["data"];
    assert_eq!(d["needs_confirm_count"], false);
    assert_eq!(
        d["requests"][0]["body"],
        json!([
            {"href": "/orgs/5/workloads/w1", "labels": [{"href": "/orgs/5/labels/1"}, {"href": "/orgs/5/labels/3"}]},
            {"href": "/orgs/5/workloads/w2", "labels": [{"href": "/orgs/5/labels/3"}]},
        ]),
        "managed and unmanaged: labels only"
    );
    assert_eq!(d["workloads"][0]["change"], "+app=shop · -env=prod");
    assert_eq!(count(&server, "PUT", BULK).await, 0);
    writer(&server, state.path())
        .args([
            "workloads",
            "relabel",
            "--all",
            "--add-label",
            "app=shop",
            "--remove-label",
            "env",
            "--yes",
        ])
        .assert()
        .success();
    assert_eq!(count(&server, "PUT", BULK).await, 1);
    assert_eq!(audit_lines(state.path()).len(), 1);
    // Labels resolve through the labels list alone, never the 4-list index.
    for p in [
        "/api/v2/orgs/5/sec_policy/draft/label_groups",
        "/api/v2/orgs/5/sec_policy/draft/ip_lists",
        "/api/v2/orgs/5/sec_policy/draft/services",
    ] {
        assert_eq!(count(&server, "GET", p).await, 0, "{p}");
    }
    // An unknown label is refused with the known values, nothing sent.
    let a = writer(&server, state.path())
        .args([
            "workloads",
            "relabel",
            "--all",
            "--set",
            "app=nope",
            "--yes",
        ])
        .assert()
        .code(2);
    let err = String::from_utf8_lossy(&a.get_output().stderr).to_string();
    assert!(err.contains("known values for app: shop"), "{err}");
    assert_eq!(count(&server, "PUT", BULK).await, 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_refused_workload_exits_4_after_the_report() {
    let server = MockServer::start().await;
    mount_list(&server, three()).await;
    Mock::given(method("PUT"))
        .and(path(BULK))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            {"href": "/orgs/5/workloads/w1", "status": "updated"},
            {"href": "/orgs/5/workloads/w2", "status": "validation_failure", "errors": [{"message": "bad"}]},
        ])))
        .expect(1)
        .mount(&server)
        .await;
    let state = tempfile::tempdir().unwrap();
    let a = writer(&server, state.path())
        .args(["workloads", "enforce", "--all", "idle", "--yes"])
        .assert()
        .code(4);
    let out = stdout_json(&a);
    assert_eq!(out["data"]["steps"][0]["status"], "partial");
    let err = String::from_utf8_lossy(&a.get_output().stderr).to_string();
    assert!(err.contains("write_incomplete"), "{err}");
    assert!(err.contains("1 failed"), "{err}");
    server.verify().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn csv_is_refused_and_the_table_prints_the_plan() {
    let server = MockServer::start().await;
    cmd(&server)
        .env("ILLUMIO_WRITES", "1")
        .args([
            "--csv",
            "workloads",
            "enforce",
            "--all",
            "idle",
            "--dry-run",
        ])
        .assert()
        .code(2);
    assert!(server.received_requests().await.unwrap().is_empty());
    mount_list(&server, three()).await;
    let state = tempfile::tempdir().unwrap();
    let a = cmd(&server)
        .env("ILLUMIO_WRITES", "1")
        .env("XDG_STATE_HOME", state.path())
        .args([
            "--table",
            "workloads",
            "enforce",
            "--all",
            "idle",
            "--dry-run",
        ])
        .assert()
        .success();
    let out = String::from_utf8_lossy(&a.get_output().stdout).to_string();
    assert!(out.contains("visibility_only → idle"), "{out}");
    assert!(out.contains("no re-read (the selection read ran"), "{out}");
    assert!(out.contains("+ 1 write: 1 bulk_update"), "{out}");
    assert!(
        out.contains(r#"[{"enforcement_mode":"idle","href":"/orgs/5/workloads/w1"}"#),
        "{out}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_truncated_selection_is_never_sent() {
    let server = MockServer::start().await;
    mount_list(&server, three()).await;
    let state = tempfile::tempdir().unwrap();
    // A cap of 3 with 3 rows back: more may exist.
    let a = writer(&server, state.path())
        .args([
            "workloads",
            "enforce",
            "--all",
            "idle",
            "--max-results",
            "3",
            "--yes",
        ])
        .assert()
        .code(2);
    let err = String::from_utf8_lossy(&a.get_output().stderr).to_string();
    assert!(err.contains("truncated"), "{err}");
    assert_eq!(count(&server, "PUT", BULK).await, 0);
    let a = writer(&server, state.path())
        .args([
            "workloads",
            "enforce",
            "--all",
            "idle",
            "--max-results",
            "3",
            "--dry-run",
        ])
        .assert()
        .success();
    assert_eq!(stdout_json(&a)["meta"]["truncated"], true);
}

#[tokio::test(flavor = "multi_thread")]
async fn nothing_to_change_prints_the_summary_and_exits_0() {
    let server = MockServer::start().await;
    mount_list(
        &server,
        json!([
            wl(1, "full", true, json!([])),
            wl(2, "idle", false, json!([]))
        ]),
    )
    .await;
    let state = tempfile::tempdir().unwrap();
    let a = writer(&server, state.path())
        .args(["workloads", "enforce", "--all", "full", "--yes"])
        .assert()
        .success();
    let out = stdout_json(&a);
    assert_eq!(out["data"]["count"], 0);
    assert_eq!(
        out["data"]["summary"],
        "2 workloads: 1 already full (skipped) · 1 unmanaged (skipped)"
    );
    assert_eq!(count(&server, "PUT", BULK).await, 0);
    assert!(audit_lines(state.path()).is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn no_filter_needs_all_and_sends_nothing() {
    let server = MockServer::start().await;
    let state = tempfile::tempdir().unwrap();
    for args in [
        vec!["workloads", "enforce", "idle", "--yes"],
        vec!["workloads", "enforce", "idle", "--dry-run"],
        vec!["workloads", "relabel", "--set", "app=shop", "--yes"],
        vec![
            "workloads",
            "relabel",
            "--set",
            "app=shop",
            "--max-results",
            "5",
            "--dry-run",
        ],
    ] {
        let a = writer(&server, state.path()).args(&args).assert().code(2);
        let err = String::from_utf8_lossy(&a.get_output().stderr).to_string();
        assert!(err.contains("\"kind\":\"invalid\""), "{args:?}: {err}");
        assert!(err.contains("--all"), "{err}");
    }
    assert!(server.received_requests().await.unwrap().is_empty());
    // With --all: read and planned.
    mount_list(&server, three()).await;
    let a = writer(&server, state.path())
        .args(["workloads", "enforce", "idle", "--all", "--dry-run"])
        .assert()
        .success();
    assert_eq!(stdout_json(&a)["data"]["count"], 2);
    // --all with filters: the filters still apply.
    let a = writer(&server, state.path())
        .args([
            "workloads",
            "enforce",
            "idle",
            "--all",
            "--managed",
            "true",
            "--dry-run",
        ])
        .assert()
        .success();
    assert_eq!(stdout_json(&a)["data"]["count"], 2);
}

#[tokio::test(flavor = "multi_thread")]
async fn more_than_twenty_five_workloads_need_the_count() {
    let server = MockServer::start().await;
    mount_index(&server).await;
    let list = Value::Array(
        (0..26)
            .map(|n| wl(n, "visibility_only", true, json!([])))
            .collect(),
    );
    mount_list(&server, list).await;
    Mock::given(method("PUT"))
        .and(path(BULK))
        .respond_with(all_updated)
        .mount(&server)
        .await;
    let state = tempfile::tempdir().unwrap();
    let a = writer(&server, state.path())
        .args(["workloads", "enforce", "idle", "--all", "--yes"])
        .assert()
        .code(2);
    let err = String::from_utf8_lossy(&a.get_output().stderr).to_string();
    assert!(
        err.contains("more than 25 workloads: pass --confirm-count"),
        "{err}"
    );
    writer(&server, state.path())
        .args([
            "workloads",
            "relabel",
            "--all",
            "--set",
            "app=shop",
            "--yes",
        ])
        .assert()
        .code(2);
    assert_eq!(count(&server, "PUT", BULK).await, 0);
    let a = writer(&server, state.path())
        .args(["workloads", "enforce", "idle", "--all", "--dry-run"])
        .assert()
        .success();
    assert_eq!(stdout_json(&a)["data"]["needs_confirm_count"], true);
    writer(&server, state.path())
        .args([
            "workloads",
            "enforce",
            "idle",
            "--all",
            "--yes",
            "--confirm-count",
            "26",
        ])
        .assert()
        .success();
    assert_eq!(count(&server, "PUT", BULK).await, 1);
}
