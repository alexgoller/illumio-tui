//! `policy impact` and `policy provision --impact` (#86) against a mock
//! PCE: the three sections, every request counted (one pending read,
//! nothing else when nothing is pending; the ruleset, list and workload
//! reads only when a ruleset is pending; one draft query), the CSV pair
//! rows, `--fail-on-blocked`, the cost guard before any request, and the
//! summary in front of a provision request with its targets marked. The
//! helpers repeat those of the other CLI tests (test files share no code
//! without a module).
#[path = "common/schema_check.rs"]
mod schema_check;
use assert_cmd::Command;
use serde_json::{Value, json};
use std::path::Path;
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

const QHREF: &str = "/orgs/5/traffic_flows/async_queries/q-1";

fn cmd(server: &MockServer, home: &Path) -> Command {
    let url = url::Url::parse(&server.uri()).unwrap();
    let mut c = Command::cargo_bin("illumio-tui").unwrap();
    c.env_clear()
        .env("PATH", std::env::var("PATH").unwrap_or_default())
        .env("PCE_SCHEME", "http")
        .env("PCE_HOST", url.host_str().unwrap())
        .env("PCE_PORT", url.port().unwrap().to_string())
        .env("PCE_ORG_ID", "5")
        .env("API_KEY", "key")
        .env("API_SECRET", "s3cr3t-impact-test")
        .env("XDG_CONFIG_HOME", home)
        .env("XDG_CACHE_HOME", home.join("cache"))
        .env("XDG_STATE_HOME", home.join("state"))
        .env("XDG_DATA_HOME", home.join("data"));
    c
}

fn answer(body: &str) -> ResponseTemplate {
    ResponseTemplate::new(200)
        .set_body_string(body)
        .insert_header("Content-Type", "application/json")
}

/// A status answer whose `updated_at` moves on every call, as the PCE's
/// does while it works: a draft `completed` after the PUT is then new.
struct Ticking {
    body: Value,
    n: std::sync::atomic::AtomicU64,
}

impl wiremock::Respond for Ticking {
    fn respond(&self, _: &wiremock::Request) -> ResponseTemplate {
        let n = self.n.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let mut body = self.body.clone();
        body["updated_at"] = json!(format!("2026-10-28T00:00:{:02}Z", n % 60));
        ResponseTemplate::new(200).set_body_json(body)
    }
}

/// The fixture's flows after `update_rules`: the potentially blocked one
/// allowed in draft, the allowed 443 one blocked, the rest kept.
fn draft_flows() -> String {
    let mut flows: Vec<Value> =
        serde_json::from_str(include_str!("../../../fixtures/traffic_flows.json")).unwrap();
    for f in &mut flows {
        let draft = match (f["policy_decision"].as_str(), f["service"]["port"].as_u64()) {
            (Some("potentially_blocked"), _) => "allowed",
            (Some("allowed"), Some(443)) => "blocked",
            (Some(d), _) => d,
            _ => unreachable!(),
        };
        f["draft_policy_decision"] = json!(draft);
    }
    serde_json::to_string(&flows).unwrap()
}

/// Shop-Prod as provisioned: rule 2 from role=db instead of Corp; no
/// Global-Infra yet.
fn active_rule_sets() -> String {
    let text = include_str!("../../../fixtures/rule_sets.json")
        .replace("/sec_policy/draft/", "/sec_policy/active/");
    let mut v: Vec<Value> = serde_json::from_str(&text).unwrap();
    v.truncate(1);
    v[0]["update_type"] = json!(null);
    v[0]["rules"][1]["consumers"] = json!([{"label": {"href": "/orgs/5/labels/2"}}]);
    serde_json::to_string(&v).unwrap()
}

fn pending_body(rulesets: bool) -> Value {
    let mut p = vec![
        json!({"href": "/orgs/5/sec_policy/draft/ip_lists/21", "change_type": "create", "name": "Corp"}),
    ];
    if rulesets {
        p.push(json!({"href": "/orgs/5/sec_policy/draft/rule_sets/101", "change_type": "create", "name": "Global-Infra"}));
        p.push(json!({"href": "/orgs/5/sec_policy/draft/rule_sets/100/sec_rules/2", "change_type": "update"}));
    }
    Value::Array(p)
}

/// Every request the summary may make, each pinned: `pending` the pending
/// list; `lists` the name index's four lists; `rules` the draft and active
/// rulesets, active label groups and IP lists and the workloads; `queries`
/// the draft query (POST, PUT, download).
async fn mount(server: &MockServer, pending: Value, lists: u64, rules: u64, queries: u64) {
    Mock::given(method("GET"))
        .and(path("/api/v2/orgs/5/sec_policy/pending"))
        .respond_with(ResponseTemplate::new(200).set_body_json(pending))
        .expect(1)
        .mount(server)
        .await;
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
            .expect(lists)
            .mount(server)
            .await;
    }
    let active = active_rule_sets();
    for (p, body) in [
        (
            "/api/v2/orgs/5/sec_policy/draft/rule_sets",
            include_str!("../../../fixtures/rule_sets.json"),
        ),
        (
            "/api/v2/orgs/5/sec_policy/active/rule_sets",
            active.as_str(),
        ),
        (
            "/api/v2/orgs/5/sec_policy/active/label_groups",
            include_str!("../../../fixtures/label_groups.json"),
        ),
        ("/api/v2/orgs/5/sec_policy/active/ip_lists", "[]"),
        (
            "/api/v2/orgs/5/workloads",
            include_str!("../../../fixtures/workloads.json"),
        ),
    ] {
        Mock::given(method("GET"))
            .and(path(p))
            .respond_with(answer(body))
            .expect(rules)
            .mount(server)
            .await;
    }
    Mock::given(method("POST"))
        .and(path("/api/v2/orgs/5/traffic_flows/async_queries"))
        .respond_with(
            ResponseTemplate::new(202).set_body_json(json!({"href": QHREF, "status": "queued"})),
        )
        .expect(queries)
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
        .and(query_param("label_based_rules", "false"))
        .respond_with(ResponseTemplate::new(204))
        .expect(queries)
        .mount(server)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("/api/v2{QHREF}/download")))
        .respond_with(answer(&draft_flows()))
        .expect(queries)
        .mount(server)
        .await;
}

fn envelope(a: &assert_cmd::assert::Assert) -> Value {
    schema_check::envelope(&a.get_output().stdout)
}

fn stdout(a: &assert_cmd::assert::Assert) -> String {
    String::from_utf8_lossy(&a.get_output().stdout).into_owned()
}

fn stderr(a: &assert_cmd::assert::Assert) -> String {
    String::from_utf8_lossy(&a.get_output().stderr).into_owned()
}

#[tokio::test(flavor = "multi_thread")]
async fn three_sections_from_one_pending_read_and_one_draft_query() {
    let server = MockServer::start().await;
    mount(&server, pending_body(true), 1, 1, 1).await;
    let home = tempfile::tempdir().unwrap();
    let a = cmd(&server, home.path())
        .args(["--json", "policy", "impact"])
        .assert()
        .success();
    let v = envelope(&a);
    let d = &v["data"];
    assert_eq!(v["meta"]["schema"], "policy-impact");
    assert_eq!(d["pending"], 3);
    let changes: Vec<(String, u64)> = d["changes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| {
            (
                format!(
                    "{} {}",
                    c["object_type"].as_str().unwrap(),
                    c["change_type"].as_str().unwrap()
                ),
                c["count"].as_u64().unwrap(),
            )
        })
        .collect();
    assert_eq!(
        changes,
        [
            ("ip_lists create".to_string(), 1),
            ("rule_sets create".to_string(), 1),
            ("sec_rules update".to_string(), 1)
        ]
    );
    // Global-Infra is new (its rule added, disabled: in force nowhere);
    // Shop-Prod's rule 2 now comes from Corp, not role=db.
    let sets = d["rulesets"].as_array().unwrap();
    assert_eq!(sets[0]["name"], "Global-Infra");
    assert_eq!(sets[0]["added"], 1);
    assert_eq!(sets[1]["name"], "Shop-Prod");
    assert_eq!(sets[1]["change"], "update");
    let r2 = &sets[1]["rules"][0];
    assert_eq!(r2["change"], "changed");
    assert!(r2["was"].as_str().unwrap().contains("role=db"), "{r2}");
    assert_eq!(r2["active"]["consumers"], 1);
    assert_eq!(r2["draft"]["consumers"], 0);
    // The traffic: 443 allowed → blocked, the potentially blocked flow
    // allowed in draft.
    let t = &d["traffic"];
    assert_eq!(t["window"], "1d");
    assert_eq!(t["flows"], 4);
    assert_eq!(t["transitions"]["newly_blocked"], 1);
    assert_eq!(t["transitions"]["newly_allowed"], 1);
    assert_eq!(t["transitions"]["unchanged"], 2);
    assert_eq!(t["blocked_pairs_total"], 1);
    assert_eq!(t["blocked_pairs"][0]["services"], json!(["443/tcp"]));
    assert_eq!(t["allowed_pairs_total"], 1);
    let m = &v["meta"];
    assert!(m["cost"].as_str().unwrap().contains("1 PUT update_rules"));
    assert_eq!(m["draft"]["newly_blocked"], 1);
    assert!(m["window"]["start"].is_string());
    // pending 1, four lists, five policy reads, POST, status polls, PUT,
    // download: at least 13.
    assert!(m["api_calls"].as_u64().unwrap() >= 13, "{m}");
}

#[tokio::test(flavor = "multi_thread")]
async fn nothing_pending_is_said_and_sends_no_query() {
    let server = MockServer::start().await;
    mount(&server, json!([]), 0, 0, 0).await;
    let home = tempfile::tempdir().unwrap();
    let a = cmd(&server, home.path())
        .args(["--json", "policy", "impact", "--fail-on-blocked"])
        .assert()
        .success();
    let v = envelope(&a);
    assert_eq!(v["data"]["pending"], 0);
    assert_eq!(v["data"]["traffic"], Value::Null);
    assert_eq!(v["meta"]["api_calls"], 1);
    assert!(
        v["meta"]["cost"]
            .as_str()
            .unwrap()
            .contains("nothing pending")
    );
    let server = MockServer::start().await;
    mount(&server, json!([]), 0, 0, 0).await;
    let t = cmd(&server, home.path())
        .args(["--table", "policy", "impact"])
        .assert()
        .success();
    assert!(stdout(&t).contains("nothing pending"), "{}", stdout(&t));
}

#[tokio::test(flavor = "multi_thread")]
async fn without_a_pending_ruleset_no_ruleset_or_workload_is_read() {
    let server = MockServer::start().await;
    mount(&server, pending_body(false), 1, 0, 1).await;
    let home = tempfile::tempdir().unwrap();
    let a = cmd(&server, home.path())
        .args(["--json", "policy", "impact"])
        .assert()
        .success();
    let v = envelope(&a);
    assert_eq!(v["data"]["rulesets"], json!([]));
    assert_eq!(v["data"]["traffic"]["transitions"]["evaluated"], 4);
    assert!(
        v["data"]["not_counted"]
            .as_array()
            .unwrap()
            .iter()
            .any(|n| n
                .as_str()
                .unwrap()
                .contains("label groups or IP lists are pending"))
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn fail_on_blocked_exits_7_after_the_report() {
    let server = MockServer::start().await;
    mount(&server, pending_body(true), 1, 1, 1).await;
    let home = tempfile::tempdir().unwrap();
    let a = cmd(&server, home.path())
        .args(["--table", "policy", "impact", "--fail-on-blocked"])
        .assert()
        .code(7);
    let out = stdout(&a);
    for s in [
        "changes · 3 pending changes",
        "rules · 2 pending rulesets · 1 added · 0 removed · 1 changed",
        "was: allow",
        "traffic · 1d · 4 flow records · 1 newly blocked · 1 newly allowed",
        "allowed → blocked",
        "newly blocked or potentially blocked · 1 pair",
        "newly allowed · 1 pair",
        "not counted:",
    ] {
        assert!(out.contains(s), "{s:?} missing:\n{out}");
    }
    assert!(stderr(&a).contains("would_block") || stderr(&a).contains("would become blocked"));
}

#[tokio::test(flavor = "multi_thread")]
async fn csv_prints_the_pair_rows_of_both_directions() {
    let server = MockServer::start().await;
    mount(&server, pending_body(false), 1, 0, 1).await;
    let home = tempfile::tempdir().unwrap();
    let a = cmd(&server, home.path())
        .args(["--csv", "policy", "impact"])
        .assert()
        .success();
    let out = stdout(&a);
    let mut lines = out.lines();
    assert_eq!(
        lines.next().unwrap(),
        "direction,src,dst,services,flows,connections,transitions"
    );
    let rows: Vec<&str> = lines.collect();
    assert_eq!(rows.len(), 2, "{out}");
    assert!(rows[0].starts_with("newly_blocked,"));
    assert!(rows[0].ends_with("allowed → blocked 1"), "{}", rows[0]);
    assert!(rows[1].starts_with("newly_allowed,"));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_long_window_is_refused_before_any_request() {
    let server = MockServer::start().await;
    mount(&server, json!([]), 0, 0, 0).await;
    // The pending read too: nothing at all.
    let home = tempfile::tempdir().unwrap();
    let a = cmd(&server, home.path())
        .args(["--json", "policy", "impact", "--window", "30d"])
        .assert()
        .code(2);
    assert!(stderr(&a).contains("--yes"), "{}", stderr(&a));
    assert!(server.received_requests().await.unwrap().is_empty());
    // With --yes it runs: one pending read, and nothing is pending.
    cmd(&server, home.path())
        .args(["--json", "policy", "impact", "--window", "30d", "--yes"])
        .assert()
        .success();
}

#[tokio::test(flavor = "multi_thread")]
async fn schema_and_fields_name_the_sections() {
    let server = MockServer::start().await;
    mount(&server, pending_body(false), 1, 0, 1).await;
    let home = tempfile::tempdir().unwrap();
    let a = cmd(&server, home.path())
        .args([
            "--json",
            "--fields",
            "pending,traffic.transitions",
            "policy",
            "impact",
        ])
        .assert()
        .success();
    let v = schema_check::picked(&a.get_output().stdout);
    let keys: Vec<&String> = v["data"].as_object().unwrap().keys().collect();
    assert_eq!(keys, ["pending", "traffic"]);
    assert!(v["data"]["traffic"]["transitions"]["changed"].is_array());
    let s = cmd(&server, home.path())
        .args(["schema", "policy", "impact"])
        .assert()
        .success();
    let doc: Value = serde_json::from_slice(&s.get_output().stdout).unwrap();
    assert!(
        doc["$comment"]
            .as_str()
            .unwrap()
            .contains("direction, src, dst, services")
    );
}

/// `policy provision --dry-run --impact`: the summary, then the request,
/// the pending list read once (the plan's read, past the cache, fills it).
#[tokio::test(flavor = "multi_thread")]
async fn provision_dry_run_prints_the_summary_in_front_of_the_request() {
    let server = MockServer::start().await;
    mount(&server, pending_body(true), 1, 1, 1).await;
    let home = tempfile::tempdir().unwrap();
    let a = cmd(&server, home.path())
        .env("ILLUMIO_WRITES", "1")
        .args([
            "--json",
            "policy",
            "provision",
            "/orgs/5/sec_policy/draft/ip_lists/21",
            "--dry-run",
            "--impact",
        ])
        .assert()
        .success();
    let v = envelope(&a);
    let d = &v["data"];
    assert_eq!(v["meta"]["schema"], "write");
    assert_eq!(d["method"], "POST");
    assert_eq!(d["counts"]["ip_lists"], 1);
    let marked: Vec<(String, u64)> = d["impact"]["changes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| {
            (
                c["object_type"].as_str().unwrap().to_string(),
                c["in_request"].as_u64().unwrap(),
            )
        })
        .collect();
    assert_eq!(
        marked,
        [
            ("ip_lists".to_string(), 1),
            ("rule_sets".to_string(), 0),
            ("sec_rules".to_string(), 0)
        ]
    );
    assert_eq!(d["impact"]["traffic"]["transitions"]["newly_blocked"], 1);
    let w = v["meta"]["warnings"].as_array().unwrap();
    assert!(w[0].as_str().unwrap().contains("whole draft"), "{w:?}");
    // A table: the summary first, then the request.
    let server = MockServer::start().await;
    mount(&server, pending_body(true), 1, 1, 1).await;
    let t = cmd(&server, home.path())
        .env("ILLUMIO_WRITES", "1")
        .args([
            "--table",
            "policy",
            "provision",
            "--all",
            "--dry-run",
            "--impact",
        ])
        .assert()
        .success();
    let out = stdout(&t);
    let at = |s: &str| {
        out.find(s)
            .unwrap_or_else(|| panic!("{s:?} missing:\n{out}"))
    };
    assert!(at("changes · 3 pending changes") < at("method"));
    assert!(at("traffic · 1d") < at("counts"));
}

/// Without `--impact` a provision dry run reads nothing more.
#[tokio::test(flavor = "multi_thread")]
async fn provision_without_impact_reads_nothing_more() {
    let server = MockServer::start().await;
    mount(&server, pending_body(true), 0, 0, 0).await;
    let home = tempfile::tempdir().unwrap();
    let a = cmd(&server, home.path())
        .env("ILLUMIO_WRITES", "1")
        .args(["--json", "policy", "provision", "--all", "--dry-run"])
        .assert()
        .success();
    let v = envelope(&a);
    assert!(v["data"].get("impact").is_none());
}
