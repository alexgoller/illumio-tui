use assert_cmd::Command;
use predicates::prelude::*;
use serde_json::{Value, json};
use wiremock::matchers::{body_partial_json, method, path, query_param};
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
        vec!["policy", "provision", "--help"],
        vec!["traffic", "query", "--help"],
        vec!["traffic", "groups", "--help"],
        vec!["traffic", "mesh", "--help"],
        vec!["app", "--help"],
        vec!["policy", "pending", "--help"],
        vec!["policy", "rulesets", "--help"],
        vec!["policy", "rules", "--help"],
        vec!["policy", "rulesets", "add", "--help"],
        vec!["policy", "rulesets", "update", "--help"],
        vec!["policy", "rulesets", "delete", "--help"],
        vec!["policy", "rules", "add", "--help"],
        vec!["policy", "rules", "update", "--help"],
        vec!["policy", "rules", "delete", "--help"],
        vec!["policy", "rules", "impact", "--help"],
        vec!["policy", "deny-rules", "add", "--help"],
        vec!["policy", "deny-rules", "update", "--help"],
        vec!["policy", "deny-rules", "delete", "--help"],
        vec!["labels", "add", "--help"],
        vec!["labels", "update", "--help"],
        vec!["labels", "delete", "--help"],
        vec!["iplists", "add", "--help"],
        vec!["iplists", "update", "--help"],
        vec!["iplists", "delete", "--help"],
        vec!["services", "add", "--help"],
        vec!["services", "update", "--help"],
        vec!["services", "delete", "--help"],
        vec!["workloads", "add", "--help"],
        vec!["workloads", "update", "--help"],
        vec!["workloads", "delete", "--help"],
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
async fn labels_repeatable_keys_value_contains_and_unused() {
    let server = MockServer::start().await;
    mount_policy(&server).await;
    Mock::given(method("GET"))
        .and(path("/api/v2/orgs/5/workloads"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(include_str!("../../../fixtures/workloads.json"))
                .insert_header("Content-Type", "application/json"),
        )
        .expect(2) // the two --unused runs; the plain list reads no workloads
        .mount(&server)
        .await;
    let out = stdout_json(
        cmd(&server)
            .args([
                "labels",
                "--key",
                "role",
                "--key",
                "env",
                "--value-contains",
                "E",
            ])
            .assert()
            .success(),
    );
    let rows = out["data"].as_array().unwrap();
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert_eq!(rows[0]["value"], "web");
    assert!(rows[0].get("workloads").is_none(), "shape unchanged");
    assert_eq!(out["meta"]["api_calls"], 1);
    let out = stdout_json(
        cmd(&server)
            .args(["labels", "--unused", "--include-deleted"])
            .assert()
            .success(),
    );
    let rows = out["data"].as_array().unwrap();
    let hrefs: Vec<&str> = rows.iter().map(|r| r["href"].as_str().unwrap()).collect();
    assert_eq!(hrefs, vec!["/orgs/5/labels/5", "/orgs/5/labels/6"]);
    assert_eq!(
        (rows[0]["workloads"].as_u64(), rows[0]["rules"].as_u64()),
        (Some(0), Some(0))
    );
    assert_eq!(
        out["meta"]["api_calls"], 4,
        "labels, workloads, draft rulesets, label groups"
    );
    cmd(&server)
        .args(["labels", "--unused", "--table"])
        .assert()
        .success()
        .stdout(predicate::str::contains("workloads  rules"));
}

#[tokio::test(flavor = "multi_thread")]
async fn other_object_lists() {
    let server = MockServer::start().await;
    mount_objects(&server).await;
    let out = stdout_json(cmd(&server).arg("label-groups").assert().success());
    assert_eq!(out["data"][0]["name"], "All prod envs");
    let out = stdout_json(cmd(&server).arg("iplists").assert().success());
    assert_eq!(out["data"][1]["name"], "Corp");
    assert_eq!(
        out["data"][1]["size"],
        json!({"v4": "65536", "v6": "0", "unbounded": null, "ranges": 1, "fqdns": 1})
    );
    assert_eq!(out["data"][0]["size"]["unbounded"], "0.0.0.0/0");
    assert_eq!(
        out["data"][1]["ip_ranges"][0]["from_ip"], "10.1.0.0",
        "the list keeps its own fields"
    );
    assert_eq!(out["meta"]["api_calls"], 1, "the size costs no request");
    cmd(&server)
        .args(["iplists", "--table"])
        .assert()
        .success()
        .stdout(
            predicate::str::contains("10.1.0.0-10.1.255.255, *.corp.example")
                .and(predicate::str::contains("65.5K"))
                .and(predicate::str::contains("any (0.0.0.0/0)")),
        );
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

/// Pending changes of three kinds against an active label group missing its
/// member and an empty active IP list. `expect` pins each GET to one call.
async fn mount_pending_diff(server: &MockServer) {
    mount_objects_expecting(server, Some(1)).await;
    let pending = json!([
        {"href": "/orgs/5/sec_policy/draft/label_groups/10", "change_type": "update", "name": "All prod envs"},
        {"href": "/orgs/5/sec_policy/draft/ip_lists/21", "change_type": "create", "name": "Corp"},
        {"href": "/orgs/5/sec_policy/draft/firewall_settings", "change_type": "update", "object_type": "firewall_settings"}
    ]);
    let active_group = json!([{"href": "/orgs/5/sec_policy/active/label_groups/10", "name": "All prod envs", "key": "env", "labels": []}]);
    for (p, body) in [
        ("/api/v2/orgs/5/sec_policy/pending", pending),
        (
            "/api/v2/orgs/5/sec_policy/active/label_groups",
            active_group,
        ),
        ("/api/v2/orgs/5/sec_policy/active/ip_lists", json!([])),
    ] {
        Mock::given(method("GET"))
            .and(path(p))
            .respond_with(ResponseTemplate::new(200).set_body_json(body))
            .expect(1)
            .mount(server)
            .await;
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn policy_pending_diff_prints_the_lines_under_each_change() {
    let server = MockServer::start().await;
    mount_pending_diff(&server).await;
    let out = cmd(&server)
        .args(["policy", "pending", "--diff", "--table"])
        .assert()
        .success();
    let text = String::from_utf8_lossy(&out.get_output().stdout).to_string();
    let lines: Vec<&str> = text.lines().collect();
    let at = |word: &str| {
        lines
            .iter()
            .position(|l| l.contains(word))
            .unwrap_or_else(|| panic!("{word} missing from\n{text}"))
    };
    // Each change row is followed by its own full-width lines.
    let group = at("label_groups");
    assert_eq!(lines[group + 1], "    name: All prod envs");
    assert!(lines[group + 2..group + 5].contains(&"  + label env=prod"));
    let corp = at("ip_lists");
    assert_eq!(lines[corp + 1], "  + name: Corp");
    let fw = at("firewall_settings");
    assert_eq!(lines[fw + 1], "    (no diff)");
    let stderr = String::from_utf8_lossy(&out.get_output().stderr).to_string();
    assert!(stderr.contains("7 api calls"), "{stderr}");
}

#[tokio::test(flavor = "multi_thread")]
async fn policy_pending_diff_csv_puts_the_lines_in_a_diff_column() {
    let server = MockServer::start().await;
    mount_pending_diff(&server).await;
    let text = csv_stdout(
        &cmd(&server)
            .args(["policy", "pending", "--diff", "--csv"])
            .assert()
            .success(),
    );
    let lines: Vec<&str> = text.split("\r\n").collect();
    assert_eq!(lines[0], "object_type,change_type,name,href,diff");
    let corp = lines
        .iter()
        .find(|l| l.starts_with("ip_lists,"))
        .expect(&text);
    assert!(
        corp.contains(",\"+ name: Corp\n+ description: Office ranges\n"),
        "one diff line per cell line: {corp}"
    );
    let fw = lines
        .iter()
        .find(|l| l.contains("/firewall_settings,"))
        .expect(&text);
    assert!(fw.ends_with(",  (no diff)"), "{fw}");
}

#[tokio::test(flavor = "multi_thread")]
async fn policy_pending_csv_has_a_diff_column_exactly_with_the_flag() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v2/orgs/5/sec_policy/pending"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([])))
        .mount(&server)
        .await;
    let text = csv_stdout(
        &cmd(&server)
            .args(["policy", "pending", "--diff", "--csv"])
            .assert()
            .success(),
    );
    assert_eq!(text, "object_type,change_type,name,href,diff\r\n");
    let text = csv_stdout(
        &cmd(&server)
            .args(["policy", "pending", "--csv"])
            .assert()
            .success(),
    );
    assert_eq!(text, "object_type,change_type,name,href\r\n");
}

#[tokio::test(flavor = "multi_thread")]
async fn policy_pending_json_carries_diff_only_with_the_flag() {
    let server = MockServer::start().await;
    mount_pending_diff(&server).await;
    let out = stdout_json(
        cmd(&server)
            .args(["policy", "pending", "--diff", "--json"])
            .assert()
            .success(),
    );
    let first = &out["data"][0];
    assert_eq!(
        first["diff"][0],
        json!({"sign": "same", "text": "name: All prod envs"})
    );
    assert!(
        first["diff"]
            .as_array()
            .unwrap()
            .contains(&json!({"sign": "added", "text": "label env=prod"}))
    );
    assert_eq!(
        out["data"][2]["diff"],
        json!([{"sign": "same", "text": "(no diff)"}])
    );
    assert_eq!(out["meta"]["api_calls"], 7);

    let plain = MockServer::start().await;
    mount_policy(&plain).await;
    let out = stdout_json(cmd(&plain).args(["policy", "pending"]).assert().success());
    let expected = pce::models::parse_pending(
        serde_json::from_str(include_str!("../../../fixtures/pending_list.json")).unwrap(),
    );
    assert_eq!(out["data"], serde_json::to_value(expected).unwrap());
    assert_eq!(out["meta"]["api_calls"], 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn policy_pending_diff_with_no_cache_reads_every_list_once() {
    let server = MockServer::start().await;
    mount_pending_diff(&server).await;
    let out = stdout_json(
        cmd(&server)
            .args(["--no-cache", "policy", "pending", "--diff", "--json"])
            .assert()
            .success(),
    );
    assert_eq!(out["meta"]["api_calls"], 7);
}

const QHREF: &str = "/orgs/5/traffic_flows/async_queries/q-1";

async fn mount_traffic(server: &MockServer, expect_post: u64) {
    mount_traffic_body(
        server,
        expect_post,
        include_str!("../../../fixtures/traffic_flows.json").to_string(),
    )
    .await;
}

/// `mount_traffic` with another download body.
async fn mount_traffic_body(server: &MockServer, expect_post: u64, body: String) {
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
                .set_body_string(body)
                .insert_header("Content-Type", "application/json"),
        )
        .mount(server)
        .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn traffic_query_blocked_prints_sections_per_destination_and_counts_its_hosts() {
    let server = MockServer::start().await;
    // The fixture plus the Corp 8443/tcp flow to a second shop/prod host.
    let mut flows: Vec<serde_json::Value> =
        serde_json::from_str(include_str!("../../../fixtures/traffic_flows.json")).unwrap();
    let mut second = flows[1].clone();
    second["dst"] = flows[2]["dst"].clone();
    flows.push(second);
    mount_traffic_body(&server, 2, serde_json::to_string(&flows).unwrap()).await;
    let out = stdout_json(
        cmd(&server)
            .args(["traffic", "query", "--window", "1h", "--blocked"])
            .assert()
            .success(),
    );
    let shop = &out["data"]["sections"][0];
    assert_eq!(shop["app"], json!(["shop", "prod"]));
    assert_eq!(shop["direction"], "in");
    assert_eq!(shop["totals"]["groups"], 2);
    assert_eq!(shop["rows"][0]["peer"], json!(["ipl:Corp", "-"]));
    assert_eq!(shop["rows"][0]["service"], "8443/tcp");
    assert_eq!(shop["rows"][0]["app_hosts"], 2);
    let table = cmd(&server)
        .args(["traffic", "query", "--window", "1h", "--blocked", "--table"])
        .assert()
        .success();
    let text = String::from_utf8_lossy(&table.get_output().stdout).to_string();
    let header = text.lines().next().unwrap();
    let cols: Vec<&str> = header
        .split("  ")
        .map(str::trim)
        .filter(|h| !h.is_empty())
        .collect();
    assert_eq!(
        cols,
        vec![
            "app / peer",
            "peer hosts",
            "app hosts",
            "service",
            "flows",
            "conns",
            "bytes",
            "last seen",
            "decision"
        ],
        "{header}"
    );
    let lines: Vec<&str> = text.lines().collect();
    assert!(
        lines[2].starts_with("shop/prod ")
            && lines[2].ends_with("in · 2 groups · 27 conns · 1 blocked · 2 pot.blocked"),
        "a header row: {text}"
    );
    let row = lines
        .iter()
        .find(|l| l.contains("8443/tcp"))
        .unwrap_or_else(|| panic!("{text}"));
    assert!(row.starts_with("  ipl:Corp/-"), "an indented peer: {row}");
    assert!(row.contains("2 hosts"), "{row}");
    assert!(!row.contains("host-1.example"), "{row}");
    assert!(!text.contains("dst labels"), "{text}");
}

/// The Explorer query for `start..end` (06:00 to 08:00 on the fixture
/// day), answered at its own href with `body` and `matches`; expected
/// `times` times.
async fn mount_range(
    server: &MockServer,
    (start, end): (&str, &str),
    times: u64,
    matches: u64,
    body: String,
) {
    let href = format!("/orgs/5/traffic_flows/async_queries/q-{start}-{end}");
    let ts = |t: &str| format!("2026-10-10T{t}:00Z");
    Mock::given(method("POST"))
        .and(path("/api/v2/orgs/5/traffic_flows/async_queries"))
        .and(body_partial_json(
            json!({"start_date": ts(start), "end_date": ts(end)}),
        ))
        .respond_with(
            ResponseTemplate::new(202).set_body_json(json!({"href": href, "status": "queued"})),
        )
        .expect(times)
        .mount(server)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("/api/v2{href}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "status": "completed",
            "result": {"href": format!("{href}/download")},
            "matches_count": matches
        })))
        .mount(server)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("/api/v2{href}/download")))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(body)
                .insert_header("Content-Type", "application/json"),
        )
        .mount(server)
        .await;
}

/// The fixture flows last seen at `times`, each on its own port from `port`.
fn flows_at(times: &[&str], port: u64) -> String {
    let mut flows: Vec<Value> =
        serde_json::from_str(include_str!("../../../fixtures/traffic_flows.json")).unwrap();
    flows.truncate(times.len());
    for (i, (f, t)) in flows.iter_mut().zip(times).enumerate() {
        f["timestamp_range"]["last_detected"] = json!(format!("2026-10-10T{t}:00Z"));
        f["service"]["port"] = json!(port + i as u64);
    }
    serde_json::to_string(&flows).unwrap()
}

/// `n` flows all last seen at 08:00, each on its own port from `port`.
fn burst(n: u64, port: u64) -> String {
    let one: Vec<Value> = serde_json::from_str(&flows_at(&["08:00"], 0)).unwrap();
    let flows: Vec<Value> = (0..n)
        .map(|i| {
            let mut f = one[0].clone();
            f["service"]["port"] = json!(port + i);
            f
        })
        .collect();
    serde_json::to_string(&flows).unwrap()
}

/// The `max_results` of every traffic query the server saw, in order.
async fn sent_caps(server: &MockServer) -> Vec<u64> {
    server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .filter(|r| r.method.as_str() == "POST")
        .map(|r| {
            serde_json::from_slice::<Value>(&r.body).unwrap()["max_results"]
                .as_u64()
                .unwrap()
        })
        .collect()
}

/// 2000 flows in under a minute at cap 2000: 2h needs 120 slices of 1m up
/// to cap 10000, and 24 of 5m at 25000. Without `--yes` the run stays at
/// 10000 (its 1m slice truncates: the cap is too small); with it, 25000.
#[tokio::test(flavor = "multi_thread")]
async fn traffic_backfill_passes_10000_only_with_yes() {
    let args = [
        "traffic",
        "query",
        "--from",
        "2026-10-10T06:00:00Z",
        "--to",
        "2026-10-10T08:00:00Z",
        "--max-results",
        "2000",
        "--backfill",
        "--max-slices",
        "1",
    ];
    let server = MockServer::start().await;
    mount_objects(&server).await;
    mount_range(&server, ("06:00", "08:00"), 1, 2000, burst(2000, 1000)).await;
    mount_range(
        &server,
        ("07:59", "08:00"),
        1,
        20_000,
        flows_at(&["08:00"], 9000),
    )
    .await;
    let a = cmd(&server).args(args).assert().success();
    let err = String::from_utf8_lossy(&a.get_output().stderr).to_string();
    assert!(err.contains("backfill raises the cap to 10000"), "{err}");
    assert!(
        err.contains(
            "cap 10000 is too small for this PCE at 1m: raise --max-results (up to 200000)"
        ),
        "{err}"
    );
    assert_eq!(sent_caps(&server).await, vec![2000, 10_000]);
    server.verify().await;

    let server = MockServer::start().await;
    mount_objects(&server).await;
    mount_range(&server, ("06:00", "08:00"), 1, 2000, burst(2000, 1000)).await;
    mount_range(
        &server,
        ("07:55", "08:00"),
        1,
        1,
        flows_at(&["07:58"], 9000),
    )
    .await;
    let a = cmd(&server).args(args).arg("--yes").assert().success();
    let err = String::from_utf8_lossy(&a.get_output().stderr).to_string();
    assert!(err.contains("backfill raises the cap to 25000"), "{err}");
    assert_eq!(sent_caps(&server).await, vec![2000, 25_000]);
    server.verify().await;
}

/// 06:00 to 08:00 at cap 4: four flows over the last hour, truncated. The
/// rate fits 30m slices, so the walk is 07:30, 07:00, 06:30, 06:00.
async fn mount_truncated_first(server: &MockServer, times: u64) {
    mount_objects(server).await;
    let first = flows_at(&["08:00", "07:40", "07:20", "07:00"], 1000);
    mount_range(server, ("06:00", "08:00"), times, 4, first).await;
}

const BACKFILL_ARGS: [&str; 8] = [
    "traffic",
    "query",
    "--from",
    "2026-10-10T06:00:00Z",
    "--to",
    "2026-10-10T08:00:00Z",
    "--max-results",
    "4",
];

fn ports(out: &Value) -> Vec<u64> {
    out["data"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["port"].as_u64().unwrap())
        .collect()
}

#[tokio::test(flavor = "multi_thread")]
async fn traffic_backfill_walks_the_window_and_replaces_the_truncated_rows() {
    let server = MockServer::start().await;
    mount_truncated_first(&server, 1).await;
    for (i, (s, e, last)) in [
        ("07:30", "08:00", "07:50"),
        ("07:00", "07:30", "07:10"),
        ("06:30", "07:00", "06:45"),
        ("06:00", "06:30", "06:15"),
    ]
    .into_iter()
    .enumerate()
    {
        mount_range(&server, (s, e), 1, 1, flows_at(&[last], 5000 + i as u64)).await;
    }
    let out = stdout_json(
        cmd(&server)
            .args(BACKFILL_ARGS)
            .arg("--backfill")
            .assert()
            .success(),
    );
    assert_eq!(
        ports(&out),
        vec![5000, 5001, 5002, 5003],
        "each slice's row, newest first; the truncated rows are gone"
    );
    assert_eq!(out["meta"]["slices"], 4);
    assert_eq!(out["meta"]["covered"], "2h");
    assert_eq!(out["meta"]["truncated"], false);
    assert_eq!(out["meta"]["window"]["start"], "2026-10-10T06:00:00Z");
    // Name index (4 lists) + 5 queries of POST, status, download.
    assert_eq!(out["meta"]["api_calls"], 4 + 5 * 3);
    server.verify().await;
}

/// The first 30m slice truncates: its range is fetched again in 15m halves;
/// `--max-slices 3` ends the run there and stderr says where to continue.
#[tokio::test(flavor = "multi_thread")]
async fn traffic_backfill_halves_a_truncated_slice_and_stops_at_the_limit() {
    let server = MockServer::start().await;
    mount_truncated_first(&server, 1).await;
    let full = flows_at(&["07:59", "07:50", "07:40", "07:31"], 6000);
    mount_range(&server, ("07:30", "08:00"), 1, 9, full).await;
    mount_range(
        &server,
        ("07:45", "08:00"),
        1,
        1,
        flows_at(&["07:55"], 7000),
    )
    .await;
    mount_range(
        &server,
        ("07:30", "07:45"),
        1,
        1,
        flows_at(&["07:35"], 7100),
    )
    .await;
    mount_range(
        &server,
        ("07:15", "07:30"),
        0,
        1,
        flows_at(&["07:20"], 7200),
    )
    .await;
    let a = cmd(&server)
        .args(BACKFILL_ARGS)
        .args(["--backfill", "--max-slices", "3"])
        .assert()
        .success();
    let err = String::from_utf8_lossy(&a.get_output().stderr).to_string();
    assert!(
        err.contains(
            "slice 2026-10-10T07:30:00Z to 2026-10-10T08:00:00Z truncated at 4: halved to 15m"
        ),
        "{err}"
    );
    assert!(
        err.contains(
            "backfill stopped after 3 queries: 6 slices of 15m left; --from 2026-10-10T06:00:00Z --to 2026-10-10T07:30:00Z continues"
        ),
        "{err}"
    );
    let out = stdout_json(a);
    assert_eq!(ports(&out), vec![7000, 7100]);
    assert_eq!(out["meta"]["slices"], 2);
    assert_eq!(out["meta"]["covered"], "30m");
    assert_eq!(out["meta"]["truncated"], true);
    assert_eq!(out["meta"]["api_calls"], 4 + 4 * 3);
    server.verify().await;
    // At most 50 per run.
    cmd(&server)
        .args(BACKFILL_ARGS)
        .args(["--backfill", "--max-slices", "51"])
        .assert()
        .code(2);
}

#[tokio::test(flavor = "multi_thread")]
async fn traffic_without_backfill_reports_coverage_only() {
    let server = MockServer::start().await;
    mount_truncated_first(&server, 1).await;
    let out = stdout_json(cmd(&server).args(BACKFILL_ARGS).assert().success());
    assert_eq!(out["data"].as_array().unwrap().len(), 4);
    assert_eq!(out["meta"]["covered"], "1h");
    assert_eq!(out["meta"]["truncated"], true);
    assert!(out["meta"].get("slices").is_none(), "{}", out["meta"]);
    server.verify().await;
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
    assert_eq!(
        out["meta"]["covered"], "2h",
        "a complete result covers its window"
    );
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
    let sections = out["data"]["sections"].as_array().unwrap();
    assert_eq!(sections.len(), 1, "both blocked flows go to shop/prod");
    assert_eq!(sections[0]["rows"].as_array().unwrap().len(), 2);
    assert_eq!(
        sections[0]["rows"][0]["app_names"],
        json!(["host-1.example"])
    );
    assert_eq!(sections[0]["rows"][0]["service"], "8443/tcp");
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
    assert_eq!(out["data"]["max_results"], 10_000);
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
    // Past the hard maximum even --yes does not help.
    let assert = cmd(&quiet)
        .args(["traffic", "query", "--max-results", "200001", "--yes"])
        .assert()
        .code(2);
    let err: Value = serde_json::from_slice(&assert.get_output().stderr).unwrap();
    assert!(
        err["error"]["message"]
            .as_str()
            .unwrap()
            .contains("maximum of 200000"),
        "{err}"
    );
    let out = stdout_json(
        cmd(&quiet)
            .args(["traffic", "query", "--max-results", "200000", "--dry-run"])
            .assert()
            .success(),
    );
    assert_eq!(out["data"]["max_results"], 200_000);
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
    let csv = Command::cargo_bin("illumio-tui")
        .unwrap()
        .env_clear()
        .args([
            "--config",
            cfg.to_str().unwrap(),
            "profile",
            "list",
            "--csv",
        ])
        .assert()
        .success();
    let text = String::from_utf8_lossy(&csv.get_output().stdout).to_string();
    assert_eq!(
        text,
        "name,host,org_id,default\r\na,a.example,1,false\r\nb,b.example,2,true\r\n"
    );
    // `profile use` prints a record, not a table: refused before the file is touched.
    let before = std::fs::read_to_string(&cfg).unwrap();
    Command::cargo_bin("illumio-tui")
        .unwrap()
        .env_clear()
        .args([
            "--config",
            cfg.to_str().unwrap(),
            "profile",
            "use",
            "a",
            "--csv",
        ])
        .assert()
        .code(2);
    assert_eq!(std::fs::read_to_string(&cfg).unwrap(), before);
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

/// The fixture workloads behind `--impact`, read `n` times (once per command
/// that needs them); `total` makes the PCE report more than it returned.
async fn mount_workloads(server: &MockServer, n: u64, total: Option<u64>) {
    use wiremock::matchers::query_param;
    let mut response = ResponseTemplate::new(200)
        .set_body_string(include_str!("../../../fixtures/workloads.json"))
        .insert_header("Content-Type", "application/json");
    if let Some(t) = total {
        response = response.insert_header("X-Total-Count", t.to_string());
    }
    Mock::given(method("GET"))
        .and(path("/api/v2/orgs/5/workloads"))
        .and(query_param("max_results", "10000"))
        .respond_with(response)
        .expect(n)
        .mount(server)
        .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn policy_rules_impact_reads_the_workloads_once_per_command() {
    let server = MockServer::start().await;
    mount_policy(&server).await;
    mount_workloads(&server, 2, None).await; // the JSON and the table command
    let out = stdout_json(
        cmd(&server)
            .args(["policy", "rules", "shop-prod", "--impact"])
            .assert()
            .success(),
    );
    let rules = out["data"]["rules"].as_array().unwrap();
    let pc = |i: usize| {
        (
            rules[i]["impact"]["providers"].clone(),
            rules[i]["impact"]["consumers"].clone(),
        )
    };
    assert_eq!(pc(0), (json!(1), json!(1)), "db ← web");
    assert_eq!(
        pc(1),
        (json!(1), json!(0)),
        "an IP list consumer matches no workload"
    );
    assert_eq!(pc(2), (json!(2), json!(0)));
    assert_eq!(pc(3), (json!(1), json!(2)));
    assert_eq!(
        rules[0]["impact"]["per_scope"],
        json!([{"scope": ["app=shop", "env=prod"], "providers": 1, "consumers": 1}])
    );
    assert_eq!(
        rules[1]["impact"]["ip_lists"],
        json!([{
            "name": "Corp", "href": "/orgs/5/sec_policy/draft/ip_lists/21",
            "size": {"v4": "65536", "v6": "0", "unbounded": null, "ranges": 1, "fqdns": 1},
            "side": "consumer"
        }])
    );
    assert_eq!(
        rules[2]["impact"]["ip_lists"][0]["size"]["unbounded"],
        "0.0.0.0/0"
    );
    assert_eq!(rules[0]["impact"]["unsupported"], json!([]));
    assert_eq!(rules[0]["impact"]["truncated"], false);
    assert_eq!(
        rules[1]["consumers"][0]["name"], "ipl:Corp",
        "the rule's own fields stay"
    );
    assert_eq!(out["meta"]["truncated"], false);
    assert_eq!(
        out["meta"]["api_calls"], 6,
        "four object lists + rulesets + one workloads read"
    );
    cmd(&server)
        .args(["policy", "rules", "shop-prod", "--impact", "--table"])
        .assert()
        .success()
        .stdout(predicate::str::contains("P/C").and(predicate::str::contains("1/2")));
    let out = stdout_json(
        cmd(&server)
            .args(["policy", "rules", "shop-prod"])
            .assert()
            .success(),
    );
    assert!(
        out["data"]["rules"][0].get("impact").is_none(),
        "no --impact, no field and no workloads read"
    );
    assert_eq!(out["meta"]["api_calls"], 5);
}

#[tokio::test(flavor = "multi_thread")]
async fn policy_rules_impact_reports_one_rule_by_href() {
    let server = MockServer::start().await;
    // A ruleset href where a rule href belongs: exit 2 before any request.
    let a = cmd(&server)
        .args([
            "policy",
            "rules",
            "impact",
            "/orgs/5/sec_policy/draft/rule_sets/100",
        ])
        .assert()
        .code(2);
    assert!(
        String::from_utf8_lossy(&a.get_output().stderr).contains("sec_rules"),
        "names the expected shape"
    );
    assert!(server.received_requests().await.unwrap().is_empty());
    mount_policy(&server).await;
    mount_workloads(&server, 3, Some(40)).await; // the three rules found below
    let out = stdout_json(
        cmd(&server)
            .args([
                "policy",
                "rules",
                "impact",
                "/orgs/5/sec_policy/active/rule_sets/100/sec_rules/2",
            ])
            .assert()
            .success(),
    );
    assert_eq!(
        out["data"]["href"],
        "/orgs/5/sec_policy/draft/rule_sets/100/sec_rules/2"
    );
    assert_eq!(out["data"]["impact"]["providers"], 1);
    assert_eq!(out["data"]["impact"]["ip_lists"][0]["name"], "Corp");
    assert_eq!(out["data"]["impact"]["truncated"], true);
    assert_eq!(
        out["meta"]["truncated"], true,
        "the workload read was cut short"
    );
    assert_eq!(out["meta"]["api_calls"], 6);
    let out = stdout_json(
        cmd(&server)
            .args([
                "policy",
                "rules",
                "impact",
                "/orgs/5/sec_policy/draft/rule_sets/100/deny_rules/8",
            ])
            .assert()
            .success(),
    );
    assert_eq!(out["data"]["kind"], "override_deny");
    assert_eq!(
        (
            out["data"]["impact"]["providers"].clone(),
            out["data"]["impact"]["consumers"].clone()
        ),
        (json!(1), json!(2))
    );
    let a = cmd(&server)
        .args([
            "policy",
            "rules",
            "impact",
            "/orgs/5/sec_policy/draft/rule_sets/100/sec_rules/99",
        ])
        .assert()
        .code(4);
    let err: Value = serde_json::from_slice(&a.get_output().stderr).unwrap();
    assert_eq!(err["error"]["kind"], "not_found");
    cmd(&server)
        .args([
            "policy",
            "rules",
            "impact",
            "/orgs/5/sec_policy/draft/rule_sets/101/sec_rules/3",
            "--table",
        ])
        .assert()
        .success()
        .stdout(
            predicate::str::contains("impact")
                .and(predicate::str::contains("scope All"))
                .and(predicate::str::contains("4/2")),
        );
}

#[tokio::test(flavor = "multi_thread")]
async fn traffic_groups_sum_bytes_and_honour_keys() {
    let server = MockServer::start().await;
    mount_traffic(&server, 2).await; // two processes, no shared cache
    let out = stdout_json(
        cmd(&server)
            .args(["traffic", "groups", "--window", "1h"])
            .assert()
            .success(),
    );
    let data = out["data"].as_array().unwrap();
    assert_eq!(data.len(), 4, "the profile's app,env pairs");
    assert_eq!(data[0]["src"], json!(["shop", "prod"]));
    assert_eq!(data[0]["dst"], json!(["shop", "prod"]));
    assert_eq!(data[0]["connections"], 310);
    assert_eq!(data[0]["bytes"], 124_000);
    assert_eq!(data[1]["src"], json!(["ipl:Corp", "-"]));
    assert_eq!(data[1]["bytes"], 0, "no byte counts: 0 in JSON");
    assert_eq!(out["meta"]["cache"], "miss");
    assert_eq!(out["meta"]["truncated"], false);
    let table = cmd(&server)
        .args([
            "traffic", "groups", "--window", "1h", "--keys", "role", "--table",
        ])
        .assert()
        .success();
    let text = String::from_utf8_lossy(&table.get_output().stdout).to_string();
    let header = text.lines().next().unwrap();
    for h in [
        "src",
        "dst",
        "flows",
        "conns",
        "bytes",
        "allowed",
        "pot.blocked",
        "blocked",
    ] {
        assert!(header.contains(h), "{header}");
    }
    assert!(text.contains("web") && text.contains("124000"), "{text}");
    cmd(&server)
        .args(["traffic", "groups", "--blocked"])
        .assert()
        .code(2);
}

#[tokio::test(flavor = "multi_thread")]
async fn traffic_groups_services_adds_the_service_to_each_pair() {
    let server = MockServer::start().await;
    mount_traffic(&server, 3).await; // --by with --services asks for nothing
    let out = stdout_json(
        cmd(&server)
            .args(["traffic", "groups", "--window", "1h", "--services"])
            .assert()
            .success(),
    );
    let data = out["data"].as_array().unwrap();
    assert_eq!(data.len(), 4);
    assert_eq!(data[0]["src"], json!(["shop", "prod"]));
    assert_eq!(data[0]["service"], "5432/tcp");
    assert_eq!(data[0]["connections"], 310);
    let nas = data
        .iter()
        .find(|g| g["src"] == json!(["unmanaged-nas", "-"]))
        .unwrap();
    assert_eq!(nas["service"], "icmp", "no port: just the protocol");
    let table = cmd(&server)
        .args([
            "traffic",
            "groups",
            "--window",
            "1h",
            "--services",
            "--table",
        ])
        .assert()
        .success();
    let text = String::from_utf8_lossy(&table.get_output().stdout).to_string();
    let header: Vec<&str> = text.lines().next().unwrap().split_whitespace().collect();
    assert_eq!(
        header,
        [
            "src",
            "dst",
            "service",
            "flows",
            "conns",
            "bytes",
            "allowed",
            "pot.blocked",
            "blocked"
        ]
    );
    assert!(text.contains("5432/tcp"), "{text}");
    let text = csv_stdout(
        &cmd(&server)
            .args(["traffic", "groups", "--window", "1h", "--services", "--csv"])
            .assert()
            .success(),
    );
    assert_eq!(
        first_two(&text),
        [
            "src,dst,service,flows,conns,bytes,allowed,pot.blocked,blocked",
            "shop/prod,shop/prod,5432/tcp,1,310,124000,1,0,0"
        ]
    );
    cmd(&server)
        .args([
            "traffic",
            "groups",
            "--window",
            "1h",
            "--services",
            "--by",
            "in",
        ])
        .assert()
        .code(2);
}

#[tokio::test(flavor = "multi_thread")]
async fn traffic_groups_by_in_out_both_with_show_and_app() {
    let server = MockServer::start().await;
    // --by in, --by out (name-keyed app: unfocused), --by both, --by in
    // --app shop/prod (focused: 1), --app shop/prod (pairs, focused: 2).
    mount_traffic(&server, 6).await;
    let out = stdout_json(
        cmd(&server)
            .args(["traffic", "groups", "--window", "1h", "--by", "in"])
            .assert()
            .success(),
    );
    let sections = out["data"]["sections"].as_array().unwrap();
    assert_eq!(sections[0]["app"], json!(["shop", "prod"]));
    assert_eq!(sections[0]["totals"]["connections"], 325);
    assert_eq!(sections[0]["rows"].as_array().unwrap().len(), 3);
    let out = stdout_json(
        cmd(&server)
            .args([
                "traffic",
                "groups",
                "--window",
                "1h",
                "--by",
                "out",
                "--show",
                "blocked-only",
                "--app",
                "unmanaged-nas/-",
            ])
            .assert()
            .success(),
    );
    let sections = out["data"]["sections"].as_array().unwrap();
    assert_eq!(
        sections.len(),
        1,
        "a name-keyed app keeps narrowing the sections"
    );
    assert_eq!(sections[0]["direction"], "out");
    assert_eq!(sections[0]["totals"]["blocked"], 1);
    assert!(out["meta"].get("queries").is_none(), "not a focused query");
    let table = cmd(&server)
        .args([
            "traffic", "groups", "--window", "1h", "--by", "both", "--table",
        ])
        .assert()
        .success();
    let text = String::from_utf8_lossy(&table.get_output().stdout).to_string();
    assert!(
        text.contains("\nshop/prod  ← 325 conns · 1 blocked · 1 pot.blocked  → 315 conns\n"),
        "{text}"
    );
    assert!(
        text.contains("\n  ← ipl:Corp/-") && text.contains("\n  → "),
        "{text}"
    );
    let out = stdout_json(
        cmd(&server)
            .args([
                "traffic",
                "groups",
                "--window",
                "1h",
                "--by",
                "in",
                "--app",
                "shop/prod",
            ])
            .assert()
            .success(),
    );
    let sections = out["data"]["sections"].as_array().unwrap();
    assert_eq!(sections.len(), 1);
    assert_eq!(
        sections[0]["totals"]["connections"], 325,
        "the same section, now from a focused query"
    );
    assert_eq!(out["meta"]["queries"], 1, "--by in focuses the query on in");
    let miss = cmd(&server)
        .args([
            "traffic", "groups", "--window", "1h", "--by", "in", "--app", "pos/PCI",
        ])
        .assert()
        .code(2);
    let err = String::from_utf8_lossy(&miss.get_output().stderr);
    assert!(err.contains(r#"unknown app \"pos/PCI\""#), "{err}");
    let out = stdout_json(
        cmd(&server)
            .args(["traffic", "groups", "--window", "1h", "--app", "shop/prod"])
            .assert()
            .success(),
    );
    let pairs = out["data"]
        .as_array()
        .expect("--app without --by gives the pairs");
    assert!(
        pairs
            .iter()
            .all(|p| p["src"] == json!(["shop", "prod"]) || p["dst"] == json!(["shop", "prod"])),
        "{pairs:?}"
    );
    assert_eq!(out["meta"]["queries"], 2);
}

#[tokio::test(flavor = "multi_thread")]
async fn traffic_app_dry_run_prints_one_body_per_side() {
    let server = MockServer::start().await;
    mount_traffic(&server, 0).await;
    let labels =
        json!([{"label": {"href": "/orgs/5/labels/3"}}, {"label": {"href": "/orgs/5/labels/4"}}]);
    let out = stdout_json(
        cmd(&server)
            .args([
                "traffic",
                "query",
                "--window",
                "1h",
                "--app",
                "shop/prod",
                "--dry-run",
            ])
            .assert()
            .success(),
    );
    let bodies = out["data"].as_array().expect("one body per side");
    assert_eq!(bodies.len(), 2);
    assert_eq!(
        bodies[0]["destinations"]["include"],
        json!([labels.clone()])
    );
    assert_eq!(bodies[0]["sources"]["include"], json!([[]]));
    assert_eq!(bodies[1]["sources"]["include"], json!([labels.clone()]));
    assert_eq!(bodies[1]["destinations"]["include"], json!([[]]));
    assert_eq!(out["meta"]["cache"], "none");
    for (sub, dir, side) in [("groups", "in", "destinations"), ("mesh", "out", "sources")] {
        let out = stdout_json(
            cmd(&server)
                .args([
                    "traffic",
                    sub,
                    "--window",
                    "1h",
                    "--app",
                    "shop/prod",
                    "--direction",
                    dir,
                    "--dry-run",
                ])
                .assert()
                .success(),
        );
        let bodies = out["data"].as_array().unwrap();
        assert_eq!(bodies.len(), 1, "{sub} --direction {dir}");
        assert_eq!(bodies[0][side]["include"], json!([labels.clone()]));
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn traffic_app_spends_the_cap_on_the_app_and_counts_its_queries() {
    let server = MockServer::start().await;
    mount_traffic(&server, 4).await; // groups --direction in: 1; query (both): 2; query out: 1
    let out = stdout_json(
        cmd(&server)
            .args([
                "traffic",
                "groups",
                "--window",
                "1h",
                "--app",
                "shop/prod",
                "--direction",
                "in",
            ])
            .assert()
            .success(),
    );
    let pairs = out["data"].as_array().unwrap();
    assert_eq!(pairs.len(), 3, "only the flows into shop/prod: {pairs:?}");
    assert!(pairs.iter().all(|p| p["dst"] == json!(["shop", "prod"])));
    assert_eq!(out["meta"]["queries"], 1);
    let out = stdout_json(
        cmd(&server)
            .args(["traffic", "query", "--window", "1h", "--app", "shop/prod"])
            .assert()
            .success(),
    );
    assert_eq!(
        out["data"].as_array().unwrap().len(),
        4,
        "both queries returned flow 0: it counts once"
    );
    assert_eq!(out["meta"]["queries"], 2);
    let table = cmd(&server)
        .args([
            "traffic",
            "query",
            "--window",
            "1h",
            "--app",
            "shop/prod",
            "--direction",
            "out",
            "--table",
        ])
        .assert()
        .success();
    let err = String::from_utf8_lossy(&table.get_output().stderr);
    assert!(
        err.contains("· 1 query"),
        "the meta line counts the query: {err}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn traffic_app_mistakes_exit_2_before_any_traffic_request() {
    let server = MockServer::start().await;
    mount_traffic(&server, 0).await;
    let err = cmd(&server)
        .args(["traffic", "groups", "--window", "1h", "--app", "shp/prod"])
        .assert()
        .code(2);
    let err = String::from_utf8_lossy(&err.get_output().stderr).to_string();
    assert!(
        err.contains(r#"unknown app \"shp/prod\": no app=shp label (closest: shop)"#),
        "{err}"
    );
    let err = cmd(&server)
        .args([
            "traffic",
            "groups",
            "--window",
            "1h",
            "--by",
            "in",
            "--app",
            "shop/prod",
            "--direction",
            "in",
        ])
        .assert()
        .code(2);
    assert!(
        String::from_utf8_lossy(&err.get_output().stderr)
            .contains("--direction conflicts with --by")
    );
    cmd(&server)
        .args(["traffic", "query", "--window", "1h", "--direction", "in"])
        .assert()
        .code(2);
    let err = cmd(&server)
        .args([
            "traffic",
            "query",
            "--window",
            "1h",
            "--app",
            "shop/prod",
            "--backfill",
        ])
        .assert()
        .code(2);
    assert!(
        String::from_utf8_lossy(&err.get_output().stderr)
            .contains("--backfill does not combine with --app")
    );
    let err = cmd(&server)
        .args([
            "traffic",
            "query",
            "--window",
            "1h",
            "--app",
            "shop/prod",
            "--max-results",
            "20000",
        ])
        .assert()
        .code(2);
    let err = String::from_utf8_lossy(&err.get_output().stderr).to_string();
    assert!(
        err.contains("(--app runs it as 2 queries); re-run with --yes to confirm"),
        "the guard names the count: {err}"
    );
    let err = cmd(&server)
        .args([
            "traffic",
            "query",
            "--window",
            "1h",
            "--app",
            "shop/prod",
            "--direction",
            "in",
            "--max-results",
            "20000",
        ])
        .assert()
        .code(2);
    assert!(!String::from_utf8_lossy(&err.get_output().stderr).contains("runs it as"));
    let err = cmd(&server)
        .args([
            "traffic",
            "query",
            "--window",
            "1h",
            "--app",
            "unmanaged-nas/-",
        ])
        .assert()
        .code(2);
    assert!(
        String::from_utf8_lossy(&err.get_output().stderr).contains("a - slot cannot be queried"),
        "a name-keyed app without --by cannot focus a query"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn traffic_groups_show_filters_the_pairs_and_the_services() {
    let server = MockServer::start().await;
    mount_traffic(&server, 4).await;
    let all = stdout_json(
        cmd(&server)
            .args(["traffic", "groups", "--window", "1h"])
            .assert()
            .success(),
    );
    let out = stdout_json(
        cmd(&server)
            .args(["traffic", "groups", "--window", "1h", "--show", "blocked"])
            .assert()
            .success(),
    );
    let data = out["data"].as_array().unwrap();
    assert!(
        !data.is_empty() && data.len() < all["data"].as_array().unwrap().len(),
        "{out}"
    );
    assert!(data.iter().all(|g| g["allowed"] == 0), "{out}");
    let out = stdout_json(
        cmd(&server)
            .args([
                "traffic",
                "groups",
                "--window",
                "1h",
                "--services",
                "--show",
                "blocked-only",
            ])
            .assert()
            .success(),
    );
    let data = out["data"].as_array().unwrap();
    assert!(!data.is_empty(), "{out}");
    assert!(
        data.iter().all(|g| g["allowed"] == 0
            && g["potentially_blocked"] == 0
            && g["service"].is_string()),
        "{out}"
    );
    let text = csv_stdout(
        &cmd(&server)
            .args([
                "traffic",
                "groups",
                "--window",
                "1h",
                "--services",
                "--show",
                "blocked",
                "--csv",
            ])
            .assert()
            .success(),
    );
    let mut lines = text.lines();
    assert_eq!(
        lines.next(),
        Some("src,dst,service,flows,conns,bytes,allowed,pot.blocked,blocked")
    );
    let rows: Vec<&str> = lines.collect();
    assert!(!rows.is_empty(), "{text}");
    assert!(
        rows.iter().all(|r| r.split(',').nth(6) == Some("0")),
        "{text}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn traffic_mesh_places_processes_by_direction_and_drops_hidden_columns() {
    let server = MockServer::start().await;
    mount_traffic(&server, 3).await;
    let out = stdout_json(
        cmd(&server)
            .args(["traffic", "mesh", "--window", "1h"])
            .assert()
            .success(),
    );
    let rows = out["data"].as_array().unwrap();
    assert_eq!(rows.len(), 4);
    assert_eq!(rows[0]["user"], "postgres");
    assert_eq!(
        rows[0]["process"], "postgres",
        "outbound: the source's process"
    );
    assert!(rows[0]["dst_process"].is_null());
    assert_eq!(rows[0]["port"], 5432);
    assert_eq!(rows[0]["proto"], "tcp");
    assert_eq!(rows[0]["bytes"], 124_000);
    assert_eq!(rows[0]["allowed"], 1);
    let corp = rows
        .iter()
        .find(|r| r["src"] == json!(["ipl:Corp", "-"]))
        .unwrap();
    assert!(corp["user"].is_null() && corp["process"].is_null());
    let out = stdout_json(
        cmd(&server)
            .args([
                "traffic",
                "mesh",
                "--window",
                "1h",
                "--no-user",
                "--keys",
                "role",
            ])
            .assert()
            .success(),
    );
    assert!(out["data"][0]["user"].is_null());
    assert_eq!(out["data"][0]["process"], "postgres");
    assert_eq!(out["data"][0]["src"], json!(["web"]));
    let table = cmd(&server)
        .args([
            "traffic",
            "mesh",
            "--window",
            "1h",
            "--no-process",
            "--table",
        ])
        .assert()
        .success();
    let text = String::from_utf8_lossy(&table.get_output().stdout).to_string();
    let header = text.lines().next().unwrap();
    for h in [
        "src",
        "dst",
        "service",
        "dst process",
        "flows",
        "conns",
        "bytes",
        "decision",
    ] {
        assert!(header.contains(h), "{header}");
    }
    assert_eq!(header.split_whitespace().next(), Some("user"), "{header}");
    let (process, src) = (header.find(" process "), header.find(" src "));
    assert!(process.is_some() && src.is_some(), "{header}");
    assert!(process < src, "process comes before src: {header}");
    assert!(text.contains("potentially_blocked"), "{text}");
}

#[tokio::test(flavor = "multi_thread")]
async fn traffic_groups_and_mesh_dry_runs_send_nothing() {
    let server = MockServer::start().await;
    mount_traffic(&server, 0).await;
    for sub in ["groups", "mesh"] {
        let out = stdout_json(
            cmd(&server)
                .args([
                    "traffic",
                    sub,
                    "--window",
                    "1h",
                    "--port",
                    "443",
                    "--dry-run",
                ])
                .assert()
                .success(),
        );
        assert_eq!(out["data"]["services"]["include"], json!([{"port": 443}]));
        assert_eq!(out["data"]["max_results"], 10_000);
        assert_eq!(out["meta"]["cache"], "none");
    }
}

/// stdout of a `--csv` run, which must hold nothing but CSV: the meta line
/// goes to stderr.
fn csv_stdout(assert: &assert_cmd::assert::Assert) -> String {
    let out = assert.get_output();
    let text = String::from_utf8(out.stdout.clone()).expect("UTF-8");
    assert!(!text.contains("api call"), "meta on stdout: {text}");
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("api call"),
        "meta on stderr"
    );
    assert!(text.ends_with("\r\n"), "CRLF line ends: {text:?}");
    text
}

/// The first two CSV lines.
fn first_two(text: &str) -> Vec<&str> {
    text.split("\r\n").take(2).collect()
}

#[tokio::test(flavor = "multi_thread")]
async fn workloads_csv_prints_full_cells_and_meta_on_stderr() {
    let server = MockServer::start().await;
    mount_objects(&server).await;
    Mock::given(method("GET"))
        .and(path("/api/v2/orgs/5/workloads"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(include_str!("../../../fixtures/workloads.json"))
                .insert_header("Content-Type", "application/json"),
        )
        .expect(1)
        .mount(&server)
        .await;
    let text = csv_stdout(&cmd(&server).args(["workloads", "--csv"]).assert().success());
    let lines = first_two(&text);
    assert_eq!(
        lines[0],
        "hostname,name,ip,labels,enforcement,online,agent,sync,heartbeat_age_secs,health,href"
    );
    // The heartbeat age and the health follow real time.
    let cells: Vec<&str> = lines[1].split(',').collect();
    assert_eq!(
        cells[..8],
        [
            "host-1.example",
            "web01",
            "10.0.1.10",
            "app=shop; env=prod; role=web",
            "full",
            "true",
            "active",
            "applied"
        ]
    );
    assert!(cells[8].parse::<u64>().is_ok(), "seconds: {}", cells[8]);
    assert_eq!(cells[10], "/orgs/5/workloads/aaaa-1");
}

#[tokio::test(flavor = "multi_thread")]
async fn object_lists_csv() {
    let server = MockServer::start().await;
    mount_objects(&server).await;
    let text = csv_stdout(&cmd(&server).args(["labels", "--csv"]).assert().success());
    assert_eq!(
        first_two(&text),
        ["key,value,deleted,href", "role,web,false,/orgs/5/labels/1"]
    );
    let text = csv_stdout(&cmd(&server).args(["iplists", "--csv"]).assert().success());
    assert!(
        text.contains(",10.1.0.0-10.1.255.255; *.corp.example,"),
        "{text}"
    );
    let text = csv_stdout(&cmd(&server).args(["services", "--csv"]).assert().success());
    assert!(text.contains(",80/tcp; 443/tcp; 8000-8099/tcp,"), "{text}");
}

#[tokio::test(flavor = "multi_thread")]
async fn traffic_query_csv_lists_every_label() {
    let server = MockServer::start().await;
    mount_traffic(&server, 1).await;
    let text = csv_stdout(
        &cmd(&server)
            .args(["traffic", "query", "--window", "1h", "--csv"])
            .assert()
            .success(),
    );
    assert_eq!(
        first_two(&text),
        [
            "last seen,src,src labels,dst,dst labels,service,conns,decision,process",
            "2026-10-10T07:31:02Z,host-1.example,app=shop; env=prod; role=web,host-2.example,\
             app=shop; env=prod; role=db,5432/tcp,310,allowed,postgres"
        ]
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn traffic_groups_and_blocked_csv_print_one_row_per_peer() {
    let server = MockServer::start().await;
    mount_traffic(&server, 2).await;
    let header = "app,direction,peer,peer hosts,peer names,app hosts,app names,service,\
                  flows,conns,bytes,last seen,decision";
    let text = csv_stdout(
        &cmd(&server)
            .args(["traffic", "groups", "--window", "1h", "--by", "in", "--csv"])
            .assert()
            .success(),
    );
    assert_eq!(
        first_two(&text),
        [
            header,
            "shop/prod,in,shop/prod,1,host-1.example,1,host-2.example,5432/tcp,1,310,124000,\
             2026-10-10T07:31:02Z,allowed"
        ]
    );
    let text = csv_stdout(
        &cmd(&server)
            .args(["traffic", "query", "--window", "1h", "--blocked", "--csv"])
            .assert()
            .success(),
    );
    assert_eq!(
        first_two(&text),
        [
            header,
            "shop/prod,in,ipl:Corp/-,1,10.1.4.9,1,host-1.example,8443/tcp,1,12,0,\
             2026-10-10T07:30:58Z,potentially_blocked"
        ]
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn csv_is_a_usage_error_with_json_table_and_where_there_is_no_table() {
    let server = MockServer::start().await;
    // Nothing may be asked of the PCE.
    Mock::given(wiremock::matchers::any())
        .respond_with(ResponseTemplate::new(500))
        .expect(0)
        .mount(&server)
        .await;
    for args in [
        vec!["labels", "--csv", "--json"],
        vec!["labels", "--csv", "--table"],
    ] {
        cmd(&server)
            .args(&args)
            .assert()
            .code(2)
            .stderr(predicate::str::contains("cannot be used with"));
    }
    for (args, name) in [
        (vec!["health", "--csv"], "health"),
        (
            vec!["labels", "add", "app=x", "--dry-run", "--csv"],
            "labels add/update/delete",
        ),
        (
            vec!["policy", "provision", "--all", "--dry-run", "--csv"],
            "policy provision",
        ),
        (
            vec!["policy", "rules", "impact", "/orgs/5/x", "--csv"],
            "policy rules impact",
        ),
        (
            vec!["traffic", "query", "--window", "1h", "--dry-run", "--csv"],
            "traffic --dry-run",
        ),
    ] {
        let a = cmd(&server).args(&args).assert().code(2);
        assert!(a.get_output().stdout.is_empty(), "{args:?}");
        let err: Value = serde_json::from_slice(&a.get_output().stderr).unwrap();
        assert_eq!(err["error"]["kind"], "args", "{args:?}");
        assert!(
            err["error"]["message"]
                .as_str()
                .unwrap()
                .contains(&format!("--csv: {name} prints no table")),
            "{err}"
        );
    }
}
async fn mount_app(server: &MockServer, posts: u64) {
    mount_app_counting(server, posts, None).await;
}

/// `mount_app`; `total` sets `X-Total-Count` on the workloads and rulesets
/// answers (more than they hold: both reads are truncated).
async fn mount_app_counting(server: &MockServer, posts: u64, total: Option<u64>) {
    let answer = |body: &str| {
        let r = ResponseTemplate::new(200)
            .set_body_string(body)
            .insert_header("Content-Type", "application/json");
        match total {
            Some(n) => r.insert_header("X-Total-Count", n.to_string().as_str()),
            None => r,
        }
    };
    mount_traffic(server, posts).await;
    Mock::given(method("GET"))
        .and(path("/api/v2/orgs/5/workloads"))
        .and(query_param(
            "labels",
            r#"[["/orgs/5/labels/3","/orgs/5/labels/4"]]"#,
        ))
        .respond_with(answer(include_str!("../../../fixtures/workloads.json")))
        .mount(server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v2/orgs/5/sec_policy/draft/rule_sets"))
        .respond_with(answer(include_str!("../../../fixtures/rule_sets.json")))
        .mount(server)
        .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn app_reports_truncated_workloads_and_rulesets_in_their_sections() {
    let server = MockServer::start().await;
    mount_app_counting(&server, 4, Some(50_000)).await;
    let out = stdout_json(
        cmd(&server)
            .args(["app", "shop/prod", "--window", "1h"])
            .assert()
            .success(),
    );
    assert_eq!(out["data"]["workloads"]["truncated"], true);
    assert_eq!(out["data"]["policy"]["truncated"], true);
    assert_eq!(out["meta"]["truncated"], false, "the traffic was complete");
    let table = cmd(&server)
        .args(["app", "shop/prod", "--window", "1h", "--table"])
        .assert()
        .success();
    let text = String::from_utf8_lossy(&table.get_output().stdout).to_string();
    assert!(text.contains("stale · truncated"), "{text}");
    assert!(text.contains("policy · draft · 2 · truncated"), "{text}");
}

#[tokio::test(flavor = "multi_thread")]
async fn app_json_section_keeps_only_that_section() {
    let server = MockServer::start().await;
    mount_app(&server, 2).await;
    let out = stdout_json(
        cmd(&server)
            .args(["app", "shop/prod", "--window", "1h", "--section", "policy"])
            .assert()
            .success(),
    );
    let keys: Vec<&str> = out["data"]
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(keys, vec!["app", "direction", "policy"]);
    assert_eq!(out["data"]["policy"]["rows"].as_array().unwrap().len(), 2);
}

#[tokio::test(flavor = "multi_thread")]
async fn app_prints_one_envelope_with_every_section() {
    let server = MockServer::start().await;
    mount_app(&server, 2).await;
    let out = stdout_json(
        cmd(&server)
            .args(["app", "shop/prod", "--window", "1h"])
            .assert()
            .success(),
    );
    let d = &out["data"];
    assert_eq!(d["app"], "shop/prod");
    assert_eq!(d["direction"], "both");
    assert_eq!(d["workloads"]["summary"]["workloads"], 2);
    assert_eq!(d["workloads"]["summary"]["enforcement"]["enforced"], 1);
    assert_eq!(d["workloads"]["summary"]["enforcement"]["visibility"], 1);
    let rows = d["workloads"]["rows"].as_array().unwrap();
    assert_eq!(rows[0]["name"], "host-1.example");
    assert_eq!(rows[0]["role"], "web");
    assert_eq!(d["inbound"].as_array().unwrap().len(), 3);
    assert_eq!(d["outbound"].as_array().unwrap().len(), 2);
    assert_eq!(d["inbound"][2]["decision"], "blocked");
    assert_eq!(d["policy"]["scope"], "draft");
    assert_eq!(d["policy"]["truncated"], false);
    assert_eq!(d["workloads"]["truncated"], false);
    assert_eq!(out["meta"]["truncated"], false);
    let policy: Vec<(&str, &str)> = d["policy"]["rows"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| (r["name"].as_str().unwrap(), r["reason"].as_str().unwrap()))
        .collect();
    assert_eq!(
        policy,
        vec![("Shop-Prod", "scope"), ("Global-Infra", "all_workloads")]
    );
    assert_eq!(d["timeline"]["exact"], false);
    assert_eq!(d["timeline"]["buckets"].as_array().unwrap().len(), 24);
    assert_eq!(out["meta"]["queries"], 2);
    assert_eq!(
        out["meta"]["api_calls"],
        4 + 3 + 3 + 1 + 1,
        "4 lists, POST + poll + download per side, workloads, rulesets"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn app_tables_and_csv_print_per_section() {
    let server = MockServer::start().await;
    mount_app(&server, 2).await;
    let table = cmd(&server)
        .args([
            "app",
            "shop/prod",
            "--window",
            "1h",
            "--direction",
            "in",
            "--table",
        ])
        .assert()
        .success();
    let text = String::from_utf8_lossy(&table.get_output().stdout).to_string();
    for s in [
        "workloads · 2 · enforced 1 · visibility 1 · VENs",
        "inbound · 3",
        "outbound · 0",
        "policy · draft · 2",
        "timeline · approx",
    ] {
        assert!(text.contains(s), "missing {s:?}:\n{text}");
    }
    let csv = cmd(&server)
        .args([
            "app",
            "shop/prod",
            "--window",
            "1h",
            "--direction",
            "out",
            "--csv",
            "--section",
            "outbound",
        ])
        .assert()
        .success();
    let text = csv_stdout(&csv);
    assert!(
        text.starts_with("peer,service,decision,flows,conns,bytes\r\n"),
        "{text}"
    );
    assert_eq!(text.lines().count(), 3, "header and two peers: {text}");
}

#[tokio::test(flavor = "multi_thread")]
async fn app_mistakes_exit_2_before_any_request() {
    let server = MockServer::start().await;
    mount_traffic_quiet(&server).await;
    let err = cmd(&server)
        .args(["app", "shop/prod", "--csv"])
        .assert()
        .code(2);
    assert!(
        String::from_utf8_lossy(&err.get_output().stderr)
            .contains("--csv prints one table: add --section")
    );
    cmd(&server)
        .args(["app", "shop/prod", "--window", "30d"])
        .assert()
        .code(2);
    let server = MockServer::start().await;
    mount_app(&server, 0).await;
    let err = cmd(&server).args(["app", "shp/prod"]).assert().code(2);
    assert!(String::from_utf8_lossy(&err.get_output().stderr).contains("closest: shop"));
}

#[tokio::test(flavor = "multi_thread")]
async fn app_timeline_runs_one_blocked_only_query_per_bucket() {
    let server = MockServer::start().await;
    // 1 focused query (in) for the peers, then 24 buckets of 1h/24, one side each.
    mount_app(&server, 1 + 24).await;
    let out = stdout_json(
        cmd(&server)
            .args([
                "app",
                "shop/prod",
                "--window",
                "1h",
                "--direction",
                "in",
                "--timeline",
            ])
            .assert()
            .success(),
    );
    let t = &out["data"]["timeline"];
    assert_eq!(t["exact"], true);
    assert_eq!(t["partial"], false);
    assert_eq!(t["truncated"], false);
    let buckets = t["buckets"].as_array().unwrap();
    assert_eq!(buckets.len(), 24);
    assert!(
        buckets
            .iter()
            .all(|b| b["count"] == 3 && b["truncated"] == false),
        "every bucket's answer holds the NAS flow's 3 blocked connections"
    );
    assert!(t.get("spike").is_none(), "a flat timeline has no spike");
    assert_eq!(out["meta"]["queries"], 25);
    assert_eq!(out["meta"]["truncated"], false);
    let posts: Vec<Value> = server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .filter(|r| r.method.as_str() == "POST")
        .map(|r| serde_json::from_slice(&r.body).unwrap())
        .collect();
    assert_eq!(posts.len(), 25);
    assert!(
        posts[0]["policy_decisions"] != json!(["blocked"]),
        "the peers' query keeps every decision"
    );
    assert!(
        posts[1..]
            .iter()
            .all(|b| b["policy_decisions"] == json!(["blocked"])),
        "bucket queries are blocked-only"
    );
    let starts: Vec<&str> = posts[1..]
        .iter()
        .map(|b| b["start_date"].as_str().unwrap())
        .collect();
    let mut sorted = starts.clone();
    sorted.sort();
    assert_eq!(starts, sorted, "oldest bucket first, one at a time");
    sorted.dedup();
    assert_eq!(sorted.len(), 24, "every bucket once");
}

#[tokio::test(flavor = "multi_thread")]
async fn app_timeline_marks_capped_buckets_as_lower_bounds() {
    let server = MockServer::start().await;
    mount_app(&server, 1 + 24).await;
    // A cap of 4 rows: every 4-row answer hit it. (One run: each costs 25
    // polled queries.)
    let out = stdout_json(
        cmd(&server)
            .args([
                "app",
                "shop/prod",
                "--window",
                "1h",
                "--direction",
                "out",
                "--max-results",
                "4",
                "--timeline",
            ])
            .assert()
            .success(),
    );
    let t = &out["data"]["timeline"];
    assert_eq!(
        t["truncated"], true,
        "the timeline says it is a lower bound"
    );
    assert!(
        t["buckets"]
            .as_array()
            .unwrap()
            .iter()
            .all(|b| b["truncated"] == true)
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn app_timeline_above_the_threshold_needs_yes_before_any_request() {
    let server = MockServer::start().await;
    mount_traffic_quiet(&server).await;
    for (args, queries) in [
        (vec!["app", "shop/prod", "--window", "1h", "--timeline"], 48),
        (vec!["app", "shop/prod", "--window", "1d", "--timeline"], 48),
        (
            vec![
                "app",
                "shop/prod",
                "--window",
                "7d",
                "--direction",
                "in",
                "--timeline",
            ],
            28,
        ),
    ] {
        let err = cmd(&server).args(&args).assert().code(2);
        let err = String::from_utf8_lossy(&err.get_output().stderr).to_string();
        assert!(
            err.contains(&format!("--timeline sends {queries} queries")) && err.contains("--yes"),
            "{args:?}: {err}"
        );
    }
    let err = cmd(&server)
        .args([
            "app",
            "shop/prod",
            "--direction",
            "in",
            "--timeline",
            "--section",
            "policy",
        ])
        .assert()
        .code(2);
    let err = String::from_utf8_lossy(&err.get_output().stderr).to_string();
    assert!(
        err.contains("--timeline only adds to the timeline section"),
        "{err}"
    );
}
