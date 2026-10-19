//! `illumio-tui enforce` (#56) against a mock PCE: readiness first and
//! the refusal before any write, the dry run that sends nothing, the move
//! and its change record (also after a partial result), `--force`,
//! `--watch` (exit 0 / 7), and `--rollback` (one re-read, a changed
//! workload skipped, the rest grouped). Records go to a temporary
//! `XDG_STATE_HOME`. The helpers repeat those of cli_readiness.rs (test
//! files share no code without a module).
use assert_cmd::Command;
use serde_json::{Value, json};
use std::path::PathBuf;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, Request, ResponseTemplate};

const QHREF: &str = "/orgs/5/traffic_flows/async_queries/q-1";
const BULK: &str = "/api/v2/orgs/5/workloads/bulk_update";
const SHOP_LABELS: &str = r#"[["/orgs/5/labels/3","/orgs/5/labels/4"]]"#;

/// Marks that accept the fixture's two would-block flows: the NAS's
/// pings, egress 443.
const ACCEPT_GAPS: &str = r#"
[[mark]]
id = "m-nas-ok"
action = "accept-blocked"
src = { ip = "10.0.3.30" }
dst = "any"
decision = "blocked"
comment = "the NAS pings, harmless"

[[mark]]
id = "m-egress"
action = "squelch"
src = "any"
dst = "any"
service = "443/tcp"
comment = "egress is handled elsewhere"
"#;

struct Env {
    config: tempfile::TempDir,
    state: tempfile::TempDir,
}

impl Env {
    fn new(marks: &str) -> Self {
        let config = tempfile::tempdir().unwrap();
        if !marks.is_empty() {
            let dir = config.path().join("illumio-tui");
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join("marks.toml"), marks).unwrap();
        }
        Self {
            config,
            state: tempfile::tempdir().unwrap(),
        }
    }

    fn records_dir(&self) -> PathBuf {
        self.state.path().join("illumio-tui").join("enforcements")
    }

    fn records(&self) -> Vec<(PathBuf, Value)> {
        let mut v: Vec<(PathBuf, Value)> = match std::fs::read_dir(self.records_dir()) {
            Ok(d) => d
                .map(|e| {
                    let p = e.unwrap().path();
                    let body = serde_json::from_slice(&std::fs::read(&p).unwrap()).unwrap();
                    (p, body)
                })
                .collect(),
            Err(_) => Vec::new(),
        };
        v.sort_by(|a, b| a.0.cmp(&b.0));
        v
    }

    fn write_record(&self, name: &str, body: &Value) -> PathBuf {
        let d = self.records_dir();
        std::fs::create_dir_all(&d).unwrap();
        let p = d.join(name);
        std::fs::write(&p, serde_json::to_vec(body).unwrap()).unwrap();
        p
    }

    fn cmd(&self, server: &MockServer) -> Command {
        let mut c = self.bare(server);
        c.arg("--json");
        c
    }

    /// Without `--json`: the caller picks the format.
    fn bare(&self, server: &MockServer) -> Command {
        let url = url::Url::parse(&server.uri()).unwrap();
        let mut c = Command::cargo_bin("illumio-tui").unwrap();
        c.env_clear()
            .env("PATH", std::env::var("PATH").unwrap_or_default())
            .env("PCE_SCHEME", "http")
            .env("PCE_HOST", url.host_str().unwrap())
            .env("PCE_PORT", url.port().unwrap().to_string())
            .env("PCE_ORG_ID", "5")
            .env("API_KEY", "key")
            .env("API_SECRET", "s3cr3t-enforce-test")
            .env("XDG_CONFIG_HOME", self.config.path())
            .env("XDG_STATE_HOME", self.state.path())
            .env("ILLUMIO_WRITES", "1");
        c
    }
}

fn json_out(a: &assert_cmd::assert::Assert) -> Value {
    serde_json::from_slice(&a.get_output().stdout).expect("stdout is JSON")
}

fn stderr(a: &assert_cmd::assert::Assert) -> String {
    String::from_utf8_lossy(&a.get_output().stderr).to_string()
}

fn answer(body: &str) -> ResponseTemplate {
    ResponseTemplate::new(200)
        .set_body_string(body)
        .insert_header("Content-Type", "application/json")
}

/// The fixture's flows with draft decisions (as cli_readiness.rs).
fn draft_flows() -> String {
    let mut flows: Vec<Value> =
        serde_json::from_str(include_str!("../../../fixtures/traffic_flows.json")).unwrap();
    for f in &mut flows {
        let draft = match (f["policy_decision"].as_str(), f["service"]["port"].as_u64()) {
            (Some("potentially_blocked"), _) => "allowed",
            (_, Some(443)) => "blocked",
            (Some(d), _) => d,
            _ => unreachable!(),
        };
        f["draft_policy_decision"] = json!(draft);
    }
    serde_json::to_string(&flows).unwrap()
}

struct Ticking {
    body: Value,
    n: std::sync::atomic::AtomicU64,
}

impl wiremock::Respond for Ticking {
    fn respond(&self, _: &Request) -> ResponseTemplate {
        let n = self.n.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let mut body = self.body.clone();
        body["updated_at"] = json!(format!("2026-10-14T00:00:{:02}Z", n % 60));
        ResponseTemplate::new(200).set_body_json(body)
    }
}

/// A shop/prod workload (labels by href only: the keys come from the
/// index).
fn wl(n: u32, mode: &str, vis: &str, managed: bool) -> Value {
    json!({
        "href": format!("/orgs/5/workloads/w{n}"),
        "hostname": format!("web-{n}"),
        "managed": managed,
        "enforcement_mode": mode,
        "visibility_level": vis,
        "labels": [{"href": "/orgs/5/labels/3"}, {"href": "/orgs/5/labels/4"}],
    })
}

/// shop/prod: w1 already full, w2 and w3 visibility only, w4 unmanaged;
/// w9 has another app's labels (the PCE's filter would not return it;
/// enforce leaves it out too).
fn shop_workloads() -> Value {
    let mut other = wl(9, "visibility_only", "flow_summary", true);
    other["labels"] = json!([{"href": "/orgs/5/labels/5"}]);
    json!([
        wl(1, "full", "flow_summary", true),
        wl(2, "visibility_only", "flow_summary", true),
        wl(3, "visibility_only", "flow_off", true),
        wl(4, "idle", "flow_summary", false),
        other,
    ])
}

async fn mount_index(server: &MockServer) {
    for (p, body) in [
        (
            "/api/v2/orgs/5/labels",
            include_str!("../../../fixtures/labels.json"),
        ),
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
    ] {
        Mock::given(method("GET"))
            .and(path(p))
            .respond_with(answer(body))
            .mount(server)
            .await;
    }
}

/// The async query: `flows` as its download; `update_rules` answered
/// (expected `puts` times).
async fn mount_query(server: &MockServer, flows: String, puts: u64) {
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
        .respond_with(Ticking {
            body: json!({
                "status": "completed", "rules": "completed",
                "result": {"href": format!("{QHREF}/download")}, "flows_count": 4
            }),
            n: std::sync::atomic::AtomicU64::new(0),
        })
        .mount(server)
        .await;
    Mock::given(method("PUT"))
        .and(path(format!("/api/v2{QHREF}/update_rules")))
        .respond_with(ResponseTemplate::new(204))
        .expect(puts)
        .mount(server)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("/api/v2{QHREF}/download")))
        .respond_with(answer(&flows))
        .expect(1)
        .mount(server)
        .await;
}

/// Readiness's requests: the index, one draft query, the workloads, the
/// pending changes (`pending`), the draft rulesets.
async fn mount_readiness(server: &MockServer, workloads: Value, pending: &str) {
    mount_index(server).await;
    mount_query(server, draft_flows(), 1).await;
    Mock::given(method("GET"))
        .and(path("/api/v2/orgs/5/workloads"))
        .and(wiremock::matchers::query_param("labels", SHOP_LABELS))
        .respond_with(ResponseTemplate::new(200).set_body_json(workloads))
        .expect(1)
        .mount(server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v2/orgs/5/sec_policy/pending"))
        .respond_with(answer(pending))
        .expect(1)
        .mount(server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v2/orgs/5/sec_policy/draft/rule_sets"))
        .respond_with(answer(include_str!("../../../fixtures/rule_sets.json")))
        .expect(1)
        .mount(server)
        .await;
}

/// Answers a bulk_update: every workload updated, except `failing`.
fn bulk_answer(failing: &'static [&'static str]) -> impl Fn(&Request) -> ResponseTemplate {
    move |req: &Request| {
        let sent: Value = serde_json::from_slice(&req.body).unwrap();
        let out: Vec<Value> = sent
            .as_array()
            .unwrap()
            .iter()
            .map(|i| {
                let href = i["href"].as_str().unwrap();
                if failing.contains(&href) {
                    json!({"href": href, "status": "validation_failure", "message": "nope"})
                } else {
                    json!({"href": href, "status": "updated"})
                }
            })
            .collect();
        ResponseTemplate::new(200).set_body_json(out)
    }
}

async fn mount_bulk(server: &MockServer, failing: &'static [&'static str]) {
    Mock::given(method("PUT"))
        .and(path(BULK))
        .respond_with(bulk_answer(failing))
        .mount(server)
        .await;
}

/// Every request after the object lists, in order: `METHOD path`.
async fn sequence(server: &MockServer) -> Vec<String> {
    server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .map(|r| {
            format!(
                "{} {}",
                r.method,
                r.url.path().trim_start_matches("/api/v2")
            )
        })
        .filter(|s| {
            !["/labels", "/label_groups", "/ip_lists", "/services"]
                .iter()
                .any(|o| s.ends_with(o))
        })
        .collect()
}

async fn bulk_bodies(server: &MockServer) -> Vec<Value> {
    server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .filter(|r| r.method.as_str() == "PUT" && r.url.path() == BULK)
        .map(|r| serde_json::from_slice(&r.body).unwrap())
        .collect()
}

fn readiness_sequence() -> Vec<String> {
    vec![
        "POST /orgs/5/traffic_flows/async_queries".to_string(),
        format!("GET {QHREF}"),
        format!("PUT {QHREF}/update_rules"),
        format!("GET {QHREF}"),
        format!("GET {QHREF}/download"),
        "GET /orgs/5/workloads".into(),
        "GET /orgs/5/sec_policy/pending".into(),
        "GET /orgs/5/sec_policy/draft/rule_sets".into(),
    ]
}

#[tokio::test(flavor = "multi_thread")]
async fn enforce_refuses_a_not_ready_app_after_readiness_and_before_any_write() {
    let env = Env::new("");
    for flag in ["--dry-run", "--yes"] {
        let server = MockServer::start().await;
        mount_readiness(&server, shop_workloads(), "[]").await;
        mount_bulk(&server, &[]).await;
        let a = env
            .cmd(&server)
            .args(["enforce", "shop/prod", "--to", "full", flag])
            .assert()
            .code(6);
        let err = stderr(&a);
        assert!(err.contains("write_refused"), "{err}");
        assert!(err.contains("is not ready (needs rules)"), "{err}");
        assert!(err.contains("in unmanaged-nas/- icmp"), "{err}");
        assert!(err.contains("--force"), "{err}");
        assert_eq!(sequence(&server).await, readiness_sequence(), "{flag}");
        assert!(bulk_bodies(&server).await.is_empty());
        server.verify().await;
    }
    assert!(env.records().is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn enforce_refuses_without_writes_or_flags_before_any_request() {
    let server = MockServer::start().await;
    let env = Env::new("");
    let a = env
        .cmd(&server)
        .env_remove("ILLUMIO_WRITES")
        .args(["enforce", "shop/prod", "--to", "full", "--yes"])
        .assert()
        .code(6);
    assert!(stderr(&a).contains("write_refused"));
    env.cmd(&server)
        .args(["enforce", "shop/prod", "--to", "full"])
        .assert()
        .code(2);
    env.cmd(&server)
        .args(["enforce", "shop/prod", "--rollback"])
        .assert()
        .code(2);
    // No action at all, or two.
    env.cmd(&server)
        .args(["enforce", "shop/prod", "--yes"])
        .assert()
        .code(2);
    env.cmd(&server)
        .args(["enforce", "shop/prod", "--watch", "--rollback"])
        .assert()
        .code(2);
    assert!(server.received_requests().await.unwrap().is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn enforce_dry_run_of_a_ready_app_plans_the_move_and_sends_nothing() {
    let server = MockServer::start().await;
    mount_readiness(&server, shop_workloads(), "[]").await;
    mount_bulk(&server, &[]).await;
    let env = Env::new(ACCEPT_GAPS);
    let a = env
        .cmd(&server)
        .args(["enforce", "shop/prod", "--to", "full", "--dry-run"])
        .assert()
        .success();
    let out = json_out(&a);
    let d = &out["data"];
    assert_eq!(d["app"], "shop/prod");
    assert_eq!(d["dry_run"], true);
    assert_eq!(d["forced"], false);
    assert_eq!(d["to"], json!({"mode": "full", "visibility": null}));
    assert_eq!(d["readiness"]["verdict"], "ready");
    let p = &d["plan"];
    assert_eq!(p["count"], 2);
    assert_eq!(p["needs_confirm_count"], true);
    let planned: Vec<&str> = p["workloads"]
        .as_array()
        .unwrap()
        .iter()
        .map(|w| w["href"].as_str().unwrap())
        .collect();
    assert_eq!(planned, ["/orgs/5/workloads/w2", "/orgs/5/workloads/w3"]);
    let skipped: Vec<(&str, &str)> = p["skipped"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| (s["name"].as_str().unwrap(), s["reason"].as_str().unwrap()))
        .collect();
    assert_eq!(
        skipped,
        [("web-1", "already full"), ("web-4", "unmanaged")],
        "w9 (another app) is not in the plan"
    );
    assert_eq!(p["requests"][0]["path"], "/orgs/5/workloads/bulk_update");
    assert_eq!(sequence(&server).await, readiness_sequence());
    assert!(env.records().is_empty());
    // 4 object lists, 5 traffic requests, 3 reads; nothing written.
    assert_eq!(out["meta"]["api_calls"], 12);
    server.verify().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn enforce_yes_needs_the_typed_count_then_moves_and_writes_the_record() {
    let server = MockServer::start().await;
    mount_readiness(&server, shop_workloads(), "[]").await;
    mount_bulk(&server, &[]).await;
    let env = Env::new(ACCEPT_GAPS);
    // Full on two workloads: the count first (after readiness, before any
    // write).
    let a = env
        .cmd(&server)
        .args(["enforce", "shop/prod", "--to", "full", "--yes"])
        .assert()
        .code(2);
    assert!(stderr(&a).contains("--confirm-count"), "{}", stderr(&a));
    assert!(bulk_bodies(&server).await.is_empty());

    let server = MockServer::start().await;
    mount_readiness(&server, shop_workloads(), "[]").await;
    mount_bulk(&server, &[]).await;
    let a = env
        .cmd(&server)
        .args([
            "enforce",
            "shop/prod",
            "--to",
            "full",
            "--visibility",
            "flow_drops",
            "--yes",
            "--confirm-count",
            "3",
        ])
        .assert()
        .success();
    let out = json_out(&a);
    let mut want = readiness_sequence();
    want.push("PUT /orgs/5/workloads/bulk_update".into());
    assert_eq!(
        sequence(&server).await,
        want,
        "the selection was read moments ago: no re-read"
    );
    assert_eq!(
        bulk_bodies(&server).await,
        vec![json!([
            {"href": "/orgs/5/workloads/w1", "enforcement_mode": "full", "visibility_level": "flow_drops"},
            {"href": "/orgs/5/workloads/w2", "enforcement_mode": "full", "visibility_level": "flow_drops"},
            {"href": "/orgs/5/workloads/w3", "enforcement_mode": "full", "visibility_level": "flow_drops"},
        ])]
    );
    let d = &out["data"];
    // Three updated, the unmanaged one skipped.
    assert_eq!(d["bulk"]["items"].as_array().unwrap().len(), 4);
    let recs = env.records();
    assert_eq!(recs.len(), 1);
    let (p, rec) = &recs[0];
    assert_eq!(d["record"], p.display().to_string());
    assert!(
        p.file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with("shop_prod-")
    );
    assert_eq!(rec["app"], "shop/prod");
    assert_eq!(rec["org_id"], 5);
    assert_eq!(rec["profile"], "default");
    assert_eq!(
        rec["to"],
        json!({"mode": "full", "visibility": "flow_drops"})
    );
    assert_eq!(rec["readiness"]["verdict"], "ready");
    assert_eq!(rec["readiness"]["window"], "7d");
    assert_eq!(
        rec["workloads"],
        json!([
            {"href": "/orgs/5/workloads/w1", "hostname": "web-1",
             "before": {"mode": "full", "visibility": "flow_summary"},
             "after": {"mode": "full", "visibility": "flow_drops"}, "result": "updated"},
            {"href": "/orgs/5/workloads/w2", "hostname": "web-2",
             "before": {"mode": "visibility_only", "visibility": "flow_summary"},
             "after": {"mode": "full", "visibility": "flow_drops"}, "result": "updated"},
            {"href": "/orgs/5/workloads/w3", "hostname": "web-3",
             "before": {"mode": "visibility_only", "visibility": "flow_off"},
             "after": {"mode": "full", "visibility": "flow_drops"}, "result": "updated"},
        ])
    );
    let text = std::fs::read_to_string(p).unwrap();
    assert!(!text.contains("s3cr3t") && !text.contains("\"key\""));
    // Audited like any bulk write.
    let audit =
        std::fs::read_to_string(env.state.path().join("illumio-tui").join("writes.jsonl")).unwrap();
    assert_eq!(audit.lines().count(), 1);
    server.verify().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn enforce_force_moves_a_not_ready_app_and_a_partial_result_still_writes_a_record() {
    let server = MockServer::start().await;
    mount_readiness(&server, shop_workloads(), "[]").await;
    mount_bulk(&server, &["/orgs/5/workloads/w3"]).await;
    let env = Env::new("");
    let a = env
        .cmd(&server)
        .args([
            "enforce",
            "shop/prod",
            "--to",
            "selective",
            "--force",
            "--yes",
            "--confirm-count",
            "3",
        ])
        .assert()
        .code(4);
    let out = json_out(&a);
    assert_eq!(out["data"]["forced"], true);
    assert_eq!(out["data"]["readiness"]["verdict"], "needs_rules");
    assert!(
        out["meta"]["warnings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|w| w.as_str().unwrap().starts_with("--force"))
    );
    assert_eq!(bulk_bodies(&server).await.len(), 1);
    let recs = env.records();
    assert_eq!(recs.len(), 1, "partially successful: recorded");
    let rec = &recs[0].1;
    assert_eq!(rec["readiness"]["forced"], true);
    let results: Vec<(&str, &str)> = rec["workloads"]
        .as_array()
        .unwrap()
        .iter()
        .map(|w| {
            (
                w["hostname"].as_str().unwrap(),
                w["result"].as_str().unwrap(),
            )
        })
        .collect();
    assert_eq!(
        results,
        [
            ("web-1", "updated"),
            ("web-2", "updated"),
            ("web-3", "failed")
        ],
        "full → selective too"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn enforce_with_nothing_moved_keeps_the_record_with_the_results() {
    let env = Env::new(ACCEPT_GAPS);
    // Only w2 to move, and the PCE refuses it.
    let mut one = shop_workloads();
    one.as_array_mut().unwrap().remove(2);
    let server = MockServer::start().await;
    mount_readiness(&server, one, "[]").await;
    mount_bulk(&server, &["/orgs/5/workloads/w2"]).await;
    let a = env
        .cmd(&server)
        .args(["enforce", "shop/prod", "--to", "full", "--yes"])
        .assert()
        .code(4);
    assert_eq!(bulk_bodies(&server).await.len(), 1);
    let recs = env.records();
    assert_eq!(recs.len(), 1, "written before the send, kept");
    assert_eq!(
        json_out(&a)["data"]["record"],
        recs[0].0.display().to_string()
    );
    assert_eq!(recs[0].1["workloads"][0]["result"], "failed");
}

fn record_body(at: &str) -> Value {
    json!({
        "app": "shop/prod", "profile": "default", "pce": "127.0.0.1", "org_id": 5,
        "at": at, "to": {"mode": "full", "visibility": null},
        "readiness": {"verdict": "ready", "coverage": 1.0, "window": "7d", "flows": 412},
        "workloads": [
            {"href": "/orgs/5/workloads/w1", "hostname": "web-1",
             "before": {"mode": "visibility_only", "visibility": "flow_summary"},
             "after": {"mode": "full", "visibility": null}, "result": "updated"},
            {"href": "/orgs/5/workloads/w2", "hostname": "web-2",
             "before": {"mode": "idle", "visibility": "flow_off"},
             "after": {"mode": "full", "visibility": null}, "result": "updated"},
            {"href": "/orgs/5/workloads/w3", "hostname": "web-3",
             "before": {"mode": "visibility_only", "visibility": "flow_summary"},
             "after": {"mode": "full", "visibility": null}, "result": "updated"},
            {"href": "/orgs/5/workloads/w5", "hostname": "web-5",
             "before": {"mode": "visibility_only", "visibility": "flow_summary"},
             "after": {"mode": "full", "visibility": null}, "result": "updated"},
        ]
    })
}

fn recent(secs_ago: i64) -> String {
    let t = jiff::Timestamp::now() - jiff::SignedDuration::from_secs(secs_ago);
    jiff::Timestamp::from_second(t.as_second())
        .unwrap()
        .to_string()
}

#[tokio::test(flavor = "multi_thread")]
async fn watch_queries_blocked_flows_since_the_newest_record_and_exits_7() {
    let server = MockServer::start().await;
    mount_index(&server).await;
    mount_query(
        &server,
        include_str!("../../../fixtures/traffic_flows.json").into(),
        0,
    )
    .await;
    let env = Env::new("");
    let at = recent(3600);
    env.write_record(
        "shop_prod-20000101T000000.000Z.json",
        &record_body("2000-01-01T00:00:00Z"),
    );
    let newest = env.write_record("shop_prod-20991231T000000.000Z.json", &record_body(&at));
    let a = env
        .cmd(&server)
        .args(["enforce", "shop/prod", "--watch"])
        .assert()
        .code(7);
    assert!(
        stderr(&a).contains("blocked_since_enforcement"),
        "{}",
        stderr(&a)
    );
    let out = json_out(&a);
    let d = &out["data"];
    assert_eq!(d["since"], at);
    assert_eq!(d["record"], newest.display().to_string());
    assert_eq!(d["flows"], 1);
    assert_eq!(d["connections"], 3);
    assert_eq!(d["tuples"][0]["direction"], "in");
    assert_eq!(d["tuples"][0]["peer_text"], "unmanaged-nas/-");
    assert_eq!(d["tuples"][0]["service"], "icmp");
    assert_eq!(
        sequence(&server).await,
        vec![
            "POST /orgs/5/traffic_flows/async_queries".to_string(),
            format!("GET {QHREF}"),
            format!("GET {QHREF}/download"),
        ],
        "one query, no draft evaluation, no write"
    );
    let reqs = server.received_requests().await.unwrap();
    let body: Value = reqs
        .iter()
        .find(|r| r.method.as_str() == "POST")
        .map(|r| serde_json::from_slice(&r.body).unwrap())
        .unwrap();
    assert_eq!(body["start_date"], at);
    assert_eq!(body["policy_decisions"], json!(["blocked"]));
    assert_eq!(body["sources_destinations_query_op"], "or");
    let shop =
        json!([[{"label": {"href": "/orgs/5/labels/3"}}, {"label": {"href": "/orgs/5/labels/4"}}]]);
    assert_eq!(body["sources"]["include"], shop);
    assert_eq!(body["destinations"]["include"], shop);
    assert_eq!(out["meta"]["queries"], 1);
    server.verify().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn watch_exits_0_without_blocked_flows_or_when_marks_accept_them() {
    let at = recent(600);
    for (marks, flows) in [
        ("", "[]".to_string()),
        (
            ACCEPT_GAPS,
            include_str!("../../../fixtures/traffic_flows.json").to_string(),
        ),
    ] {
        let server = MockServer::start().await;
        mount_index(&server).await;
        mount_query(&server, flows, 0).await;
        let env = Env::new(marks);
        let a = env
            .cmd(&server)
            .args(["enforce", "shop/prod", "--watch", "--since", &at])
            .assert()
            .success();
        let d = &json_out(&a)["data"];
        assert_eq!(d["flows"], 0);
        assert!(d.get("record").is_none());
        assert_eq!(d["accepted"], u64::from(!marks.is_empty()));
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn watch_without_a_record_or_over_7d_refuses_before_any_request() {
    let server = MockServer::start().await;
    let env = Env::new("");
    let a = env
        .cmd(&server)
        .args(["enforce", "shop/prod", "--watch"])
        .assert()
        .code(2);
    assert!(stderr(&a).contains("no change record for shop/prod"));
    env.cmd(&server)
        .args([
            "enforce",
            "shop/prod",
            "--watch",
            "--since",
            &recent(9 * 86_400),
        ])
        .assert()
        .code(2);
    assert!(server.received_requests().await.unwrap().is_empty());
}

/// The rollback's read of the app: w1 and w2 still full (as set), w3
/// changed by someone else since, w5 gone.
async fn mount_rollback(server: &MockServer) {
    mount_index(server).await;
    Mock::given(method("GET"))
        .and(path("/api/v2/orgs/5/workloads"))
        .and(wiremock::matchers::query_param("labels", SHOP_LABELS))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            wl(1, "full", "flow_summary", true),
            wl(2, "full", "flow_off", true),
            wl(3, "selective", "flow_summary", true),
        ])))
        .expect(1)
        .mount(server)
        .await;
    mount_bulk(server, &[]).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn rollback_rereads_once_skips_the_changed_and_restores_the_rest_grouped() {
    let env = Env::new("");
    let rec = env.write_record(
        "shop_prod-20261019T100000.000Z.json",
        &record_body("2026-10-19T10:00:00Z"),
    );
    // The dry run: one read, nothing sent, no record.
    let server = MockServer::start().await;
    mount_rollback(&server).await;
    let a = env
        .cmd(&server)
        .args(["enforce", "shop/prod", "--rollback", "--dry-run"])
        .assert()
        .success();
    let d = json_out(&a)["data"].clone();
    assert_eq!(d["dry_run"], true);
    assert_eq!(d["count"], 2);
    assert_eq!(d["record"], rec.display().to_string());
    assert_eq!(sequence(&server).await, ["GET /orgs/5/workloads"]);
    assert_eq!(env.records().len(), 1);
    let skipped: Vec<(&str, &str)> = d["skipped"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| {
            (
                s["hostname"].as_str().unwrap(),
                s["reason"].as_str().unwrap(),
            )
        })
        .collect();
    assert_eq!(
        skipped,
        [
            (
                "web-3",
                "changed since (selective · flow_summary now, full set): left as it is"
            ),
            ("web-5", "not found among the app's workloads"),
        ]
    );

    // --yes: two bulk requests, one per (mode, visibility).
    let server = MockServer::start().await;
    mount_rollback(&server).await;
    let a = env
        .cmd(&server)
        .args(["enforce", "shop/prod", "--rollback", "--yes"])
        .assert()
        .success();
    let d = json_out(&a)["data"].clone();
    assert_eq!(
        sequence(&server).await,
        [
            "GET /orgs/5/workloads",
            "PUT /orgs/5/workloads/bulk_update",
            "PUT /orgs/5/workloads/bulk_update",
        ]
    );
    assert_eq!(
        bulk_bodies(&server).await,
        vec![
            // The move set no level: the rollback leaves the level alone.
            json!([{"href": "/orgs/5/workloads/w1", "enforcement_mode": "visibility_only"}]),
            json!([{"href": "/orgs/5/workloads/w2", "enforcement_mode": "idle"}]),
        ]
    );
    let recs = env.records();
    assert_eq!(recs.len(), 2, "the original is kept");
    let (p, rb) = recs
        .iter()
        .find(|(p, _)| p.to_string_lossy().ends_with("-rollback.json"))
        .unwrap();
    assert_eq!(d["rollback_record"], p.display().to_string());
    assert_eq!(rb["rollback_of"], rec.display().to_string());
    assert!(rb.get("to").is_none());
    let moved: Vec<(&str, &str, &str)> = rb["workloads"]
        .as_array()
        .unwrap()
        .iter()
        .map(|w| {
            (
                w["hostname"].as_str().unwrap(),
                w["after"]["mode"].as_str().unwrap(),
                w["result"].as_str().unwrap(),
            )
        })
        .collect();
    assert_eq!(
        moved,
        [
            ("web-1", "visibility_only", "updated"),
            ("web-2", "idle", "updated")
        ]
    );
    server.verify().await;

    // A second rollback of the newest record is refused before any
    // request; naming it with --record goes ahead.
    let server = MockServer::start().await;
    let a = env
        .cmd(&server)
        .args(["enforce", "shop/prod", "--rollback", "--dry-run"])
        .assert()
        .code(6);
    assert!(
        stderr(&a).contains("already rolled back by"),
        "{}",
        stderr(&a)
    );
    assert!(server.received_requests().await.unwrap().is_empty());
    let server = MockServer::start().await;
    mount_rollback(&server).await;
    let a = env
        .cmd(&server)
        .args([
            "enforce",
            "shop/prod",
            "--rollback",
            "--dry-run",
            "--record",
        ])
        .arg(&rec)
        .assert()
        .success();
    assert_eq!(json_out(&a)["data"]["record"], rec.display().to_string());

    // Rolling back the rollback re-applies the move: only with --force.
    let server = MockServer::start().await;
    let a = env
        .cmd(&server)
        .args([
            "enforce",
            "shop/prod",
            "--rollback",
            "--dry-run",
            "--record",
        ])
        .arg(p)
        .assert()
        .code(6);
    assert!(
        stderr(&a).contains("is a rollback record"),
        "{}",
        stderr(&a)
    );
    assert!(server.received_requests().await.unwrap().is_empty());
    let server = MockServer::start().await;
    mount_rollback(&server).await;
    env.cmd(&server)
        .args([
            "enforce",
            "shop/prod",
            "--rollback",
            "--dry-run",
            "--force",
            "--record",
        ])
        .arg(p)
        .assert()
        .success();
}

#[tokio::test(flavor = "multi_thread")]
async fn rollback_refuses_another_pces_record_and_a_missing_one_before_any_request() {
    let server = MockServer::start().await;
    let env = Env::new("");
    let a = env
        .cmd(&server)
        .args(["enforce", "shop/prod", "--rollback", "--yes"])
        .assert()
        .code(2);
    assert!(stderr(&a).contains("no change record"));
    let mut body = record_body("2026-10-19T10:00:00Z");
    body["pce"] = json!("other.example");
    let p = env.write_record("elsewhere.json", &body);
    let a = env
        .cmd(&server)
        .args(["enforce", "shop/prod", "--rollback", "--yes", "--record"])
        .arg(&p)
        .assert()
        .code(6);
    assert!(stderr(&a).contains("other.example"), "{}", stderr(&a));
    assert!(server.received_requests().await.unwrap().is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn tables_print_and_csv_is_refused_for_a_write() {
    let env = Env::new(ACCEPT_GAPS);
    let server = MockServer::start().await;
    mount_readiness(&server, shop_workloads(), "[]").await;
    let a = env
        .bare(&server)
        .args([
            "--table",
            "enforce",
            "shop/prod",
            "--to",
            "full",
            "--dry-run",
        ])
        .assert()
        .success();
    let text = String::from_utf8_lossy(&a.get_output().stdout).to_string();
    for want in [
        "readiness · shop/prod",
        "ready",
        "visibility_only → full",
        "requests",
    ] {
        assert!(text.contains(want), "{want:?} in {text}");
    }
    let server = MockServer::start().await;
    mount_index(&server).await;
    mount_query(
        &server,
        include_str!("../../../fixtures/traffic_flows.json").into(),
        0,
    )
    .await;
    let a = env
        .bare(&server)
        .args([
            "--table",
            "enforce",
            "shop/prod",
            "--watch",
            "--since",
            &recent(60),
        ])
        .assert()
        .success();
    let text = String::from_utf8_lossy(&a.get_output().stdout).to_string();
    assert!(text.contains("blocked since"), "{text}");
    assert!(text.contains("1 accepted by marks"), "{text}");
    let server = MockServer::start().await;
    env.bare(&server)
        .args(["--csv", "enforce", "shop/prod", "--rollback", "--dry-run"])
        .assert()
        .code(2);
    assert!(server.received_requests().await.unwrap().is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn an_unwritable_records_dir_refuses_before_any_request() {
    let env = Env::new(ACCEPT_GAPS);
    std::fs::create_dir_all(env.state.path().join("illumio-tui")).unwrap();
    std::fs::write(env.records_dir(), "not a directory").unwrap();
    let server = MockServer::start().await;
    for args in [
        vec!["enforce", "shop/prod", "--to", "full", "--yes"],
        vec!["enforce", "shop/prod", "--rollback", "--yes"],
    ] {
        let a = env.cmd(&server).args(&args).assert().code(6);
        let err = stderr(&a);
        assert!(err.contains("change records cannot be written"), "{err}");
        assert!(err.contains("nothing sent"), "{err}");
    }
    assert!(server.received_requests().await.unwrap().is_empty());
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn a_record_that_cannot_be_updated_after_the_send_goes_to_stderr_loudly() {
    use std::os::unix::fs::PermissionsExt;
    let env = Env::new(ACCEPT_GAPS);
    let server = MockServer::start().await;
    mount_readiness(&server, shop_workloads(), "[]").await;
    // The records directory turns read-only while the bulk request is
    // answered: the record before the send exists, its update fails.
    let dir = env.records_dir();
    let lock = dir.clone();
    let ok = bulk_answer(&[]);
    Mock::given(method("PUT"))
        .and(path(BULK))
        .respond_with(move |req: &Request| {
            std::fs::set_permissions(&lock, std::fs::Permissions::from_mode(0o555)).unwrap();
            ok(req)
        })
        .mount(&server)
        .await;
    let a = env
        .cmd(&server)
        .args([
            "enforce",
            "shop/prod",
            "--to",
            "full",
            "--yes",
            "--confirm-count",
            "2",
        ])
        .assert()
        .code(1);
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
    let err = stderr(&a);
    assert!(err.contains("CHANGE RECORD NOT UPDATED"), "{err}");
    assert!(
        err.contains("\"result\": \"updated\""),
        "the record with results: {err}"
    );
    let out = json_out(&a);
    let updated = out["data"]["bulk"]["items"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|i| i["status"] == "updated")
        .count();
    assert_eq!(updated, 2);
    // The record before the send is there, every workload pending.
    let recs = env.records();
    assert_eq!(recs.len(), 1);
    assert!(
        recs[0].1["workloads"]
            .as_array()
            .unwrap()
            .iter()
            .all(|w| w["result"] == "pending")
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn an_unanswered_bulk_is_recorded_and_rolled_back_where_it_applied() {
    let env = Env::new(ACCEPT_GAPS);
    let server = MockServer::start().await;
    mount_readiness(&server, shop_workloads(), "[]").await;
    Mock::given(method("PUT"))
        .and(path(BULK))
        .respond_with(ResponseTemplate::new(502))
        .mount(&server)
        .await;
    env.cmd(&server)
        .args([
            "enforce",
            "shop/prod",
            "--to",
            "full",
            "--yes",
            "--confirm-count",
            "2",
        ])
        .assert()
        .code(4);
    let recs = env.records();
    assert_eq!(recs.len(), 1, "unanswered: it may have applied");
    let results: Vec<&str> = recs[0].1["workloads"]
        .as_array()
        .unwrap()
        .iter()
        .map(|w| w["result"].as_str().unwrap())
        .collect();
    assert_eq!(results, ["unanswered", "unanswered"]);
    // It applied to w2, not to w3.
    let server = MockServer::start().await;
    mount_index(&server).await;
    Mock::given(method("GET"))
        .and(path("/api/v2/orgs/5/workloads"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            wl(2, "full", "flow_summary", true),
            wl(3, "visibility_only", "flow_off", true),
        ])))
        .mount(&server)
        .await;
    let a = env
        .cmd(&server)
        .args(["enforce", "shop/prod", "--rollback", "--dry-run"])
        .assert()
        .success();
    let d = &json_out(&a)["data"];
    assert_eq!(d["count"], 1);
    assert_eq!(
        d["groups"][0]["plan"]["workloads"][0]["href"],
        "/orgs/5/workloads/w2"
    );
    assert_eq!(
        d["skipped"][0]["reason"],
        "not applied (not answered: still visibility_only · flow_off)"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn pending_items_after_a_crash_are_restored_where_applied() {
    let env = Env::new("");
    let mut body = record_body("2026-10-19T10:00:00Z");
    for w in body["workloads"].as_array_mut().unwrap() {
        w["result"] = json!("pending");
    }
    env.write_record("shop_prod-20261019T100000.000Z.json", &body);
    let server = MockServer::start().await;
    mount_index(&server).await;
    Mock::given(method("GET"))
        .and(path("/api/v2/orgs/5/workloads"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            wl(1, "full", "flow_summary", true),
            wl(2, "idle", "flow_off", true),
        ])))
        .mount(&server)
        .await;
    let a = env
        .cmd(&server)
        .args(["enforce", "shop/prod", "--rollback", "--dry-run"])
        .assert()
        .success();
    let d = &json_out(&a)["data"];
    assert_eq!(d["count"], 1);
    let reasons: Vec<&str> = d["skipped"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["reason"].as_str().unwrap())
        .collect();
    assert_eq!(
        reasons,
        [
            "not applied (the send did not finish: still idle · flow_off)",
            "not found among the app's workloads",
            "not found among the app's workloads",
        ]
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn rollback_without_writes_or_with_a_wrong_count_sends_nothing() {
    let env = Env::new("");
    env.write_record(
        "shop_prod-20261019T100000.000Z.json",
        &record_body("2026-10-19T10:00:00Z"),
    );
    let server = MockServer::start().await;
    let a = env
        .cmd(&server)
        .env_remove("ILLUMIO_WRITES")
        .args(["enforce", "shop/prod", "--rollback", "--yes"])
        .assert()
        .code(6);
    assert!(stderr(&a).contains("write_refused"));
    assert!(server.received_requests().await.unwrap().is_empty());
    let server = MockServer::start().await;
    mount_rollback(&server).await;
    let a = env
        .cmd(&server)
        .args([
            "enforce",
            "shop/prod",
            "--rollback",
            "--yes",
            "--confirm-count",
            "5",
        ])
        .assert()
        .code(2);
    assert!(
        stderr(&a).contains("2 workloads would be restored"),
        "{}",
        stderr(&a)
    );
    assert_eq!(sequence(&server).await, ["GET /orgs/5/workloads"]);
}

#[tokio::test(flavor = "multi_thread")]
async fn watch_with_nothing_found_in_a_truncated_result_exits_4() {
    let server = MockServer::start().await;
    mount_index(&server).await;
    mount_query(&server, "[]".into(), 0).await;
    let env = Env::new("");
    let a = env
        .cmd(&server)
        .args([
            "enforce",
            "shop/prod",
            "--watch",
            "--since",
            &recent(600),
            "--max-results",
            "3",
        ])
        .assert()
        .code(4);
    assert!(stderr(&a).contains("truncated"), "{}", stderr(&a));
    assert_eq!(json_out(&a)["meta"]["truncated"], true);
}
