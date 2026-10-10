//! Write commands: guard, --dry-run/--yes, envelope, audit, exit codes.
#![allow(dead_code, unused_imports)]
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
