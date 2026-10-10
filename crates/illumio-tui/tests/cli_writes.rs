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
const ALL_WRITES: [&[&str]; 12] = [
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
