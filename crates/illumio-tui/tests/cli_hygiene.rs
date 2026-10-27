//! `hygiene` (#83): the labels, label groups, IP lists and services nothing
//! names, and the workloads missing label keys. Every request is pinned
//! with `.expect`: each list read once and only for the kinds asked, with
//! or without the cache.
#[path = "common/schema_check.rs"]
mod schema_check;
use assert_cmd::Command;
use serde_json::{Value, json};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const TEST_SECRET: &str = "s3cr3t-cli-test-9f2a";

fn no_config_dir() -> std::path::PathBuf {
    static N: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = N.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    std::env::temp_dir().join(format!(
        "illumio-tui-no-config-hygiene-{}-{n}",
        std::process::id()
    ))
}

fn cmd(server: &MockServer) -> Command {
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
        .env("XDG_CONFIG_HOME", no_config_dir())
        .env("XDG_CACHE_HOME", no_config_dir());
    c
}

fn stdout(assert: &assert_cmd::assert::Assert) -> String {
    String::from_utf8_lossy(&assert.get_output().stdout).to_string()
}

fn stderr(assert: &assert_cmd::assert::Assert) -> String {
    String::from_utf8_lossy(&assert.get_output().stderr).to_string()
}

/// One GET of `p` answered with `body`, expected `times` times.
async fn expect(server: &MockServer, p: &str, body: &str, times: u64) {
    Mock::given(method("GET"))
        .and(path(p))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(body.to_string())
                .insert_header("Content-Type", "application/json"),
        )
        .expect(times)
        .mount(server)
        .await;
}

const LABELS: &str = "/api/v2/orgs/5/labels";
const GROUPS: &str = "/api/v2/orgs/5/sec_policy/draft/label_groups";
const IP_LISTS: &str = "/api/v2/orgs/5/sec_policy/draft/ip_lists";
const SERVICES: &str = "/api/v2/orgs/5/sec_policy/draft/services";
const RULE_SETS: &str = "/api/v2/orgs/5/sec_policy/draft/rule_sets";
const PAIRING: &str = "/api/v2/orgs/5/pairing_profiles";
const WORKLOADS: &str = "/api/v2/orgs/5/workloads";

/// Each read expected `n` times, in order: labels, label groups, IP lists,
/// services, rulesets, pairing profiles, workloads.
async fn server_with(n: [u64; 7]) -> MockServer {
    let server = MockServer::start().await;
    let reads = [
        (LABELS, include_str!("../../../fixtures/labels.json")),
        (GROUPS, include_str!("../../../fixtures/label_groups.json")),
        (IP_LISTS, include_str!("../../../fixtures/ip_lists.json")),
        (SERVICES, include_str!("../../../fixtures/services.json")),
        (RULE_SETS, include_str!("../../../fixtures/rule_sets.json")),
        (
            PAIRING,
            include_str!("../../../fixtures/pairing_profiles.json"),
        ),
        (WORKLOADS, include_str!("../../../fixtures/workloads.json")),
    ];
    for ((p, body), times) in reads.into_iter().zip(n) {
        expect(&server, p, body, times).await;
    }
    server
}

fn rows(v: &Value) -> Vec<String> {
    v["data"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| {
            format!(
                "{} {}",
                r["kind"].as_str().unwrap(),
                r["name"].as_str().unwrap()
            )
        })
        .collect()
}

#[tokio::test]
async fn every_kind_in_seven_reads_unused_objects_then_workloads() {
    let server = server_with([1; 7]).await;
    let a = cmd(&server).args(["--json", "hygiene"]).assert().success();
    let v = schema_check::envelope(&a.get_output().stdout);
    assert_eq!(v["meta"]["schema"], "hygiene");
    assert_eq!(v["meta"]["api_calls"], 7);
    // loc=eu: nothing names it; Web: no rule names it. Any and All
    // Services are built-ins; the deleted label is left out.
    assert_eq!(
        rows(&v),
        [
            "label loc=eu",
            "service Web",
            "workload host-1.example",
            "workload host-2.example",
            "workload unmanaged-nas",
            "workload host-4.example",
        ]
    );
    assert_eq!(
        v["data"][0],
        json!({"kind": "label", "name": "loc=eu", "href": "/orgs/5/labels/5",
            "reason": "unused", "missing": null})
    );
    assert_eq!(v["data"][4]["reason"], "missing labels: role,app,env,loc");
    assert_eq!(
        v["data"][4]["missing"],
        json!(["role", "app", "env", "loc"])
    );
    let h = &v["meta"]["hygiene"];
    assert_eq!(
        h["unused"],
        json!([
            {"kind": "label", "count": 1},
            {"kind": "label_group", "count": 0},
            {"kind": "ip_list", "count": 0},
            {"kind": "service", "count": 1}
        ])
    );
    assert_eq!(h["keys"], json!(["role", "app", "env", "loc"]));
    assert_eq!(
        h["missing"],
        json!([
            {"key": "role", "count": 2},
            {"key": "app", "count": 2},
            {"key": "env", "count": 2},
            {"key": "loc", "count": 4}
        ])
    );
    assert_eq!(h["workloads"], 4);
    assert_eq!(h["deny_unread"], 0);
    let ns = v["meta"]["not_searched"].as_str().unwrap();
    assert!(
        ns.starts_with("not searched: virtual services") && ns.contains("RBAC scopes"),
        "{ns}"
    );
    server.verify().await;
}

#[tokio::test]
async fn kind_reads_only_what_the_kinds_need() {
    // IP lists and services: their lists and the rulesets.
    let server = server_with([0, 0, 1, 1, 1, 0, 0]).await;
    let a = cmd(&server)
        .args(["--json", "hygiene", "--kind", "ip_list,service"])
        .assert()
        .success();
    let v = schema_check::envelope(&a.get_output().stdout);
    assert_eq!(v["meta"]["api_calls"], 3);
    assert_eq!(rows(&v), ["service Web"]);
    let h = &v["meta"]["hygiene"];
    assert_eq!(h["kinds"], json!(["ip_list", "service"]));
    assert_eq!(
        (h["keys"].clone(), h["missing"].clone()),
        (Value::Null, Value::Null)
    );
    assert_eq!(h["workloads"], Value::Null);
    server.verify().await;

    // Workloads: the workload list alone.
    let server = server_with([0, 0, 0, 0, 0, 0, 1]).await;
    let a = cmd(&server)
        .args(["--json", "hygiene", "--kind", "workload"])
        .assert()
        .success();
    let v = schema_check::envelope(&a.get_output().stdout);
    assert_eq!(v["meta"]["api_calls"], 1);
    assert_eq!(v["data"].as_array().unwrap().len(), 4);
    assert_eq!(v["meta"]["hygiene"]["unused"], json!([]));
    server.verify().await;

    // Label groups: the groups and the rulesets.
    let server = server_with([0, 1, 0, 0, 1, 0, 0]).await;
    let a = cmd(&server)
        .args(["--json", "hygiene", "--kind", "label-group"])
        .assert()
        .success();
    let v = schema_check::envelope(&a.get_output().stdout);
    assert_eq!(v["meta"]["api_calls"], 2);
    assert_eq!(rows(&v), Vec::<String>::new());
    server.verify().await;

    // Labels: everything that can hold one.
    let server = server_with([1, 1, 0, 0, 1, 1, 1]).await;
    let a = cmd(&server)
        .args(["--json", "hygiene", "--kind", "label"])
        .assert()
        .success();
    let v = schema_check::envelope(&a.get_output().stdout);
    assert_eq!(v["meta"]["api_calls"], 5);
    assert_eq!(rows(&v), ["label loc=eu"]);
    server.verify().await;
}

#[tokio::test]
async fn no_cache_still_reads_each_list_once() {
    let server = server_with([1; 7]).await;
    let a = cmd(&server)
        .args(["--json", "--no-cache", "hygiene"])
        .assert()
        .success();
    let v = schema_check::envelope(&a.get_output().stdout);
    assert_eq!(v["meta"]["api_calls"], 7);
    assert_eq!(v["meta"]["cache"], "bypass");
    server.verify().await;
}

#[tokio::test]
async fn an_unknown_kind_is_exit_2_before_any_request() {
    let server = server_with([0; 7]).await;
    let a = cmd(&server)
        .args(["hygiene", "--kind", "rule"])
        .assert()
        .code(2);
    assert!(stderr(&a).contains("label_group"), "{}", stderr(&a));
    server.verify().await;
}

#[tokio::test]
async fn the_table_and_csv_print_the_rows_and_the_meta_line_the_counts() {
    let server = server_with([1; 7]).await;
    let a = cmd(&server).args(["--table", "hygiene"]).assert().success();
    let out = stdout(&a);
    assert!(out.contains("loc=eu") && out.contains("unused"), "{out}");
    assert!(out.contains("role, app, env, loc"), "{out}");
    let err = stderr(&a);
    assert!(err.contains("7 api calls"), "{err}");
    assert!(
        err.contains("unused: 1 label, 0 label groups, 0 IP lists, 1 service"),
        "{err}"
    );
    assert!(
        err.contains("missing labels: role 2, app 2, env 2, loc 4 (of 4 workloads)"),
        "{err}"
    );
    assert!(err.contains("not searched: virtual services"), "{err}");
    server.verify().await;

    let server = server_with([0, 0, 1, 1, 1, 0, 1]).await;
    let a = cmd(&server)
        .args(["--csv", "hygiene", "--kind", "ip_list,service,workload"])
        .assert()
        .success();
    let out = stdout(&a);
    let mut lines = out.lines();
    assert_eq!(lines.next().unwrap(), "kind,name,href,reason,missing");
    assert_eq!(
        lines.next().unwrap(),
        "service,Web,/orgs/5/sec_policy/draft/services/32,unused,"
    );
    assert_eq!(
        lines.nth(1).unwrap(),
        "workload,host-2.example,/orgs/5/workloads/aaaa-2,missing labels: loc,loc"
    );
    assert_eq!(
        lines.next().unwrap(),
        r#"workload,unmanaged-nas,/orgs/5/workloads/aaaa-3,"missing labels: role,app,env,loc",role; app; env; loc"#
    );
    server.verify().await;
}

#[tokio::test]
async fn fields_pick_columns_and_are_checked_before_any_request() {
    // Unused IP lists as CSV for a cleanup ticket: the docs' recipe.
    let server = server_with([0, 0, 1, 0, 1, 0, 0]).await;
    let a = cmd(&server)
        .args([
            "--csv",
            "--fields",
            "name,href",
            "hygiene",
            "--kind",
            "ip_list",
        ])
        .assert()
        .success();
    assert_eq!(stdout(&a), "name,href\r\n");
    server.verify().await;

    let server = server_with([0, 0, 0, 1, 1, 0, 0]).await;
    let a = cmd(&server)
        .args([
            "--json",
            "--fields",
            "kind,name",
            "hygiene",
            "--kind",
            "service",
        ])
        .assert()
        .success();
    let v = schema_check::picked(&a.get_output().stdout);
    assert_eq!(v["data"], json!([{"kind": "service", "name": "Web"}]));
    server.verify().await;

    let server = server_with([0; 7]).await;
    let a = cmd(&server)
        .args(["--json", "--fields", "ruleset", "hygiene"])
        .assert()
        .code(2);
    assert!(stderr(&a).contains("ruleset"), "{}", stderr(&a));
    server.verify().await;
}

#[tokio::test]
async fn a_failed_pairing_profiles_read_keeps_the_report_and_says_so() {
    let server = server_with([1, 1, 0, 0, 1, 0, 1]).await;
    Mock::given(method("GET"))
        .and(path(PAIRING))
        .respond_with(ResponseTemplate::new(403).set_body_string("{}"))
        // Ahead of the fixture's answer, mounted to be read never.
        .with_priority(1)
        .expect(1)
        .mount(&server)
        .await;
    let a = cmd(&server)
        .args(["--json", "hygiene", "--kind", "label"])
        .assert()
        .success();
    let v = schema_check::envelope(&a.get_output().stdout);
    assert_eq!(rows(&v), ["label loc=eu"]);
    let ns = v["meta"]["not_searched"].as_str().unwrap();
    assert!(ns.contains("pairing profiles (read failed: "), "{ns}");
    let w = v["meta"]["warnings"].as_array().expect("a warning");
    assert!(
        w.iter().any(|w| w
            .as_str()
            .unwrap()
            .starts_with("pairing profiles not searched: ")),
        "{w:?}"
    );
    server.verify().await;
}

#[tokio::test]
async fn rulesets_without_inline_deny_rules_are_warned() {
    let server = MockServer::start().await;
    expect(
        &server,
        IP_LISTS,
        include_str!("../../../fixtures/ip_lists.json"),
        1,
    )
    .await;
    // No ruleset carries deny rules inline: this PCE keeps them apart.
    let mut sets: Value =
        serde_json::from_str(include_str!("../../../fixtures/rule_sets.json")).unwrap();
    for rs in sets.as_array_mut().unwrap() {
        rs["deny_rules"] = json!([]);
    }
    expect(&server, RULE_SETS, &sets.to_string(), 1).await;
    let a = cmd(&server)
        .args(["--json", "hygiene", "--kind", "ip_list"])
        .assert()
        .success();
    let v = schema_check::envelope(&a.get_output().stdout);
    assert_eq!(v["meta"]["hygiene"]["deny_unread"], 2);
    let w = v["meta"]["warnings"].as_array().expect("a warning");
    assert!(
        w[0].as_str()
            .unwrap()
            .contains("may hold deny rules not searched"),
        "{w:?}"
    );
    server.verify().await;
}

#[test]
fn schema_hygiene_prints_the_schema_and_its_csv_columns() {
    let a = Command::cargo_bin("illumio-tui")
        .unwrap()
        .env("XDG_CONFIG_HOME", no_config_dir())
        .env("XDG_CACHE_HOME", no_config_dir())
        .args(["schema", "hygiene"])
        .assert()
        .success();
    let out = stdout(&a);
    let v: Value = serde_json::from_str(&out).unwrap();
    assert_eq!(
        v["$comment"], "--csv columns: kind, name, href, reason, missing",
        "{out}"
    );
    let a = Command::cargo_bin("illumio-tui")
        .unwrap()
        .env("XDG_CONFIG_HOME", no_config_dir())
        .env("XDG_CACHE_HOME", no_config_dir())
        .args(["schema", "meta"])
        .assert()
        .success();
    let out = stdout(&a);
    assert!(
        out.contains("not_searched") && out.contains("deny_unread"),
        "{out}"
    );
}
