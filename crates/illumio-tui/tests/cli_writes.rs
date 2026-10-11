//! Write commands: guard, --dry-run/--yes, envelope, audit, exit codes.
#![allow(dead_code)]
use assert_cmd::Command;
use predicates::prelude::*;
use serde_json::{Value, json};
use wiremock::matchers::{body_json, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const TEST_SECRET: &str = "s3cr3t-cli-test-9f2a";

/// A command with credentials but writes neither enabled nor allowed.
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

/// Writes enabled by env, allowed by flag, audit log in `state`.
pub fn writer(server: &MockServer, state: &std::path::Path) -> Command {
    let mut c = cmd(server);
    c.env("ILLUMIO_WRITES", "1")
        .env("XDG_STATE_HOME", state)
        .arg("--allow-writes")
        .arg("--json");
    c
}

pub fn stdout_json(assert: assert_cmd::assert::Assert) -> Value {
    serde_json::from_slice(&assert.get_output().stdout).expect("stdout is JSON")
}

pub fn stderr_json(assert: &assert_cmd::assert::Assert) -> Value {
    serde_json::from_slice(&assert.get_output().stderr).expect("stderr is JSON")
}

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

pub fn audit_lines(state: &std::path::Path) -> Vec<Value> {
    let p = state.join("illumio-tui").join("writes.jsonl");
    match std::fs::read_to_string(p) {
        Ok(t) => t
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect(),
        Err(_) => Vec::new(),
    }
}

/// The four lists behind the name index.
pub async fn mount_index(server: &MockServer, labels: Value, ip_lists: Value, services: Value) {
    for (p, body) in [
        ("/api/v2/orgs/5/labels", labels),
        ("/api/v2/orgs/5/sec_policy/draft/label_groups", json!([])),
        ("/api/v2/orgs/5/sec_policy/draft/ip_lists", ip_lists),
        ("/api/v2/orgs/5/sec_policy/draft/services", services),
    ] {
        Mock::given(method("GET"))
            .and(path(p))
            .respond_with(ResponseTemplate::new(200).set_body_json(body))
            .mount(server)
            .await;
    }
}

/// How many requests went to `method path`.
pub async fn count(server: &MockServer, m: &str, p: &str) -> usize {
    server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .filter(|r| r.method.as_str() == m && r.url.path() == p)
        .count()
}

#[tokio::test(flavor = "multi_thread")]
async fn health_reports_the_writes_setting() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v2/health"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"status": "normal"})))
        .mount(&server)
        .await;
    let out = stdout_json(cmd(&server).args(["health", "--json"]).assert().success());
    assert_eq!(out["data"]["writes"], false);
    let out = stdout_json(
        cmd(&server)
            .env("ILLUMIO_WRITES", "true")
            .args(["health", "--json"])
            .assert()
            .success(),
    );
    assert_eq!(out["data"]["writes"], true);
    assert!(
        out["meta"].get("write").is_none(),
        "reads carry no write meta"
    );
}

#[test]
fn allow_writes_is_a_global_flag_with_help() {
    let out = Command::cargo_bin("illumio-tui")
        .unwrap()
        .env_clear()
        .args(["--help"])
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("--allow-writes"), "{text}");
}

/// Every write subcommand, with arguments that would otherwise be valid.
const ALL_WRITES: [&[&str]; 27] = [
    &["labels", "add", "a=b"],
    &["labels", "update", "/orgs/5/labels/1", "--value", "x"],
    &["labels", "delete", "/orgs/5/labels/1"],
    &["iplists", "add", "X", "--range", "10.0.0.1"],
    &[
        "iplists",
        "update",
        "/orgs/5/sec_policy/draft/ip_lists/1",
        "--add-fqdn",
        "a",
    ],
    &["iplists", "delete", "/orgs/5/sec_policy/draft/ip_lists/1"],
    &["services", "add", "X", "--port", "1/tcp"],
    &[
        "services",
        "update",
        "/orgs/5/sec_policy/draft/services/1",
        "--add-port",
        "1/tcp",
    ],
    &["services", "delete", "/orgs/5/sec_policy/draft/services/1"],
    &["workloads", "add", "X"],
    &[
        "workloads",
        "update",
        "/orgs/5/workloads/u1",
        "--description",
        "d",
    ],
    &["workloads", "delete", "/orgs/5/workloads/u1"],
    &[
        "workloads",
        "update",
        "/orgs/5/workloads/m1",
        "--add-label",
        "app=x",
        "--managed-labels",
    ],
    &["policy", "rulesets", "add", "X"],
    &[
        "policy",
        "rulesets",
        "update",
        "/orgs/5/sec_policy/draft/rule_sets/1",
        "--description",
        "d",
    ],
    &[
        "policy",
        "rulesets",
        "delete",
        "/orgs/5/sec_policy/draft/rule_sets/1",
    ],
    &[
        "policy",
        "rules",
        "add",
        "/orgs/5/sec_policy/draft/rule_sets/1",
        "--provider",
        "all",
        "--consumer",
        "all",
        "--service",
        "443/tcp",
    ],
    &[
        "policy",
        "rules",
        "update",
        "/orgs/5/sec_policy/draft/rule_sets/1/sec_rules/2",
        "--description",
        "d",
    ],
    &[
        "policy",
        "rules",
        "delete",
        "/orgs/5/sec_policy/draft/rule_sets/1/sec_rules/2",
    ],
    &[
        "policy",
        "deny-rules",
        "add",
        "/orgs/5/sec_policy/draft/rule_sets/1",
        "--provider",
        "all",
        "--consumer",
        "all",
        "--service",
        "22/tcp",
    ],
    &[
        "policy",
        "deny-rules",
        "update",
        "/orgs/5/sec_policy/draft/rule_sets/1/deny_rules/3",
        "--override",
    ],
    &[
        "policy",
        "deny-rules",
        "delete",
        "/orgs/5/sec_policy/draft/rule_sets/1/deny_rules/3",
    ],
    &[
        "policy",
        "rules",
        "add",
        "/orgs/5/sec_policy/draft/rule_sets/1",
        "--provider",
        "all",
        "--consumer",
        "all",
        "--service",
        "443/tcp",
        "--impact",
    ],
    &[
        "policy",
        "rules",
        "update",
        "/orgs/5/sec_policy/draft/rule_sets/1/sec_rules/2",
        "--description",
        "d",
        "--impact",
    ],
    &[
        "policy",
        "deny-rules",
        "add",
        "/orgs/5/sec_policy/draft/rule_sets/1",
        "--provider",
        "all",
        "--consumer",
        "all",
        "--service",
        "22/tcp",
        "--impact",
    ],
    &[
        "policy",
        "deny-rules",
        "update",
        "/orgs/5/sec_policy/draft/rule_sets/1/deny_rules/3",
        "--override",
        "--impact",
    ],
    &["policy", "provision", "--all"],
];

#[tokio::test(flavor = "multi_thread")]
async fn every_write_command_is_guarded_and_needs_yes() {
    let server = MockServer::start().await;
    let state = tempfile::tempdir().unwrap();
    for args in ALL_WRITES {
        // Writes enabled in the profile, but no --allow-writes: exit 6.
        let a = cmd(&server)
            .env("ILLUMIO_WRITES", "1")
            .env("XDG_STATE_HOME", state.path())
            .arg("--json")
            .args(args)
            .arg("--yes")
            .assert()
            .code(6);
        assert_eq!(
            stderr_json(&a)["error"]["kind"],
            "write_refused",
            "{args:?}"
        );
        assert_no_secret(&a);
        // Flag present, profile writes off (ILLUMIO_WRITES unset): exit 6 for
        // both --yes and --dry-run, before any request.
        for mode in ["--yes", "--dry-run"] {
            let a = cmd(&server)
                .env("XDG_STATE_HOME", state.path())
                .arg("--allow-writes")
                .arg("--json")
                .args(args)
                .arg(mode)
                .assert()
                .code(6);
            assert_eq!(
                stderr_json(&a)["error"]["kind"],
                "write_refused",
                "{args:?} {mode}"
            );
            assert_no_secret(&a);
        }
        // --allow-writes without --yes (or --dry-run): exit 2.
        let a = writer(&server, state.path()).args(args).assert().code(2);
        assert_eq!(stderr_json(&a)["error"]["kind"], "args", "{args:?}");
        assert!(
            server.received_requests().await.unwrap().is_empty(),
            "{args:?} sent a request"
        );
    }
    assert!(audit_lines(state.path()).is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn dry_run_and_yes_together_are_an_args_error() {
    let server = MockServer::start().await;
    let state = tempfile::tempdir().unwrap();
    writer(&server, state.path())
        .args(["labels", "add", "a=b", "--dry-run", "--yes"])
        .assert()
        .code(2);
    assert!(server.received_requests().await.unwrap().is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn no_state_home_and_no_home_refuses_writes() {
    let server = MockServer::start().await;
    // cmd() clears the environment: neither HOME nor XDG_STATE_HOME is set.
    let a = cmd(&server)
        .env("ILLUMIO_WRITES", "1")
        .args(["--allow-writes", "--json", "labels", "add", "a=b", "--yes"])
        .assert()
        .code(6);
    let err = stderr_json(&a);
    assert_eq!(err["error"]["kind"], "write_refused");
    assert!(
        err["error"]["message"]
            .as_str()
            .unwrap()
            .contains("no $XDG_STATE_HOME or $HOME to place the audit log"),
        "{err}"
    );
    assert!(server.received_requests().await.unwrap().is_empty());
}

mod labels {
    use super::*;

    #[tokio::test(flavor = "multi_thread")]
    async fn add_is_refused_without_allow_writes_or_profile_setting() {
        let server = MockServer::start().await;
        let state = tempfile::tempdir().unwrap();
        // Flag present, profile denies.
        let a = cmd(&server)
            .env("XDG_STATE_HOME", state.path())
            .args([
                "--allow-writes",
                "--json",
                "labels",
                "add",
                "app=web",
                "--yes",
            ])
            .assert()
            .code(6);
        let err = stderr_json(&a);
        assert_eq!(err["error"]["kind"], "write_refused");
        assert_eq!(err["error"]["exit_code"], 6);
        assert!(
            err["error"]["message"]
                .as_str()
                .unwrap()
                .contains("writes = true")
        );
        // Profile allows, flag missing.
        let a = cmd(&server)
            .env("ILLUMIO_WRITES", "1")
            .env("XDG_STATE_HOME", state.path())
            .args(["--json", "labels", "add", "app=web", "--yes"])
            .assert()
            .code(6);
        assert!(
            stderr_json(&a)["error"]["message"]
                .as_str()
                .unwrap()
                .contains("--allow-writes")
        );
        // Even a dry run needs the guard.
        cmd(&server)
            .args(["--json", "labels", "add", "app=web", "--dry-run"])
            .assert()
            .code(6);
        assert!(server.received_requests().await.unwrap().is_empty());
        assert!(audit_lines(state.path()).is_empty());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn add_without_yes_is_exit_2_and_sends_nothing() {
        let server = MockServer::start().await;
        let state = tempfile::tempdir().unwrap();
        let a = writer(&server, state.path())
            .args(["labels", "add", "app=web"])
            .assert()
            .code(2);
        let err = stderr_json(&a);
        assert_eq!(err["error"]["kind"], "args");
        assert!(err["error"]["message"].as_str().unwrap().contains("--yes"));
        assert!(server.received_requests().await.unwrap().is_empty());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn add_dry_run_prints_the_request_and_sends_nothing() {
        let server = MockServer::start().await;
        let state = tempfile::tempdir().unwrap();
        let a = writer(&server, state.path())
            .args(["labels", "add", "app=web", "--dry-run"])
            .assert()
            .success();
        assert_no_secret(&a);
        let out = stdout_json(a);
        assert_eq!(out["data"]["method"], "POST");
        assert_eq!(out["data"]["path"], "/orgs/5/labels");
        assert_eq!(out["data"]["body"], json!({"key": "app", "value": "web"}));
        assert_eq!(out["data"]["summary"], "create label app=web");
        assert_eq!(out["meta"]["cache"], "none");
        assert_eq!(out["meta"]["api_calls"], 0);
        assert!(out["meta"].get("write").is_none());
        assert!(server.received_requests().await.unwrap().is_empty());
        assert!(
            audit_lines(state.path()).is_empty(),
            "dry runs are not audited"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn label_spec_splits_on_the_first_equals() {
        let server = MockServer::start().await;
        let state = tempfile::tempdir().unwrap();
        let out = stdout_json(
            writer(&server, state.path())
                .args(["labels", "add", "app=a=b", "--dry-run"])
                .assert()
                .success(),
        );
        assert_eq!(out["data"]["body"], json!({"key": "app", "value": "a=b"}));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn add_sends_once_and_audits() {
        let server = MockServer::start().await;
        let state = tempfile::tempdir().unwrap();
        Mock::given(method("POST"))
            .and(path("/api/v2/orgs/5/labels"))
            .and(body_json(json!({"key": "app", "value": "web"})))
            .respond_with(
                ResponseTemplate::new(201).set_body_json(
                    json!({"href": "/orgs/5/labels/9", "key": "app", "value": "web"}),
                ),
            )
            .expect(1)
            .mount(&server)
            .await;
        let a = writer(&server, state.path())
            .args(["labels", "add", "app=web", "--yes"])
            .assert()
            .success();
        assert_no_secret(&a);
        let out = stdout_json(a);
        assert_eq!(out["data"]["href"], "/orgs/5/labels/9");
        assert_eq!(out["data"]["pending"], false);
        assert_eq!(out["data"]["status"], 201);
        assert_eq!(out["data"]["object"]["value"], "web");
        assert_eq!(out["data"]["dropped_fields"], json!([]));
        assert_eq!(out["meta"]["write"]["method"], "POST");
        assert_eq!(out["meta"]["write"]["path"], "/orgs/5/labels");
        assert_eq!(out["meta"]["write"]["status"], 201);
        assert!(
            out["meta"]["write"]["audit"]
                .as_str()
                .unwrap()
                .ends_with("writes.jsonl")
        );
        assert_eq!(out["meta"]["api_calls"], 1);
        let lines = audit_lines(state.path());
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0]["method"], "POST");
        assert_eq!(lines[0]["status"], 201);
        assert_eq!(lines[0]["href"], "/orgs/5/labels/9");
        assert_eq!(lines[0]["source"], "cli");
        assert_eq!(lines[0]["org_id"], 5);
        assert!(
            !serde_json::to_string(&lines[0])
                .unwrap()
                .contains(TEST_SECRET)
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn add_if_missing_skips_an_exact_match() {
        let server = MockServer::start().await;
        let state = tempfile::tempdir().unwrap();
        Mock::given(method("GET"))
            .and(path("/api/v2/orgs/5/labels"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(
                    json!([{"href": "/orgs/5/labels/1", "key": "app", "value": "web"}]),
                ),
            )
            .expect(1)
            .mount(&server)
            .await;
        let out = stdout_json(
            writer(&server, state.path())
                .args(["labels", "add", "app=web", "--if-missing", "--yes"])
                .assert()
                .success(),
        );
        assert_eq!(out["data"]["skipped"], true);
        assert_eq!(out["data"]["href"], "/orgs/5/labels/1");
        assert!(out["meta"].get("write").is_none());
        assert!(audit_lines(state.path()).is_empty());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn rejected_write_is_exit_4_with_the_pce_message() {
        let server = MockServer::start().await;
        let state = tempfile::tempdir().unwrap();
        Mock::given(method("POST"))
            .and(path("/api/v2/orgs/5/labels"))
            .respond_with(ResponseTemplate::new(406).set_body_json(
                json!([{"token": "label_key_value_must_be_unique", "message": "exists"}]),
            ))
            .expect(1)
            .mount(&server)
            .await;
        let a = writer(&server, state.path())
            .args(["labels", "add", "app=web", "--yes"])
            .assert()
            .code(4);
        let err = stderr_json(&a);
        assert_eq!(err["error"]["kind"], "rejected");
        assert!(
            err["error"]["message"]
                .as_str()
                .unwrap()
                .contains("label_key_value_must_be_unique")
        );
        assert_eq!(audit_lines(state.path())[0]["status"], 406);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn update_resolves_key_value_and_refetches_after_204() {
        let server = MockServer::start().await;
        let state = tempfile::tempdir().unwrap();
        mount_index(
            &server,
            json!([{"href": "/orgs/5/labels/1", "key": "env", "value": "test"}]),
            json!([]),
            json!([]),
        )
        .await;
        Mock::given(method("PUT"))
            .and(path("/api/v2/orgs/5/labels/1"))
            .and(body_json(json!({"value": "prod"})))
            .respond_with(ResponseTemplate::new(204))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/api/v2/orgs/5/labels/1"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(
                    json!({"href": "/orgs/5/labels/1", "key": "env", "value": "prod"}),
                ),
            )
            .expect(1)
            .mount(&server)
            .await;
        let out = stdout_json(
            writer(&server, state.path())
                .args(["labels", "update", "env=test", "--value", "prod", "--yes"])
                .assert()
                .success(),
        );
        assert_eq!(out["data"]["href"], "/orgs/5/labels/1");
        assert_eq!(out["data"]["status"], 204);
        assert_eq!(out["data"]["object"]["value"], "prod");
        // Unknown label: exit 2 listing known values, nothing written.
        let a = writer(&server, state.path())
            .args(["labels", "update", "env=nope", "--value", "x", "--yes"])
            .assert()
            .code(2);
        assert!(
            stderr_json(&a)["error"]["message"]
                .as_str()
                .unwrap()
                .contains("test")
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn delete_by_href_and_table_output() {
        let server = MockServer::start().await;
        let state = tempfile::tempdir().unwrap();
        Mock::given(method("DELETE"))
            .and(path("/api/v2/orgs/5/labels/9"))
            .respond_with(ResponseTemplate::new(204))
            .expect(1)
            .mount(&server)
            .await;
        let mut c = writer(&server, state.path());
        c.args(["labels", "delete", "/orgs/5/labels/9", "--yes"]);
        let out = stdout_json(c.assert().success());
        assert_eq!(out["data"]["href"], "/orgs/5/labels/9");
        assert!(out["data"]["object"].is_null());
        // Table form of a dry run (built from cmd(): writer() already passes --json).
        let mut t = cmd(&server);
        t.env("ILLUMIO_WRITES", "1")
            .env("XDG_STATE_HOME", state.path());
        t.args([
            "--allow-writes",
            "--table",
            "labels",
            "delete",
            "/orgs/5/labels/9",
            "--dry-run",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("method").and(predicate::str::contains("DELETE")))
        .stderr(predicate::str::contains("cache none"));
        assert_eq!(audit_lines(state.path()).len(), 1);
    }
}

mod iplists {
    use super::*;

    fn corp() -> Value {
        json!({
            "href": "/orgs/5/sec_policy/draft/ip_lists/3", "name": "Corp",
            "ip_ranges": [{"from_ip": "10.0.0.0/8"}], "fqdns": [{"fqdn": "*.corp.example"}]
        })
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn add_dry_run_body() {
        let server = MockServer::start().await;
        let state = tempfile::tempdir().unwrap();
        let out = stdout_json(
            writer(&server, state.path())
                .args([
                    "iplists",
                    "add",
                    "Corp",
                    "--range",
                    "10.0.0.0/8",
                    "--range",
                    "192.168.1.1-192.168.1.9",
                    "--exclude",
                    "10.0.5.0/24",
                    "--fqdn",
                    "*.corp.example",
                    "--description",
                    "corp nets",
                    "--dry-run",
                ])
                .assert()
                .success(),
        );
        assert_eq!(out["data"]["method"], "POST");
        assert_eq!(out["data"]["path"], "/orgs/5/sec_policy/draft/ip_lists");
        assert_eq!(
            out["data"]["body"],
            json!({
                "name": "Corp", "description": "corp nets",
                "ip_ranges": [
                    {"from_ip": "10.0.0.0/8"},
                    {"from_ip": "192.168.1.1", "to_ip": "192.168.1.9"},
                    {"from_ip": "10.0.5.0/24", "exclusion": true}
                ],
                "fqdns": [{"fqdn": "*.corp.example"}]
            })
        );
        // A bad range is exit 2 before anything else.
        let a = writer(&server, state.path())
            .args([
                "iplists",
                "add",
                "Corp",
                "--range",
                "10.0.0.0/99",
                "--dry-run",
            ])
            .assert()
            .code(2);
        assert!(
            stderr_json(&a)["error"]["message"]
                .as_str()
                .unwrap()
                .contains("10.0.0.0/99")
        );
        // Neither a range nor an FQDN is exit 2.
        writer(&server, state.path())
            .args(["iplists", "add", "Empty", "--dry-run"])
            .assert()
            .code(2);
        assert!(server.received_requests().await.unwrap().is_empty());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn add_sends_and_marks_pending() {
        let server = MockServer::start().await;
        let state = tempfile::tempdir().unwrap();
        Mock::given(method("POST"))
            .and(path("/api/v2/orgs/5/sec_policy/draft/ip_lists"))
            .respond_with(ResponseTemplate::new(201).set_body_json(corp()))
            .expect(1)
            .mount(&server)
            .await;
        let out = stdout_json(
            writer(&server, state.path())
                .args([
                    "iplists",
                    "add",
                    "Corp",
                    "--range",
                    "10.0.0.0/8",
                    "--fqdn",
                    "*.corp.example",
                    "--yes",
                ])
                .assert()
                .success(),
        );
        assert_eq!(out["data"]["pending"], true);
        assert_eq!(out["data"]["href"], "/orgs/5/sec_policy/draft/ip_lists/3");
        assert_eq!(audit_lines(state.path()).len(), 1);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn update_by_name_puts_only_the_changed_lists() {
        let server = MockServer::start().await;
        let state = tempfile::tempdir().unwrap();
        mount_index(&server, json!([]), json!([corp()]), json!([])).await;
        Mock::given(method("GET"))
            .and(path("/api/v2/orgs/5/sec_policy/draft/ip_lists/3"))
            .respond_with(ResponseTemplate::new(200).set_body_json(corp()))
            .mount(&server)
            .await;
        Mock::given(method("PUT"))
            .and(path("/api/v2/orgs/5/sec_policy/draft/ip_lists/3"))
            .and(body_json(json!({
                "ip_ranges": [{"from_ip": "10.0.0.0/8"}, {"from_ip": "10.2.0.0/16"}],
                "fqdns": []
            })))
            .respond_with(ResponseTemplate::new(204))
            .expect(1)
            .mount(&server)
            .await;
        let out = stdout_json(
            writer(&server, state.path())
                .args([
                    "iplists",
                    "update",
                    "Corp",
                    "--add-range",
                    "10.2.0.0/16",
                    "--remove-fqdn",
                    "*.corp.example",
                    "--yes",
                ])
                .assert()
                .success(),
        );
        assert_eq!(out["data"]["status"], 204);
        assert_eq!(out["data"]["pending"], true);
        assert_eq!(out["meta"]["write"]["method"], "PUT");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn removing_the_last_range_is_refused() {
        let server = MockServer::start().await;
        let state = tempfile::tempdir().unwrap();
        let mut only_range = corp();
        only_range["fqdns"] = json!([]);
        Mock::given(method("GET"))
            .and(path("/api/v2/orgs/5/sec_policy/draft/ip_lists/3"))
            .respond_with(ResponseTemplate::new(200).set_body_json(only_range))
            .mount(&server)
            .await;
        let a = writer(&server, state.path())
            .args([
                "iplists",
                "update",
                "/orgs/5/sec_policy/draft/ip_lists/3",
                "--remove-range",
                "10.0.0.0/8",
                "--yes",
            ])
            .assert()
            .code(2);
        assert_eq!(stderr_json(&a)["error"]["kind"], "invalid");
        let sent = server.received_requests().await.unwrap();
        assert!(
            sent.iter().all(|r| r.method.as_str() == "GET"),
            "nothing was written"
        );
        assert!(audit_lines(state.path()).is_empty());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn ambiguous_name_lists_candidates_and_delete_by_href_works() {
        let server = MockServer::start().await;
        let state = tempfile::tempdir().unwrap();
        let mut dup = corp();
        dup["href"] = json!("/orgs/5/sec_policy/draft/ip_lists/4");
        mount_index(&server, json!([]), json!([corp(), dup]), json!([])).await;
        let a = writer(&server, state.path())
            .args(["iplists", "delete", "Corp", "--yes"])
            .assert()
            .code(2);
        let msg = stderr_json(&a)["error"]["message"]
            .as_str()
            .unwrap()
            .to_string();
        assert!(
            msg.contains("ip_lists/3") && msg.contains("ip_lists/4"),
            "{msg}"
        );
        Mock::given(method("DELETE"))
            .and(path("/api/v2/orgs/5/sec_policy/draft/ip_lists/4"))
            .respond_with(ResponseTemplate::new(204))
            .expect(1)
            .mount(&server)
            .await;
        writer(&server, state.path())
            .args([
                "iplists",
                "delete",
                "/orgs/5/sec_policy/active/ip_lists/4",
                "--yes",
            ])
            .assert()
            .success();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn empty_update_is_exit_2_without_requests() {
        let server = MockServer::start().await;
        let state = tempfile::tempdir().unwrap();
        let a = writer(&server, state.path())
            .args(["iplists", "update", "Corp", "--yes"])
            .assert()
            .code(2);
        assert!(
            stderr_json(&a)["error"]["message"]
                .as_str()
                .unwrap()
                .contains("nothing to change")
        );
        assert!(server.received_requests().await.unwrap().is_empty());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn update_by_name_loads_only_the_ip_list_collection_once() {
        let server = MockServer::start().await;
        let state = tempfile::tempdir().unwrap();
        mount_index(&server, json!([]), json!([corp()]), json!([])).await;
        Mock::given(method("GET"))
            .and(path("/api/v2/orgs/5/sec_policy/draft/ip_lists/3"))
            .respond_with(ResponseTemplate::new(200).set_body_json(corp()))
            .mount(&server)
            .await;
        Mock::given(method("PUT"))
            .and(path("/api/v2/orgs/5/sec_policy/draft/ip_lists/3"))
            .respond_with(ResponseTemplate::new(204))
            .expect(1)
            .mount(&server)
            .await;
        writer(&server, state.path())
            .args([
                "--no-cache",
                "iplists",
                "update",
                "Corp",
                "--add-fqdn",
                "db.corp.example",
                "--yes",
            ])
            .assert()
            .success();
        let reqs = server.received_requests().await.unwrap();
        let seen: Vec<String> = reqs
            .iter()
            .map(|r| format!("{} {}", r.method, r.url.path()))
            .collect();
        assert_eq!(
            seen,
            vec![
                "GET /api/v2/orgs/5/sec_policy/draft/ip_lists",
                "GET /api/v2/orgs/5/sec_policy/draft/ip_lists/3",
                "PUT /api/v2/orgs/5/sec_policy/draft/ip_lists/3",
                "GET /api/v2/orgs/5/sec_policy/draft/ip_lists/3",
            ],
            "one collection GET, the href fetch, the PUT and the re-fetch"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_pending_delete_does_not_count_as_existing() {
        let server = MockServer::start().await;
        let state = tempfile::tempdir().unwrap();
        let mut gone = corp();
        gone["update_type"] = json!("delete");
        mount_index(&server, json!([]), json!([gone]), json!([])).await;
        let out = stdout_json(
            writer(&server, state.path())
                .args([
                    "iplists",
                    "add",
                    "Corp",
                    "--range",
                    "10.0.0.0/8",
                    "--if-missing",
                    "--dry-run",
                ])
                .assert()
                .success(),
        );
        assert_eq!(out["data"]["method"], "POST", "{out}");
        let a = writer(&server, state.path())
            .args(["iplists", "delete", "Corp", "--yes"])
            .assert()
            .code(2);
        let msg = stderr_json(&a)["error"]["message"]
            .as_str()
            .unwrap()
            .to_string();
        assert!(msg.contains("no IP list named \"Corp\""), "{msg}");
        assert!(
            server
                .received_requests()
                .await
                .unwrap()
                .iter()
                .all(|r| r.method.as_str() == "GET"),
            "nothing written"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn an_unknown_name_lists_similar_names() {
        let server = MockServer::start().await;
        let state = tempfile::tempdir().unwrap();
        mount_index(&server, json!([]), json!([corp()]), json!([])).await;
        let a = writer(&server, state.path())
            .args(["iplists", "delete", "corp", "--yes"])
            .assert()
            .code(2);
        assert_eq!(
            stderr_json(&a)["error"]["message"],
            "no IP list named \"corp\"; similar: Corp"
        );
    }
}

mod services {
    use super::*;

    fn pg() -> Value {
        json!({"href": "/orgs/5/sec_policy/draft/services/7", "name": "PostgreSQL",
               "service_ports": [{"port": 5432, "proto": 6}]})
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn add_dry_run_body_has_only_service_ports() {
        let server = MockServer::start().await;
        let state = tempfile::tempdir().unwrap();
        let out = stdout_json(
            writer(&server, state.path())
                .args([
                    "services",
                    "add",
                    "PostgreSQL",
                    "--port",
                    "5432/tcp",
                    "--port",
                    "8000-8100/udp",
                    "--port",
                    "icmp",
                    "--description",
                    "db",
                    "--dry-run",
                ])
                .assert()
                .success(),
        );
        assert_eq!(
            out["data"]["body"],
            json!({"name": "PostgreSQL", "description": "db",
                   "service_ports": [{"port": 5432, "proto": 6}, {"port": 8000, "to_port": 8100, "proto": 17}, {"proto": 1}]})
        );
        assert!(out["data"]["body"].get("windows_services").is_none());
        let a = writer(&server, state.path())
            .args(["services", "add", "X", "--port", "443", "--dry-run"])
            .assert()
            .code(2);
        assert!(
            stderr_json(&a)["error"]["message"]
                .as_str()
                .unwrap()
                .contains("443")
        );
        // --port is required.
        writer(&server, state.path())
            .args(["services", "add", "X", "--dry-run"])
            .assert()
            .code(2);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn add_reports_dropped_fields() {
        let server = MockServer::start().await;
        let state = tempfile::tempdir().unwrap();
        let mut dropped = pg();
        dropped["service_ports"] = json!([]);
        Mock::given(method("POST"))
            .and(path("/api/v2/orgs/5/sec_policy/draft/services"))
            .respond_with(ResponseTemplate::new(201).set_body_json(dropped))
            .expect(1)
            .mount(&server)
            .await;
        let out = stdout_json(
            writer(&server, state.path())
                .args([
                    "services",
                    "add",
                    "PostgreSQL",
                    "--port",
                    "5432/tcp",
                    "--yes",
                ])
                .assert()
                .success(),
        );
        assert_eq!(out["data"]["dropped_fields"], json!(["service_ports"]));
        assert_eq!(out["data"]["pending"], true);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn update_by_name_and_delete() {
        let server = MockServer::start().await;
        let state = tempfile::tempdir().unwrap();
        mount_index(&server, json!([]), json!([]), json!([pg()])).await;
        Mock::given(method("GET"))
            .and(path("/api/v2/orgs/5/sec_policy/draft/services/7"))
            .respond_with(ResponseTemplate::new(200).set_body_json(pg()))
            .mount(&server)
            .await;
        Mock::given(method("PUT"))
            .and(path("/api/v2/orgs/5/sec_policy/draft/services/7"))
            .and(body_json(json!({
                 "service_ports": [{"port": 5432, "proto": 6}, {"port": 5433, "proto": 6}]})))
            .respond_with(ResponseTemplate::new(204))
            .expect(1)
            .mount(&server)
            .await;
        let out = stdout_json(
            writer(&server, state.path())
                .args([
                    "services",
                    "update",
                    "PostgreSQL",
                    "--add-port",
                    "5433/tcp",
                    "--yes",
                ])
                .assert()
                .success(),
        );
        assert_eq!(out["data"]["status"], 204);
        let a = writer(&server, state.path())
            .args([
                "services",
                "update",
                "PostgreSQL",
                "--remove-port",
                "5432/tcp",
                "--yes",
            ])
            .assert()
            .code(2);
        assert!(
            stderr_json(&a)["error"]["message"]
                .as_str()
                .unwrap()
                .contains("at least one")
        );
        Mock::given(method("DELETE"))
            .and(path("/api/v2/orgs/5/sec_policy/draft/services/7"))
            .respond_with(ResponseTemplate::new(204))
            .expect(1)
            .mount(&server)
            .await;
        writer(&server, state.path())
            .args(["services", "delete", "PostgreSQL", "--yes"])
            .assert()
            .success();
        assert_eq!(audit_lines(state.path()).len(), 2);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn empty_update_is_exit_2_without_requests() {
        let server = MockServer::start().await;
        let state = tempfile::tempdir().unwrap();
        let a = writer(&server, state.path())
            .args(["services", "update", "PostgreSQL", "--yes"])
            .assert()
            .code(2);
        assert!(
            stderr_json(&a)["error"]["message"]
                .as_str()
                .unwrap()
                .contains("nothing to change")
        );
        assert!(server.received_requests().await.unwrap().is_empty());
    }
}

mod workloads {
    use super::*;

    fn printer() -> Value {
        json!({
            "href": "/orgs/5/workloads/u1", "name": "printer-1", "hostname": "printer-1.lab",
            "managed": false, "interfaces": [{"name": "eth0", "address": "10.0.0.50", "cidr_block": 24}],
            "labels": [{"href": "/orgs/5/labels/1", "key": "app", "value": "print"}]
        })
    }

    fn labels() -> Value {
        json!([
            {"href": "/orgs/5/labels/1", "key": "app", "value": "print"},
            {"href": "/orgs/5/labels/2", "key": "env", "value": "lab"}
        ])
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn add_dry_run_resolves_labels() {
        let server = MockServer::start().await;
        let state = tempfile::tempdir().unwrap();
        mount_index(&server, labels(), json!([]), json!([])).await;
        let out = stdout_json(
            writer(&server, state.path())
                .args([
                    "workloads",
                    "add",
                    "printer-1",
                    "--hostname",
                    "printer-1.lab",
                    "--ip",
                    "10.0.0.50/24",
                    "--ip",
                    "fe80::1",
                    "--label",
                    "app=print",
                    "--label",
                    "env=lab",
                    "--description",
                    "lobby",
                    "--dry-run",
                ])
                .assert()
                .success(),
        );
        assert_eq!(out["data"]["method"], "POST");
        assert_eq!(out["data"]["path"], "/orgs/5/workloads");
        assert_eq!(
            out["data"]["body"],
            json!({
                "name": "printer-1", "hostname": "printer-1.lab", "description": "lobby",
                "interfaces": [{"name": "eth0", "address": "10.0.0.50", "cidr_block": 24}, {"name": "eth1", "address": "fe80::1"}],
                "labels": [{"href": "/orgs/5/labels/1"}, {"href": "/orgs/5/labels/2"}]
            })
        );
        let a = writer(&server, state.path())
            .args(["workloads", "add", "p", "--label", "app=nope", "--dry-run"])
            .assert()
            .code(2);
        assert!(
            stderr_json(&a)["error"]["message"]
                .as_str()
                .unwrap()
                .contains("print")
        );
        assert!(
            server
                .received_requests()
                .await
                .unwrap()
                .iter()
                .all(|r| r.method.as_str() == "GET")
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn add_sends_and_is_not_pending() {
        let server = MockServer::start().await;
        let state = tempfile::tempdir().unwrap();
        Mock::given(method("POST"))
            .and(path("/api/v2/orgs/5/workloads"))
            .and(body_json(json!({"name": "printer-1", "hostname": "printer-1", "interfaces": [], "labels": []})))
            .respond_with(ResponseTemplate::new(201).set_body_json(printer()))
            .expect(1)
            .mount(&server)
            .await;
        let out = stdout_json(
            writer(&server, state.path())
                .args(["workloads", "add", "printer-1", "--yes"])
                .assert()
                .success(),
        );
        assert_eq!(out["data"]["pending"], false);
        assert_eq!(out["data"]["href"], "/orgs/5/workloads/u1");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn update_by_hostname_sends_only_changed_keys() {
        let server = MockServer::start().await;
        let state = tempfile::tempdir().unwrap();
        mount_index(&server, labels(), json!([]), json!([])).await;
        Mock::given(method("GET"))
            .and(path("/api/v2/orgs/5/workloads"))
            .and(wiremock::matchers::query_param("hostname", "printer-1.lab"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([printer()])))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/api/v2/orgs/5/workloads/u1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(printer()))
            .mount(&server)
            .await;
        Mock::given(method("PUT"))
            .and(path("/api/v2/orgs/5/workloads/u1"))
            .and(body_json(json!({
                "description": "moved",
                "labels": [{"href": "/orgs/5/labels/1"}, {"href": "/orgs/5/labels/2"}]
            })))
            .respond_with(ResponseTemplate::new(204))
            .expect(1)
            .mount(&server)
            .await;
        let out = stdout_json(
            writer(&server, state.path())
                .args([
                    "workloads",
                    "update",
                    "printer-1.lab",
                    "--description",
                    "moved",
                    "--add-label",
                    "env=lab",
                    "--yes",
                ])
                .assert()
                .success(),
        );
        assert_eq!(out["data"]["status"], 204);
        assert_eq!(out["data"]["object"]["hostname"], "printer-1.lab");
        // --set-labels conflicts with --add-label (clap, exit 2).
        writer(&server, state.path())
            .args([
                "workloads",
                "update",
                "printer-1.lab",
                "--set-labels",
                "env=lab",
                "--add-label",
                "env=lab",
                "--yes",
            ])
            .assert()
            .code(2);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn managed_workloads_are_refused_locally() {
        let server = MockServer::start().await;
        let state = tempfile::tempdir().unwrap();
        mount_index(&server, labels(), json!([]), json!([])).await;
        Mock::given(method("GET"))
            .and(path("/api/v2/orgs/5/workloads/m1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(
                json!({"href": "/orgs/5/workloads/m1", "hostname": "web-1", "managed": true}),
            ))
            .mount(&server)
            .await;
        let a = writer(&server, state.path())
            .args(["workloads", "delete", "/orgs/5/workloads/m1", "--yes"])
            .assert()
            .code(2);
        assert!(
            stderr_json(&a)["error"]["message"]
                .as_str()
                .unwrap()
                .contains("VEN-managed")
        );
        let a = writer(&server, state.path())
            .args([
                "workloads",
                "update",
                "/orgs/5/workloads/m1",
                "--description",
                "x",
                "--yes",
            ])
            .assert()
            .code(2);
        assert_eq!(stderr_json(&a)["error"]["kind"], "invalid");
        assert!(
            server
                .received_requests()
                .await
                .unwrap()
                .iter()
                .all(|r| r.method.as_str() == "GET")
        );
        assert!(audit_lines(state.path()).is_empty());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn removing_the_last_interface_without_a_hostname_is_refused() {
        let server = MockServer::start().await;
        let state = tempfile::tempdir().unwrap();
        let mut no_host = printer();
        no_host["hostname"] = Value::Null;
        Mock::given(method("GET"))
            .and(path("/api/v2/orgs/5/workloads/u1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(no_host))
            .mount(&server)
            .await;
        mount_index(&server, labels(), json!([]), json!([])).await;
        let a = writer(&server, state.path())
            .args([
                "workloads",
                "update",
                "/orgs/5/workloads/u1",
                "--remove-ip",
                "10.0.0.50",
                "--yes",
            ])
            .assert()
            .code(2);
        assert_eq!(stderr_json(&a)["error"]["kind"], "invalid");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn delete_by_href() {
        let server = MockServer::start().await;
        let state = tempfile::tempdir().unwrap();
        Mock::given(method("GET"))
            .and(path("/api/v2/orgs/5/workloads/u1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(printer()))
            .mount(&server)
            .await;
        Mock::given(method("DELETE"))
            .and(path("/api/v2/orgs/5/workloads/u1"))
            .respond_with(ResponseTemplate::new(204))
            .expect(1)
            .mount(&server)
            .await;
        let out = stdout_json(
            writer(&server, state.path())
                .args(["workloads", "delete", "/orgs/5/workloads/u1", "--yes"])
                .assert()
                .success(),
        );
        assert_eq!(out["data"]["href"], "/orgs/5/workloads/u1");
        assert_eq!(audit_lines(state.path())[0]["method"], "DELETE");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn listing_still_works_without_a_subcommand() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v2/orgs/5/workloads"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([printer()])))
            .mount(&server)
            .await;
        let out = stdout_json(
            cmd(&server)
                .args(["workloads", "--managed", "false", "--json"])
                .assert()
                .success(),
        );
        assert_eq!(out["data"][0]["hostname"], "printer-1.lab");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn update_with_nothing_to_change_sends_no_request() {
        let server = MockServer::start().await;
        let state = tempfile::tempdir().unwrap();
        let a = writer(&server, state.path())
            .args(["workloads", "update", "printer-1.lab", "--yes"])
            .assert()
            .code(2);
        assert!(
            stderr_json(&a)["error"]["message"]
                .as_str()
                .unwrap()
                .contains("nothing to change")
        );
        assert!(server.received_requests().await.unwrap().is_empty());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn set_labels_loads_the_labels_list_once_and_nothing_else() {
        let server = MockServer::start().await;
        let state = tempfile::tempdir().unwrap();
        mount_index(&server, labels(), json!([]), json!([])).await;
        Mock::given(method("GET"))
            .and(path("/api/v2/orgs/5/workloads/u1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(printer()))
            .mount(&server)
            .await;
        Mock::given(method("PUT"))
            .and(path("/api/v2/orgs/5/workloads/u1"))
            .and(body_json(json!({
                "labels": [{"href": "/orgs/5/labels/1"}, {"href": "/orgs/5/labels/2"}]
            })))
            .respond_with(ResponseTemplate::new(204))
            .expect(1)
            .mount(&server)
            .await;
        writer(&server, state.path())
            .args([
                "--no-cache",
                "workloads",
                "update",
                "/orgs/5/workloads/u1",
                "--set-labels",
                "app=print,env=lab",
                "--yes",
            ])
            .assert()
            .success();
        assert_eq!(count(&server, "GET", "/api/v2/orgs/5/labels").await, 1);
        assert_eq!(
            count(&server, "GET", "/api/v2/orgs/5/workloads/u1").await,
            2
        );
        assert_eq!(
            server.received_requests().await.unwrap().len(),
            4,
            "labels GET, workload GET, PUT, re-fetch"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn description_only_update_does_not_load_the_index() {
        let server = MockServer::start().await;
        let state = tempfile::tempdir().unwrap();
        Mock::given(method("GET"))
            .and(path("/api/v2/orgs/5/workloads/u1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(printer()))
            .mount(&server)
            .await;
        Mock::given(method("PUT"))
            .and(path("/api/v2/orgs/5/workloads/u1"))
            .respond_with(ResponseTemplate::new(204))
            .expect(1)
            .mount(&server)
            .await;
        writer(&server, state.path())
            .args([
                "workloads",
                "update",
                "/orgs/5/workloads/u1",
                "--description",
                "x",
                "--yes",
            ])
            .assert()
            .success();
        assert!(
            server
                .received_requests()
                .await
                .unwrap()
                .iter()
                .all(|r| r.url.path() == "/api/v2/orgs/5/workloads/u1")
        );
    }
}

mod policy {
    use super::*;
    use wiremock::matchers::query_param;

    pub const RS_100: &str = "/orgs/5/sec_policy/draft/rule_sets/100";
    pub const RULE_2: &str = "/orgs/5/sec_policy/draft/rule_sets/100/sec_rules/2";
    pub const DENY_8: &str = "/orgs/5/sec_policy/draft/rule_sets/100/deny_rules/8";

    pub fn fixture_labels() -> Value {
        serde_json::from_str(include_str!("../../../fixtures/labels.json")).unwrap()
    }
    pub fn fixture_label_groups() -> Value {
        serde_json::from_str(include_str!("../../../fixtures/label_groups.json")).unwrap()
    }
    pub fn fixture_ip_lists() -> Value {
        serde_json::from_str(include_str!("../../../fixtures/ip_lists.json")).unwrap()
    }
    pub fn fixture_services() -> Value {
        serde_json::from_str(include_str!("../../../fixtures/services.json")).unwrap()
    }
    pub fn fixture_rule_sets() -> Value {
        serde_json::from_str(include_str!("../../../fixtures/rule_sets.json")).unwrap()
    }
    pub fn fixture_workloads() -> Value {
        serde_json::from_str(include_str!("../../../fixtures/workloads.json")).unwrap()
    }

    pub async fn mount_get(server: &MockServer, p: &str, body: Value) {
        Mock::given(method("GET"))
            .and(path(format!("/api/v2{p}")))
            .respond_with(ResponseTemplate::new(200).set_body_json(body))
            .mount(server)
            .await;
    }

    /// Labels, label groups, IP lists, services and the draft rulesets.
    pub async fn mount_lists(server: &MockServer) {
        mount_get(server, "/orgs/5/labels", fixture_labels()).await;
        mount_get(
            server,
            "/orgs/5/sec_policy/draft/label_groups",
            fixture_label_groups(),
        )
        .await;
        mount_get(
            server,
            "/orgs/5/sec_policy/draft/ip_lists",
            fixture_ip_lists(),
        )
        .await;
        mount_get(
            server,
            "/orgs/5/sec_policy/draft/services",
            fixture_services(),
        )
        .await;
        mount_get(
            server,
            "/orgs/5/sec_policy/draft/rule_sets",
            fixture_rule_sets(),
        )
        .await;
    }

    async fn gets(server: &MockServer, p: &str) -> usize {
        count(server, "GET", &format!("/api/v2{p}")).await
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn rulesets_add_resolves_scopes_with_one_get_per_kind() {
        let server = MockServer::start().await;
        let state = tempfile::tempdir().unwrap();
        mount_lists(&server).await;
        let out = stdout_json(
            writer(&server, state.path())
                .args([
                    "policy",
                    "rulesets",
                    "add",
                    "Web-Tier",
                    "--scope",
                    "app=shop,env=prod",
                    "--scope",
                    "group:All prod envs",
                    "--dry-run",
                ])
                .assert()
                .success(),
        );
        assert_eq!(out["data"]["method"], "POST");
        assert_eq!(out["data"]["path"], "/orgs/5/sec_policy/draft/rule_sets");
        assert_eq!(
            out["data"]["body"],
            json!({
                "name": "Web-Tier", "enabled": true,
                "scopes": [
                    [{"label": {"href": "/orgs/5/labels/3"}}, {"label": {"href": "/orgs/5/labels/4"}}],
                    [{"label_group": {"href": "/orgs/5/sec_policy/draft/label_groups/10"}}]
                ]
            })
        );
        assert_eq!(gets(&server, "/orgs/5/labels").await, 1);
        assert_eq!(
            gets(&server, "/orgs/5/sec_policy/draft/label_groups").await,
            1
        );
        assert_eq!(server.received_requests().await.unwrap().len(), 2);
        assert_eq!(out["meta"]["api_calls"], 2);
        assert!(
            audit_lines(state.path()).is_empty(),
            "a dry run is not audited"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn rulesets_add_unscoped_disabled_and_if_missing() {
        let server = MockServer::start().await;
        let state = tempfile::tempdir().unwrap();
        mount_lists(&server).await;
        let out = stdout_json(
            writer(&server, state.path())
                .args([
                    "policy",
                    "rulesets",
                    "add",
                    "Global",
                    "--disabled",
                    "--dry-run",
                ])
                .assert()
                .success(),
        );
        assert_eq!(
            out["data"]["body"],
            json!({"name": "Global", "enabled": false, "scopes": [[]]})
        );
        assert!(server.received_requests().await.unwrap().is_empty());
        let out = stdout_json(
            writer(&server, state.path())
                .args([
                    "policy",
                    "rulesets",
                    "add",
                    "shop-prod",
                    "--if-missing",
                    "--dry-run",
                ])
                .assert()
                .success(),
        );
        assert_eq!(out["data"]["skipped"], true);
        assert_eq!(out["data"]["href"], RS_100);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn rulesets_add_scope_with_an_ip_list_is_refused_before_any_request() {
        let server = MockServer::start().await;
        let state = tempfile::tempdir().unwrap();
        let a = writer(&server, state.path())
            .args([
                "policy",
                "rulesets",
                "add",
                "X",
                "--scope",
                "iplist:Corp",
                "--dry-run",
            ])
            .assert()
            .code(2);
        assert_eq!(
            stderr_json(&a)["error"]["message"],
            "scopes take labels and label groups only"
        );
        writer(&server, state.path())
            .args(["policy", "rulesets", "add", "X", "--scope", "--dry-run"])
            .assert()
            .code(2);
        assert!(server.received_requests().await.unwrap().is_empty());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn an_empty_ruleset_name_is_exit_2_before_any_request() {
        let server = MockServer::start().await;
        let state = tempfile::tempdir().unwrap();
        for args in [
            &["policy", "rulesets", "add", "", "--if-missing", "--dry-run"][..],
            &["policy", "rulesets", "add", "  ", "--dry-run"][..],
            &[
                "policy",
                "rulesets",
                "update",
                "Shop-Prod",
                "--name",
                "",
                "--dry-run",
            ][..],
        ] {
            let a = writer(&server, state.path()).args(args).assert().code(2);
            assert_eq!(stderr_json(&a)["error"]["kind"], "invalid", "{args:?}");
        }
        assert!(server.received_requests().await.unwrap().is_empty());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn rulesets_update_by_name_and_by_href() {
        let server = MockServer::start().await;
        let state = tempfile::tempdir().unwrap();
        mount_lists(&server).await;
        mount_get(&server, RS_100, fixture_rule_sets()[0].clone()).await;
        let out = stdout_json(
            writer(&server, state.path())
                .args([
                    "policy",
                    "rulesets",
                    "update",
                    "shop-prod",
                    "--clear-scopes",
                    "--disabled",
                    "--dry-run",
                ])
                .assert()
                .success(),
        );
        assert_eq!(out["data"]["method"], "PUT");
        assert_eq!(out["data"]["path"], RS_100);
        assert_eq!(
            out["data"]["body"],
            json!({"enabled": false, "scopes": [[]]})
        );
        let out = stdout_json(
            writer(&server, state.path())
                .args([
                    "policy",
                    "rulesets",
                    "update",
                    RS_100,
                    "--description",
                    "Ringfence v2",
                    "--enabled",
                    "--dry-run",
                ])
                .assert()
                .success(),
        );
        assert_eq!(out["data"]["body"], json!({"description": "Ringfence v2"}));
        assert_eq!(
            gets(&server, "/orgs/5/sec_policy/draft/rule_sets").await,
            1,
            "only the name needed the list"
        );
        assert_eq!(gets(&server, RS_100).await, 2);
        writer(&server, state.path())
            .args([
                "policy",
                "rulesets",
                "update",
                RS_100,
                "--scope",
                "app=shop",
                "--clear-scopes",
                "--dry-run",
            ])
            .assert()
            .code(2);
        let a = writer(&server, state.path())
            .args(["policy", "rulesets", "update", RS_100, "--dry-run"])
            .assert()
            .code(2);
        assert!(
            stderr_json(&a)["error"]["message"]
                .as_str()
                .unwrap()
                .starts_with("nothing to change")
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn rulesets_unknown_and_ambiguous_names() {
        let server = MockServer::start().await;
        let state = tempfile::tempdir().unwrap();
        mount_lists(&server).await;
        let a = writer(&server, state.path())
            .args(["policy", "rulesets", "delete", "shop", "--dry-run"])
            .assert()
            .code(2);
        assert_eq!(
            stderr_json(&a)["error"]["message"],
            r#"no ruleset named "shop"; similar: Shop-Prod"#
        );
        let dup = MockServer::start().await;
        mount_get(
            &dup,
            "/orgs/5/sec_policy/draft/rule_sets",
            json!([
                {"href": "/orgs/5/sec_policy/draft/rule_sets/200", "name": "Dup"},
                {"href": "/orgs/5/sec_policy/draft/rule_sets/201", "name": "DUP"}
            ]),
        )
        .await;
        let a = writer(&dup, state.path())
            .args(["policy", "rulesets", "delete", "dup", "--dry-run"])
            .assert()
            .code(2);
        assert_eq!(
            stderr_json(&a)["error"]["message"],
            r#""dup" names several rulesets: /orgs/5/sec_policy/draft/rule_sets/200, /orgs/5/sec_policy/draft/rule_sets/201; use the href"#
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn rulesets_delete_by_name_with_table_output() {
        let server = MockServer::start().await;
        let state = tempfile::tempdir().unwrap();
        mount_lists(&server).await;
        let mut t = cmd(&server);
        t.env("ILLUMIO_WRITES", "1")
            .env("XDG_STATE_HOME", state.path());
        t.args([
            "--allow-writes",
            "--table",
            "policy",
            "rulesets",
            "delete",
            "Global-Infra",
            "--dry-run",
        ])
        .assert()
        .success()
        .stdout(
            predicate::str::contains("DELETE").and(predicate::str::contains(
                "/orgs/5/sec_policy/draft/rule_sets/101",
            )),
        )
        .stderr(predicate::str::contains("cache none"));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn rules_add_resolves_the_mixed_grammar_with_one_get_per_kind() {
        let server = MockServer::start().await;
        let state = tempfile::tempdir().unwrap();
        mount_lists(&server).await;
        let out = stdout_json(
            writer(&server, state.path())
                .args([
                    "policy",
                    "rules",
                    "add",
                    "shop-prod",
                    "--provider",
                    "role=web",
                    "--provider",
                    "group:All prod envs",
                    "--consumer",
                    "iplist:Corp",
                    "--consumer",
                    "!iplist:Any (0.0.0.0/0 and ::/0)",
                    "--consumer",
                    "all",
                    "--service",
                    "PostgreSQL",
                    "--service",
                    "443/tcp",
                    "--service",
                    "8000-8099/tcp",
                    "--service",
                    "/orgs/5/sec_policy/active/services/32",
                    "--description",
                    "web tier",
                    "--unscoped-consumers",
                    "--resolve-providers-as",
                    "workloads",
                    "--dry-run",
                ])
                .assert()
                .success(),
        );
        assert_eq!(out["data"]["method"], "POST");
        assert_eq!(out["data"]["path"], format!("{RS_100}/sec_rules"));
        assert_eq!(
            out["data"]["body"],
            json!({
                "enabled": true, "description": "web tier",
                "providers": [
                    {"label": {"href": "/orgs/5/labels/1"}},
                    {"label_group": {"href": "/orgs/5/sec_policy/draft/label_groups/10"}}
                ],
                "consumers": [
                    {"ip_list": {"href": "/orgs/5/sec_policy/draft/ip_lists/21"}},
                    {"ip_list": {"href": "/orgs/5/sec_policy/draft/ip_lists/20"}, "exclusion": true},
                    {"actors": "ams"}
                ],
                "ingress_services": [
                    {"href": "/orgs/5/sec_policy/draft/services/31"},
                    {"port": 443, "proto": 6},
                    {"port": 8000, "to_port": 8099, "proto": 6},
                    {"href": "/orgs/5/sec_policy/draft/services/32"}
                ],
                "unscoped_consumers": true,
                "resolve_labels_as": {"providers": ["workloads"], "consumers": ["workloads"]}
            })
        );
        for p in [
            "/orgs/5/sec_policy/draft/rule_sets",
            "/orgs/5/labels",
            "/orgs/5/sec_policy/draft/label_groups",
            "/orgs/5/sec_policy/draft/ip_lists",
            "/orgs/5/sec_policy/draft/services",
        ] {
            assert_eq!(gets(&server, p).await, 1, "{p}");
        }
        assert_eq!(server.received_requests().await.unwrap().len(), 5);
    }

    fn add_icmp(
        server: &MockServer,
        state: &std::path::Path,
        service: &str,
    ) -> assert_cmd::assert::Assert {
        writer(server, state)
            .args([
                "policy",
                "rules",
                "add",
                RS_100,
                "--provider",
                "all",
                "--consumer",
                "all",
                "--service",
                service,
                "--dry-run",
            ])
            .assert()
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn rules_add_icmp_resolves_to_the_icmp_service_object() {
        let server = MockServer::start().await;
        let state = tempfile::tempdir().unwrap();
        let mut services = fixture_services();
        services.as_array_mut().unwrap().push(json!({
            "href": "/orgs/5/sec_policy/draft/services/40", "name": "S-ICMP",
            "service_ports": [{"proto": 1}, {"proto": 58}],
            "external_data_set": "com.illumio.ilo_defaults", "external_data_reference": "ICMP"
        }));
        mount_get(&server, "/orgs/5/sec_policy/draft/services", services).await;
        for token in ["icmp", "icmpv6"] {
            let out = stdout_json(add_icmp(&server, state.path(), token).success());
            assert_eq!(
                out["data"]["body"]["ingress_services"],
                json!([{"href": "/orgs/5/sec_policy/draft/services/40"}]),
                "{token}"
            );
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn rules_add_icmp_without_an_icmp_service_is_invalid() {
        let server = MockServer::start().await;
        let state = tempfile::tempdir().unwrap();
        mount_get(
            &server,
            "/orgs/5/sec_policy/draft/services",
            fixture_services(),
        )
        .await;
        let a = add_icmp(&server, state.path(), "icmp").code(2);
        let err = &stderr_json(&a)["error"];
        assert_eq!(err["kind"], "invalid");
        assert_eq!(
            err["message"],
            "inline rule services take tcp or udp (optionally with a port, for example 443/tcp or tcp); for icmp and other protocols use a service object (service:<name> or an href); no ICMP service object found"
        );
        assert_eq!(
            count(
                &server,
                "POST",
                "/api/v2/orgs/5/sec_policy/draft/rule_sets/100/sec_rules"
            )
            .await,
            0
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn rules_add_workload_actor_uses_find_workload() {
        let server = MockServer::start().await;
        let state = tempfile::tempdir().unwrap();
        Mock::given(method("GET"))
            .and(path("/api/v2/orgs/5/workloads"))
            .and(query_param("max_results", "500"))
            .and(query_param("hostname", "HOST-1.example"))
            .respond_with(ResponseTemplate::new(200).set_body_json(fixture_workloads()))
            .expect(1)
            .mount(&server)
            .await;
        let out = stdout_json(
            writer(&server, state.path())
                .args([
                    "policy",
                    "rules",
                    "add",
                    RS_100,
                    "--provider",
                    "workload:HOST-1.example",
                    "--consumer",
                    "all",
                    "--service",
                    "443/tcp",
                    "--dry-run",
                ])
                .assert()
                .success(),
        );
        assert_eq!(
            out["data"]["body"],
            json!({
                "enabled": true,
                "providers": [{"workload": {"href": "/orgs/5/workloads/aaaa-1"}}],
                "consumers": [{"actors": "ams"}],
                "ingress_services": [{"port": 443, "proto": 6}],
                "unscoped_consumers": false,
                "resolve_labels_as": {"providers": ["workloads"], "consumers": ["workloads"]}
            })
        );
        assert_eq!(server.received_requests().await.unwrap().len(), 1);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn rules_add_explicit_resolve_providers_as_overrides_the_default() {
        let server = MockServer::start().await;
        let state = tempfile::tempdir().unwrap();
        let out = stdout_json(
            writer(&server, state.path())
                .args([
                    "policy",
                    "rules",
                    "add",
                    RS_100,
                    "--provider",
                    "all",
                    "--consumer",
                    "all",
                    "--service",
                    "443/tcp",
                    "--resolve-providers-as",
                    "virtual_services",
                    "--dry-run",
                ])
                .assert()
                .success(),
        );
        assert_eq!(
            out["data"]["body"]["resolve_labels_as"],
            json!({"providers": ["virtual_services"], "consumers": ["workloads"]})
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn rules_add_errors_before_any_request() {
        let server = MockServer::start().await;
        let state = tempfile::tempdir().unwrap();
        let a = writer(&server, state.path())
            .args([
                "policy",
                "rules",
                "add",
                "shop-prod",
                "--consumer",
                "all",
                "--service",
                "443/tcp",
                "--dry-run",
            ])
            .assert()
            .code(2);
        assert_eq!(stderr_json(&a)["error"]["kind"], "invalid");
        assert_eq!(
            stderr_json(&a)["error"]["message"],
            "a rule needs at least one provider"
        );
        for (args, msg) in [
            (["--provider", "!all"], "all cannot be excluded"),
            (["--provider", "web"], "is not an actor"),
            (["--provider", "!workload:x"], "can be excluded"),
        ] {
            let a = writer(&server, state.path())
                .args(["policy", "rules", "add", RS_100])
                .args(args)
                .args(["--consumer", "all", "--service", "443/tcp", "--dry-run"])
                .assert()
                .code(2);
            let m = stderr_json(&a)["error"]["message"]
                .as_str()
                .unwrap()
                .to_string();
            assert!(m.contains(msg), "{args:?}: {m}");
        }
        let a = writer(&server, state.path())
            .args([
                "policy",
                "rules",
                "add",
                RS_100,
                "--provider",
                "all",
                "--consumer",
                "all",
                "--service",
                "443",
                "--dry-run",
            ])
            .assert()
            .code(2);
        assert!(
            stderr_json(&a)["error"]["message"]
                .as_str()
                .unwrap()
                .contains("bare number is ambiguous")
        );
        assert!(server.received_requests().await.unwrap().is_empty());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn rules_add_unknown_names_list_known_values_and_similar() {
        let server = MockServer::start().await;
        let state = tempfile::tempdir().unwrap();
        mount_lists(&server).await;
        let a = writer(&server, state.path())
            .args([
                "policy",
                "rules",
                "add",
                RS_100,
                "--provider",
                "role=we",
                "--consumer",
                "all",
                "--service",
                "443/tcp",
                "--dry-run",
            ])
            .assert()
            .code(2);
        assert_eq!(
            stderr_json(&a)["error"]["message"],
            "unknown label role=we; known values for role: db, web; similar: web"
        );
        let a = writer(&server, state.path())
            .args([
                "policy",
                "rules",
                "add",
                RS_100,
                "--provider",
                "all",
                "--consumer",
                "iplist:corp",
                "--service",
                "443/tcp",
                "--dry-run",
            ])
            .assert()
            .code(2);
        assert_eq!(
            stderr_json(&a)["error"]["message"],
            r#"no IP list named "corp"; similar: Corp"#
        );
        assert!(
            server
                .received_requests()
                .await
                .unwrap()
                .iter()
                .all(|r| r.method.as_str() == "GET")
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn rules_add_sends_once_and_reports_the_nested_href() {
        let server = MockServer::start().await;
        let state = tempfile::tempdir().unwrap();
        let body = json!({
            "enabled": true,
            "providers": [{"actors": "ams"}],
            "consumers": [{"actors": "ams"}],
            "ingress_services": [{"port": 443, "proto": 6}],
            "unscoped_consumers": false,
            "resolve_labels_as": {"providers": ["workloads"], "consumers": ["workloads"]}
        });
        let mut created = body.clone();
        created["href"] = json!(format!("{RS_100}/sec_rules/9"));
        Mock::given(method("POST"))
            .and(path(format!("/api/v2{RS_100}/sec_rules")))
            .and(body_json(body))
            .respond_with(ResponseTemplate::new(201).set_body_json(created))
            .expect(1)
            .mount(&server)
            .await;
        let a = writer(&server, state.path())
            .args([
                "policy",
                "rules",
                "add",
                RS_100,
                "--provider",
                "all",
                "--consumer",
                "all",
                "--service",
                "443/tcp",
                "--yes",
            ])
            .assert()
            .success();
        assert_no_secret(&a);
        let out = stdout_json(a);
        assert_eq!(out["data"]["href"], format!("{RS_100}/sec_rules/9"));
        assert_eq!(out["data"]["pending"], true);
        assert_eq!(out["meta"]["write"]["method"], "POST");
        assert_eq!(out["meta"]["write"]["status"], 201);
        let lines = audit_lines(state.path());
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0]["path"], format!("{RS_100}/sec_rules"));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn rules_update_by_href_adds_and_removes() {
        let server = MockServer::start().await;
        let state = tempfile::tempdir().unwrap();
        mount_get(&server, RULE_2, fixture_rule_sets()[0]["rules"][1].clone()).await;
        let out = stdout_json(
            writer(&server, state.path())
                .args([
                    "policy",
                    "rules",
                    "update",
                    "/orgs/5/sec_policy/active/rule_sets/100/sec_rules/2",
                    "--add-consumer",
                    "all",
                    "--remove-service",
                    "8000-8099/tcp",
                    "--scoped-consumers",
                    "--dry-run",
                ])
                .assert()
                .success(),
        );
        assert_eq!(out["data"]["method"], "PUT");
        assert_eq!(out["data"]["path"], RULE_2);
        assert_eq!(
            out["data"]["body"],
            json!({
                "consumers": [
                    {"ip_list": {"href": "/orgs/5/sec_policy/draft/ip_lists/21"}},
                    {"actors": "ams"}
                ],
                "ingress_services": [{"port": 443, "proto": 6}],
                "unscoped_consumers": false
            })
        );
        assert_eq!(
            server.received_requests().await.unwrap().len(),
            1,
            "the rule GET only"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn rules_update_refuses_the_wrong_href_shape_before_any_request() {
        let server = MockServer::start().await;
        let state = tempfile::tempdir().unwrap();
        for href in [
            RS_100,
            DENY_8,
            "/orgs/5/sec_policy/draft/rule_sets/100/sec_rules/../2",
        ] {
            let a = writer(&server, state.path())
                .args([
                    "policy",
                    "rules",
                    "update",
                    href,
                    "--description",
                    "x",
                    "--dry-run",
                ])
                .assert()
                .code(2);
            let e = stderr_json(&a);
            assert_eq!(e["error"]["kind"], "invalid", "{href}");
            assert!(
                e["error"]["message"].as_str().unwrap().contains(
                    "expected /orgs/<org>/sec_policy/draft/rule_sets/<id>/sec_rules/<id>"
                ),
                "{href}: {e}"
            );
        }
        let a = writer(&server, state.path())
            .args(["policy", "rules", "update", RULE_2, "--dry-run"])
            .assert()
            .code(2);
        assert!(
            stderr_json(&a)["error"]["message"]
                .as_str()
                .unwrap()
                .starts_with("nothing to change")
        );
        assert!(server.received_requests().await.unwrap().is_empty());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn rules_update_removing_an_absent_consumer_is_exit_2() {
        let server = MockServer::start().await;
        let state = tempfile::tempdir().unwrap();
        mount_get(&server, RULE_2, fixture_rule_sets()[0]["rules"][1].clone()).await;
        let a = writer(&server, state.path())
            .args([
                "policy",
                "rules",
                "update",
                RULE_2,
                "--remove-consumer",
                "all",
                "--yes",
            ])
            .assert()
            .code(2);
        assert_eq!(
            stderr_json(&a)["error"]["message"],
            "all is not among the consumers"
        );
        assert_eq!(server.received_requests().await.unwrap().len(), 1);
        assert!(audit_lines(state.path()).is_empty());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn deny_rules_add_update_delete_and_rule_delete() {
        let server = MockServer::start().await;
        let state = tempfile::tempdir().unwrap();
        mount_lists(&server).await;
        let deny: Value =
            serde_json::from_str(include_str!("../../../fixtures/deny_rules.json")).unwrap();
        mount_get(&server, DENY_8, deny[1].clone()).await;
        let out = stdout_json(
            writer(&server, state.path())
                .args([
                    "policy",
                    "deny-rules",
                    "add",
                    RS_100,
                    "--provider",
                    "all",
                    "--consumer",
                    "iplist:Corp",
                    "--service",
                    "22/tcp",
                    "--override",
                    "--dry-run",
                ])
                .assert()
                .success(),
        );
        assert_eq!(out["data"]["path"], format!("{RS_100}/deny_rules"));
        assert_eq!(
            out["data"]["body"],
            json!({
                "enabled": true,
                "providers": [{"actors": "ams"}],
                "consumers": [{"ip_list": {"href": "/orgs/5/sec_policy/draft/ip_lists/21"}}],
                "ingress_services": [{"port": 22, "proto": 6}],
                "override": true
            })
        );
        let out = stdout_json(
            writer(&server, state.path())
                .args([
                    "policy",
                    "deny-rules",
                    "update",
                    DENY_8,
                    "--no-override",
                    "--dry-run",
                ])
                .assert()
                .success(),
        );
        assert_eq!(out["data"]["body"], json!({"override": false}));
        let out = stdout_json(
            writer(&server, state.path())
                .args(["policy", "deny-rules", "delete", DENY_8, "--dry-run"])
                .assert()
                .success(),
        );
        assert_eq!(out["data"]["method"], "DELETE");
        assert_eq!(out["data"]["path"], DENY_8);
        let out = stdout_json(
            writer(&server, state.path())
                .args(["policy", "rules", "delete", RULE_2, "--dry-run"])
                .assert()
                .success(),
        );
        assert_eq!(out["data"]["method"], "DELETE");
        assert_eq!(out["data"]["path"], RULE_2);
        writer(&server, state.path())
            .args(["policy", "deny-rules", "delete", RULE_2, "--dry-run"])
            .assert()
            .code(2);
        writer(&server, state.path())
            .args([
                "policy",
                "deny-rules",
                "update",
                DENY_8,
                "--override",
                "--no-override",
                "--dry-run",
            ])
            .assert()
            .code(2);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn rules_listing_still_needs_a_ruleset() {
        let server = MockServer::start().await;
        let a = cmd(&server)
            .args(["--json", "policy", "rules"])
            .assert()
            .code(2);
        assert!(
            stderr_json(&a)["error"]["message"]
                .as_str()
                .unwrap()
                .contains("needs a ruleset name or href")
        );
        assert!(server.received_requests().await.unwrap().is_empty());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn iplist_only_tokens_read_neither_services_nor_labels() {
        let server = MockServer::start().await;
        let state = tempfile::tempdir().unwrap();
        mount_lists(&server).await;
        writer(&server, state.path())
            .args([
                "policy",
                "rules",
                "add",
                RS_100,
                "--provider",
                "iplist:Corp",
                "--consumer",
                "!iplist:Corp",
                "--service",
                "443/tcp",
                "--dry-run",
            ])
            .assert()
            .success();
        assert_eq!(gets(&server, "/orgs/5/sec_policy/draft/ip_lists").await, 1);
        assert_eq!(server.received_requests().await.unwrap().len(), 1);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn port_href_and_all_tokens_cost_no_list_get() {
        let server = MockServer::start().await;
        let state = tempfile::tempdir().unwrap();
        mount_lists(&server).await;
        writer(&server, state.path())
            .args([
                "policy",
                "rules",
                "add",
                RS_100,
                "--provider",
                "all",
                "--provider",
                "/orgs/5/labels/1",
                "--consumer",
                "all",
                "--service",
                "443/tcp",
                "--service",
                "8000-8099/udp",
                "--service",
                "/orgs/5/sec_policy/active/services/32",
                "--dry-run",
            ])
            .assert()
            .success();
        assert!(server.received_requests().await.unwrap().is_empty());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_parse_failure_after_a_valid_token_sends_nothing() {
        let server = MockServer::start().await;
        let state = tempfile::tempdir().unwrap();
        mount_lists(&server).await;
        writer(&server, state.path())
            .args([
                "policy",
                "rules",
                "add",
                "shop-prod",
                "--provider",
                "role=web",
                "--provider",
                "web",
                "--consumer",
                "all",
                "--service",
                "443/tcp",
                "--dry-run",
            ])
            .assert()
            .code(2);
        assert!(server.received_requests().await.unwrap().is_empty());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn group_token_reads_only_label_groups() {
        let server = MockServer::start().await;
        let state = tempfile::tempdir().unwrap();
        mount_lists(&server).await;
        writer(&server, state.path())
            .args([
                "policy",
                "rules",
                "add",
                RS_100,
                "--provider",
                "group:All prod envs",
                "--consumer",
                "all",
                "--service",
                "443/tcp",
                "--dry-run",
            ])
            .assert()
            .success();
        assert_eq!(
            gets(&server, "/orgs/5/sec_policy/draft/label_groups").await,
            1
        );
        assert_eq!(server.received_requests().await.unwrap().len(), 1);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn two_label_tokens_make_one_labels_get() {
        let server = MockServer::start().await;
        let state = tempfile::tempdir().unwrap();
        mount_lists(&server).await;
        writer(&server, state.path())
            .args([
                "policy",
                "rules",
                "add",
                RS_100,
                "--provider",
                "role=web",
                "--consumer",
                "app=shop",
                "--service",
                "443/tcp",
                "--dry-run",
            ])
            .assert()
            .success();
        assert_eq!(gets(&server, "/orgs/5/labels").await, 1);
        assert_eq!(server.received_requests().await.unwrap().len(), 1);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_noop_ruleset_update_is_skipped() {
        let server = MockServer::start().await;
        let state = tempfile::tempdir().unwrap();
        mount_get(&server, RS_100, fixture_rule_sets()[0].clone()).await;
        let out = stdout_json(
            writer(&server, state.path())
                .args([
                    "policy",
                    "rulesets",
                    "update",
                    RS_100,
                    "--enabled",
                    "--dry-run",
                ])
                .assert()
                .success(),
        );
        assert_eq!(out["data"]["skipped"], true);
    }

    const SHOP_RULE: [&str; 10] = [
        "policy",
        "rules",
        "add",
        "Shop-Prod",
        "--provider",
        "role=web",
        "--consumer",
        "iplist:Corp",
        "--service",
        "443/tcp",
    ];

    #[tokio::test(flavor = "multi_thread")]
    async fn rules_add_dry_run_impact_reads_workloads_once_and_sends_nothing() {
        let server = MockServer::start().await;
        let state = tempfile::tempdir().unwrap();
        mount_lists(&server).await;
        mount_get(&server, "/orgs/5/workloads", fixture_workloads()).await;
        let out = stdout_json(
            writer(&server, state.path())
                .args(SHOP_RULE)
                .args(["--dry-run", "--impact"])
                .assert()
                .success(),
        );
        assert_eq!(out["data"]["method"], "POST");
        assert_eq!(out["data"]["path"], format!("{RS_100}/sec_rules"));
        assert_eq!(
            out["data"]["body"]["providers"],
            json!([{"label": {"href": "/orgs/5/labels/1"}}])
        );
        let impact = &out["data"]["impact"];
        assert_eq!(
            (impact["providers"].clone(), impact["consumers"].clone()),
            (json!(1), json!(0))
        );
        assert_eq!(
            impact["per_scope"],
            json!([{"scope": ["app=shop", "env=prod"], "providers": 1, "consumers": 0}])
        );
        assert_eq!(impact["ip_lists"][0]["name"], "Corp");
        assert_eq!(impact["ip_lists"][0]["side"], "consumer");
        assert_eq!(out["meta"]["truncated"], false);
        assert_eq!(gets(&server, "/orgs/5/workloads").await, 1);
        assert_eq!(
            gets(&server, "/orgs/5/sec_policy/draft/rule_sets").await,
            1,
            "the name and the scopes share one read"
        );
        assert!(
            server
                .received_requests()
                .await
                .unwrap()
                .iter()
                .all(|r| r.method.as_str() == "GET"),
            "a dry run sends no write"
        );
        assert!(audit_lines(state.path()).is_empty());
        // Without --impact the dry run keeps its shape and reads no workloads.
        let out = stdout_json(
            writer(&server, state.path())
                .args(SHOP_RULE)
                .arg("--dry-run")
                .assert()
                .success(),
        );
        assert!(out["data"].get("impact").is_none());
        assert_eq!(gets(&server, "/orgs/5/workloads").await, 1);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn rules_add_impact_with_providers_resolved_as_virtual_services_counts_none() {
        let server = MockServer::start().await;
        let state = tempfile::tempdir().unwrap();
        mount_lists(&server).await;
        mount_get(&server, "/orgs/5/workloads", fixture_workloads()).await;
        let out = stdout_json(
            writer(&server, state.path())
                .args(SHOP_RULE)
                .args([
                    "--resolve-providers-as",
                    "virtual_services",
                    "--dry-run",
                    "--impact",
                ])
                .assert()
                .success(),
        );
        let impact = &out["data"]["impact"];
        assert_eq!(impact["providers"], 0);
        assert_eq!(
            impact["unsupported"],
            json!(["providers resolve as virtual_services only"])
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn rules_add_yes_impact_is_computed_before_the_post() {
        let server = MockServer::start().await;
        let state = tempfile::tempdir().unwrap();
        mount_lists(&server).await;
        mount_get(&server, "/orgs/5/workloads", fixture_workloads()).await;
        Mock::given(method("POST"))
            .and(path(format!("/api/v2{RS_100}/sec_rules")))
            .respond_with(ResponseTemplate::new(201).set_body_json(json!({
                "href": format!("{RS_100}/sec_rules/9"), "enabled": true,
                "providers": [{"label": {"href": "/orgs/5/labels/1"}}],
                "consumers": [{"ip_list": {"href": "/orgs/5/sec_policy/draft/ip_lists/21"}}],
                "ingress_services": [{"port": 443, "proto": 6}]
            })))
            .expect(1)
            .mount(&server)
            .await;
        let out = stdout_json(
            writer(&server, state.path())
                .args(SHOP_RULE)
                .args(["--yes", "--impact"])
                .assert()
                .success(),
        );
        assert_eq!(out["data"]["href"], format!("{RS_100}/sec_rules/9"));
        assert_eq!(out["data"]["pending"], true);
        assert_eq!(out["data"]["impact"]["providers"], 1);
        assert_eq!(out["meta"]["write"]["method"], "POST");
        let requests = server.received_requests().await.unwrap();
        let at = |m: &str, p: &str| {
            requests
                .iter()
                .position(|r| r.method.as_str() == m && r.url.path() == p)
                .unwrap()
        };
        assert!(
            at("GET", "/api/v2/orgs/5/workloads")
                < at("POST", &format!("/api/v2{RS_100}/sec_rules")),
            "the impact is computed before the send"
        );
        assert_eq!(audit_lines(state.path()).len(), 1);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn rule_and_deny_rule_updates_with_impact_merge_the_loaded_rule() {
        let server = MockServer::start().await;
        let state = tempfile::tempdir().unwrap();
        mount_lists(&server).await;
        mount_get(&server, "/orgs/5/workloads", fixture_workloads()).await;
        mount_get(&server, RULE_2, fixture_rule_sets()[0]["rules"][1].clone()).await;
        let deny: Value =
            serde_json::from_str(include_str!("../../../fixtures/deny_rules.json")).unwrap();
        mount_get(&server, DENY_8, deny[1].clone()).await;
        let pc = |out: &Value| {
            (
                out["data"]["impact"]["providers"].clone(),
                out["data"]["impact"]["consumers"].clone(),
            )
        };
        let out = stdout_json(
            writer(&server, state.path())
                .args([
                    "policy",
                    "rules",
                    "update",
                    RULE_2,
                    "--add-consumer",
                    "all",
                    "--dry-run",
                    "--impact",
                ])
                .assert()
                .success(),
        );
        assert_eq!(out["data"]["method"], "PUT");
        assert_eq!(
            pc(&out),
            (json!(1), json!(4)),
            "providers from the loaded rule; unscoped, so every workload consumes"
        );
        let out = stdout_json(
            writer(&server, state.path())
                .args([
                    "policy",
                    "deny-rules",
                    "update",
                    DENY_8,
                    "--add-provider",
                    "role=web",
                    "--dry-run",
                    "--impact",
                ])
                .assert()
                .success(),
        );
        assert_eq!(
            pc(&out),
            (json!(2), json!(2)),
            "role=db or role=web inside the scope; deny consumers stay scoped"
        );
        let out = stdout_json(
            writer(&server, state.path())
                .args([
                    "policy",
                    "deny-rules",
                    "add",
                    "Shop-Prod",
                    "--provider",
                    "all",
                    "--consumer",
                    "role=web",
                    "--service",
                    "22/tcp",
                    "--dry-run",
                    "--impact",
                ])
                .assert()
                .success(),
        );
        assert_eq!(pc(&out), (json!(2), json!(1)));
        // Three --impact runs (three processes, no shared cache): each reads
        // the workloads once and the rulesets once (the by-name run shares
        // that read for the name), so an href adds no name-resolution read;
        // the planner GETs each updated rule once.
        assert_eq!(gets(&server, "/orgs/5/workloads").await, 3);
        assert_eq!(gets(&server, "/orgs/5/sec_policy/draft/rule_sets").await, 3);
        assert_eq!(gets(&server, RULE_2).await, 1);
        assert_eq!(gets(&server, DENY_8).await, 1);
        assert!(
            server
                .received_requests()
                .await
                .unwrap()
                .iter()
                .all(|r| r.method.as_str() == "GET")
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn update_impact_of_a_rule_missing_from_the_rulesets_is_exit_4_before_workloads() {
        let server = MockServer::start().await;
        let state = tempfile::tempdir().unwrap();
        mount_lists(&server).await;
        mount_get(&server, "/orgs/5/workloads", fixture_workloads()).await;
        let missing = format!("{RS_100}/sec_rules/77");
        let mut rule = fixture_rule_sets()[0]["rules"][1].clone();
        rule["href"] = json!(missing);
        mount_get(&server, &missing, rule).await;
        let a = writer(&server, state.path())
            .args([
                "policy",
                "rules",
                "update",
                &missing,
                "--add-consumer",
                "all",
                "--dry-run",
                "--impact",
            ])
            .assert()
            .code(4);
        assert_eq!(stderr_json(&a)["error"]["kind"], "not_found");
        assert_eq!(gets(&server, "/orgs/5/workloads").await, 0);
        assert!(
            server
                .received_requests()
                .await
                .unwrap()
                .iter()
                .all(|r| r.method.as_str() == "GET")
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn rules_add_yes_impact_fails_on_a_workloads_error_before_any_post() {
        let server = MockServer::start().await;
        let state = tempfile::tempdir().unwrap();
        mount_lists(&server).await;
        Mock::given(method("GET"))
            .and(path("/api/v2/orgs/5/workloads"))
            .respond_with(ResponseTemplate::new(500))
            .mount(&server)
            .await;
        writer(&server, state.path())
            .args(SHOP_RULE)
            .args(["--yes", "--impact"])
            .assert()
            .failure();
        assert!(
            server
                .received_requests()
                .await
                .unwrap()
                .iter()
                .all(|r| r.method.as_str() == "GET"),
            "no POST"
        );
        assert!(audit_lines(state.path()).is_empty());
    }
}

mod provision {
    use super::policy::{fixture_rule_sets, mount_get};
    use super::*;

    fn pending_dict() -> Value {
        serde_json::from_str(include_str!("../../../fixtures/pending_dict.json")).unwrap()
    }

    fn all_body() -> Value {
        json!({
            "update_description": "illumio-tui provision",
            "change_subset": {
                "ip_lists": [{"href": "/orgs/5/sec_policy/draft/ip_lists/21"}],
                "rule_sets": [{"href": "/orgs/5/sec_policy/draft/rule_sets/101"}]
            }
        })
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn all_dry_run_lists_every_pending_href_with_counts() {
        let server = MockServer::start().await;
        let state = tempfile::tempdir().unwrap();
        mount_get(&server, "/orgs/5/sec_policy/pending", pending_dict()).await;
        let out = stdout_json(
            writer(&server, state.path())
                .args(["policy", "provision", "--all", "--dry-run"])
                .assert()
                .success(),
        );
        assert_eq!(out["data"]["method"], "POST");
        assert_eq!(out["data"]["path"], "/orgs/5/sec_policy");
        assert_eq!(out["data"]["body"], all_body());
        assert_eq!(
            out["data"]["counts"],
            json!({"ip_lists": 1, "rule_sets": 1})
        );
        assert_eq!(out["data"]["summary"], "provision 1 ip_list, 1 rule_set");
        assert_eq!(server.received_requests().await.unwrap().len(), 1);
        assert!(audit_lines(state.path()).is_empty());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn by_ruleset_name_and_href_with_a_description() {
        let server = MockServer::start().await;
        let state = tempfile::tempdir().unwrap();
        mount_get(&server, "/orgs/5/sec_policy/pending", pending_dict()).await;
        mount_get(
            &server,
            "/orgs/5/sec_policy/draft/rule_sets",
            fixture_rule_sets(),
        )
        .await;
        let out = stdout_json(
            writer(&server, state.path())
                .args([
                    "policy",
                    "provision",
                    "global-infra",
                    "/orgs/5/sec_policy/active/ip_lists/21",
                    "--description",
                    "change 42",
                    "--dry-run",
                ])
                .assert()
                .success(),
        );
        let mut expected = all_body();
        expected["update_description"] = json!("change 42");
        assert_eq!(out["data"]["body"], expected);
        assert_eq!(
            count(&server, "GET", "/api/v2/orgs/5/sec_policy/draft/rule_sets").await,
            1
        );
        assert_eq!(
            count(&server, "GET", "/api/v2/orgs/5/sec_policy/pending").await,
            1
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_ruleset_pending_deletion_is_provisioned_by_name() {
        let server = MockServer::start().await;
        let state = tempfile::tempdir().unwrap();
        mount_get(
            &server,
            "/orgs/5/sec_policy/pending",
            json!({"rule_sets": [{"href": "/orgs/5/sec_policy/draft/rule_sets/102", "update_type": "delete", "name": "Gone"}],
                   "ip_lists": [], "services": [], "label_groups": []}),
        )
        .await;
        mount_get(
            &server,
            "/orgs/5/sec_policy/draft/rule_sets",
            json!([{"href": "/orgs/5/sec_policy/draft/rule_sets/102", "update_type": "delete", "name": "Gone", "enabled": true}]),
        )
        .await;
        let out = stdout_json(
            writer(&server, state.path())
                .args(["policy", "provision", "gone", "--dry-run"])
                .assert()
                .success(),
        );
        assert_eq!(
            out["data"]["body"]["change_subset"],
            json!({"rule_sets": [{"href": "/orgs/5/sec_policy/draft/rule_sets/102"}]})
        );
        assert_eq!(
            count(&server, "GET", "/api/v2/orgs/5/sec_policy/draft/rule_sets").await,
            1
        );
        assert_eq!(
            count(&server, "GET", "/api/v2/orgs/5/sec_policy/pending").await,
            1
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_named_ruleset_without_pending_changes_is_reported_by_name_and_href() {
        let server = MockServer::start().await;
        let state = tempfile::tempdir().unwrap();
        mount_get(&server, "/orgs/5/sec_policy/pending", pending_dict()).await;
        mount_get(
            &server,
            "/orgs/5/sec_policy/draft/rule_sets",
            fixture_rule_sets(),
        )
        .await;
        let a = writer(&server, state.path())
            .args(["policy", "provision", "Shop-Prod", "--dry-run"])
            .assert()
            .code(2);
        assert_eq!(
            stderr_json(&a)["error"]["message"],
            "Shop-Prod (/orgs/5/sec_policy/draft/rule_sets/100) has no pending changes"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_non_pending_href_is_exit_2_and_nothing_pending_is_skipped() {
        let server = MockServer::start().await;
        let state = tempfile::tempdir().unwrap();
        mount_get(&server, "/orgs/5/sec_policy/pending", pending_dict()).await;
        let a = writer(&server, state.path())
            .args([
                "policy",
                "provision",
                "/orgs/5/sec_policy/draft/services/31",
                "--yes",
            ])
            .assert()
            .code(2);
        assert_eq!(stderr_json(&a)["error"]["kind"], "invalid");
        assert_eq!(
            stderr_json(&a)["error"]["message"],
            "/orgs/5/sec_policy/draft/services/31 has no pending changes"
        );
        let empty = MockServer::start().await;
        mount_get(&empty, "/orgs/5/sec_policy/pending", json!([])).await;
        let out = stdout_json(
            writer(&empty, state.path())
                .args(["policy", "provision", "--all", "--yes"])
                .assert()
                .success(),
        );
        assert_eq!(out["data"]["skipped"], true);
        assert_eq!(out["data"]["reason"], "nothing pending");
        assert!(audit_lines(state.path()).is_empty());
        writer(&empty, state.path())
            .args(["policy", "provision", "--dry-run"])
            .assert()
            .code(2);
        writer(&empty, state.path())
            .args(["policy", "provision", "--all", "Global-Infra", "--dry-run"])
            .assert()
            .code(2);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn sends_once_and_reports_the_version_in_json_and_table() {
        let server = MockServer::start().await;
        let state = tempfile::tempdir().unwrap();
        mount_get(&server, "/orgs/5/sec_policy/pending", pending_dict()).await;
        Mock::given(method("POST"))
            .and(path("/api/v2/orgs/5/sec_policy"))
            .and(body_json(all_body()))
            .respond_with(
                ResponseTemplate::new(201)
                    .set_body_json(json!({"href": "/orgs/5/sec_policy/12", "version": 12})),
            )
            .expect(2)
            .mount(&server)
            .await;
        let a = writer(&server, state.path())
            .args(["policy", "provision", "--all", "--yes"])
            .assert()
            .success();
        assert_no_secret(&a);
        let out = stdout_json(a);
        assert_eq!(out["data"]["href"], "/orgs/5/sec_policy/12");
        assert_eq!(out["data"]["pending"], false);
        assert_eq!(
            out["data"]["counts"],
            json!({"ip_lists": 1, "rule_sets": 1})
        );
        assert_eq!(out["data"]["dropped_fields"], json!([]));
        assert_eq!(out["meta"]["write"]["method"], "POST");
        assert_eq!(out["meta"]["write"]["path"], "/orgs/5/sec_policy");
        let mut t = cmd(&server);
        t.env("ILLUMIO_WRITES", "1")
            .env("XDG_STATE_HOME", state.path());
        t.args([
            "--allow-writes",
            "--table",
            "policy",
            "provision",
            "--all",
            "--yes",
        ])
        .assert()
        .success()
        .stdout(
            predicate::str::contains("provisioned")
                .and(predicate::str::contains("/orgs/5/sec_policy/12"))
                .and(predicate::str::contains("ip_lists=1, rule_sets=1")),
        );
        let lines = audit_lines(state.path());
        assert_eq!(lines.len(), 2, "one audit line per send");
        assert_eq!(lines[0]["body"], all_body());
    }
}

mod managed_labels {
    use super::policy::{fixture_labels, fixture_workloads, mount_get};
    use super::*;

    #[tokio::test(flavor = "multi_thread")]
    async fn managed_labels_relabels_a_managed_workload_with_labels_only() {
        let server = MockServer::start().await;
        let state = tempfile::tempdir().unwrap();
        mount_get(&server, "/orgs/5/labels", fixture_labels()).await;
        mount_get(
            &server,
            "/orgs/5/workloads/aaaa-1",
            fixture_workloads()[0].clone(),
        )
        .await;
        Mock::given(method("PUT"))
            .and(path("/api/v2/orgs/5/workloads/aaaa-1"))
            .and(body_json(json!({"labels": [
                {"href": "/orgs/5/labels/1"}, {"href": "/orgs/5/labels/3"},
                {"href": "/orgs/5/labels/4"}, {"href": "/orgs/5/labels/5"}
            ]})))
            .respond_with(ResponseTemplate::new(204))
            .expect(1)
            .mount(&server)
            .await;
        let out = stdout_json(
            writer(&server, state.path())
                .args([
                    "workloads",
                    "update",
                    "/orgs/5/workloads/aaaa-1",
                    "--add-label",
                    "loc=eu",
                    "--managed-labels",
                    "--yes",
                ])
                .assert()
                .success(),
        );
        assert_eq!(out["data"]["href"], "/orgs/5/workloads/aaaa-1");
        assert_eq!(out["data"]["pending"], false);
        assert_eq!(audit_lines(state.path()).len(), 1);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn managed_labels_by_hostname_dry_run() {
        let server = MockServer::start().await;
        let state = tempfile::tempdir().unwrap();
        mount_get(&server, "/orgs/5/labels", fixture_labels()).await;
        mount_get(&server, "/orgs/5/workloads", fixture_workloads()).await;
        mount_get(
            &server,
            "/orgs/5/workloads/aaaa-1",
            fixture_workloads()[0].clone(),
        )
        .await;
        let out = stdout_json(
            writer(&server, state.path())
                .args([
                    "workloads",
                    "update",
                    "host-1.example",
                    "--remove-label",
                    "role",
                    "--managed-labels",
                    "--dry-run",
                ])
                .assert()
                .success(),
        );
        assert_eq!(out["data"]["method"], "PUT");
        assert_eq!(
            out["data"]["body"],
            json!({"labels": [{"href": "/orgs/5/labels/3"}, {"href": "/orgs/5/labels/4"}]})
        );
        assert_eq!(out["data"]["summary"], "relabel workload host-1.example");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn managed_labels_refuses_other_changes_before_any_request() {
        let server = MockServer::start().await;
        let state = tempfile::tempdir().unwrap();
        for extra in [
            ["--description", "x"],
            ["--name", "y"],
            ["--hostname", "h"],
            ["--add-ip", "10.0.0.9"],
            ["--remove-ip", "10.0.0.9"],
        ] {
            let a = writer(&server, state.path())
                .args([
                    "workloads",
                    "update",
                    "/orgs/5/workloads/aaaa-1",
                    "--add-label",
                    "loc=eu",
                    "--managed-labels",
                    "--yes",
                ])
                .args(extra)
                .assert()
                .code(2);
            assert_eq!(
                stderr_json(&a)["error"]["message"],
                "--managed-labels allows label changes only",
                "{extra:?}"
            );
        }
        assert!(server.received_requests().await.unwrap().is_empty());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_managed_workload_without_the_flag_is_still_refused() {
        let server = MockServer::start().await;
        let state = tempfile::tempdir().unwrap();
        mount_get(&server, "/orgs/5/labels", fixture_labels()).await;
        mount_get(
            &server,
            "/orgs/5/workloads/aaaa-1",
            fixture_workloads()[0].clone(),
        )
        .await;
        let a = writer(&server, state.path())
            .args([
                "workloads",
                "update",
                "/orgs/5/workloads/aaaa-1",
                "--add-label",
                "loc=eu",
                "--yes",
            ])
            .assert()
            .code(2);
        assert!(
            stderr_json(&a)["error"]["message"]
                .as_str()
                .unwrap()
                .contains("VEN-managed")
        );
        assert!(
            server
                .received_requests()
                .await
                .unwrap()
                .iter()
                .all(|r| r.method.as_str() == "GET")
        );
        assert!(audit_lines(state.path()).is_empty());
    }
}
