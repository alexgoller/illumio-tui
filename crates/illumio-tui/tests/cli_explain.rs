//! `traffic explain` (issue #50): one narrowed query evaluated against the
//! draft (update_rules), each flow's rules resolved through the draft
//! rulesets, the cost in `--help` and `meta.cost`. The 4-flow fixture
//! through a mock PCE; the helpers repeat those of cli.rs (test files share
//! no code without a module).
#[path = "common/schema_check.rs"]
mod schema_check;
use assert_cmd::Command;
use serde_json::{Value, json};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const QHREF: &str = "/orgs/5/traffic_flows/async_queries/q-1";
const RS101: &str = "/orgs/5/sec_policy/draft/rule_sets/101";

fn cmd(server: &MockServer, home: &std::path::Path) -> Command {
    let url = url::Url::parse(&server.uri()).unwrap();
    let mut c = Command::cargo_bin("illumio-tui").unwrap();
    c.env_clear()
        .env("PATH", std::env::var("PATH").unwrap_or_default())
        .env("PCE_SCHEME", "http")
        .env("PCE_HOST", url.host_str().unwrap())
        .env("PCE_PORT", url.port().unwrap().to_string())
        .env("PCE_ORG_ID", "5")
        .env("API_KEY", "key")
        .env("API_SECRET", "s3cr3t-explain-test")
        .env("XDG_CONFIG_HOME", home);
    c
}

fn json_out(a: assert_cmd::assert::Assert) -> Value {
    schema_check::envelope(&a.get_output().stdout)
}

/// A status that changes on every poll (`updated_at`), as the PCE's does
/// while it evaluates the draft rules.
struct Ticking {
    body: Value,
    n: std::sync::atomic::AtomicU64,
}

impl wiremock::Respond for Ticking {
    fn respond(&self, _: &wiremock::Request) -> ResponseTemplate {
        let n = self.n.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let mut body = self.body.clone();
        body["updated_at"] = json!(format!("2026-10-14T00:00:{:02}Z", n % 60));
        ResponseTemplate::new(200).set_body_json(body)
    }
}

/// The fixture after update_rules: the 443 flow allowed by Shop-Prod's
/// sec_rules/2, the icmp flow blocked by an override deny rule listed
/// beside `rules` (Global-Infra's, not inline: fetched), the rest with no
/// rule. `rules` names `ruleset` for the object lists, the rulesets and
/// their deny rules.
async fn mount(server: &MockServer, rules: bool) {
    mount_with(server, rules, false).await;
}

/// `mount`, its per-ruleset deny rules endpoint answering 404 when `gone`
/// (a PCE without it).
async fn mount_with(server: &MockServer, rules: bool, gone: bool) {
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
        (
            "/api/v2/orgs/5/sec_policy/draft/rule_sets",
            include_str!("../../../fixtures/rule_sets.json"),
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
    Mock::given(method("GET"))
        .and(path(format!("/api/v2{RS101}/deny_rules")))
        .respond_with(if gone {
            ResponseTemplate::new(404)
        } else {
            ResponseTemplate::new(200).set_body_json(json!([{
                "href": format!("{RS101}/deny_rules/9"), "enabled": true, "override": true,
                "providers": [{"actors": "ams"}], "consumers": [{"actors": "ams"}],
                "ingress_services": [{"proto": 1}]
            }]))
        })
        .mount(server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/v2/orgs/5/traffic_flows/async_queries"))
        .respond_with(
            ResponseTemplate::new(202).set_body_json(json!({"href": QHREF, "status": "queued"})),
        )
        .expect(1)
        .mount(server)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("/api/v2{QHREF}")))
        .respond_with(Ticking {
            body: json!({
                "status": "completed", "rules": "completed",
                "result": {"href": format!("{QHREF}/download")}, "flows_count": 4
            }),
            n: std::sync::atomic::AtomicU64::new(0),
        })
        .mount(server)
        .await;
    Mock::given(method("PUT"))
        .and(path(format!("/api/v2{QHREF}/update_rules")))
        .respond_with(ResponseTemplate::new(204))
        .expect(1)
        .mount(server)
        .await;
    let mut flows: Vec<Value> =
        serde_json::from_str(include_str!("../../../fixtures/traffic_flows.json")).unwrap();
    for f in &mut flows {
        let d = f["policy_decision"].as_str().unwrap().to_string();
        f["draft_policy_decision"] = json!(d);
        f["rules"] = json!([]);
    }
    if rules {
        flows[3]["rules"] = json!([{"href": "/orgs/5/sec_policy/draft/rule_sets/100/sec_rules/2"}]);
        flows[2]["override_deny_rules"] = json!([{"href": format!("{RS101}/deny_rules/9")}]);
    }
    Mock::given(method("GET"))
        .and(path(format!("/api/v2{QHREF}/download")))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(Value::Array(flows))
                .insert_header("Content-Type", "application/json"),
        )
        .mount(server)
        .await;
}

/// The requests the server saw, in order, object lists left out.
async fn sequence(server: &MockServer) -> Vec<String> {
    server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .map(|r| {
            format!(
                "{} {}",
                r.method,
                r.url.path().trim_start_matches("/api/v2")
            )
        })
        .filter(|s| s.contains("traffic_flows") || s.contains("rule_sets"))
        .collect()
}

const ARGS: [&str; 10] = [
    "traffic", "explain", "--src", "role=web", "--dst", "role=db", "--port", "443", "--proto",
    "tcp",
];

#[test]
fn help_states_the_cost() {
    let out = Command::cargo_bin("illumio-tui")
        .unwrap()
        .args(["traffic", "explain", "--help"])
        .assert()
        .success();
    let text = String::from_utf8_lossy(&out.get_output().stdout).to_string();
    for s in ["1 POST", "1 PUT", "1 download", "update_rules", "meta.cost"] {
        assert!(text.contains(s), "missing {s:?}:\n{text}");
    }
}

#[test]
fn a_missing_or_wrong_argument_exits_2_before_any_request() {
    for args in [
        vec!["traffic", "explain", "--port", "443", "--proto", "tcp"],
        vec![
            "traffic", "explain", "--src", "role=web", "--dst", "role=db", "--proto", "tcp",
        ],
        vec![
            "traffic", "explain", "--src", "a=b", "--dst", "c=d", "--port", "1", "--proto", "icmp",
        ],
    ] {
        Command::cargo_bin("illumio-tui")
            .unwrap()
            .env_clear()
            .args(&args)
            .assert()
            .code(2);
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn explain_runs_one_draft_query_and_names_each_flows_rules() {
    let server = MockServer::start().await;
    mount(&server, true).await;
    let home = tempfile::tempdir().unwrap();
    let out = json_out(cmd(&server, home.path()).args(ARGS).assert().success());
    let seq = sequence(&server).await;
    assert_eq!(seq[0], "POST /orgs/5/traffic_flows/async_queries");
    assert_eq!(
        seq.iter().filter(|s| s.starts_with("PUT")).count(),
        1,
        "{seq:?}"
    );
    assert_eq!(
        &seq[seq.len() - 3..],
        [
            format!("GET {QHREF}/download"),
            "GET /orgs/5/sec_policy/draft/rule_sets".to_string(),
            format!("GET {RS101}/deny_rules"),
        ],
        "one download, the draft rulesets, the one ruleset whose deny rules are not inline"
    );
    // The query is narrowed to the port and protocol.
    let post = &server.received_requests().await.unwrap()[..]
        .iter()
        .find(|r| r.method.as_str() == "POST")
        .map(|r| serde_json::from_slice::<Value>(&r.body).unwrap())
        .unwrap();
    assert_eq!(
        post["services"]["include"],
        json!([{"port": 443, "proto": 6}])
    );

    let flows = out["data"]["flows"].as_array().unwrap();
    assert_eq!(flows.len(), 4);
    let allowed = &flows[3];
    assert_eq!(allowed["verdict"], "allowed by 1 rule (draft)");
    assert_eq!(allowed["reported"], "allowed");
    assert_eq!(allowed["draft"], "allowed");
    let r = &allowed["rules"][0];
    assert_eq!(r["kind"], "allow");
    assert_eq!(r["ruleset"], "Shop-Prod");
    assert_eq!(r["scopes"], json!(["app=shop & env=prod"]));
    assert_eq!(r["consumers"], json!(["ipl:Corp"]));
    assert_eq!(r["providers"], json!(["role=web"]));
    assert_eq!(r["services"], json!(["443/tcp", "8000-8099/tcp"]));
    assert_eq!(r["enabled"], true);
    let denied = &flows[2];
    assert_eq!(denied["rules"][0]["kind"], "override_deny");
    assert_eq!(denied["rules"][0]["resolved"], true);
    assert_eq!(denied["verdict"], "blocked by 1 override deny rule (draft)");
    assert_eq!(flows[1]["verdict"], "no rule allows this flow");
    assert_eq!(flows[1]["rules"], json!([]));
    assert!(
        out["meta"]["cost"]
            .as_str()
            .unwrap()
            .contains("1 PUT update_rules"),
        "{}",
        out["meta"]
    );
    assert_eq!(out["meta"]["draft"]["evaluated"], true);
}

#[tokio::test(flavor = "multi_thread")]
async fn no_rule_named_loads_no_rulesets_and_the_table_says_so() {
    let server = MockServer::start().await;
    mount(&server, false).await;
    let home = tempfile::tempdir().unwrap();
    let out = cmd(&server, home.path())
        .args(ARGS)
        .arg("--table")
        .assert()
        .success();
    let text = String::from_utf8_lossy(&out.get_output().stdout).to_string();
    assert!(text.contains("no rule allows this flow"), "{text}");
    assert!(
        !sequence(&server)
            .await
            .iter()
            .any(|s| s.contains("rule_sets")),
        "no rule to resolve: no ruleset request"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_404_on_the_deny_rules_leaves_that_rule_an_href_and_the_command_succeeds() {
    let server = MockServer::start().await;
    mount_with(&server, true, true).await;
    let home = tempfile::tempdir().unwrap();
    let out = json_out(cmd(&server, home.path()).args(ARGS).assert().success());
    let r = &out["data"]["flows"][2]["rules"][0];
    assert_eq!(r["resolved"], false);
    assert_eq!(r["kind"], "override_deny", "its side field's kind");
    assert_eq!(r["href"], format!("{RS101}/deny_rules/9"));
    assert_eq!(out["data"]["flows"][3]["rules"][0]["resolved"], true);
}

#[tokio::test(flavor = "multi_thread")]
async fn one_side_alone_is_enough() {
    let server = MockServer::start().await;
    mount(&server, true).await;
    let home = tempfile::tempdir().unwrap();
    let out = json_out(
        cmd(&server, home.path())
            .args([
                "traffic", "explain", "--dst", "role=db", "--port", "443", "--proto", "tcp",
            ])
            .assert()
            .success(),
    );
    assert_eq!(out["data"]["flows"].as_array().unwrap().len(), 4);
    let help = Command::cargo_bin("illumio-tui")
        .unwrap()
        .args(["traffic", "explain", "--help"])
        .assert()
        .success();
    assert!(String::from_utf8_lossy(&help.get_output().stdout).contains("wider query"));
}
