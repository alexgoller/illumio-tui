use assert_cmd::Command;
use predicates::prelude::*;
use serde_json::{Value, json};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

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
        .env("API_SECRET", "secret")
        .env(
            "XDG_CONFIG_HOME",
            std::env::temp_dir().join("illumio-tui-no-config"),
        );
    c
}

pub fn stdout_json(assert: assert_cmd::assert::Assert) -> Value {
    serde_json::from_slice(&assert.get_output().stdout).expect("stdout is JSON")
}

#[test]
fn version_and_no_subcommand() {
    Command::cargo_bin("illumio-tui")
        .unwrap()
        .arg("--version")
        .assert()
        .success()
        .stdout(predicate::str::starts_with("illumio-tui "));
    Command::cargo_bin("illumio-tui")
        .unwrap()
        .env_clear()
        .assert()
        .success()
        .stdout(predicate::str::contains("Plan 2"));
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
    let out = stdout_json(cmd(&server).args(["health", "--json"]).assert().success());
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
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_string(body)
                    .insert_header("Content-Type", "application/json"),
            )
            .mount(server)
            .await;
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
