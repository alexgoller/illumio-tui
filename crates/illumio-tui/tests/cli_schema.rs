//! `illumio-tui schema` and `--fields`; the validator every CLI test runs
//! its envelopes through (`common/schema_check.rs`).
#[path = "common/schema_check.rs"]
mod schema_check;

use assert_cmd::Command;
use serde_json::{Value, json};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// No profile, no PCE, no config: what `schema` needs. A config and cache
/// directory of this call only, never created.
fn bare() -> Command {
    static N: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = N.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!("illumio-tui-schema-{}-{n}", std::process::id()));
    let mut c = Command::cargo_bin("illumio-tui").unwrap();
    c.env_clear()
        .env("PATH", std::env::var("PATH").unwrap_or_default())
        .env("XDG_CONFIG_HOME", dir.join("config"))
        .env("XDG_CACHE_HOME", dir.join("cache"));
    c
}

/// A server that fails the test if anything reaches it.
async fn silent() -> MockServer {
    let server = MockServer::start().await;
    Mock::given(wiremock::matchers::any())
        .respond_with(ResponseTemplate::new(500))
        .expect(0)
        .mount(&server)
        .await;
    server
}

fn cmd(server: &MockServer) -> Command {
    let url = url::Url::parse(&server.uri()).unwrap();
    let mut c = bare();
    c.env("PCE_SCHEME", "http")
        .env("PCE_HOST", url.host_str().unwrap())
        .env("PCE_PORT", url.port().unwrap().to_string())
        .env("PCE_ORG_ID", "5")
        .env("API_KEY", "key")
        .env("API_SECRET", "s3cr3t-schema-test");
    c
}

fn json_out(a: &assert_cmd::assert::Assert) -> Value {
    schema_check::envelope(&a.get_output().stdout)
}

fn picked(a: &assert_cmd::assert::Assert) -> Value {
    schema_check::picked(&a.get_output().stdout)
}

fn stderr(a: &assert_cmd::assert::Assert) -> String {
    String::from_utf8_lossy(&a.get_output().stderr).into_owned()
}

fn stdout(a: &assert_cmd::assert::Assert) -> String {
    String::from_utf8_lossy(&a.get_output().stdout).into_owned()
}

async fn mount(server: &MockServer, p: &str, body: &str) {
    Mock::given(method("GET"))
        .and(path(p))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(body.to_string())
                .insert_header("Content-Type", "application/json"),
        )
        .mount(server)
        .await;
}

async fn labels_server(body: &str) -> MockServer {
    let server = MockServer::start().await;
    mount(&server, "/api/v2/orgs/5/labels", body).await;
    server
}

const LABELS: &str = include_str!("../../../fixtures/labels.json");

// ---- schema ----

#[test]
fn schema_lists_every_command_and_the_shared_shapes_without_a_pce() {
    let a = bare().args(["schema", "--json"]).assert().success();
    let out = json_out(&a);
    assert_eq!(out["meta"]["schema"], "schema-list");
    assert_eq!(out["meta"]["api_calls"], 0);
    let rows = out["data"].as_array().unwrap();
    let row = |c: &str| rows.iter().find(|r| r["command"] == c).cloned();
    assert_eq!(
        row("workloads ven-summary").unwrap()["schema"],
        "workloads-ven-summary"
    );
    assert_eq!(row("labels add").unwrap()["schema"], "write");
    for shared in ["envelope", "meta", "error"] {
        assert!(
            rows.iter()
                .any(|r| r["kind"] == "shared" && r["schema"] == shared && r["command"].is_null()),
            "{shared}"
        );
    }
    let t = bare().args(["schema", "--table"]).assert().success();
    let text = stdout(&t);
    assert!(
        text.contains("workloads ven-summary") && text.contains("saved-filter"),
        "{text}"
    );
}

#[test]
fn schema_of_a_command_is_its_envelope_with_the_shared_meta() {
    let a = bare()
        .args(["schema", "workloads", "ven-summary"])
        .assert()
        .success();
    let doc: Value = serde_json::from_slice(&a.get_output().stdout).unwrap();
    assert_eq!(
        doc["$schema"],
        "https://json-schema.org/draft/2020-12/schema"
    );
    assert_eq!(doc["title"], "illumio-tui workloads-ven-summary");
    assert_eq!(doc["required"], json!(["data", "meta"]));
    assert_eq!(doc["properties"]["meta"]["$ref"], "#/$defs/meta");
    assert!(doc["properties"]["data"]["properties"]["versions"].is_object());
    let meta = &doc["$defs"]["meta"]["properties"];
    assert!(
        meta["warnings"].is_object() && meta["schema"].is_object(),
        "{meta}"
    );
    assert!(
        doc["$defs"]["ven_health"].is_object(),
        "the defs it uses come along"
    );
    // A schema name works too, and the shared shapes print.
    for name in ["write", "envelope", "meta", "error"] {
        let a = bare().args(["schema", name]).assert().success();
        let doc: Value = serde_json::from_slice(&a.get_output().stdout).unwrap();
        assert_eq!(doc["title"], format!("illumio-tui {name}"));
    }
}

#[test]
fn a_row_with_a_flag_types_as_listed() {
    for (words, title) in [
        (vec!["events", "--save-filter"], "saved-filter"),
        (
            vec!["traffic", "query", "--dry-run", "--json"],
            "traffic-dry-run",
        ),
        (vec!["enforce", "--watch"], "enforce-watch"),
    ] {
        let a = bare().arg("schema").args(&words).assert().success();
        let doc: Value = serde_json::from_slice(&a.get_output().stdout).unwrap();
        assert_eq!(doc["title"], format!("illumio-tui {title}"), "{words:?}");
    }
}

#[test]
fn schema_of_an_unknown_command_lists_the_valid_ones() {
    let a = bare()
        .args(["schema", "workloads", "nope"])
        .assert()
        .code(2);
    let err = stderr(&a);
    assert!(
        err.contains("no command or schema \\\"workloads nope\\\""),
        "{err}"
    );
    assert!(
        err.contains("workloads ven-summary") && err.contains("envelope"),
        "{err}"
    );
}

#[test]
fn the_error_envelope_follows_its_schema() {
    let a = bare().args(["schema", "nope"]).assert().code(2);
    let err: Value = serde_json::from_str(stderr(&a).trim()).unwrap();
    let doc = illumio_tui::cli::schema::document("error").unwrap();
    assert!(schema_check::validate(&doc, &err).is_empty(), "{err}");
}

#[test]
fn a_closed_stdout_ends_quietly_with_141() {
    // The read end is closed before the run starts: its first write fails.
    let (reader, writer) = std::io::pipe().unwrap();
    drop(reader);
    for args in [vec!["schema", "--json"], vec!["schema", "workloads"]] {
        let dir = std::env::temp_dir().join(format!("illumio-tui-pipe-{}", std::process::id()));
        let out = std::process::Command::new(assert_cmd::cargo::cargo_bin("illumio-tui"))
            .env_clear()
            .env("XDG_CONFIG_HOME", dir.join("config"))
            .env("XDG_CACHE_HOME", dir.join("cache"))
            .args(&args)
            .stdout(writer.try_clone().unwrap())
            .stderr(std::process::Stdio::piped())
            .output()
            .unwrap();
        assert_eq!(out.status.code(), Some(141), "{args:?}");
        assert!(
            out.stderr.is_empty(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
}

// ---- --fields, JSON ----

#[tokio::test(flavor = "multi_thread")]
async fn fields_keep_only_the_named_fields_of_each_element() {
    let server = labels_server(LABELS).await;
    let a = cmd(&server)
        .args([
            "labels",
            "--json",
            "--fields",
            "key,value",
            "--fields",
            "key",
        ])
        .assert()
        .success();
    let out = picked(&a);
    let rows = out["data"].as_array().unwrap();
    assert!(!rows.is_empty());
    for r in rows {
        let keys: Vec<&String> = r.as_object().unwrap().keys().collect();
        assert_eq!(keys, ["key", "value"], "{r}");
    }
    // meta is never filtered.
    assert_eq!(out["meta"]["schema"], "labels");
    assert!(out["meta"]["elapsed_ms"].is_number());
}

#[tokio::test(flavor = "multi_thread")]
async fn dotted_fields_reach_through_arrays_and_keep_the_nesting() {
    let server = MockServer::start().await;
    mount(
        &server,
        "/api/v2/orgs/5/workloads",
        include_str!("../../../fixtures/workloads.json"),
    )
    .await;
    let a = cmd(&server)
        .args(["workloads", "--json", "--fields", "hostname,labels.key"])
        .assert()
        .success();
    let out = picked(&a);
    let first = &out["data"][0];
    assert_eq!(
        first,
        &json!({"hostname": "host-1.example", "labels": [{"key": "role"}, {"key": "app"}, {"key": "env"}]})
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn fields_on_object_data_and_into_a_raw_pce_answer() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v2/health"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"status": "normal", "type": "cluster"})),
        )
        .mount(&server)
        .await;
    let a = cmd(&server)
        .args(["health", "--json", "--fields", "ok,status.status"])
        .assert()
        .success();
    assert_eq!(
        picked(&a)["data"],
        json!({"ok": true, "status": {"status": "normal"}})
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn fields_check_against_the_schema_so_an_empty_result_checks_too() {
    let server = labels_server("[]").await;
    let a = cmd(&server)
        .args(["labels", "--json", "--fields", "key"])
        .assert()
        .success();
    assert_eq!(picked(&a)["data"], json!([]));
    let bad = cmd(&server)
        .args(["labels", "--json", "--fields", "key,nope"])
        .assert()
        .code(2);
    let err = stderr(&bad);
    assert!(err.contains(r#"no field \"nope\" in labels"#), "{err}");
    assert!(err.contains("href") && err.contains("value"), "{err}");
}

#[tokio::test(flavor = "multi_thread")]
async fn an_unknown_field_exits_2_before_any_request() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_body_string("[]"))
        .expect(0)
        .mount(&server)
        .await;
    let a = cmd(&server)
        .args(["workloads", "--json", "--fields", "labels.nope"])
        .assert()
        .code(2);
    let err: Value = serde_json::from_str(stderr(&a).trim()).unwrap();
    assert_eq!(err["error"]["kind"], "args");
    assert!(
        err["error"]["message"]
            .as_str()
            .unwrap()
            .contains("labels.key"),
        "{err}"
    );
}

// ---- --fields, CSV and table ----

#[tokio::test(flavor = "multi_thread")]
async fn csv_fields_pick_the_columns_in_their_order() {
    let server = labels_server(LABELS).await;
    let a = cmd(&server)
        .args(["labels", "--csv", "--fields", "href,key"])
        .assert()
        .success();
    let text = stdout(&a);
    let mut lines = text.lines();
    assert_eq!(lines.next(), Some("href,key"));
    assert_eq!(lines.next(), Some("/orgs/5/labels/1,role"));
    // A column this run does not print (only --unused has it) is empty.
    let a = cmd(&server)
        .args(["labels", "--csv", "--fields", "key,workloads"])
        .assert()
        .success();
    assert_eq!(stdout(&a).lines().nth(1), Some("role,"));
}

#[tokio::test(flavor = "multi_thread")]
async fn an_unknown_csv_column_exits_2_before_any_request() {
    let server = silent().await;
    let bad = cmd(&server)
        .args(["labels", "--csv", "--fields", "nope"])
        .assert()
        .code(2);
    let err = stderr(&bad);
    assert!(err.contains(r#"no CSV column \"nope\""#), "{err}");
    assert!(
        err.contains("key, value, workloads, rules, deleted, href"),
        "{err}"
    );
    // Traffic too: no Explorer query for a typo.
    let bad = cmd(&server)
        .args(["traffic", "query", "--csv", "--fields", "nope"])
        .assert()
        .code(2);
    assert!(stderr(&bad).contains("last seen"), "{}", stderr(&bad));
    // A JSON name is no CSV column.
    cmd(&server)
        .args(["profile", "list", "--csv", "--fields", "is_default"])
        .assert()
        .code(2);
}

#[tokio::test(flavor = "multi_thread")]
async fn names_under_a_pass_through_object_are_open_but_not_at_the_top() {
    let server = silent().await;
    for args in [
        vec!["events", "--json", "--fields", "evnt_type"],
        vec!["marks", "list", "--json", "--fields", "typo"],
        // A --group field on the plain list.
        vec!["workloads", "--json", "--fields", "group_by"],
        vec!["traffic", "query", "--json", "--fields", "sections"],
    ] {
        let a = cmd(&server).args(&args).assert().code(2);
        assert!(stderr(&a).contains("no field"), "{args:?}: {}", stderr(&a));
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn empty_field_names_are_refused() {
    let server = silent().await;
    for f in ["", ",", "status..x", ".a"] {
        let a = cmd(&server)
            .args(["health", "--json", "--fields", f])
            .assert()
            .code(2);
        assert!(stderr(&a).contains("--fields"), "{f:?}: {}", stderr(&a));
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn services_follow_their_schema() {
    let server = MockServer::start().await;
    mount(
        &server,
        "/api/v2/orgs/5/sec_policy/draft/services",
        include_str!("../../../fixtures/services.json"),
    )
    .await;
    let a = cmd(&server).args(["services", "--json"]).assert().success();
    let out = json_out(&a);
    assert_eq!(out["meta"]["schema"], "services");
    assert!(!out["data"].as_array().unwrap().is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn csv_refuses_a_dotted_path_and_a_table_refuses_fields() {
    let server = MockServer::start().await;
    let a = cmd(&server)
        .args(["labels", "--csv", "--fields", "a.b"])
        .assert()
        .code(2);
    assert!(
        stderr(&a).contains("a dotted path is for --json"),
        "{}",
        stderr(&a)
    );
    let t = cmd(&server)
        .args(["labels", "--table", "--fields", "key"])
        .assert()
        .code(2);
    assert!(
        stderr(&t).contains("--fields needs --json or --csv"),
        "{}",
        stderr(&t)
    );
    assert!(server.received_requests().await.unwrap().is_empty());
}

#[test]
fn fields_on_a_command_without_an_envelope_is_refused() {
    let a = bare()
        .args(["completions", "bash", "--fields", "x"])
        .assert()
        .code(2);
    assert!(
        stderr(&a).contains("prints no JSON envelope"),
        "{}",
        stderr(&a)
    );
}

// ---- the validator ----

use schema_check::validate;

fn errs(schema: Value, v: Value) -> Vec<String> {
    validate(&schema, &v)
}

#[test]
fn validator_checks_types_and_null_unions() {
    let s = json!({"type": ["string", "null"]});
    assert!(errs(s.clone(), json!("a")).is_empty());
    assert!(errs(s.clone(), json!(null)).is_empty());
    assert_eq!(errs(s, json!(1)).len(), 1);
    assert!(errs(json!({"type": "integer"}), json!(1.5))[0].contains("type"));
    assert!(errs(json!({"type": "number"}), json!(1.5)).is_empty());
}

#[test]
fn validator_checks_properties_required_and_extras() {
    let s = json!({
        "type": "object",
        "properties": {"a": {"type": "string"}, "b": {"type": "integer"}},
        "required": ["a"],
        "additionalProperties": false
    });
    assert!(errs(s.clone(), json!({"a": "x"})).is_empty());
    assert!(errs(s.clone(), json!({}))[0].contains("missing required \"a\""));
    assert!(errs(s.clone(), json!({"a": "x", "c": 1}))[0].contains("unexpected field \"c\""));
    assert!(errs(s, json!({"a": "x", "b": "no"}))[0].contains("$.b"));
    let open = json!({"type": "object", "additionalProperties": {"type": "integer"}});
    assert!(errs(open.clone(), json!({"x": 1})).is_empty());
    assert_eq!(errs(open, json!({"x": "1"})).len(), 1);
}

#[test]
fn validator_follows_refs_items_enums_and_branches() {
    let s = json!({
        "type": "array",
        "items": {"$ref": "#/$defs/e"},
        "$defs": {"e": {"enum": ["a", "b"]}}
    });
    assert!(errs(s.clone(), json!(["a", "b"])).is_empty());
    assert!(errs(s, json!(["a", "c"]))[0].contains("$[1]"));
    let one = json!({"oneOf": [{"type": "string"}, {"type": "integer"}]});
    assert!(errs(one.clone(), json!(1)).is_empty());
    assert_eq!(errs(one, json!(true)).len(), 1);
    let both = json!({"oneOf": [{"type": "integer"}, {"type": "number"}]});
    assert_eq!(errs(both, json!(1)).len(), 1, "two branches match");
    assert!(errs(json!({"const": 1}), json!(1)).is_empty());
}

#[test]
#[should_panic(expected = "outside the validator's subset")]
fn validator_refuses_keywords_it_does_not_know() {
    errs(json!({"minimum": 1}), json!(1));
}
