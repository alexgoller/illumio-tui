#[path = "common/schema_check.rs"]
mod schema_check;
use assert_cmd::Command;
use predicates::prelude::*;
use serde_json::{Value, json};
use wiremock::matchers::{body_partial_json, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// Distinctive so a leak check cannot trip on the word "secret" in prose
/// such as "check API key, secret and org id".
const TEST_SECRET: &str = "s3cr3t-cli-test-9f2a";
/// A config directory of this call only, never created: no config.toml and
/// no marks.toml, and nothing shared with a parallel test or an earlier run.
pub fn no_config_dir() -> std::path::PathBuf {
    static N: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = N.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    std::env::temp_dir().join(format!("illumio-tui-no-config-{}-{n}", std::process::id()))
}

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
        .env("XDG_CONFIG_HOME", no_config_dir());
    c
}

pub fn stdout_json(assert: assert_cmd::assert::Assert) -> Value {
    schema_check::envelope(&assert.get_output().stdout)
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
fn traffic_help_states_the_count_units() {
    for sub in ["query", "groups", "mesh"] {
        let out = Command::cargo_bin("illumio-tui")
            .unwrap()
            .args(["traffic", sub, "--help"])
            .assert()
            .success();
        let text = String::from_utf8_lossy(&out.get_output().stdout).to_string();
        assert!(text.contains("lifetime counts"), "{sub}: {text}");
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
        vec!["gen", "--help"],
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
        vec!["schema", "--help"],
        vec!["marks", "--help"],
        vec!["marks", "list", "--help"],
        vec!["marks", "add", "--help"],
        vec!["marks", "rm", "--help"],
        vec!["marks", "export", "--help"],
        vec!["marks", "import", "--help"],
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
        .env("XDG_CONFIG_HOME", no_config_dir())
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
        .env("XDG_CONFIG_HOME", no_config_dir())
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
        "labels (usage asked, none given), workloads, draft rulesets, label groups"
    );
    assert_eq!(
        out["meta"]["usage"], "workloads, rules and scopes only",
        "a PCE without usage flags: our own counts, and the output says so"
    );
    cmd(&server)
        .args(["labels", "--unused", "--table"])
        .assert()
        .success()
        .stdout(predicate::str::contains("workloads  rules"))
        .stderr(predicate::str::contains(
            "usage from workloads, rules and scopes only",
        ));
}

#[tokio::test(flavor = "multi_thread")]
async fn labels_unused_trusts_the_pce_usage_flags_in_one_request() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v2/orgs/5/labels"))
        .and(query_param("usage", "true"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(include_str!("../../../fixtures/labels_usage.json"))
                .insert_header("Content-Type", "application/json"),
        )
        .with_priority(1)
        .expect(2)
        .mount(&server)
        .await;
    mount_policy(&server).await;
    Mock::given(method("GET"))
        .and(path("/api/v2/orgs/5/workloads"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([])))
        .expect(0) // the PCE's flags already say whether a workload holds it
        .mount(&server)
        .await;
    let out = stdout_json(
        cmd(&server)
            .args(["labels", "--unused", "--include-deleted"])
            .assert()
            .success(),
    );
    let rows = out["data"].as_array().unwrap();
    let hrefs: Vec<&str> = rows.iter().map(|r| r["href"].as_str().unwrap()).collect();
    assert_eq!(
        hrefs,
        vec!["/orgs/5/labels/6", "/orgs/5/labels/7"],
        "loc=eu is held by a pairing profile and a permission: in use"
    );
    assert_eq!(
        (rows[1]["workloads"].as_u64(), rows[1]["rules"].as_u64()),
        (Some(0), Some(0))
    );
    assert_eq!(rows[1]["usage"]["pairing_profile"], false);
    assert_eq!(
        out["meta"]["api_calls"], 1,
        "the labels with usage, nothing else"
    );
    assert_eq!(out["meta"]["usage"], "pce");
    cmd(&server)
        .args(["labels", "--unused", "--table"])
        .assert()
        .success()
        .stdout(predicate::str::contains("os   windows"))
        .stdout(predicate::str::contains("eu").not())
        .stderr(predicate::str::contains("usage from the PCE"));
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
    // The mock answers all four; the label filter is checked on the answer
    // too (as the app does), so the two unlabeled ones drop.
    assert_eq!(out["data"].as_array().unwrap().len(), 2);
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

/// #90: `--label` ignores case: the same one request as the exact term, the
/// local `!` check too; `meta` names the terms as typed. Several labels
/// that differ only in case are exit 2, before any workloads request.
#[tokio::test(flavor = "multi_thread")]
async fn workloads_label_ignores_case_and_ambiguity_is_exit_2() {
    use wiremock::matchers::query_param;
    let server = MockServer::start().await;
    mount_objects(&server).await;
    Mock::given(method("GET"))
        .and(path("/api/v2/orgs/5/workloads"))
        .and(query_param(
            "labels",
            r#"[["/orgs/5/labels/3","/orgs/5/labels/4"]]"#,
        ))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(include_str!("../../../fixtures/workloads.json"))
                .insert_header("Content-Type", "application/json"),
        )
        .expect(1)
        .mount(&server)
        .await;
    let out = stdout_json(
        cmd(&server)
            .args([
                "workloads",
                "--label",
                "APP=Shop",
                "--label",
                "Env=PROD",
                "--label",
                "!ROLE=DB",
            ])
            .assert()
            .success(),
    );
    // host-1 is shop, prod and web; the other shop host is db.
    assert_eq!(out["data"].as_array().unwrap().len(), 1);
    assert_eq!(out["meta"]["local_filter"], json!(["labels !ROLE=DB"]));
    server.verify().await;
    server.reset().await;
    // Applied here alone, the filter is spelt from the answer's own labels:
    // no labels list is read (none is mounted).
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
    let out = stdout_json(
        cmd(&server)
            .args(["workloads", "--label", "!ROLE=DB"])
            .assert()
            .success(),
    );
    assert_eq!(out["meta"]["api_calls"], 1);
    assert_eq!(out["data"].as_array().unwrap().len(), 3);
    server.verify().await;
    server.reset().await;
    // Two labels differing in case under one key.
    let labels = r#"[
        {"href":"/orgs/5/labels/1","key":"app","value":"Web"},
        {"href":"/orgs/5/labels/2","key":"app","value":"web"}]"#;
    Mock::given(method("GET"))
        .and(path("/api/v2/orgs/5/labels"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(labels)
                .insert_header("Content-Type", "application/json"),
        )
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v2/orgs/5/workloads"))
        .respond_with(ResponseTemplate::new(200).set_body_string("[]"))
        .expect(0)
        .mount(&server)
        .await;
    cmd(&server)
        .args(["workloads", "--label", "app=WEB"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains(
            "app=WEB matches app=Web and app=web; write one exactly",
        ));
    server.verify().await;
}

/// `--label k=a,b` is any of the values: the cross product of the terms
/// in the one `labels` parameter (outer list OR, inner AND), so each
/// invocation is exactly one list request; `!` and `-` filter its answer.
#[tokio::test(flavor = "multi_thread")]
async fn workloads_label_any_of_is_one_request_and_negation_is_local() {
    use wiremock::matchers::{query_param, query_param_is_missing};
    let server = MockServer::start().await;
    mount_objects(&server).await;
    let body = || {
        ResponseTemplate::new(200)
            .set_body_string(include_str!("../../../fixtures/workloads.json"))
            .insert_header("Content-Type", "application/json")
    };
    Mock::given(method("GET"))
        .and(path("/api/v2/orgs/5/workloads"))
        .and(query_param(
            "labels",
            r#"[["/orgs/5/labels/1","/orgs/5/labels/3"],["/orgs/5/labels/2","/orgs/5/labels/3"]]"#,
        ))
        .respond_with(body())
        .expect(1)
        .mount(&server)
        .await;
    let out = stdout_json(
        cmd(&server)
            .args([
                "workloads",
                "--label",
                "role=web,db",
                "--label",
                "app=shop",
                "--label",
                "!role=db",
            ])
            .assert()
            .success(),
    );
    // The mock answers every workload; the whole filter holds locally:
    // only host-1 is shop, web or db, and not db.
    let hrefs: Vec<&str> = out["data"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["href"].as_str().unwrap())
        .collect();
    assert_eq!(hrefs, ["/orgs/5/workloads/aaaa-1"]);
    assert_eq!(out["meta"]["local_filter"], json!(["labels !role=db"]));
    server.verify().await;
    server.reset().await;
    // Only local terms: no labels parameter, and no name index either.
    Mock::given(method("GET"))
        .and(path("/api/v2/orgs/5/workloads"))
        .and(query_param_is_missing("labels"))
        .respond_with(body())
        .expect(1)
        .mount(&server)
        .await;
    let out = stdout_json(
        cmd(&server)
            .args(["workloads", "--label", "app=-"])
            .assert()
            .success(),
    );
    assert_eq!(out["meta"]["api_calls"], 1);
    assert_eq!(out["data"].as_array().unwrap().len(), 2);
    server.verify().await;
    server.reset().await;
    // Same key twice: the values intersect in the one request...
    mount_objects(&server).await;
    Mock::given(method("GET"))
        .and(path("/api/v2/orgs/5/workloads"))
        .and(query_param("labels", r#"[["/orgs/5/labels/3"]]"#))
        .respond_with(body())
        .expect(1)
        .mount(&server)
        .await;
    let out = stdout_json(
        cmd(&server)
            .args([
                "workloads",
                "--label",
                "app=shop,nope",
                "--label",
                "app=shop",
            ])
            .assert()
            .success(),
    );
    assert_eq!(out["data"].as_array().unwrap().len(), 2);
    server.verify().await;
    server.reset().await;
    // ...and none in common is an empty answer with no request at all,
    // never an unfiltered list.
    Mock::given(method("GET"))
        .respond_with(body())
        .expect(0)
        .mount(&server)
        .await;
    let a = cmd(&server)
        .args(["workloads", "--label", "app=shop", "--label", "app=crm"])
        .assert()
        .success();
    let stderr = String::from_utf8_lossy(&a.get_output().stderr).to_string();
    let out = stdout_json(a);
    assert_eq!(out["data"].as_array().unwrap().len(), 0);
    assert_eq!(out["meta"]["api_calls"], 0);
    assert!(stderr.contains("no request sent"), "{stderr}");
    server.verify().await;
    server.reset().await;
    // Too many combinations: refused before any request.
    mount_objects(&server).await;
    Mock::given(method("GET"))
        .and(path("/api/v2/orgs/5/workloads"))
        .respond_with(body())
        .expect(0)
        .mount(&server)
        .await;
    let a = cmd(&server)
        .args([
            "workloads",
            "--label",
            "a=1,2,3,4,5,6,7,8",
            "--label",
            "b=1,2,3,4,5,6,7",
        ])
        .assert()
        .code(2);
    let stderr = String::from_utf8_lossy(&a.get_output().stderr).to_string();
    assert!(stderr.contains("56 label combinations"), "{stderr}");
    cmd(&server)
        .args(["workloads", "--label", "app=shop,,crm"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("empty value"));
}

#[tokio::test(flavor = "multi_thread")]
async fn workloads_group_prints_sections_json_groups_and_a_csv_group_column() {
    let server = MockServer::start().await;
    mount_objects(&server).await;
    Mock::given(method("GET"))
        .and(path("/api/v2/orgs/5/workloads"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(include_str!("../../../fixtures/workloads.json"))
                .insert_header("Content-Type", "application/json"),
        )
        .expect(4)
        .mount(&server)
        .await;
    let out = stdout_json(
        cmd(&server)
            .args(["workloads", "--group"])
            .assert()
            .success(),
    );
    assert_eq!(out["meta"]["api_calls"], 1, "grouping is local");
    let data = &out["data"];
    assert_eq!(data["group_by"], json!(["app", "env"]));
    let groups = data["groups"].as_array().unwrap();
    assert_eq!(groups.len(), 2);
    assert_eq!(groups[0]["key"], json!({"app": "shop", "env": "prod"}));
    assert_eq!(groups[0]["count"], 2);
    assert_eq!(groups[0]["filter"], "app=shop env=prod");
    assert_eq!(groups[0]["workloads"][0]["hostname"], "host-1.example");
    assert_eq!(groups[0]["enforcement"]["full"], 1);
    assert!(groups[0]["health"]["offline"].as_u64().unwrap() >= 1);
    assert_eq!(groups[1]["key"], json!({"app": null, "env": null}));
    assert_eq!(groups[1]["filter"], "app=- env=-");
    assert_eq!(groups[1]["health"]["unmanaged"], 1);
    // Named keys.
    let out = stdout_json(
        cmd(&server)
            .args(["workloads", "--group", "role"])
            .assert()
            .success(),
    );
    let keys: Vec<Value> = out["data"]["groups"]
        .as_array()
        .unwrap()
        .iter()
        .map(|g| g["key"]["role"].clone())
        .collect();
    assert_eq!(keys, [json!(null), json!("db"), json!("web")]);
    cmd(&server)
        .args(["workloads", "--group", "--table"])
        .assert()
        .success()
        .stdout(
            predicate::str::starts_with("hostname")
                .and(predicate::str::contains("shop | prod · 2 workloads"))
                .and(predicate::str::contains(
                    "(no app) | (no env) · 2 workloads",
                ))
                .and(predicate::str::contains("host-4.example")),
        );
    let a = cmd(&server)
        .args(["workloads", "--group", "--csv"])
        .assert()
        .success();
    let csv = String::from_utf8_lossy(&a.get_output().stdout).to_string();
    let lines: Vec<&str> = csv.lines().collect();
    assert!(!lines[0].is_empty());
    assert!(lines[0].starts_with("group,hostname,"), "{}", lines[0]);
    assert!(
        lines[1].starts_with("app=shop env=prod,host-1.example,"),
        "{}",
        lines[1]
    );
    assert!(lines[3].starts_with("app=- env=-,"), "{}", lines[3]);
    assert_eq!(lines.len(), 5, "one record per workload, no headings");
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
    assert_eq!(rows[2]["enforcement"], "unmanaged", "effective: no VEN");
    assert_eq!(rows[2]["enforcement_mode"], "idle", "raw API value");
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
async fn workloads_offline_and_unlabeled_filter_locally_after_one_request() {
    use wiremock::matchers::query_param_is_missing;
    let server = MockServer::start().await;
    mount_objects(&server).await;
    // No `online` sent: offline is the VEN health, a stale heartbeat too.
    Mock::given(method("GET"))
        .and(path("/api/v2/orgs/5/workloads"))
        .and(query_param_is_missing("online"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(include_str!("../../../fixtures/workloads.json"))
                .insert_header("Content-Type", "application/json"),
        )
        .expect(5)
        .mount(&server)
        .await;
    let hosts = |out: &Value| -> Vec<String> {
        out["data"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| r["href"].as_str().unwrap().to_string())
            .collect()
    };
    let out = stdout_json(
        cmd(&server)
            .args(["workloads", "--unlabeled"])
            .assert()
            .success(),
    );
    assert_eq!(
        hosts(&out),
        ["/orgs/5/workloads/aaaa-3", "/orgs/5/workloads/aaaa-4"]
    );
    assert_eq!(
        out["data"][0]["missing_labels"],
        json!(["app", "env", "role"])
    );
    assert_eq!(out["meta"]["api_calls"], 1);
    assert_eq!(
        out["meta"]["local_filter"],
        json!(["unlabeled (app, env, role)"])
    );
    // Named keys; a workload with all of them is kept out.
    let a = cmd(&server)
        .args(["workloads", "--unlabeled", "env,os,ENV"])
        .assert()
        .success();
    let stderr = String::from_utf8_lossy(&a.get_output().stderr).to_string();
    let out = stdout_json(a);
    assert_eq!(hosts(&out).len(), 4, "no workload has an os label");
    assert_eq!(out["data"][0]["missing_labels"], json!(["os", "ENV"]));
    assert_eq!(
        out["meta"]["local_filter"],
        json!(["unlabeled (env, os, ENV)"])
    );
    // Keys match exactly, as label keys do everywhere; one that is neither
    // in the label model nor on any workload is likely a typo.
    let warnings = out["meta"]["warnings"].as_array().unwrap();
    assert_eq!(warnings.len(), 2, "{warnings:?}");
    assert!(
        warnings[0]
            .as_str()
            .unwrap()
            .contains("unknown label key os")
    );
    assert!(warnings[1].as_str().unwrap().contains("did you mean env"));
    assert!(stderr.contains("unknown label key os"), "{stderr}");
    assert!(out["meta"].get("warnings").is_some());
    // Offline: the fixture's heartbeats are days old, so every VEN; never
    // the workload without one.
    let out = stdout_json(
        cmd(&server)
            .args(["workloads", "--offline"])
            .assert()
            .success(),
    );
    let rows = out["data"].as_array().unwrap();
    assert!(rows.iter().all(|r| r["health"] == "offline"), "{rows:?}");
    assert!(hosts(&out).contains(&"/orgs/5/workloads/aaaa-2".to_string()));
    assert!(!hosts(&out).contains(&"/orgs/5/workloads/aaaa-3".to_string()));
    assert!(
        rows[0].get("missing_labels").is_none(),
        "only with --unlabeled"
    );
    assert_eq!(out["meta"]["local_filter"], json!(["offline"]));
    assert!(
        out["meta"].get("warnings").is_none(),
        "none without --unlabeled keys"
    );
    // Table: the missing column, and the meta line names the local filter.
    cmd(&server)
        .args(["workloads", "--unlabeled", "--csv"])
        .assert()
        .success()
        .stdout(
            predicate::str::contains(",missing,").and(predicate::str::contains("app; env; role")),
        );
    cmd(&server)
        .args(["workloads", "--unlabeled", "--table"])
        .assert()
        .success()
        .stdout(predicate::str::contains("missing").and(predicate::str::contains("app, env, role")))
        .stderr(predicate::str::contains(
            "filtered locally: unlabeled (app, env, role)",
        ));
    // A contradiction: no request (the mock expects exactly five).
    cmd(&server)
        .args(["workloads", "--offline", "--online", "true"])
        .assert()
        .code(2);
}

#[tokio::test(flavor = "multi_thread")]
async fn workloads_enforcement_filter_is_the_effective_mode() {
    use wiremock::matchers::query_param_is_missing;
    let server = MockServer::start().await;
    mount_objects(&server).await;
    let body = workloads_with_unmanaged_in_shop();
    // unmanaged: managed=false and no enforcement_mode.
    Mock::given(method("GET"))
        .and(path("/api/v2/orgs/5/workloads"))
        .and(query_param("managed", "false"))
        .and(query_param_is_missing("enforcement_mode"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(body.clone())
                .insert_header("Content-Type", "application/json"),
        )
        .expect(1)
        .mount(&server)
        .await;
    // A mode: managed=true too, so workloads without a VEN stay out. The
    // answer here still carries one (as a PCE ignoring the flag would);
    // the local check drops it.
    Mock::given(method("GET"))
        .and(path("/api/v2/orgs/5/workloads"))
        .and(query_param("enforcement_mode", "visibility_only"))
        .and(query_param("managed", "true"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(body)
                .insert_header("Content-Type", "application/json"),
        )
        .expect(1)
        .mount(&server)
        .await;
    let names = |out: Value| -> Vec<String> {
        out["data"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| r["href"].as_str().unwrap().to_string())
            .collect()
    };
    let out = stdout_json(
        cmd(&server)
            .args(["workloads", "--enforcement", "unmanaged"])
            .assert()
            .success(),
    );
    assert_eq!(names(out), ["/orgs/5/workloads/aaaa-3"]);
    let out = stdout_json(
        cmd(&server)
            .args(["workloads", "--enforcement", "visibility_only"])
            .assert()
            .success(),
    );
    assert_eq!(names(out), ["/orgs/5/workloads/aaaa-2"]);
    // A contradiction exits 2 without a request (the mocks expect one each).
    let a = cmd(&server)
        .args([
            "workloads",
            "--enforcement",
            "unmanaged",
            "--managed",
            "true",
        ])
        .assert()
        .code(2);
    let err: Value = serde_json::from_slice(&a.get_output().stderr).unwrap();
    assert_eq!(err["error"]["kind"], "args");
}

#[tokio::test(flavor = "multi_thread")]
async fn workloads_unknown_enforcement_exits_2_without_querying() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([])))
        .expect(0)
        .mount(&server)
        .await;
    let a = cmd(&server)
        .args(["workloads", "--enforcement", "visibility"])
        .assert()
        .code(2);
    let err = String::from_utf8_lossy(&a.get_output().stderr).to_string();
    assert!(
        err.contains("visibility_only") && err.contains("unmanaged"),
        "{err}"
    );
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
    // The unit line, then the column header.
    let header = text.lines().nth(1).unwrap();
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
        lines[3].starts_with("shop/prod ")
            && lines[3].ends_with("in · 2 groups · 27 conns · 1 blocked · 2 pot.blocked"),
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

/// The 06:00 to 08:00 query at `cap`, answered at its own href with `body`
/// and the status's `flows_count`; expected `times` times.
async fn mount_cap(server: &MockServer, cap: u64, times: u64, flows_count: u64, body: String) {
    let href = format!("/orgs/5/traffic_flows/async_queries/q-{cap}");
    Mock::given(method("POST"))
        .and(path("/api/v2/orgs/5/traffic_flows/async_queries"))
        .and(body_partial_json(json!({
            "start_date": "2026-10-10T06:00:00Z",
            "end_date": "2026-10-10T08:00:00Z",
            "max_results": cap
        })))
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
            // A bounded raw count: never a total, never the cap signal.
            "matches_count": 2 * flows_count,
            "flows_count": flows_count
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

/// `n` fixture flows, each on its own port from `port`.
fn rows(n: usize, port: u64) -> String {
    let one: Vec<Value> =
        serde_json::from_str(include_str!("../../../fixtures/traffic_flows.json")).unwrap();
    let flows: Vec<Value> = (0..n)
        .map(|i| {
            let mut f = one[0].clone();
            f["service"]["port"] = json!(port + i as u64);
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

/// Truncated at 4, truncated again at 10000: without `--yes` the climb
/// stops there and stderr names the next cap.
#[tokio::test(flavor = "multi_thread")]
async fn traffic_backfill_climbs_to_10000_and_asks_for_yes_above() {
    let server = MockServer::start().await;
    mount_objects(&server).await;
    mount_cap(&server, 4, 1, 4, rows(4, 1000)).await;
    mount_cap(&server, 10_000, 1, 10_000, rows(5, 2000)).await;
    let a = cmd(&server)
        .args(BACKFILL_ARGS)
        .arg("--backfill")
        .assert()
        .success();
    let err = String::from_utf8_lossy(&a.get_output().stderr).to_string();
    assert!(err.contains("backfill raises the cap to 10000"), "{err}");
    assert!(
        err.contains("backfill stopped: truncated at cap 10000; a cap of 25000 needs --yes"),
        "{err}"
    );
    let out = stdout_json(a);
    assert_eq!(ports(&out), (2000..2005).collect::<Vec<_>>(), "replaced");
    assert_eq!(out["meta"]["cap_steps"], json!([4, 10_000]));
    assert_eq!(out["meta"]["cap"], 10_000);
    assert_eq!(out["meta"]["truncated"], true);
    assert!(out["meta"]["covered"].is_null(), "{}", out["meta"]);
    assert_eq!(sent_caps(&server).await, vec![4, 10_000]);
    server.verify().await;
}

/// With `--yes` the climb goes on until a result is complete; each answer
/// replaces the last (nothing summed), and a complete one covers the window.
#[tokio::test(flavor = "multi_thread")]
async fn traffic_backfill_with_yes_climbs_until_complete() {
    let server = MockServer::start().await;
    mount_objects(&server).await;
    mount_cap(&server, 4, 1, 4, rows(4, 1000)).await;
    mount_cap(&server, 10_000, 1, 10_000, rows(5, 2000)).await;
    mount_cap(&server, 25_000, 1, 3, rows(3, 3000)).await;
    mount_cap(&server, 50_000, 0, 3, rows(3, 4000)).await;
    let out = stdout_json(
        cmd(&server)
            .args(BACKFILL_ARGS)
            .args(["--backfill", "--yes"])
            .assert()
            .success(),
    );
    assert_eq!(ports(&out), vec![3000, 3001, 3002]);
    assert_eq!(out["meta"]["cap_steps"], json!([4, 10_000, 25_000]));
    assert_eq!(out["meta"]["truncated"], false);
    assert_eq!(out["meta"]["incomplete"], false);
    assert_eq!(out["meta"]["covered"], "2h");
    assert!(out["meta"].get("slices").is_none(), "{}", out["meta"]);
    // Name index (4 lists) + 3 queries of POST, status, download.
    assert_eq!(out["meta"]["api_calls"], 4 + 3 * 3);
    server.verify().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn traffic_backfill_on_a_complete_result_sends_nothing_more() {
    let server = MockServer::start().await;
    mount_objects(&server).await;
    mount_cap(&server, 4, 1, 3, rows(3, 1000)).await;
    mount_cap(&server, 10_000, 0, 3, rows(3, 2000)).await;
    let out = stdout_json(
        cmd(&server)
            .args(BACKFILL_ARGS)
            .arg("--backfill")
            .assert()
            .success(),
    );
    assert_eq!(out["meta"]["cap_steps"], json!([4]));
    assert_eq!(out["meta"]["covered"], "2h");
    server.verify().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn max_slices_was_removed_and_exits_2_before_any_request() {
    let server = MockServer::start().await;
    for extra in [
        &["--max-slices", "3"][..],
        &["--backfill", "--max-slices", "3"],
        &["--max-slices"],
    ] {
        cmd(&server)
            .args(BACKFILL_ARGS)
            .args(extra)
            .assert()
            .code(2)
            .stderr(predicate::str::contains(
                "time slices were removed: --backfill raises the cap",
            ));
    }
    assert!(server.received_requests().await.unwrap().is_empty());
}

/// A truncated result claims no span: `covered` is null and the table
/// footer names the cap.
#[tokio::test(flavor = "multi_thread")]
async fn traffic_truncated_reports_no_coverage_and_names_the_cap() {
    let server = MockServer::start().await;
    mount_objects(&server).await;
    mount_cap(&server, 4, 2, 4, rows(4, 1000)).await;
    let out = stdout_json(cmd(&server).args(BACKFILL_ARGS).assert().success());
    assert_eq!(out["data"].as_array().unwrap().len(), 4);
    assert!(out["meta"]["covered"].is_null(), "{}", out["meta"]);
    assert_eq!(out["meta"]["truncated"], true);
    assert!(out["meta"].get("cap_steps").is_none(), "{}", out["meta"]);
    let a = cmd(&server)
        .args(BACKFILL_ARGS)
        .arg("--table")
        .assert()
        .success();
    let err = String::from_utf8_lossy(&a.get_output().stderr).to_string();
    assert!(
        err.contains("WARNING: result truncated, capped at 4; --backfill raises the cap"),
        "{err}"
    );
    assert!(!err.contains("covers"), "{err}");
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

#[tokio::test(flavor = "multi_thread")]
async fn traffic_dry_run_shows_every_exclude_list() {
    let server = MockServer::start().await;
    mount_traffic(&server, 0).await;
    let out = stdout_json(
        cmd(&server)
            .args([
                "traffic",
                "query",
                "--window",
                "1h",
                "--dst",
                "env=prod",
                "--exclude-src",
                "app=shop",
                "--exclude-dst",
                "fqdn:x.example",
                "--exclude-service",
                "22/tcp",
                "--exclude-service",
                "udp",
                "--dry-run",
            ])
            .assert()
            .success(),
    );
    assert_eq!(
        out["data"]["sources"]["exclude"],
        json!([{"label": {"href": "/orgs/5/labels/3"}}])
    );
    assert_eq!(out["data"]["sources"]["include"], json!([[]]));
    assert_eq!(
        out["data"]["destinations"]["exclude"],
        json!([{"fqdn": "x.example"}])
    );
    assert_eq!(
        out["data"]["services"]["exclude"],
        json!([{"port": 22, "proto": 6}, {"proto": 17}])
    );
    assert_eq!(out["data"]["services"]["include"], json!([]));
    // Only a service exclude: no object list is needed.
    let quiet = MockServer::start().await;
    mount_traffic_quiet(&quiet).await;
    let out = stdout_json(
        cmd(&quiet)
            .args([
                "traffic",
                "groups",
                "--exclude-service",
                "443/tcp",
                "--dry-run",
            ])
            .assert()
            .success(),
    );
    assert_eq!(
        out["data"]["services"]["exclude"],
        json!([{"port": 443, "proto": 6}])
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn exclude_service_by_name_reads_only_the_cached_service_list() {
    let server = MockServer::start().await;
    mount_traffic(&server, 0).await;
    let out = stdout_json(
        cmd(&server)
            .args([
                "traffic",
                "query",
                "--no-cache",
                "--exclude-service",
                "svc:postgresql",
                "--exclude-service",
                "udp",
                "--dry-run",
            ])
            .assert()
            .success(),
    );
    assert_eq!(
        out["data"]["services"]["exclude"],
        json!([{"port": 5432, "proto": 6}, {"proto": 17}])
    );
    assert_eq!(out["meta"]["api_calls"], 1, "the service list only");
    let out = stdout_json(
        cmd(&server)
            .args([
                "traffic",
                "query",
                "--no-cache",
                "--dst",
                "env=prod",
                "--exclude-service",
                "svc:PostgreSQL",
                "--dry-run",
            ])
            .assert()
            .success(),
    );
    assert_eq!(
        out["data"]["services"]["exclude"],
        json!([{"port": 5432, "proto": 6}])
    );
    assert_eq!(
        out["meta"]["api_calls"], 4,
        "the index the actors load anyway"
    );
    let out = stdout_json(
        cmd(&server)
            .args([
                "traffic",
                "query",
                "--exclude-service",
                "svc:Web",
                "--exclude-service",
                "9000-9100/udp",
                "--dry-run",
            ])
            .assert()
            .success(),
    );
    assert_eq!(
        out["data"]["services"]["exclude"],
        json!([
            {"port": 80, "proto": 6},
            {"port": 443, "proto": 6},
            {"port": 8000, "to_port": 8099, "proto": 6},
            {"port": 9000, "to_port": 9100, "proto": 17}
        ]),
        "a service's range stays one range"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn port_takes_a_range_with_tcp_or_udp() {
    let server = MockServer::start().await;
    mount_traffic(&server, 0).await;
    let out = stdout_json(
        cmd(&server)
            .args([
                "traffic",
                "query",
                "--port",
                "8000-8100",
                "--proto",
                "tcp",
                "--dry-run",
            ])
            .assert()
            .success(),
    );
    assert_eq!(
        out["data"]["services"]["include"],
        json!([{"port": 8000, "to_port": 8100, "proto": 6}])
    );
    for (args, why) in [
        (&["--port", "8000-8100"][..], "needs tcp or udp"),
        (
            &["--port", "8000-8100", "--proto", "icmp"],
            "needs tcp or udp",
        ),
        (&["--port", "100-10", "--proto", "udp"], "lower to a higher"),
        (&["--port", "tcp"], "--port takes a port or a range"),
        (&["--port", "99999"], "PORT/PROTO"),
        (&["--exclude-service", "0-10/tcp"], "starts at 0"),
    ] {
        let assert = cmd(&server)
            .args(["traffic", "query", "--dry-run"])
            .args(args)
            .assert()
            .code(2);
        let err: Value = serde_json::from_slice(&assert.get_output().stderr).unwrap();
        let msg = err["error"]["message"].as_str().unwrap();
        assert!(msg.contains(why), "{args:?}: {msg}");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn traffic_rejects_what_explorer_cannot_exclude_before_any_request() {
    let quiet = MockServer::start().await;
    mount_traffic_quiet(&quiet).await;
    let assert = cmd(&quiet)
        .args(["traffic", "query", "--exclude-service", "ssh", "--dry-run"])
        .assert()
        .code(2);
    let err: Value = serde_json::from_slice(&assert.get_output().stderr).unwrap();
    let msg = err["error"]["message"].as_str().unwrap();
    assert!(
        msg.contains("--exclude-service") && msg.contains("PORT/PROTO"),
        "{msg}"
    );
    for flag in ["--exclude-src", "--src"] {
        let assert = cmd(&quiet)
            .args(["traffic", "query", flag, "fqdn:x.example", "--dry-run"])
            .assert()
            .failure();
        let err: Value = serde_json::from_slice(&assert.get_output().stderr).unwrap();
        let msg = err["error"]["message"].as_str().unwrap();
        assert!(msg.contains("destination filter only"), "{flag}: {msg}");
    }
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

#[tokio::test(flavor = "multi_thread")]
async fn events_diff_reads_the_index_once_and_names_hrefs() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v2/orgs/5/events"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(include_str!("../../../fixtures/events_changes.json"))
                .insert_header("Content-Type", "application/json"),
        )
        .expect(4)
        .mount(&server)
        .await;
    // Only the three `--diff` runs read the four lists.
    mount_objects_expecting(&server, Some(3)).await;
    let plain = stdout_json(cmd(&server).args(["events"]).assert().success());
    assert_eq!(plain["meta"]["api_calls"], 1, "plain events: no index");
    assert!(plain["data"][0].get("changes").is_none());
    let out = stdout_json(cmd(&server).args(["events", "--diff"]).assert().success());
    assert_eq!(out["meta"]["api_calls"], 5, "events plus the four lists");
    let fields = &out["data"][0]["changes"][0]["fields"];
    assert_eq!(fields[1]["field"], "providers");
    assert_eq!(fields[1]["added"], serde_json::json!(["role=db"]));
    cmd(&server)
        .args(["events", "--diff", "--table"])
        .assert()
        .success()
        .stdout(
            predicate::str::contains("    + labels: role=db").and(predicate::str::contains(
                "        src_ip       198.51.100.7",
            )),
        );
    // The type filter works with `--diff`: the hidden rows are not read.
    let out = stdout_json(
        cmd(&server)
            .args(["events", "--diff", "--exclude-type", "sec_rule.*"])
            .assert()
            .success(),
    );
    let types: Vec<&str> = out["data"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["event_type"].as_str().unwrap())
        .collect();
    assert_eq!(
        types,
        [
            "rule_set.create",
            "workload.update",
            "request.authorization_failed"
        ]
    );
    assert!(out["data"][0]["changes"].is_array());
    assert_eq!(out["meta"]["hidden"], 2);
}

#[tokio::test(flavor = "multi_thread")]
async fn events_type_terms_excludes_profile_defaults_and_severity_floor() {
    use wiremock::matchers::query_param_is_missing;
    let server = MockServer::start().await;
    // `warning+` cannot be asked of the PCE: no `severity` is sent.
    Mock::given(method("GET"))
        .and(path("/api/v2/orgs/5/events"))
        .and(query_param_is_missing("severity"))
        .and(query_param_is_missing("event_type"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(include_str!("../../../fixtures/events.json"))
                .insert_header("Content-Type", "application/json"),
        )
        .expect(6)
        .mount(&server)
        .await;
    let types = |out: &Value| -> Vec<String> {
        out["data"]
            .as_array()
            .unwrap()
            .iter()
            .map(|e| e["event_type"].as_str().unwrap().to_string())
            .collect()
    };
    let out = stdout_json(
        cmd(&server)
            .args(["events", "--type", "agent, sec_policy.*", "--type", "user"])
            .assert()
            .success(),
    );
    assert_eq!(
        types(&out),
        ["user.login", "sec_policy.create", "agent.tampering"]
    );
    assert_eq!(out["meta"]["hidden"], 1);
    assert_eq!(
        out["meta"]["local_filter"],
        json!(["type agent, sec_policy.*, user"])
    );
    let out = stdout_json(
        cmd(&server)
            .args(["events", "--severity", "warning+"])
            .assert()
            .success(),
    );
    assert_eq!(types(&out), ["agent.tampering", "system.license_expiring"]);
    assert_eq!(out["meta"]["hidden"], 2);
    // The profile's excludes, and --all-types without them.
    let dir = tempfile::tempdir().unwrap();
    let cfg = dir.path().join("config.toml");
    std::fs::write(
        &cfg,
        "[profiles.default]\nhost = \"ignored.example\"\norg_id = 5\n\n[profiles.default.events]\nexclude_types = [\"user.login\", \"system.*\"]\n",
    )
    .unwrap();
    let config = cfg.to_str().unwrap();
    let out = stdout_json(
        cmd(&server)
            .args(["--config", config, "events"])
            .assert()
            .success(),
    );
    assert_eq!(types(&out), ["sec_policy.create", "agent.tampering"]);
    assert_eq!(out["meta"]["hidden"], 2);
    let out = stdout_json(
        cmd(&server)
            .args(["--config", config, "events", "--all-types"])
            .assert()
            .success(),
    );
    assert_eq!(types(&out).len(), 4);
    assert!(out["meta"].get("hidden").is_none(), "no local filter");
    cmd(&server)
        .args(["--config", config, "events", "--table"])
        .assert()
        .success()
        .stderr(predicate::str::contains(
            "filtered locally: type !user.login, !system.* · 2 hidden",
        ));
    cmd(&server)
        .args([
            "--config",
            config,
            "events",
            "--type",
            "agent",
            "--exclude-type",
            "agent.tampering",
        ])
        .assert()
        .success()
        .stdout(
            predicate::str::contains("\"data\": []").or(predicate::str::contains("\"data\":[]")),
        );
    // Refused before any request.
    for bad in [["--type", "!"], ["--severity", "loud+"]] {
        cmd(&server)
            .args(["events", bad[0], bad[1]])
            .assert()
            .code(2);
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn events_actor_terms_filter_locally_in_every_format_and_from_a_saved_filter() {
    use wiremock::matchers::query_param_is_missing;
    let server = MockServer::start().await;
    // The actor is never asked of the PCE.
    Mock::given(method("GET"))
        .and(path("/api/v2/orgs/5/events"))
        .and(query_param_is_missing("created_by"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(include_str!("../../../fixtures/events.json"))
                .insert_header("Content-Type", "application/json"),
        )
        .expect(6)
        .mount(&server)
        .await;
    let types = |out: &Value| -> Vec<String> {
        out["data"]
            .as_array()
            .unwrap()
            .iter()
            .map(|e| e["event_type"].as_str().unwrap().to_string())
            .collect()
    };
    let out = stdout_json(
        cmd(&server)
            .args(["events", "--actor", "user:*", "--exclude-actor", "=user-2"])
            .assert()
            .success(),
    );
    assert_eq!(types(&out), ["user.login"]);
    assert_eq!(out["meta"]["hidden"], 3);
    assert_eq!(
        out["meta"]["local_filter"],
        json!(["actor user:*, !=user-2"])
    );
    // Repeatable, ORed; `!` tolerated in --exclude-actor; with --type.
    let out = stdout_json(
        cmd(&server)
            .args([
                "events",
                "--actor",
                "system",
                "--actor",
                "agent:*",
                "--exclude-actor",
                "!host-1*",
                "--type",
                "system",
            ])
            .assert()
            .success(),
    );
    assert_eq!(types(&out), ["system.license_expiring"]);
    cmd(&server)
        .args(["--csv", "events", "--exclude-actor", "system,agent:*"])
        .assert()
        .success()
        .stdout(predicate::str::contains("user:user-1"))
        .stdout(predicate::str::contains("system.license_expiring").not())
        .stdout(predicate::str::contains("host-1.example").not());
    cmd(&server)
        .args(["--table", "events", "--actor", "=host-1.example"])
        .assert()
        .success()
        .stdout(predicate::str::contains("agent:host-1.example"))
        .stdout(predicate::str::contains("user:user-1").not())
        .stderr(predicate::str::contains(
            "filtered locally: actor =host-1.example · 3 hidden",
        ));
    // A saved filter's actor, and a flag replacing it.
    let dir = tempfile::tempdir().unwrap();
    let cfg = dir.path().join("config.toml");
    std::fs::write(
        &cfg,
        "[profiles.default]\nhost = \"ignored.example\"\norg_id = 5\n\n[profiles.default.filters.events.people]\nactor = \"user:*\"\n",
    )
    .unwrap();
    let config = cfg.to_str().unwrap();
    let out = stdout_json(
        cmd(&server)
            .args(["--config", config, "events", "--filter", "people"])
            .assert()
            .success(),
    );
    assert_eq!(types(&out), ["user.login", "sec_policy.create"]);
    let out = stdout_json(
        cmd(&server)
            .args([
                "--config", config, "events", "--filter", "people", "--actor", "system",
            ])
            .assert()
            .success(),
    );
    assert_eq!(types(&out), ["system.license_expiring"], "the flag wins");
    // Refused before any request.
    for bad in [["--actor", "!"], ["--exclude-actor", "a!b"]] {
        cmd(&server)
            .args(["events", bad[0], bad[1]])
            .assert()
            .code(2)
            .stderr(predicate::str::contains("--actor"));
    }
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
    assert_eq!(data[0]["allowed_conns"], 310, "connections per decision");
    assert_eq!(data[0]["pot_blocked_conns"], 0);
    assert_eq!(data[0]["blocked_conns"], 0);
    assert_eq!(out["meta"]["cache"], "miss");
    assert_eq!(out["meta"]["truncated"], false);
    let table = cmd(&server)
        .args([
            "traffic", "groups", "--window", "1h", "--keys", "role", "--table",
        ])
        .assert()
        .success();
    let text = String::from_utf8_lossy(&table.get_output().stdout).to_string();
    assert_eq!(
        text.lines().next(),
        Some("flows: flow records · conns, bytes: lifetime counts of those records"),
        "the unit above the table"
    );
    let header = text.lines().nth(1).unwrap();
    assert!(
        !header.contains("_conns"),
        "the table keeps its width: {header}"
    );
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
    assert_eq!(
        (
            &nas["allowed_conns"],
            &nas["pot_blocked_conns"],
            &nas["blocked_conns"]
        ),
        (&json!(0), &json!(0), &nas["connections"]),
        "per-decision connections"
    );
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
    let header: Vec<&str> = text.lines().nth(1).unwrap().split_whitespace().collect();
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
            "src,dst,service,flows,conns,bytes,allowed,pot.blocked,blocked,\
             allowed_conns,pot_blocked_conns,blocked_conns,unknown_conns",
            "shop/prod,shop/prod,5432/tcp,1,310,124000,1,0,0,310,0,0,0"
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
        Some(
            "src,dst,service,flows,conns,bytes,allowed,pot.blocked,blocked,\
             allowed_conns,pot_blocked_conns,blocked_conns,unknown_conns"
        )
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
    // The unit line, then the column header.
    let header = text.lines().nth(1).unwrap();
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

#[tokio::test(flavor = "multi_thread")]
async fn transmission_excludes_the_other_kinds_on_the_destination_side() {
    let server = MockServer::start().await;
    mount_traffic(&server, 1).await;
    for sub in ["query", "groups", "mesh"] {
        let out = stdout_json(
            cmd(&server)
                .args([
                    "traffic",
                    sub,
                    "--window",
                    "1h",
                    "--transmission",
                    "unicast",
                    "--dry-run",
                ])
                .assert()
                .success(),
        );
        assert_eq!(
            out["data"]["destinations"]["exclude"],
            json!([{"transmission": "broadcast"}, {"transmission": "multicast"}]),
            "{sub}"
        );
        assert_eq!(out["data"]["sources"]["exclude"], json!([]), "{sub}");
    }
    let out = stdout_json(
        cmd(&server)
            .args([
                "traffic",
                "query",
                "--window",
                "1h",
                "--transmission",
                "broadcast",
                "--transmission",
                "multicast",
                "--dry-run",
            ])
            .assert()
            .success(),
    );
    assert_eq!(
        out["data"]["destinations"]["exclude"],
        json!([{"transmission": "unicast"}]),
        "repeatable"
    );
    let out = stdout_json(
        cmd(&server)
            .args(["traffic", "query", "--window", "1h", "--dry-run"])
            .assert()
            .success(),
    );
    assert_eq!(
        out["data"]["destinations"]["exclude"],
        json!([]),
        "the default is every transmission"
    );
    cmd(&server)
        .args(["traffic", "query", "--transmission", "anycast", "--dry-run"])
        .assert()
        .code(2);
    // A real run sends the same filter (one query, counted by the mock).
    cmd(&server)
        .args([
            "traffic",
            "groups",
            "--window",
            "1h",
            "--transmission",
            "unicast",
        ])
        .assert()
        .success();
    let posts: Vec<Value> = server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .filter(|r| r.method.as_str() == "POST")
        .map(|r| serde_json::from_slice(&r.body).unwrap())
        .collect();
    assert_eq!(posts.len(), 1);
    assert_eq!(
        posts[0]["destinations"]["exclude"],
        json!([{"transmission": "broadcast"}, {"transmission": "multicast"}])
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn transmission_joins_other_destination_excludes_on_both_focused_sides() {
    let server = MockServer::start().await;
    mount_traffic(&server, 0).await;
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
                "both",
                "--exclude-dst",
                "ip:10.0.0.1",
                "--transmission",
                "unicast",
                "--dry-run",
            ])
            .assert()
            .success(),
    );
    let bodies = out["data"].as_array().expect("one body per side");
    assert_eq!(bodies.len(), 2);
    for b in bodies {
        assert_eq!(
            b["destinations"]["exclude"],
            json!([
                {"ip_address": "10.0.0.1"},
                {"transmission": "broadcast"},
                {"transmission": "multicast"}
            ]),
            "{b}"
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn app_transmission_goes_into_every_focused_query() {
    let server = MockServer::start().await;
    mount_app(&server, 2).await;
    cmd(&server)
        .args([
            "app",
            "shop/prod",
            "--window",
            "1h",
            "--transmission",
            "unicast",
        ])
        .assert()
        .success();
    let posts: Vec<Value> = server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .filter(|r| r.method.as_str() == "POST")
        .map(|r| serde_json::from_slice(&r.body).unwrap())
        .collect();
    assert_eq!(posts.len(), 2, "in and out");
    for b in &posts {
        assert_eq!(
            b["destinations"]["exclude"],
            json!([{"transmission": "broadcast"}, {"transmission": "multicast"}]),
            "{b}"
        );
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
        "hostname,name,ip,labels,enforcement,online,agent,sync,heartbeat_age_secs,health,href,enforcement_mode"
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
                  flows,conns,bytes,last seen,decision,allowed_conns,pot_blocked_conns,blocked_conns,unknown_conns";
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
             2026-10-10T07:31:02Z,allowed,310,0,0,0"
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
             2026-10-10T07:30:58Z,potentially_blocked,0,12,0,0"
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
    let answer = |body: &str| {
        ResponseTemplate::new(200)
            .set_body_string(body)
            .insert_header("Content-Type", "application/json")
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
    // A label-filtered workloads read is cut only when it fills the row cap
    // (X-Total-Count is the whole collection's size); the unfiltered
    // rulesets read is cut when the total exceeds the rows.
    let mut fixture: Vec<Value> =
        serde_json::from_str(include_str!("../../../fixtures/workloads.json")).unwrap();
    let one = fixture.swap_remove(0);
    let full: Vec<Value> = (0..10_000)
        .map(|i| {
            let mut w = one.clone();
            w["href"] = format!("/orgs/5/workloads/w{i}").into();
            w
        })
        .collect();
    mount_traffic(&server, 4).await;
    Mock::given(method("GET"))
        .and(path("/api/v2/orgs/5/workloads"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(full)
                .insert_header("X-Total-Count", "50000"),
        )
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v2/orgs/5/sec_policy/draft/rule_sets"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(include_str!("../../../fixtures/rule_sets.json"))
                .insert_header("Content-Type", "application/json")
                .insert_header("X-Total-Count", "50000"),
        )
        .mount(&server)
        .await;
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

/// The fixture's workloads, with the unmanaged one in shop/prod and
/// reporting `visibility_only`, as the PCE does for workloads without a VEN.
fn workloads_with_unmanaged_in_shop() -> String {
    let mut all: Value =
        serde_json::from_str(include_str!("../../../fixtures/workloads.json")).unwrap();
    all[2]["enforcement_mode"] = "visibility_only".into();
    all[2]["labels"] = all[0]["labels"].clone();
    all.to_string()
}

#[tokio::test(flavor = "multi_thread")]
async fn app_counts_a_workload_without_a_ven_as_unmanaged_not_visibility() {
    let server = MockServer::start().await;
    // Three runs, each two focused traffic queries (in and out).
    mount_traffic(&server, 6).await;
    Mock::given(method("GET"))
        .and(path("/api/v2/orgs/5/workloads"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(workloads_with_unmanaged_in_shop())
                .insert_header("Content-Type", "application/json"),
        )
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v2/orgs/5/sec_policy/draft/rule_sets"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(include_str!("../../../fixtures/rule_sets.json"))
                .insert_header("Content-Type", "application/json"),
        )
        .mount(&server)
        .await;
    let out = stdout_json(
        cmd(&server)
            .args([
                "app",
                "shop/prod",
                "--window",
                "1h",
                "--section",
                "workloads",
            ])
            .assert()
            .success(),
    );
    let w = &out["data"]["workloads"];
    assert_eq!(w["summary"]["workloads"], 3);
    assert_eq!(w["summary"]["enforcement"]["visibility"], 1, "{w}");
    assert_eq!(w["summary"]["enforcement"]["unmanaged"], 1, "{w}");
    let nas = w["rows"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["name"] == "unmanaged-nas")
        .unwrap();
    assert_eq!(nas["enforcement"], "unmanaged");
    assert_eq!(nas["enforcement_mode"], "visibility_only", "raw API value");
    let table = cmd(&server)
        .args(["app", "shop/prod", "--window", "1h", "--table"])
        .assert()
        .success();
    let text = String::from_utf8_lossy(&table.get_output().stdout).to_string();
    assert!(
        text.contains("enforced 1 · visibility 1 · unmanaged 1"),
        "{text}"
    );
    let csv = csv_stdout(
        &cmd(&server)
            .args([
                "app",
                "shop/prod",
                "--window",
                "1h",
                "--section",
                "workloads",
                "--csv",
            ])
            .assert()
            .success(),
    );
    let mut lines = csv.lines();
    assert_eq!(
        lines.next(),
        Some("href,name,role,enforcement,enforcement_mode,ven")
    );
    assert!(
        csv.contains("unmanaged-nas,web,unmanaged,visibility_only,unmanaged"),
        "{csv}"
    );
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
    assert!(d.get("timeline").is_none(), "the timeline was removed");
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
    ] {
        assert!(text.contains(s), "missing {s:?}:\n{text}");
    }
    assert!(!text.contains("timeline"), "{text}");
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
    // The timeline was removed: its flag and its section are unknown to clap.
    for args in [
        vec!["app", "shop/prod", "--section", "timeline"],
        vec!["app", "shop/prod", "--timeline"],
    ] {
        let err = cmd(&server).args(&args).assert().code(2);
        let err = String::from_utf8_lossy(&err.get_output().stderr).to_string();
        assert!(
            err.contains("invalid value 'timeline'")
                || err.contains("unexpected argument '--timeline'"),
            "{args:?}: {err}"
        );
    }
    let server = MockServer::start().await;
    mount_app(&server, 0).await;
    let err = cmd(&server).args(["app", "shp/prod"]).assert().code(2);
    assert!(String::from_utf8_lossy(&err.get_output().stderr).contains("closest: shop"));
}

// ---- `--draft`: the draft policy decision ----

/// The fixture's flows after `update_rules`: the potentially blocked
/// 8443/tcp flow is allowed in draft, the allowed 443/tcp one blocked.
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

/// `mount_traffic` for `--draft`: the status reports the rules completed,
/// `update_rules` expects `puts` PUTs, the download carries draft decisions.
async fn mount_traffic_draft(server: &MockServer, posts: u64, puts: u64) {
    Mock::given(method("GET"))
        .and(path(format!("/api/v2{QHREF}")))
        .respond_with(Ticking::new(json!({
            "status": "completed", "rules": "completed",
            "result": {"href": format!("{QHREF}/download")}, "flows_count": 4
        })))
        .mount(server)
        .await;
    Mock::given(method("PUT"))
        .and(path(format!("/api/v2{QHREF}/update_rules")))
        .and(query_param("label_based_rules", "false"))
        .and(query_param("offset", "0"))
        .respond_with(ResponseTemplate::new(204))
        .expect(puts)
        .mount(server)
        .await;
    mount_traffic_body(server, posts, draft_flows()).await;
}

/// The traffic requests the server saw, in order (object lists left out).
async fn traffic_sequence(server: &MockServer) -> Vec<String> {
    server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .map(|r| {
            let q = r.url.query().map(|q| format!("?{q}")).unwrap_or_default();
            format!(
                "{} {}{q}",
                r.method,
                r.url.path().trim_start_matches("/api/v2")
            )
        })
        .filter(|s| s.contains("traffic_flows"))
        .collect()
}

#[tokio::test(flavor = "multi_thread")]
async fn traffic_query_draft_runs_in_one_go_and_reports_what_the_draft_changes() {
    let server = MockServer::start().await;
    mount_traffic_draft(&server, 1, 1).await;
    let out = stdout_json(
        cmd(&server)
            .args([
                "traffic",
                "query",
                "--window",
                "1h",
                "--max-results",
                "200",
                "--draft",
            ])
            .assert()
            .success(),
    );
    assert_eq!(
        traffic_sequence(&server).await,
        vec![
            "POST /orgs/5/traffic_flows/async_queries".to_string(),
            format!("GET {QHREF}"),
            format!("PUT {QHREF}/update_rules?label_based_rules=false&offset=0&limit=200"),
            format!("GET {QHREF}"),
            format!("GET {QHREF}/download"),
        ],
        "the PUT's limit is the query's cap; one download"
    );
    let flows = out["data"].as_array().unwrap();
    let draft: Vec<&str> = flows
        .iter()
        .map(|f| f["draft_policy_decision"].as_str().unwrap())
        .collect();
    assert_eq!(draft, ["allowed", "allowed", "blocked", "blocked"]);
    assert_eq!(
        out["meta"]["draft"],
        json!({"evaluated": true, "fixed": 1, "newly_blocked": 1})
    );
    // 4 object lists for the index, plus the 5 traffic requests.
    assert_eq!(out["meta"]["api_calls"], 9);
}

#[tokio::test(flavor = "multi_thread")]
async fn without_draft_there_is_no_put_and_no_meta_draft() {
    let server = MockServer::start().await;
    mount_traffic_draft(&server, 1, 0).await;
    let out = stdout_json(
        cmd(&server)
            .args(["traffic", "query", "--window", "1h"])
            .assert()
            .success(),
    );
    assert!(out["meta"].get("draft").is_none(), "{}", out["meta"]);
    assert!(
        out["data"]
            .as_array()
            .unwrap()
            .iter()
            .all(|f| f["draft_policy_decision"].is_null()),
        "a draft decision left on a reported download is not shown"
    );
    assert!(
        !traffic_sequence(&server)
            .await
            .iter()
            .any(|s| s.starts_with("PUT"))
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn traffic_draft_tables_and_csv_carry_the_draft_columns() {
    let server = MockServer::start().await;
    mount_traffic_draft(&server, 3, 3).await;
    let text = csv_stdout(
        &cmd(&server)
            .args(["traffic", "query", "--window", "1h", "--draft", "--csv"])
            .assert()
            .success(),
    );
    assert_eq!(
        text.lines().next(),
        Some("last seen,src,src labels,dst,dst labels,service,conns,decision,draft,process")
    );
    assert!(text.contains(",potentially_blocked,allowed,"), "{text}");
    let text = csv_stdout(
        &cmd(&server)
            .args(["traffic", "groups", "--window", "1h", "--draft", "--csv"])
            .assert()
            .success(),
    );
    let header = text.lines().next().unwrap();
    assert!(
        header.ends_with("unknown_conns,draft_allowed,draft_blocked"),
        "{header}"
    );
    let out = stdout_json(
        cmd(&server)
            .args([
                "traffic", "groups", "--window", "1h", "--by", "in", "--draft",
            ])
            .assert()
            .success(),
    );
    let rows: Vec<&Value> = out["data"]["sections"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|s| s["rows"].as_array().unwrap())
        .collect();
    let allowed: u64 = rows
        .iter()
        .map(|r| r["draft_allowed"].as_u64().unwrap())
        .sum();
    let blocked: u64 = rows
        .iter()
        .map(|r| r["draft_blocked"].as_u64().unwrap())
        .sum();
    assert_eq!((allowed, blocked), (1, 1));
}

#[tokio::test(flavor = "multi_thread")]
async fn app_draft_re_evaluates_each_side_and_marks_the_peers() {
    let server = MockServer::start().await;
    mount_traffic_draft(&server, 2, 2).await;
    Mock::given(method("GET"))
        .and(path("/api/v2/orgs/5/workloads"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(include_str!("../../../fixtures/workloads.json"))
                .insert_header("Content-Type", "application/json"),
        )
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v2/orgs/5/sec_policy/draft/rule_sets"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(include_str!("../../../fixtures/rule_sets.json"))
                .insert_header("Content-Type", "application/json"),
        )
        .mount(&server)
        .await;
    let out = stdout_json(
        cmd(&server)
            .args(["app", "shop/prod", "--window", "1h", "--draft"])
            .assert()
            .success(),
    );
    let puts = traffic_sequence(&server)
        .await
        .iter()
        .filter(|s| s.starts_with("PUT"))
        .count();
    assert_eq!(puts, 2, "one per side");
    assert_eq!(out["meta"]["draft"]["evaluated"], true);
    let peers: Vec<&Value> = out["data"]["inbound"]
        .as_array()
        .unwrap()
        .iter()
        .chain(out["data"]["outbound"].as_array().unwrap())
        .collect();
    assert!(
        peers.iter().all(|p| p["draft_allowed"].is_u64()),
        "{peers:?}"
    );
}

/// A status answer whose `updated_at` moves on every call, as the PCE's
/// does while it works: a draft `completed` after the PUT is then new.
struct Ticking {
    body: serde_json::Value,
    n: std::sync::atomic::AtomicU64,
}

impl Ticking {
    fn new(body: serde_json::Value) -> Self {
        Self {
            body,
            n: std::sync::atomic::AtomicU64::new(0),
        }
    }
}

impl wiremock::Respond for Ticking {
    fn respond(&self, _: &wiremock::Request) -> ResponseTemplate {
        let n = self.n.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let mut body = self.body.clone();
        body["updated_at"] = json!(format!("2026-10-14T00:00:{:02}Z", n % 60));
        ResponseTemplate::new(200).set_body_json(body)
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn traffic_mesh_draft_counts_per_row_in_json_and_csv() {
    let server = MockServer::start().await;
    mount_traffic_draft(&server, 2, 2).await;
    let out = stdout_json(
        cmd(&server)
            .args(["traffic", "mesh", "--window", "1h", "--draft"])
            .assert()
            .success(),
    );
    let rows = out["data"].as_array().unwrap();
    let sum = |k: &str| -> u64 { rows.iter().map(|r| r[k].as_u64().unwrap()).sum() };
    assert_eq!((sum("draft_allowed"), sum("draft_blocked")), (1, 1));
    let text = csv_stdout(
        &cmd(&server)
            .args(["traffic", "mesh", "--window", "1h", "--draft", "--csv"])
            .assert()
            .success(),
    );
    let header = text.lines().next().unwrap();
    assert!(
        header.ends_with("decision,draft_allowed,draft_blocked"),
        "{header}"
    );
}

/// `--backfill --draft`: the caps run plain; only the result kept (cap
/// 10000) is re-evaluated, by its own href, with no new POST.
#[tokio::test(flavor = "multi_thread")]
async fn backfill_with_draft_re_evaluates_only_the_kept_rung_by_its_href() {
    let server = MockServer::start().await;
    mount_objects(&server).await;
    let kept = "/orgs/5/traffic_flows/async_queries/q-10000";
    Mock::given(method("GET"))
        .and(path(format!("/api/v2{kept}")))
        .respond_with(Ticking::new(json!({
            "status": "completed", "rules": "completed",
            "result": {"href": format!("{kept}/download")}, "flows_count": 3
        })))
        .mount(&server)
        .await;
    Mock::given(method("PUT"))
        .and(path(format!("/api/v2{kept}/update_rules")))
        .and(query_param("limit", "10000"))
        .respond_with(ResponseTemplate::new(204))
        .expect(1)
        .mount(&server)
        .await;
    let mut three: Vec<Value> = serde_json::from_str(&rows(3, 2000)).unwrap();
    for f in &mut three {
        f["draft_policy_decision"] = json!("blocked");
    }
    mount_cap(&server, 4, 1, 4, rows(4, 1000)).await;
    mount_cap(
        &server,
        10_000,
        1,
        3,
        serde_json::to_string(&three).unwrap(),
    )
    .await;
    let out = stdout_json(
        cmd(&server)
            .args(BACKFILL_ARGS)
            .args(["--backfill", "--draft"])
            .assert()
            .success(),
    );
    assert_eq!(out["meta"]["cap_steps"], json!([4, 10_000]));
    let puts: Vec<String> = server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .filter(|r| r.method.as_str() == "PUT")
        .map(|r| r.url.path().trim_start_matches("/api/v2").to_string())
        .collect();
    assert_eq!(
        puts,
        vec![format!("{kept}/update_rules")],
        "only the kept rung"
    );
    assert_eq!(
        sent_caps(&server).await,
        vec![4, 10_000],
        "no POST for the draft"
    );
    assert!(
        out["data"]
            .as_array()
            .unwrap()
            .iter()
            .all(|f| f["draft_policy_decision"] == "blocked")
    );
    assert_eq!(out["meta"]["draft"]["newly_blocked"], 3);
    server.verify().await;
}

/// The traffic (any POST answers the 4-flow fixture), the object lists and
/// the draft rulesets: what `gen` reads.
/// `runs` commands, `posts` traffic queries each.
async fn mount_gen(server: &MockServer, runs: u64, posts: u64) {
    mount_traffic(server, runs * posts).await;
    Mock::given(method("GET"))
        .and(path("/api/v2/orgs/5/sec_policy/draft/rule_sets"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(include_str!("../../../fixtures/rule_sets.json"))
                .insert_header("Content-Type", "application/json"),
        )
        .expect(runs)
        .mount(server)
        .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn gen_prints_rows_rules_and_the_preview_and_writes_nothing() {
    let server = MockServer::start().await;
    mount_gen(&server, 1, 1).await;
    let out = stdout_json(
        cmd(&server)
            .args(["gen", "shop/prod", "--window", "1h"])
            .assert()
            .success(),
    );
    let d = &out["data"];
    assert_eq!(d["app"], "shop/prod");
    assert_eq!(d["keys"], json!(["app", "env"]));
    assert_eq!(d["granularity"], "app-ports");
    assert_eq!(d["outbound"], false);
    assert_eq!(
        d["target"],
        json!({"name": "Shop-Prod", "href": "/orgs/5/sec_policy/draft/rule_sets/100", "action": "existing"})
    );
    let peers: Vec<&str> = d["rows"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["peer"].as_str().unwrap())
        .collect();
    assert_eq!(
        peers,
        vec!["intra-app", "unmanaged-nas", "IP list Corp · 1 IP"]
    );
    assert_eq!(d["rows"][0]["checked"], true);
    assert_eq!(d["rows"][0]["services"], json!(["5432/tcp"]));
    assert_eq!(
        d["rows"][1]["checkable"], false,
        "icmp only, no ICMP service"
    );
    assert_eq!(d["rows"][2]["kind"], "ip_list");
    assert_eq!(d["rows"][2]["ips"], json!(["10.1.4.9"]));
    assert_eq!(
        d["rows"][2]["ip_list"],
        json!({"name": "Corp", "href": "/orgs/5/sec_policy/draft/ip_lists/21"})
    );
    assert_eq!(d["rows"][2]["warnings"], json!(["uses IP list Corp"]));
    assert_eq!(d["rows"][2]["checked"], false, "IP rows start unchecked");
    assert!(d["rows"][0].get("ips").is_none(), "only on ip_list rows");
    assert!(d["rows"][0].get("ip_list").is_none());
    assert_eq!(d["rules"].as_array().unwrap().len(), 1);
    let r = &d["rules"][0];
    assert_eq!(r["ruleset"], "Shop-Prod");
    assert_eq!(r["action"], "create");
    assert_eq!(r["providers"], json!(["all workloads"]));
    assert_eq!(r["consumers"], json!(["all workloads"]));
    assert_eq!(r["unscoped_consumers"], false);
    assert_eq!(r["rows"], json!([1]));
    assert_eq!(r["body"]["providers"], json!([{"actors": "ams"}]));
    assert_eq!(
        r["body"]["ingress_services"],
        json!([{"port": 5432, "proto": 6}])
    );
    assert_eq!(d["preview"]["rules"], 1);
    assert_eq!(d["preview"]["create"], 1);
    assert_eq!(d["preview"]["covered_flows"], 1);
    assert_eq!(d["preview"]["uncovered_flows"], 2);
    assert_eq!(d["preview"]["uncovered_by"]["unchecked"], 1);
    assert_eq!(d["preview"]["uncovered_by"]["unsupported_proto"], 1);
    assert_eq!(
        d["preview"]["uncovered_by"],
        json!({"unchecked": 1, "folded_ips": 0, "unsupported_proto": 1,
            "broadcast_multicast": 0, "no_target": 0, "override_deny": 0}),
        "the reasons only"
    );
    assert!(d["preview"].get("blocker").is_none());
    assert_eq!(out["meta"]["queries"], 1, "the in side only");
    let sent = server.received_requests().await.unwrap();
    assert!(
        sent.iter()
            .all(|r| r.method.as_str() == "GET" || r.url.path().ends_with("/async_queries")),
        "only the traffic query is a POST"
    );
    server.verify().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn gen_granularity_role_ports_finds_the_existing_rule() {
    let server = MockServer::start().await;
    mount_gen(&server, 3, 1).await;
    let out = stdout_json(
        cmd(&server)
            .args([
                "gen",
                "shop/prod",
                "--window",
                "1h",
                "--granularity",
                "role-ports",
            ])
            .assert()
            .success(),
    );
    let r = &out["data"]["rules"][0];
    assert_eq!(r["providers"], json!(["role=db"]));
    assert_eq!(r["consumers"], json!(["role=web"]));
    assert_eq!(r["action"], "exists");
    assert_eq!(
        r["existing_rule"],
        "/orgs/5/sec_policy/draft/rule_sets/100/sec_rules/1"
    );
    assert!(r["body"].is_null());
    assert_eq!(out["data"]["preview"]["exists"], 1);
    for g in ["app-all", "role-all"] {
        let out = stdout_json(
            cmd(&server)
                .args(["gen", "shop/prod", "--window", "1h", "--granularity", g])
                .assert()
                .success(),
        );
        assert_eq!(
            out["data"]["rules"][0]["services"],
            json!(["all services"]),
            "{g}"
        );
        assert_eq!(
            out["data"]["rows"][0]["services"],
            json!(["all services"]),
            "{g}"
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn gen_outbound_adds_the_out_query_and_its_rows() {
    let server = MockServer::start().await;
    mount_gen(&server, 1, 2).await;
    let out = stdout_json(
        cmd(&server)
            .args(["gen", "shop/prod", "--window", "1h", "--outbound"])
            .assert()
            .success(),
    );
    assert_eq!(out["meta"]["queries"], 2);
    let rows = out["data"]["rows"].as_array().unwrap();
    let last = rows.last().unwrap();
    assert_eq!(last["side"], "outbound");
    assert_eq!(last["peer"], "93.184.216.34");
    assert_eq!(last["warnings"], json!(["needs an IP list"]));
    assert_eq!(last["checked"], false);
}

#[tokio::test(flavor = "multi_thread")]
async fn gen_refuses_capped_or_incomplete_traffic_and_unknown_apps() {
    let server = MockServer::start().await;
    // One capped query; the unknown apps are refused before any traffic.
    mount_traffic(&server, 1).await;
    let a = cmd(&server)
        .args(["gen", "shop/prod", "--window", "1h", "--max-results", "4"])
        .assert()
        .code(2);
    assert!(a.get_output().stdout.is_empty(), "no rules printed");
    let err = String::from_utf8_lossy(&a.get_output().stderr).to_string();
    assert!(
        err.contains("traffic capped at 4 for shop/prod: raise --max-results or narrow --window; no rules proposed"),
        "{err}"
    );
    let a = cmd(&server)
        .args(["gen", "shpo/prod", "--window", "1h"])
        .assert()
        .code(2);
    let err = String::from_utf8_lossy(&a.get_output().stderr).to_string();
    assert!(err.contains("closest: shop"), "{err}");
    let a = cmd(&server)
        .args(["gen", "shop", "--window", "1h"])
        .assert()
        .code(2);
    let err = String::from_utf8_lossy(&a.get_output().stderr).to_string();
    assert!(err.contains("write it as <app>/<env>"), "{err}");

    let server = MockServer::start().await;
    mount_objects(&server).await;
    Mock::given(method("POST"))
        .and(path("/api/v2/orgs/5/traffic_flows/async_queries"))
        .respond_with(
            ResponseTemplate::new(202).set_body_json(json!({"href": QHREF, "status": "queued"})),
        )
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("/api/v2{QHREF}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "status": "completed", "result": {"href": format!("{QHREF}/download")},
            "regions": [{"pce_fqdn": "a", "responded": true, "flows_count": 4},
                        {"pce_fqdn": "b", "responded": false}]
        })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("/api/v2{QHREF}/download")))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(include_str!("../../../fixtures/traffic_flows.json"))
                .insert_header("Content-Type", "application/json"),
        )
        .mount(&server)
        .await;
    let a = cmd(&server)
        .args(["gen", "shop/prod", "--window", "1h"])
        .assert()
        .code(4);
    assert!(a.get_output().stdout.is_empty());
    let err: Value = serde_json::from_slice(&a.get_output().stderr).unwrap();
    assert_eq!(err["error"]["kind"], "incomplete");
}

#[tokio::test(flavor = "multi_thread")]
async fn gen_table_and_csv_print_the_rule_columns() {
    let server = MockServer::start().await;
    mount_gen(&server, 2, 1).await;
    let a = cmd(&server)
        .args(["gen", "shop/prod", "--window", "1h", "--table"])
        .assert()
        .success();
    let text = String::from_utf8_lossy(&a.get_output().stdout).to_string();
    assert!(text.contains("consumers → providers"), "{text}");
    assert!(text.contains("all workloads → all workloads"), "{text}");
    let a = cmd(&server)
        .args(["gen", "shop/prod", "--window", "1h", "--csv"])
        .assert()
        .success();
    let text = String::from_utf8_lossy(&a.get_output().stdout).to_string();
    let mut lines = text.lines();
    assert_eq!(
        lines.next(),
        Some("ruleset,action,consumers,providers,services,unscoped,existing_rule,description")
    );
    let row = lines.next().unwrap();
    assert!(
        row.starts_with(
            "Shop-Prod,create,all workloads,all workloads,5432/tcp,false,,\"generated: shop/prod → shop/prod on 5432/tcp, from 1h of traffic"
        ),
        "{row}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn gen_without_the_cache_reads_the_name_index_once() {
    let server = MockServer::start().await;
    mount_gen(&server, 1, 1).await;
    cmd(&server)
        .args(["--no-cache", "gen", "shop/prod", "--window", "1h"])
        .assert()
        .success();
    let sent = server.received_requests().await.unwrap();
    let labels = sent
        .iter()
        .filter(|r| r.method.as_str() == "GET" && r.url.path() == "/api/v2/orgs/5/labels")
        .count();
    assert_eq!(
        labels, 1,
        "the index fetch resolved the flows with is reused"
    );
    server.verify().await;
}

/// A config whose generator calls 2 inbound peers on a port fan-in.
fn fan_config(dir: &tempfile::TempDir) -> std::path::PathBuf {
    let cfg = dir.path().join("config.toml");
    std::fs::write(
        &cfg,
        "[profiles.default]\nhost = \"ignored.example\"\norg_id = 5\n\n[profiles.default.generator]\nconsolidate_min_peers = 2\n",
    )
    .unwrap();
    cfg
}

/// `mount_gen` with a second IP peer (10.9.9.9, in no list) on Corp's
/// 8443/tcp: two peers on one port.
async fn mount_gen_fan(server: &MockServer, runs: u64) {
    let mut flows: Vec<Value> =
        serde_json::from_str(include_str!("../../../fixtures/traffic_flows.json")).unwrap();
    let mut other = flows[1].clone();
    other["src"] = json!({"ip": "10.9.9.9"});
    flows.push(other);
    mount_traffic_body(server, runs, Value::Array(flows).to_string()).await;
    Mock::given(method("GET"))
        .and(path("/api/v2/orgs/5/sec_policy/draft/rule_sets"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(include_str!("../../../fixtures/rule_sets.json"))
                .insert_header("Content-Type", "application/json"),
        )
        .expect(runs)
        .mount(server)
        .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn gen_lists_fan_in_candidates_and_consolidate_writes_one_rule() {
    let server = MockServer::start().await;
    mount_gen_fan(&server, 2).await;
    let dir = tempfile::tempdir().unwrap();
    let cfg = fan_config(&dir);
    let base = [
        "--config",
        cfg.to_str().unwrap(),
        "gen",
        "shop/prod",
        "--window",
        "1h",
    ];
    let out = stdout_json(cmd(&server).args(base).assert().success());
    let c = &out["data"]["consolidations"];
    assert_eq!(c.as_array().unwrap().len(), 1, "{c}");
    assert_eq!(c[0]["service"], "8443/tcp");
    assert_eq!(c[0]["reason"], "min_peers");
    assert_eq!(c[0]["mode"], "off");
    assert!(c[0]["rule"].is_null());
    let mut args = base.to_vec();
    args.extend(["--consolidate", "8443/tcp=any"]);
    let out = stdout_json(cmd(&server).args(&args).assert().success());
    let d = &out["data"];
    let c = &d["consolidations"][0];
    assert_eq!(c["mode"], "any");
    assert_eq!(c["peers"], 2);
    let i = c["rule"].as_u64().unwrap() as usize - 1;
    let r = &d["rules"][i];
    assert_eq!(r["consumers"], json!(["ipl:Any (0.0.0.0/0 and ::/0)"]));
    assert_eq!(r["unscoped_consumers"], false);
    assert_eq!(r["services"], json!(["8443/tcp"]));
    assert_eq!(
        r["body"]["consumers"],
        json!([{"ip_list": {"href": "/orgs/5/sec_policy/draft/ip_lists/20"}}])
    );
    assert_eq!(
        c["also_allows"].as_array().unwrap().last().unwrap(),
        "any source not seen"
    );
    assert!(
        c["warnings"][0]
            .as_str()
            .unwrap()
            .contains("full enforcement"),
        "{c}"
    );
    assert_eq!(c["to"], "every workload of shop/prod");
    let marked: Vec<&Value> = d["rows"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|r| r.get("consolidated").is_some())
        .collect();
    assert_eq!(marked.len(), 2, "the two IP-side rows");
    assert_eq!(marked[0]["consolidated"], json!(["8443/tcp"]));
    server.verify().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn gen_consolidate_refuses_a_bad_spec_before_any_request_and_a_non_candidate_after() {
    let server = MockServer::start().await;
    let a = cmd(&server)
        .args(["gen", "shop/prod", "--consolidate", "5432/tcp=everything"])
        .assert()
        .code(2);
    let err = String::from_utf8_lossy(&a.get_output().stderr).to_string();
    assert!(err.contains("the mode is any, all or off"), "{err}");
    let a = cmd(&server)
        .args([
            "gen",
            "shop/prod",
            "--consolidate",
            "53/udp=any",
            "--consolidate",
            "53/udp=all",
        ])
        .assert()
        .code(2);
    let err = String::from_utf8_lossy(&a.get_output().stderr).to_string();
    assert!(err.contains("53/udp is given more than once"), "{err}");
    assert!(server.received_requests().await.unwrap().is_empty());
    mount_gen(&server, 1, 1).await;
    let a = cmd(&server)
        .args([
            "gen",
            "shop/prod",
            "--window",
            "1h",
            "--consolidate",
            "5432/tcp=all",
        ])
        .assert()
        .code(2);
    let err = String::from_utf8_lossy(&a.get_output().stderr).to_string();
    assert!(
        err.contains("5432/tcp is not a fan-in port (candidates: none here)"),
        "{err}"
    );
    // `off` on a port that is no candidate: a warning, nothing else.
    let server = MockServer::start().await;
    mount_gen(&server, 1, 1).await;
    let a = cmd(&server)
        .args([
            "gen",
            "shop/prod",
            "--window",
            "1h",
            "--consolidate",
            "5432/tcp=off",
        ])
        .assert()
        .success();
    let err = String::from_utf8_lossy(&a.get_output().stderr).to_string();
    assert!(err.contains("nothing to switch off"), "{err}");
}

#[tokio::test(flavor = "multi_thread")]
async fn workloads_health_ven_version_and_ven_summary_cost_the_one_list_request() {
    let server = MockServer::start().await;
    mount_objects(&server).await;
    Mock::given(method("GET"))
        .and(path("/api/v2/orgs/5/workloads"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(include_str!("../../../fixtures/workloads.json"))
                .insert_header("Content-Type", "application/json"),
        )
        .expect(6)
        .mount(&server)
        .await;
    let hrefs = |out: &Value| -> Vec<String> {
        out["data"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| r["href"].as_str().unwrap().to_string())
            .collect()
    };
    // The fixture's heartbeats are days old: every VEN is offline.
    let out = stdout_json(
        cmd(&server)
            .args(["workloads", "--health", "unmanaged,warning"])
            .assert()
            .success(),
    );
    assert_eq!(hrefs(&out), ["/orgs/5/workloads/aaaa-3"]);
    assert_eq!(out["meta"]["api_calls"], 1);
    assert_eq!(
        out["meta"]["local_filter"],
        json!(["health warning,unmanaged"])
    );
    let out = stdout_json(
        cmd(&server)
            .args(["workloads", "--ven-version", "23.2.*"])
            .assert()
            .success(),
    );
    assert_eq!(
        hrefs(&out),
        ["/orgs/5/workloads/aaaa-1", "/orgs/5/workloads/aaaa-4"]
    );
    assert_eq!(out["meta"]["local_filter"], json!(["ven 23.2.*"]));
    // The summary: the same one read, nothing more.
    let out = stdout_json(
        cmd(&server)
            .args(["workloads", "ven-summary"])
            .assert()
            .success(),
    );
    assert_eq!(out["meta"]["api_calls"], 1, "one list request");
    let d = &out["data"];
    assert_eq!(
        (d["workloads"].as_u64(), d["vens"].as_u64()),
        (Some(4), Some(3))
    );
    assert_eq!(d["newest"], "23.2.31-3113");
    assert_eq!(d["behind"], 1);
    assert_eq!(
        d["versions"][0],
        json!({"version": "23.2.31-3113", "count": 2, "newest": true})
    );
    assert_eq!(d["health"][3], json!({"health": "offline", "count": 3}));
    assert_eq!(
        d["messages"][0]["message"],
        "policy_sync: policy out of date"
    );
    // Filters narrow the summary too.
    let out = stdout_json(
        cmd(&server)
            .args(["workloads", "ven-summary", "--ven-version", "22.*"])
            .assert()
            .success(),
    );
    assert_eq!(out["data"]["vens"], 1);
    cmd(&server)
        .args(["workloads", "ven-summary", "--table"])
        .assert()
        .success()
        .stdout(
            predicate::str::contains("health · 3 VENs · 4 workloads")
                .and(predicate::str::contains(
                    "versions · newest 23.2.31-3113 · 1 behind",
                ))
                .and(predicate::str::contains("visibility_only")),
        );
    let a = cmd(&server)
        .args(["workloads", "ven-summary", "--csv"])
        .assert()
        .success();
    let csv = String::from_utf8_lossy(&a.get_output().stdout).to_string();
    let lines: Vec<&str> = csv.lines().collect();
    assert_eq!(lines[0], "section,name,count,note");
    assert!(lines.contains(&"health,offline,3,"), "{csv}");
    assert!(lines.contains(&"version,23.2.31-3113,2,newest"), "{csv}");
    assert!(lines.contains(&"enforcement,unmanaged,1,"), "{csv}");
    assert!(
        lines.contains(&"message,policy_sync: policy out of date,1,warning"),
        "{csv}"
    );
    // A bad state is clap's argument error, before any request.
    cmd(&server)
        .args(["workloads", "--health", "warn"])
        .assert()
        .code(2);
}

#[tokio::test(flavor = "multi_thread")]
async fn health_is_one_filter_offline_joins_health_and_states_take_any_case() {
    let server = MockServer::start().await;
    mount_objects(&server).await;
    Mock::given(method("GET"))
        .and(path("/api/v2/orgs/5/workloads"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(include_str!("../../../fixtures/workloads.json"))
                .insert_header("Content-Type", "application/json"),
        )
        .expect(2)
        .mount(&server)
        .await;
    // Every fixture VEN is offline (old heartbeats): --offline joins the set.
    let out = stdout_json(
        cmd(&server)
            .args(["workloads", "--health", "Unmanaged", "--offline"])
            .assert()
            .success(),
    );
    assert_eq!(out["data"].as_array().unwrap().len(), 4, "a union, not 0");
    assert_eq!(
        out["meta"]["local_filter"],
        json!(["health offline,unmanaged"])
    );
    let out = stdout_json(
        cmd(&server)
            .args(["workloads", "--health", "WARNING", "--unhealthy"])
            .assert()
            .success(),
    );
    assert_eq!(out["data"].as_array().unwrap().len(), 3);
    assert_eq!(
        out["meta"]["local_filter"],
        json!(["health warning,error,offline"])
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn human_prints_bytes_as_1024_units_in_table_csv_and_the_meta_line_not_in_json() {
    let server = MockServer::start().await;
    mount_traffic(&server, 5).await;
    // Pairs: 124000 bytes is 121K; a row without bytes is 0B in the CSV.
    let csv = cmd(&server)
        .args(["traffic", "groups", "--window", "1h", "--csv", "--human"])
        .assert()
        .success();
    let text = csv_stdout(&csv);
    assert!(text.contains(",121K,"), "{text}");
    assert!(!text.contains("124000"), "{text}");
    let err = String::from_utf8_lossy(&csv.get_output().stderr).to_string();
    assert!(!err.contains(" KB"), "the meta line is human too: {err}");
    // The same flag short, on a peer CSV.
    let text = csv_stdout(
        &cmd(&server)
            .args([
                "traffic", "groups", "--window", "1h", "--by", "in", "--csv", "-H",
            ])
            .assert()
            .success(),
    );
    assert!(text.contains(",1,310,121K,"), "{text}");
    // Table.
    let table = cmd(&server)
        .args(["traffic", "groups", "--window", "1h", "--table", "--human"])
        .assert()
        .success();
    let text = String::from_utf8_lossy(&table.get_output().stdout).to_string();
    assert!(text.contains("121K") && !text.contains("124000"), "{text}");
    // Off: unchanged, with the meta line in KB.
    let plain = cmd(&server)
        .args(["traffic", "groups", "--window", "1h", "--csv"])
        .assert()
        .success();
    assert!(csv_stdout(&plain).contains("124000"));
    assert!(String::from_utf8_lossy(&plain.get_output().stderr).contains(" KB"));
    // JSON keeps the integers and the meta.
    let json = stdout_json(
        cmd(&server)
            .args(["traffic", "groups", "--window", "1h", "--json", "--human"])
            .assert()
            .success(),
    );
    assert_eq!(json["data"][0]["bytes"], 124_000);
    assert!(json["meta"]["bytes"].is_u64());
}
