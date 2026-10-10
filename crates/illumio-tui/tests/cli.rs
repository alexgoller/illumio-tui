use assert_cmd::Command;
use predicates::prelude::*;
use serde_json::{Value, json};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// Distinctive so a leak check cannot trip on the word "secret" in prose
/// such as "check API key, secret and org id".
const TEST_SECRET: &str = "s3cr3t-cli-test-9f2a";

pub fn cmd(server: &MockServer) -> Command {
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
        .env(
            "XDG_CONFIG_HOME",
            std::env::temp_dir().join("illumio-tui-no-config"),
        );
    c
}

pub fn stdout_json(assert: assert_cmd::assert::Assert) -> Value {
    serde_json::from_slice(&assert.get_output().stdout).expect("stdout is JSON")
}

/// No output may carry the API secret the command was given.
pub fn assert_no_secret(assert: &assert_cmd::assert::Assert) {
    let out = assert.get_output();
    for (name, bytes) in [("stdout", &out.stdout), ("stderr", &out.stderr)] {
        let text = String::from_utf8_lossy(bytes);
        assert!(
            !text.contains(TEST_SECRET),
            "{name} leaks the secret: {text}"
        );
    }
}

#[test]
fn every_flag_has_help_text() {
    for args in [
        vec!["--help"],
        vec!["workloads", "--help"],
        vec!["traffic", "query", "--help"],
        vec!["policy", "rulesets", "--help"],
        vec!["policy", "rules", "--help"],
        vec!["labels", "add", "--help"],
        vec!["labels", "update", "--help"],
        vec!["labels", "delete", "--help"],
        vec!["iplists", "add", "--help"],
        vec!["iplists", "update", "--help"],
        vec!["iplists", "delete", "--help"],
        vec!["services", "add", "--help"],
        vec!["services", "update", "--help"],
        vec!["services", "delete", "--help"],
    ] {
        let out = Command::cargo_bin("illumio-tui")
            .unwrap()
            .env_clear()
            .args(&args)
            .output()
            .unwrap();
        let text = String::from_utf8_lossy(&out.stdout);
        let lines: Vec<&str> = text.lines().collect();
        for (i, line) in lines.iter().enumerate() {
            let t = line.trim_start();
            // clap prints an option line as `--flag <VALUE>  description`, or
            // with the description on the next line when flags are long; a
            // bare flag with neither has no doc comment.
            if t.starts_with("--") {
                let inline = t.split_once("  ").map(|(_, d)| d.trim()).unwrap_or("");
                let next = lines.get(i + 1).map(|l| l.trim()).unwrap_or("");
                let wrapped = !next.is_empty() && !next.starts_with('-');
                assert!(!inline.is_empty() || wrapped, "{args:?}: no help for {t:?}");
            }
        }
    }
}

#[test]
fn version_and_no_subcommand() {
    Command::cargo_bin("illumio-tui")
        .unwrap()
        .arg("--version")
        .assert()
        .success()
        .stdout(predicate::str::starts_with("illumio-tui "));
    let assert = Command::cargo_bin("illumio-tui")
        .unwrap()
        .env_clear()
        .env(
            "XDG_CONFIG_HOME",
            std::env::temp_dir().join("illumio-tui-no-config"),
        )
        .assert()
        .code(2);
    let err: Value = serde_json::from_slice(&assert.get_output().stderr).unwrap();
    assert!(
        err["error"]["message"]
            .as_str()
            .unwrap()
            .contains("PCE_HOST")
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn health_json_envelope() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v2/health"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!([{"status": "normal", "type": "cluster"}])),
        )
        .mount(&server)
        .await;
    let assert = cmd(&server).args(["health", "--json"]).assert().success();
    assert_no_secret(&assert);
    let out = stdout_json(assert);
    assert_eq!(out["data"]["ok"], true);
    assert_eq!(out["data"]["org_id"], 5);
    assert_eq!(out["data"]["status"][0]["status"], "normal");
    assert_eq!(out["meta"]["profile"], "default");
    assert_eq!(out["meta"]["api_calls"], 1);
    assert_eq!(out["meta"]["cache"], "none");
    assert_eq!(out["meta"]["truncated"], false);
    assert!(out["meta"]["elapsed_ms"].is_number());
}

#[tokio::test(flavor = "multi_thread")]
async fn health_table_output() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v2/health"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"status": "normal"})))
        .mount(&server)
        .await;
    cmd(&server)
        .args(["health", "--table"])
        .assert()
        .success()
        .stdout(
            predicate::str::contains("field")
                .and(predicate::str::contains("ok"))
                .and(predicate::str::contains("true")),
        )
        .stderr(predicate::str::contains("1 api call"));
}

#[tokio::test(flavor = "multi_thread")]
async fn json_is_default_when_stdout_is_not_a_tty() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v2/health"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
        .mount(&server)
        .await;
    let out = stdout_json(cmd(&server).arg("health").assert().success());
    assert_eq!(out["data"]["ok"], true);
}

#[tokio::test(flavor = "multi_thread")]
async fn auth_failure_exits_3_with_json_error() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v2/health"))
        .respond_with(ResponseTemplate::new(401))
        .mount(&server)
        .await;
    let assert = cmd(&server).arg("health").assert().code(3);
    assert_no_secret(&assert);
    let err: Value = serde_json::from_slice(&assert.get_output().stderr).unwrap();
    assert_eq!(err["error"]["kind"], "auth");
    assert!(err["error"]["message"].as_str().unwrap().contains("401"));
}

#[test]
fn missing_config_exits_2() {
    let assert = Command::cargo_bin("illumio-tui")
        .unwrap()
        .env_clear()
        .env(
            "XDG_CONFIG_HOME",
            std::env::temp_dir().join("illumio-tui-no-config"),
        )
        .arg("health")
        .assert()
        .code(2);
    let err: Value = serde_json::from_slice(&assert.get_output().stderr).unwrap();
    assert_eq!(err["error"]["kind"], "config");
    assert!(
        err["error"]["message"]
            .as_str()
            .unwrap()
            .contains("PCE_HOST")
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn env_file_flag_supplies_credentials() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v2/health"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
        .mount(&server)
        .await;
    let url = url::Url::parse(&server.uri()).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let env_path = dir.path().join("pce.env");
    std::fs::write(
        &env_path,
        format!(
            "PCE_SCHEME=http\nPCE_HOST={}\nPCE_PORT={}\nPCE_ORG_ID=5\nAPI_KEY=k\nAPI_SECRET=s\n",
            url.host_str().unwrap(),
            url.port().unwrap()
        ),
    )
    .unwrap();
    let out = stdout_json(
        Command::cargo_bin("illumio-tui")
            .unwrap()
            .env_clear()
            .env("XDG_CONFIG_HOME", dir.path())
            .args(["--env-file", env_path.to_str().unwrap(), "health"])
            .assert()
            .success(),
    );
    assert_eq!(out["data"]["ok"], true);
}

async fn mount_objects(server: &MockServer) {
    mount_objects_expecting(server, None).await;
}

/// The four object lists, each with an optional exact call count.
async fn mount_objects_expecting(server: &MockServer, expect: Option<u64>) {
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
        let mock = Mock::given(method("GET")).and(path(p)).respond_with(
            ResponseTemplate::new(200)
                .set_body_string(body)
                .insert_header("Content-Type", "application/json"),
        );
        match expect {
            Some(n) => mock.expect(n).mount(server).await,
            None => mock.mount(server).await,
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn labels_filter_exactly_and_hide_deleted() {
    let server = MockServer::start().await;
    mount_objects(&server).await;
    let out = stdout_json(
        cmd(&server)
            .args(["labels", "--key", "app"])
            .assert()
            .success(),
    );
    let rows = out["data"].as_array().unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["value"], "shop");
    assert_eq!(out["meta"]["cache"], "miss");
    let out = stdout_json(
        cmd(&server)
            .args(["labels", "--key", "app", "--include-deleted"])
            .assert()
            .success(),
    );
    assert_eq!(out["data"].as_array().unwrap().len(), 2);
    let out = stdout_json(
        cmd(&server)
            .args(["labels", "--value", "prod"])
            .assert()
            .success(),
    );
    assert_eq!(out["data"][0]["key"], "env");
    cmd(&server)
        .args(["labels", "--table"])
        .assert()
        .success()
        .stdout(predicate::str::contains("role  web"));
}

#[tokio::test(flavor = "multi_thread")]
async fn other_object_lists() {
    let server = MockServer::start().await;
    mount_objects(&server).await;
    let out = stdout_json(cmd(&server).arg("label-groups").assert().success());
    assert_eq!(out["data"][0]["name"], "All prod envs");
    let out = stdout_json(cmd(&server).arg("iplists").assert().success());
    assert_eq!(out["data"][1]["name"], "Corp");
    cmd(&server)
        .args(["iplists", "--table"])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "10.1.0.0-10.1.255.255, *.corp.example",
        ));
    cmd(&server)
        .args(["services", "--table"])
        .assert()
        .success()
        .stdout(predicate::str::contains("8000-8099/tcp"));
}

#[test]
fn render_table_is_aligned() {
    use illumio_tui::cli::output::render_table;
    let t = render_table(
        &["name", "n"],
        &[
            vec!["a".into(), "1".into()],
            vec!["longer".into(), "22".into()],
        ],
    );
    let lines: Vec<&str> = t.lines().collect();
    assert_eq!(lines[0], "name    n");
    assert!(lines[1].starts_with("─"));
    assert_eq!(lines[2], "a       1");
    assert_eq!(lines[3], "longer  22");
    assert_eq!(render_table(&["x"], &[]).lines().last(), Some("(no rows)"));
}

#[tokio::test(flavor = "multi_thread")]
async fn workloads_with_label_filter_resolve_hrefs() {
    use wiremock::matchers::query_param;
    let server = MockServer::start().await;
    mount_objects(&server).await;
    Mock::given(method("GET"))
        .and(path("/api/v2/orgs/5/workloads"))
        .and(query_param(
            "labels",
            r#"[["/orgs/5/labels/3","/orgs/5/labels/4"]]"#,
        ))
        .and(query_param("online", "true"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(include_str!("../../../fixtures/workloads.json"))
                .insert_header("Content-Type", "application/json"),
        )
        .expect(2)
        .mount(&server)
        .await;
    let out = stdout_json(
        cmd(&server)
            .args([
                "workloads",
                "--label",
                "app=shop",
                "--label",
                "env=prod",
                "--online",
                "true",
            ])
            .assert()
            .success(),
    );
    assert_eq!(out["data"].as_array().unwrap().len(), 4);
    assert_eq!(out["data"][0]["hostname"], "host-1.example");
    cmd(&server)
        .args([
            "workloads",
            "--label",
            "app=shop",
            "--label",
            "env=prod",
            "--online",
            "true",
            "--table",
        ])
        .assert()
        .success()
        .stdout(
            predicate::str::contains("host-1.example")
                .and(predicate::str::contains("app=shop env=prod role=web"))
                .and(predicate::str::contains("active")),
        );
}

#[tokio::test(flavor = "multi_thread")]
async fn workloads_report_ven_health_and_unhealthy_filter() {
    let server = MockServer::start().await;
    mount_objects(&server).await;
    Mock::given(method("GET"))
        .and(path("/api/v2/orgs/5/workloads"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(include_str!("../../../fixtures/workloads.json"))
                .insert_header("Content-Type", "application/json"),
        )
        .expect(3)
        .mount(&server)
        .await;
    let out = stdout_json(cmd(&server).arg("workloads").assert().success());
    let rows = out["data"].as_array().unwrap();
    assert_eq!(rows.len(), 4);
    assert_eq!(rows[0]["sync_state"], "applied");
    assert!(
        matches!(
            rows[0]["health"].as_str(),
            Some("healthy") | Some("offline")
        ),
        "fixture heartbeat ages with real time: {}",
        rows[0]["health"]
    );
    assert_eq!(rows[1]["health"], "offline");
    assert_eq!(rows[2]["health"], "unmanaged");
    assert!(
        matches!(
            rows[3]["health"].as_str(),
            Some("warning") | Some("offline")
        ),
        "suspended VEN is never healthy; the fixture heartbeat ages with real time"
    );
    assert!(
        rows[2]["heartbeat_age_secs"].is_null(),
        "unknown age is null, not 0"
    );
    assert!(
        rows[0]["hostname"].is_string(),
        "workload fields are flattened"
    );
    let out = stdout_json(
        cmd(&server)
            .args(["workloads", "--unhealthy"])
            .assert()
            .success(),
    );
    let rows = out["data"].as_array().unwrap();
    assert!(
        rows.iter()
            .all(|r| r["health"] != "unmanaged" && r["health"] != "healthy")
    );
    assert!(rows.iter().any(|r| r["hostname"] == "host-2.example"));
    assert!(rows.iter().any(|r| r["hostname"] == "host-4.example"));
    cmd(&server)
        .args(["workloads", "--table"])
        .assert()
        .success()
        .stdout(predicate::str::contains("health").and(predicate::str::contains("offline")));
}

#[tokio::test(flavor = "multi_thread")]
async fn workloads_unknown_label_exits_2_without_querying() {
    let server = MockServer::start().await;
    mount_objects(&server).await;
    Mock::given(method("GET"))
        .and(path("/api/v2/orgs/5/workloads"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([])))
        .expect(0)
        .mount(&server)
        .await;
    let assert = cmd(&server)
        .args(["workloads", "--label", "app=nope"])
        .assert()
        .code(2);
    let err: Value = serde_json::from_slice(&assert.get_output().stderr).unwrap();
    assert!(err["error"]["message"].as_str().unwrap().contains("shop"));
}

async fn mount_policy(server: &MockServer) {
    mount_objects(server).await;
    Mock::given(method("GET"))
        .and(path("/api/v2/orgs/5/sec_policy/draft/rule_sets"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(include_str!("../../../fixtures/rule_sets.json"))
                .insert_header("Content-Type", "application/json"),
        )
        .mount(server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v2/orgs/5/sec_policy/active/rule_sets"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([])))
        .mount(server)
        .await;
    // Ruleset 100 carries its deny rules inline, so the endpoint is never hit.
    Mock::given(method("GET"))
        .and(path(
            "/api/v2/orgs/5/sec_policy/draft/rule_sets/100/deny_rules",
        ))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(include_str!("../../../fixtures/deny_rules.json"))
                .insert_header("Content-Type", "application/json"),
        )
        .expect(0)
        .mount(server)
        .await;
    Mock::given(method("GET"))
        .and(path(
            "/api/v2/orgs/5/sec_policy/draft/rule_sets/101/deny_rules",
        ))
        .respond_with(ResponseTemplate::new(404))
        .mount(server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v2/orgs/5/sec_policy/pending"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(include_str!("../../../fixtures/pending_list.json"))
                .insert_header("Content-Type", "application/json"),
        )
        .mount(server)
        .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn policy_rulesets_resolve_scopes_and_filter_by_name() {
    let server = MockServer::start().await;
    mount_policy(&server).await;
    let out = stdout_json(cmd(&server).args(["policy", "rulesets"]).assert().success());
    assert_eq!(out["data"].as_array().unwrap().len(), 2);
    assert_eq!(out["data"][0]["scopes"][0], "app=shop & env=prod");
    assert_eq!(out["data"][0]["rule_count"], 4);
    assert_eq!(out["data"][0]["deny_count"], 2);
    assert_eq!(out["data"][1]["update_type"], "create");
    let out = stdout_json(
        cmd(&server)
            .args(["policy", "rulesets", "--name", "infra"])
            .assert()
            .success(),
    );
    assert_eq!(out["data"].as_array().unwrap().len(), 1);
    let out = stdout_json(
        cmd(&server)
            .args(["policy", "rulesets", "--active"])
            .assert()
            .success(),
    );
    assert_eq!(out["data"].as_array().unwrap().len(), 0);
    let out = stdout_json(
        cmd(&server)
            .args(["policy", "rulesets", "--draft"])
            .assert()
            .success(),
    );
    assert_eq!(
        out["data"].as_array().unwrap().len(),
        2,
        "--draft is the default"
    );
    cmd(&server)
        .args(["policy", "rulesets", "--draft", "--active"])
        .assert()
        .code(2);
}

#[tokio::test(flavor = "multi_thread")]
async fn policy_rules_include_deny_rules_and_tolerate_missing_endpoint() {
    let server = MockServer::start().await;
    mount_policy(&server).await;
    let out = stdout_json(
        cmd(&server)
            .args(["policy", "rules", "shop-prod"])
            .assert()
            .success(),
    );
    assert_eq!(out["data"]["ruleset"]["name"], "Shop-Prod");
    let rules = out["data"]["rules"].as_array().unwrap();
    assert_eq!(rules.len(), 4);
    assert_eq!(rules[1]["consumers"][0]["name"], "ipl:Corp");
    assert_eq!(
        rules[1]["consumers"][0]["href"],
        "/orgs/5/sec_policy/draft/ip_lists/21"
    );
    assert_eq!(rules[1]["consumers"][0]["kind"], "ip_list");
    assert_eq!(rules[1]["consumers"][0]["exclusion"], false);
    assert_eq!(rules[1]["services"][1]["name"], "8000-8099/tcp");
    assert!(rules[1]["services"][1]["href"].is_null());
    assert_eq!(rules[0]["services"][0]["name"], "PostgreSQL");
    assert_eq!(
        rules[0]["services"][0]["href"],
        "/orgs/5/sec_policy/draft/services/31"
    );
    assert_eq!(rules[3]["kind"], "override_deny");
    assert_eq!(out["meta"]["api_calls"], 5, "four object lists + rulesets");
    let out = stdout_json(
        cmd(&server)
            .args(["policy", "rules", "/orgs/5/sec_policy/draft/rule_sets/101"])
            .assert()
            .success(),
    );
    assert_eq!(out["data"]["rules"].as_array().unwrap().len(), 1);
    assert_eq!(
        out["data"]["rules"][0]["providers"][0]["name"],
        "All workloads"
    );
    assert!(out["data"]["rules"][0]["providers"][0]["href"].is_null());
    assert_eq!(
        out["meta"]["api_calls"], 6,
        "no inline deny rules: the endpoint is tried and its 404 tolerated"
    );
    cmd(&server)
        .args(["policy", "rules", "ghost"])
        .assert()
        .code(2);
    let out = stdout_json(
        cmd(&server)
            .args(["policy", "rules", "shop-prod", "--draft"])
            .assert()
            .success(),
    );
    assert_eq!(out["data"]["rules"].as_array().unwrap().len(), 4);
    cmd(&server)
        .args(["policy", "rules", "shop-prod", "--table"])
        .assert()
        .success()
        .stdout(
            predicate::str::contains("override_deny").and(predicate::str::contains("role=web")),
        );
}

#[tokio::test(flavor = "multi_thread")]
async fn policy_pending_lists_changes() {
    let server = MockServer::start().await;
    mount_policy(&server).await;
    let out = stdout_json(cmd(&server).args(["policy", "pending"]).assert().success());
    assert_eq!(out["data"][0]["object_type"], "rule_sets");
    assert_eq!(out["data"][1]["name"], "Corp");
}

const QHREF: &str = "/orgs/5/traffic_flows/async_queries/q-1";

async fn mount_traffic(server: &MockServer, expect_post: u64) {
    mount_objects(server).await;
    Mock::given(method("POST"))
        .and(path("/api/v2/orgs/5/traffic_flows/async_queries"))
        .respond_with(
            ResponseTemplate::new(202).set_body_json(json!({"href": QHREF, "status": "queued"})),
        )
        .expect(expect_post)
        .mount(server)
        .await;
    Mock::given(method("GET")).and(path(format!("/api/v2{QHREF}"))).respond_with(ResponseTemplate::new(200).set_body_json(json!({"status": "completed", "result": {"href": format!("{QHREF}/download")}, "matches_count": 4}))).mount(server).await;
    Mock::given(method("GET"))
        .and(path(format!("/api/v2{QHREF}/download")))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(include_str!("../../../fixtures/traffic_flows.json"))
                .insert_header("Content-Type", "application/json"),
        )
        .mount(server)
        .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn traffic_query_flows_json() {
    let server = MockServer::start().await;
    mount_traffic(&server, 1).await;
    let out = stdout_json(
        cmd(&server)
            .args([
                "traffic",
                "query",
                "--window",
                "2h",
                "--src",
                "app=shop,env=prod",
                "--decision",
                "allowed,blocked",
            ])
            .assert()
            .success(),
    );
    let flows = out["data"].as_array().unwrap();
    assert_eq!(flows.len(), 4);
    assert_eq!(flows[0]["src"]["hostname"], "host-1.example");
    assert_eq!(flows[0]["src"]["labels"][0]["key"], "app");
    assert_eq!(flows[0]["dst"]["labels"][2]["value"], "db");
    assert_eq!(flows[2]["proto"], "icmp");
    assert!(flows[2]["port"].is_null());
    assert_eq!(out["meta"]["truncated"], false);
    assert!(
        out["meta"]["window"]["start"]
            .as_str()
            .unwrap()
            .ends_with('Z')
    );
    assert_eq!(out["meta"]["cache"], "miss");
}

#[tokio::test(flavor = "multi_thread")]
async fn traffic_query_group_and_blocked_views() {
    let server = MockServer::start().await;
    mount_traffic(&server, 3).await; // three separate processes, no shared cache
    let out = stdout_json(
        cmd(&server)
            .args([
                "traffic",
                "query",
                "--from",
                "2026-10-09",
                "--to",
                "2026-10-09",
                "--group",
                "app,env",
            ])
            .assert()
            .success(),
    );
    assert_eq!(out["data"][0]["src"], json!(["shop", "prod"]));
    assert_eq!(out["data"][0]["connections"], 310);
    assert_eq!(out["meta"]["window"]["start"], "2026-10-09T00:00:00Z");
    assert_eq!(out["meta"]["window"]["end"], "2026-10-09T23:59:59Z");
    let out = stdout_json(
        cmd(&server)
            .args(["traffic", "query", "--window", "1h", "--blocked"])
            .assert()
            .success(),
    );
    assert_eq!(out["data"].as_array().unwrap().len(), 2);
    assert_eq!(out["data"][0]["dst"], "host-1.example");
    assert_eq!(out["data"][0]["port"], 8443);
    cmd(&server)
        .args(["traffic", "query", "--window", "1h", "--blocked", "--table"])
        .assert()
        .success()
        .stdout(predicate::str::contains("8443/tcp"));
}

#[tokio::test(flavor = "multi_thread")]
async fn traffic_dry_run_prints_body_without_submitting() {
    let server = MockServer::start().await;
    mount_traffic(&server, 0).await;
    let out = stdout_json(
        cmd(&server)
            .args([
                "traffic",
                "query",
                "--window",
                "1h",
                "--src",
                "ipl:corp",
                "--dst",
                "10.0.0.5",
                "--port",
                "443",
                "--proto",
                "tcp",
                "--dry-run",
            ])
            .assert()
            .success(),
    );
    assert_eq!(
        out["data"]["sources"]["include"],
        json!([[{"ip_list": {"href": "/orgs/5/sec_policy/draft/ip_lists/21"}}]])
    );
    assert_eq!(
        out["data"]["destinations"]["include"],
        json!([[{"ip_address": "10.0.0.5"}]])
    );
    assert_eq!(
        out["data"]["services"]["include"],
        json!([{"port": 443, "proto": 6}])
    );
    assert_eq!(out["data"]["max_results"], 1000);
    assert_eq!(out["meta"]["cache"], "none");
}

/// Every mock answers but none may be called: the guards run offline.
async fn mount_traffic_quiet(server: &MockServer) {
    mount_objects_expecting(server, Some(0)).await;
    Mock::given(method("GET"))
        .and(path("/api/v2/orgs/5/workloads"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([])))
        .expect(0)
        .mount(server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/v2/orgs/5/traffic_flows/async_queries"))
        .respond_with(ResponseTemplate::new(202).set_body_json(json!({"href": QHREF})))
        .expect(0)
        .mount(server)
        .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn traffic_guards_exit_2() {
    // Window, order, decision and cap guards run before any request.
    let quiet = MockServer::start().await;
    mount_traffic_quiet(&quiet).await;
    let assert = cmd(&quiet)
        .args(["traffic", "query", "--window", "30d"])
        .assert()
        .code(2);
    let err: Value = serde_json::from_slice(&assert.get_output().stderr).unwrap();
    assert!(err["error"]["message"].as_str().unwrap().contains("--yes"));
    let assert = cmd(&quiet)
        .args([
            "traffic",
            "query",
            "--from",
            "2026-10-09",
            "--to",
            "2026-10-01",
        ])
        .assert()
        .code(2);
    let err: Value = serde_json::from_slice(&assert.get_output().stderr).unwrap();
    assert!(err["error"]["message"].as_str().unwrap().contains("before"));
    cmd(&quiet)
        .args(["traffic", "query", "--decision", "maybe"])
        .assert()
        .code(2);
    cmd(&quiet)
        .args(["traffic", "query", "--max-results", "20000"])
        .assert()
        .code(2);
    cmd(&quiet)
        .args(["traffic", "query", "--window", "30d", "--src", "app=shop"])
        .assert()
        .code(2);
    // A filter-less dry run needs nothing from the PCE either.
    let out = stdout_json(
        cmd(&quiet)
            .args(["traffic", "query", "--dry-run"])
            .assert()
            .success(),
    );
    assert_eq!(out["data"]["sources"]["include"], json!([[]]));
    assert_eq!(out["meta"]["api_calls"], 0);

    // An unknown label still needs the object lists to say what is known.
    let server = MockServer::start().await;
    mount_traffic(&server, 0).await;
    let assert = cmd(&server)
        .args(["traffic", "query", "--src", "app=nope"])
        .assert()
        .code(2);
    let err: Value = serde_json::from_slice(&assert.get_output().stderr).unwrap();
    assert!(err["error"]["message"].as_str().unwrap().contains("shop"));
}

#[tokio::test(flavor = "multi_thread")]
async fn no_cache_fetches_each_object_list_once() {
    let server = MockServer::start().await;
    mount_objects_expecting(&server, Some(1)).await;
    Mock::given(method("GET"))
        .and(path("/api/v2/orgs/5/workloads"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([])))
        .expect(0)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/v2/orgs/5/traffic_flows/async_queries"))
        .respond_with(ResponseTemplate::new(202).set_body_json(json!({"href": QHREF})))
        .expect(0)
        .mount(&server)
        .await;
    let out = stdout_json(
        cmd(&server)
            .args([
                "traffic",
                "query",
                "--no-cache",
                "--dry-run",
                "--src",
                "app=shop",
                "--dst",
                "ip:10.0.0.5",
                "--exclude-dst",
                "ipl:corp",
            ])
            .assert()
            .success(),
    );
    assert_eq!(out["meta"]["api_calls"], 4);
    assert_eq!(
        out["data"]["destinations"]["exclude"],
        json!([{"ip_list": {"href": "/orgs/5/sec_policy/draft/ip_lists/21"}}])
    );
}

#[test]
fn profile_list_and_use_without_credentials() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = dir.path().join("config.toml");
    std::fs::write(&cfg, "default_profile = \"a\"\n\n[profiles.a]\nhost = \"a.example\"\norg_id = 1\nunknown_key = \"kept\"\n\n[profiles.b]\nhost = \"b.example\"\norg_id = 2\n").unwrap();
    let out = stdout_json(
        Command::cargo_bin("illumio-tui")
            .unwrap()
            .env_clear()
            .args(["--config", cfg.to_str().unwrap(), "profile", "list"])
            .assert()
            .success(),
    );
    let rows = out["data"].as_array().unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0]["name"], "a");
    assert_eq!(rows[0]["is_default"], true);
    assert_eq!(rows[1]["host"], "b.example");

    let out = stdout_json(
        Command::cargo_bin("illumio-tui")
            .unwrap()
            .env_clear()
            .args(["--config", cfg.to_str().unwrap(), "profile", "use", "b"])
            .assert()
            .success(),
    );
    assert_eq!(out["data"]["default_profile"], "b");
    let text = std::fs::read_to_string(&cfg).unwrap();
    assert!(text.contains("default_profile = \"b\""));
    assert!(
        text.contains("host = \"a.example\""),
        "other profiles survive: {text}"
    );
    assert!(
        text.contains("unknown_key = \"kept\""),
        "unknown keys survive: {text}"
    );
    assert!(
        !text.contains("[profiles.a.limits]"),
        "no empty limits table: {text}"
    );

    Command::cargo_bin("illumio-tui")
        .unwrap()
        .env_clear()
        .args(["--config", cfg.to_str().unwrap(), "profile", "use", "zzz"])
        .assert()
        .code(2);
    Command::cargo_bin("illumio-tui")
        .unwrap()
        .env_clear()
        .args([
            "--config",
            cfg.to_str().unwrap(),
            "profile",
            "list",
            "--table",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("b.example"));
}

#[tokio::test(flavor = "multi_thread")]
async fn events_command_filters_and_renders() {
    use wiremock::matchers::query_param;
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v2/orgs/5/events"))
        .and(query_param("severity", "err"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(include_str!("../../../fixtures/events.json"))
                .insert_header("Content-Type", "application/json"),
        )
        .expect(3)
        .mount(&server)
        .await;
    let out = stdout_json(
        cmd(&server)
            .args(["events", "--severity", "err", "--since", "2h"])
            .assert()
            .success(),
    );
    let rows = out["data"].as_array().unwrap();
    assert_eq!(
        rows.len(),
        4,
        "the mock returns the whole fixture; the PCE does the severity filtering"
    );
    assert_eq!(rows[2]["event_type"], "agent.tampering");
    assert!(
        out["meta"]["window"]["start"]
            .as_str()
            .unwrap()
            .ends_with('Z')
    );
    let out = stdout_json(
        cmd(&server)
            .args(["events", "--severity", "err", "--type", "agent."])
            .assert()
            .success(),
    );
    assert_eq!(
        out["data"].as_array().unwrap().len(),
        1,
        "--type is a local substring filter"
    );
    cmd(&server)
        .args(["events", "--severity", "err", "--table"])
        .assert()
        .success()
        .stdout(
            predicate::str::contains("agent:host-1.example")
                .and(predicate::str::contains("warning")),
        );
    cmd(&server)
        .args(["events", "--severity", "err", "--since", "soon"])
        .assert()
        .code(2);
}
