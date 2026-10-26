//! `where-used` (#75): the references to a label, label group, IP list or
//! service, and a label's workload count. Every request is pinned with
//! `.expect`: the object's own list, the draft rulesets, the label groups
//! (labels and groups only), the pairing profiles and the one-row workload
//! probe (labels only).
#[path = "common/schema_check.rs"]
mod schema_check;
use assert_cmd::Command;
use serde_json::Value;
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

const TEST_SECRET: &str = "s3cr3t-cli-test-9f2a";

fn no_config_dir() -> std::path::PathBuf {
    static N: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = N.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    std::env::temp_dir().join(format!(
        "illumio-tui-no-config-where-used-{}-{n}",
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

/// The pairing profiles, read `times` times (0: never, for the kinds no
/// profile can hold).
async fn expect_pairing(server: &MockServer, times: u64) {
    expect(
        server,
        PAIRING,
        include_str!("../../../fixtures/pairing_profiles.json"),
        times,
    )
    .await;
}

/// The one-row workload probe for label `href`, answering `rows`.
async fn expect_probe(server: &MockServer, href: &str, rows: &str) {
    Mock::given(method("GET"))
        .and(path("/api/v2/orgs/5/workloads"))
        .and(query_param("labels", format!(r#"[["{href}"]]"#)))
        .and(query_param("max_results", "1"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(rows.to_string())
                .insert_header("Content-Type", "application/json")
                // The whole collection's size: says nothing of the label.
                .insert_header("X-Total-Count", "206"),
        )
        .expect(1)
        .mount(server)
        .await;
}

async fn label_server() -> MockServer {
    label_server_for("/orgs/5/labels/4").await
}

/// A label's five reads, the probe for `href`.
async fn label_server_for(href: &str) -> MockServer {
    let server = MockServer::start().await;
    expect(
        &server,
        LABELS,
        include_str!("../../../fixtures/labels.json"),
        1,
    )
    .await;
    expect(
        &server,
        GROUPS,
        include_str!("../../../fixtures/label_groups.json"),
        1,
    )
    .await;
    expect(
        &server,
        RULE_SETS,
        include_str!("../../../fixtures/rule_sets.json"),
        1,
    )
    .await;
    expect_pairing(&server, 1).await;
    expect_probe(&server, href, r#"[{"href": "/orgs/5/workloads/aaaa-1"}]"#).await;
    server
}

#[tokio::test]
async fn a_label_lists_its_scopes_rules_groups_and_workloads_in_five_reads() {
    let server = label_server().await;
    let a = cmd(&server)
        .args(["--json", "where-used", "label", "env=prod"])
        .assert()
        .success();
    let v = schema_check::envelope(&a.get_output().stdout);
    assert_eq!(v["meta"]["schema"], "where-used");
    assert_eq!(v["meta"]["api_calls"], 5);
    let kinds: Vec<&str> = v["data"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["kind"].as_str().unwrap())
        .collect();
    assert_eq!(kinds, ["scope", "rule", "label_group", "workloads"]);
    let rule = &v["data"][1];
    assert_eq!(rule["ruleset"], "Global-Infra");
    assert_eq!(rule["rule"], 1);
    assert_eq!(rule["side"], "consumer");
    assert_eq!(rule["action"], "allow");
    assert_eq!(rule["pending"], "create");
    assert_eq!(rule["excluded"], false);
    assert_eq!(rule["disabled"], true);
    assert_eq!(rule["via"], "All prod envs");
    assert_eq!(
        rule["href"],
        "/orgs/5/sec_policy/draft/rule_sets/101/sec_rules/3"
    );
    let wl = &v["data"][3];
    assert_eq!(
        (wl["count"].clone(), wl["exact"].clone()),
        (1.into(), false.into())
    );
    let m = &v["meta"]["where_used"];
    assert_eq!(m["object"], "label");
    assert_eq!(m["name"], "env=prod");
    assert_eq!(m["href"], "/orgs/5/labels/4");
    assert_eq!(m["total"], 3);
    assert_eq!(m["workloads"]["count"], 1);
    assert_eq!(m["workloads"]["exact"], false);
    assert!(
        m["not_searched"]
            .as_str()
            .unwrap()
            .contains("virtual services")
    );
    server.verify().await;
}

#[tokio::test]
async fn the_table_shows_rule_numbers_and_the_meta_line_the_calls() {
    let server = label_server().await;
    let a = cmd(&server)
        .args(["--table", "where-used", "label", "/orgs/5/labels/4"])
        .assert()
        .success();
    let out = stdout(&a);
    assert!(out.contains("#1"), "{out}");
    assert!(
        out.contains("via") && out.contains("All prod envs"),
        "{out}"
    );
    assert!(out.contains("≥1"), "{out}");
    let err = stderr(&a);
    assert!(err.contains("5 api calls"), "{err}");
    assert!(err.contains("3 references to label env=prod"), "{err}");
    assert!(err.contains("≥1 workloads"), "{err}");
    assert!(err.contains("not searched: virtual services"), "{err}");
    server.verify().await;
}

#[tokio::test]
async fn csv_prints_one_row_per_reference_and_fields_picks_columns() {
    let server = label_server().await;
    let a = cmd(&server)
        .args(["--csv", "where-used", "label", "env=prod"])
        .assert()
        .success();
    let out = stdout(&a);
    let mut lines = out.lines();
    assert_eq!(
        lines.next().unwrap(),
        "kind,ruleset,rule,side,action,excluded,pending,disabled,via,name,count,exact,locked,href"
    );
    assert_eq!(
        lines.nth(1).unwrap(),
        "rule,Global-Infra,1,consumer,allow,false,create,true,All prod envs,,,,,/orgs/5/sec_policy/draft/rule_sets/101/sec_rules/3"
    );
    assert_eq!(
        out.lines().last().unwrap(),
        "workloads,,,,,false,,false,,,1,false,,/orgs/5/labels/4"
    );
    server.verify().await;

    let server = label_server().await;
    let a = cmd(&server)
        .args([
            "--csv",
            "--fields",
            "kind,rule,via",
            "where-used",
            "label",
            "env=prod",
        ])
        .assert()
        .success();
    let out = stdout(&a);
    assert_eq!(out.lines().next().unwrap(), "kind,rule,via");
    assert!(out.contains("rule,1,All prod envs"), "{out}");
    server.verify().await;
}

#[tokio::test]
async fn json_fields_are_checked_against_the_schema_before_any_request() {
    let server = MockServer::start().await;
    let a = cmd(&server)
        .args([
            "--json",
            "--fields",
            "kind,nope",
            "where-used",
            "iplist",
            "Corp",
        ])
        .assert()
        .code(2);
    assert!(
        stderr(&a).contains("no field") && stderr(&a).contains("valid: action"),
        "{}",
        stderr(&a)
    );
    let a = cmd(&server)
        .args([
            "--csv",
            "--fields",
            "kind,nope",
            "where-used",
            "iplist",
            "Corp",
        ])
        .assert()
        .code(2);
    assert!(stderr(&a).contains("no CSV column"), "{}", stderr(&a));
    assert!(server.received_requests().await.unwrap().is_empty());

    let server = MockServer::start().await;
    expect(
        &server,
        IP_LISTS,
        include_str!("../../../fixtures/ip_lists.json"),
        1,
    )
    .await;
    expect(
        &server,
        RULE_SETS,
        include_str!("../../../fixtures/rule_sets.json"),
        1,
    )
    .await;
    let a = cmd(&server)
        .args([
            "--json",
            "--fields",
            "kind,rule",
            "where-used",
            "iplist",
            "Corp",
        ])
        .assert()
        .success();
    let v = schema_check::picked(&a.get_output().stdout);
    assert_eq!(v["data"], serde_json::json!([{"kind": "rule", "rule": 3}]));
    server.verify().await;
}

#[tokio::test]
async fn an_ip_list_reads_its_list_and_the_rulesets_only() {
    let server = MockServer::start().await;
    expect(
        &server,
        IP_LISTS,
        include_str!("../../../fixtures/ip_lists.json"),
        1,
    )
    .await;
    expect(
        &server,
        RULE_SETS,
        include_str!("../../../fixtures/rule_sets.json"),
        1,
    )
    .await;
    expect_pairing(&server, 0).await;
    let a = cmd(&server)
        .args(["--json", "where-used", "iplist", "Corp"])
        .assert()
        .success();
    let v = schema_check::envelope(&a.get_output().stdout);
    assert_eq!(v["meta"]["api_calls"], 2);
    let data = v["data"].as_array().unwrap();
    assert_eq!(data.len(), 1);
    assert_eq!(data[0]["side"], "consumer");
    assert_eq!(data[0]["pending"], "update", "rule 2 is a pending update");
    assert_eq!(v["meta"]["where_used"]["workloads"], Value::Null);
    server.verify().await;
}

#[tokio::test]
async fn a_service_and_a_label_group_by_name() {
    let server = MockServer::start().await;
    expect(
        &server,
        SERVICES,
        include_str!("../../../fixtures/services.json"),
        1,
    )
    .await;
    expect(
        &server,
        RULE_SETS,
        include_str!("../../../fixtures/rule_sets.json"),
        1,
    )
    .await;
    expect_pairing(&server, 0).await;
    let a = cmd(&server)
        .args(["--json", "where-used", "service", "PostgreSQL"])
        .assert()
        .success();
    let v = schema_check::envelope(&a.get_output().stdout);
    assert_eq!(v["data"][0]["side"], "service");
    assert_eq!(v["data"][0]["rule"], 2);
    server.verify().await;

    let server = MockServer::start().await;
    expect(
        &server,
        GROUPS,
        include_str!("../../../fixtures/label_groups.json"),
        1,
    )
    .await;
    expect(
        &server,
        RULE_SETS,
        include_str!("../../../fixtures/rule_sets.json"),
        1,
    )
    .await;
    expect_pairing(&server, 0).await;
    let a = cmd(&server)
        .args(["--json", "where-used", "label-group", "All prod envs"])
        .assert()
        .success();
    let v = schema_check::envelope(&a.get_output().stdout);
    assert_eq!(v["meta"]["api_calls"], 2, "the groups list read once");
    assert_eq!(v["data"][0]["via"], Value::Null, "named directly");
    assert_eq!(v["data"][0]["ruleset"], "Global-Infra");
    server.verify().await;
}

#[tokio::test]
async fn unknown_and_ambiguous_names_are_exit_2_with_the_candidates() {
    let server = MockServer::start().await;
    expect(
        &server,
        LABELS,
        include_str!("../../../fixtures/labels.json"),
        1,
    )
    .await;
    let a = cmd(&server)
        .args(["where-used", "label", "env=staging"])
        .assert()
        .code(2);
    let err = stderr(&a);
    assert!(err.contains("known values for env: prod"), "{err}");
    server.verify().await;

    let server = MockServer::start().await;
    let twice = r#"[
        {"href": "/orgs/5/sec_policy/draft/ip_lists/21", "name": "Corp", "ip_ranges": []},
        {"href": "/orgs/5/sec_policy/draft/ip_lists/22", "name": "Corp", "ip_ranges": []}
    ]"#;
    expect(&server, IP_LISTS, twice, 1).await;
    let a = cmd(&server)
        .args(["where-used", "iplist", "Corp"])
        .assert()
        .code(2);
    let err = stderr(&a);
    assert!(
        err.contains("/orgs/5/sec_policy/draft/ip_lists/21")
            && err.contains("/orgs/5/sec_policy/draft/ip_lists/22"),
        "{err}"
    );
    server.verify().await;

    let server = MockServer::start().await;
    expect(
        &server,
        SERVICES,
        include_str!("../../../fixtures/services.json"),
        2,
    )
    .await;
    let a = cmd(&server)
        .args(["where-used", "service", "Postgres"])
        .assert()
        .code(2);
    assert!(stderr(&a).contains("PostgreSQL"), "{}", stderr(&a));
    let a = cmd(&server)
        .args([
            "where-used",
            "service",
            "/orgs/5/sec_policy/draft/services/99",
        ])
        .assert()
        .code(2);
    assert!(
        stderr(&a).contains("no service with href"),
        "{}",
        stderr(&a)
    );
    server.verify().await;
}

#[tokio::test]
async fn a_label_group_under_no_cache_reads_the_groups_once() {
    let server = MockServer::start().await;
    expect(
        &server,
        GROUPS,
        include_str!("../../../fixtures/label_groups.json"),
        1,
    )
    .await;
    expect(
        &server,
        RULE_SETS,
        include_str!("../../../fixtures/rule_sets.json"),
        1,
    )
    .await;
    let a = cmd(&server)
        .args([
            "--json",
            "--no-cache",
            "where-used",
            "label-group",
            "All prod envs",
        ])
        .assert()
        .success();
    let v = schema_check::envelope(&a.get_output().stdout);
    assert_eq!(v["meta"]["api_calls"], 2);
    server.verify().await;
}

#[tokio::test]
async fn an_unknown_service_lists_the_known_ones_when_few() {
    let server = MockServer::start().await;
    expect(
        &server,
        SERVICES,
        include_str!("../../../fixtures/services.json"),
        1,
    )
    .await;
    let a = cmd(&server)
        .args(["where-used", "service", "nosuchservice"])
        .assert()
        .code(2);
    let err = stderr(&a);
    assert!(
        err.contains("known: All Services, PostgreSQL, Web"),
        "{err}"
    );
    server.verify().await;
}

#[tokio::test]
async fn a_label_in_a_pairing_profile_lists_it_with_its_lock() {
    // role=web: given by shop-prod (disabled, role locked).
    let server = label_server_for("/orgs/5/labels/1").await;
    let a = cmd(&server)
        .args(["--json", "where-used", "label", "role=web"])
        .assert()
        .success();
    let v = schema_check::envelope(&a.get_output().stdout);
    assert_eq!(v["meta"]["api_calls"], 5);
    let data = v["data"].as_array().unwrap();
    let kinds: Vec<&str> = data.iter().map(|r| r["kind"].as_str().unwrap()).collect();
    assert_eq!(kinds, ["rule", "rule", "pairing_profile", "workloads"]);
    let p = &data[2];
    assert_eq!(p["name"], "shop-prod");
    assert_eq!(p["href"], "/orgs/5/pairing_profiles/7");
    assert_eq!(p["locked"], true);
    assert_eq!(p["disabled"], true);
    for k in [
        "ruleset", "rule", "side", "action", "pending", "via", "count", "exact",
    ] {
        assert_eq!(p[k], Value::Null, "{k}");
    }
    assert_eq!(data[0]["locked"], Value::Null, "only a profile row has it");
    assert_eq!(v["meta"]["where_used"]["total"], 3, "the profile counts");
    server.verify().await;

    let server = label_server_for("/orgs/5/labels/1").await;
    let a = cmd(&server)
        .args([
            "--csv",
            "--fields",
            "kind,name,locked",
            "where-used",
            "label",
            "role=web",
        ])
        .assert()
        .success();
    let out = stdout(&a);
    assert_eq!(out.lines().next().unwrap(), "kind,name,locked");
    assert!(out.contains("pairing_profile,shop-prod,true"), "{out}");
    assert_eq!(
        out.lines().nth(1),
        Some("rule,,"),
        "only a profile row has it"
    );
    server.verify().await;

    let server = label_server_for("/orgs/5/labels/1").await;
    let a = cmd(&server)
        .args(["--table", "where-used", "label", "role=web"])
        .assert()
        .success();
    let out = stdout(&a);
    assert!(
        out.contains("pairing_profile") && out.contains("shop-prod"),
        "{out}"
    );
    assert!(stderr(&a).contains("3 references"), "{}", stderr(&a));
    server.verify().await;
}

#[tokio::test]
async fn a_label_under_no_cache_reads_the_pairing_profiles_once() {
    let server = label_server().await;
    let a = cmd(&server)
        .args(["--json", "--no-cache", "where-used", "label", "env=prod"])
        .assert()
        .success();
    let v = schema_check::envelope(&a.get_output().stdout);
    assert_eq!(v["meta"]["api_calls"], 5);
    assert!(
        v["data"]
            .as_array()
            .unwrap()
            .iter()
            .all(|r| r["kind"] != "pairing_profile"),
        "env=prod is in no profile"
    );
    server.verify().await;
}
