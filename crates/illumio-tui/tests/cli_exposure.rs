//! `illumio-tui exposure` (#82) against a mock PCE: exactly one Explorer
//! query whose services are the risky list's ports, the rows and the
//! summary, the `traffic explain` arguments of an exposed row, the
//! `--fail-on-exposed` exit code, `--service`, the CSV columns, and a
//! malformed `[exposure]` refused before any request. The helpers repeat
//! those of cli.rs (test files share no code without a module).
#[path = "common/schema_check.rs"]
mod schema_check;
use assert_cmd::Command;
use serde_json::{Value, json};
use std::path::Path;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const QHREF: &str = "/orgs/5/traffic_flows/async_queries/q-1";

fn cmd(server: &MockServer, home: &Path) -> Command {
    let url = url::Url::parse(&server.uri()).unwrap();
    let mut c = Command::cargo_bin("illumio-tui").unwrap();
    c.env_clear()
        .env("PATH", std::env::var("PATH").unwrap_or_default())
        .env("PCE_SCHEME", "http")
        .env("PCE_HOST", url.host_str().unwrap())
        .env("PCE_PORT", url.port().unwrap().to_string())
        .env("PCE_ORG_ID", "5")
        .env("API_KEY", "key")
        .env("API_SECRET", "s3cr3t-exposure-test")
        .env("XDG_CONFIG_HOME", home)
        .env("XDG_CACHE_HOME", home.join("cache"))
        .env("XDG_STATE_HOME", home.join("state"))
        .env("XDG_DATA_HOME", home.join("data"));
    c
}

fn answer(body: &str) -> ResponseTemplate {
    ResponseTemplate::new(200)
        .set_body_string(body)
        .insert_header("Content-Type", "application/json")
}

/// shop/prod's db (host-2): RDP from the Corp IP list allowed (exposed),
/// postgres from its own web tier allowed (intra), SSH from a public
/// address in the Any list blocked.
fn flows() -> String {
    let db = json!({"ip": "10.0.2.20", "workload": {
        "href": "/orgs/5/workloads/aaaa-2", "hostname": "host-2.example",
        "labels": [{"href": "/orgs/5/labels/2"}, {"href": "/orgs/5/labels/3"}, {"href": "/orgs/5/labels/4"}]
    }});
    let web = json!({"ip": "10.0.1.10", "workload": {
        "href": "/orgs/5/workloads/aaaa-1", "hostname": "host-1.example",
        "labels": [{"href": "/orgs/5/labels/1"}, {"href": "/orgs/5/labels/3"}, {"href": "/orgs/5/labels/4"}]
    }});
    let corp = json!({"ip": "10.1.4.9", "ip_lists": [{"href": "/orgs/5/sec_policy/draft/ip_lists/21", "name": "Corp"}]});
    let any = json!({"ip": "203.0.113.7", "ip_lists": [{"href": "/orgs/5/sec_policy/draft/ip_lists/1", "name": "Any (0.0.0.0/0 and ::/0)"}]});
    let flow = |src: &Value, port: u16, d: &str, n: u64| {
        json!({"src": src, "dst": db, "service": {"port": port, "proto": 6},
               "policy_decision": d, "num_connections": n, "flow_direction": "inbound"})
    };
    serde_json::to_string(&json!([
        flow(&corp, 3389, "allowed", 4),
        flow(&web, 5432, "allowed", 310),
        flow(&any, 22, "blocked", 2),
        flow(&web, 443, "allowed", 9),
    ]))
    .unwrap()
}

/// The object lists; `posts` traffic queries, each answered with `flows`.
async fn mount(server: &MockServer, posts: u64) {
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
            .respond_with(answer(body))
            .mount(server)
            .await;
    }
    Mock::given(method("POST"))
        .and(path("/api/v2/orgs/5/traffic_flows/async_queries"))
        .respond_with(
            ResponseTemplate::new(202).set_body_json(json!({"href": QHREF, "status": "queued"})),
        )
        .expect(posts)
        .mount(server)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("/api/v2{QHREF}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "status": "completed",
            "result": {"href": format!("{QHREF}/download")}, "flows_count": 4
        })))
        .mount(server)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("/api/v2{QHREF}/download")))
        .respond_with(answer(&flows()))
        .expect(posts)
        .mount(server)
        .await;
}

fn post_bodies(reqs: &[wiremock::Request]) -> Vec<Value> {
    reqs.iter()
        .filter(|r| r.method.as_str() == "POST")
        .map(|r| serde_json::from_slice(&r.body).unwrap())
        .collect()
}

/// `PORT/proto` of each include service of a query body.
fn include_ports(body: &Value) -> Vec<String> {
    body["services"]["include"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| {
            let proto = match s["proto"].as_u64() {
                Some(6) => "tcp",
                Some(17) => "udp",
                _ => "?",
            };
            format!("{}/{proto}", s["port"])
        })
        .collect()
}

#[tokio::test(flavor = "multi_thread")]
async fn one_query_over_the_risky_ports_makes_the_rows_and_names_the_explain() {
    let server = MockServer::start().await;
    mount(&server, 1).await;
    let home = tempfile::tempdir().unwrap();
    let a = cmd(&server, home.path())
        .args(["exposure", "--json"])
        .assert()
        .success();
    let out = schema_check::envelope(&a.get_output().stdout);
    let reqs = server.received_requests().await.unwrap();
    let bodies = post_bodies(&reqs);
    assert_eq!(bodies.len(), 1, "exactly one Explorer query");
    let ports = include_ports(&bodies[0]);
    assert_eq!(ports.len(), 20, "every port of the default list: {ports:?}");
    assert_eq!(ports[..3], ["3389/tcp", "3389/udp", "445/tcp"]);
    assert!(ports.contains(&"27017/tcp".to_string()));
    assert_eq!(
        bodies[0]["sources"]["include"],
        json!([[]]),
        "no other filter"
    );
    let start: jiff::Timestamp = bodies[0]["start_date"].as_str().unwrap().parse().unwrap();
    let end: jiff::Timestamp = bodies[0]["end_date"].as_str().unwrap().parse().unwrap();
    assert_eq!(
        end.duration_since(start).as_secs(),
        86_400,
        "traffic's window"
    );
    assert_eq!(out["meta"]["schema"], "exposure");
    assert_eq!(out["meta"]["cap"], 10_000);

    let data = &out["data"];
    assert_eq!(data["window"], "1d");
    assert_eq!(data["draft"], false);
    let rows = data["rows"].as_array().unwrap();
    let got: Vec<(&str, &str)> = rows
        .iter()
        .map(|r| {
            (
                r["service"].as_str().unwrap(),
                r["status"].as_str().unwrap(),
            )
        })
        .collect();
    assert_eq!(
        got,
        [
            ("rdp", "exposed"),
            ("ssh", "blocked"),
            ("postgres", "intra")
        ],
        "443 is no risky service"
    );
    let rdp = &rows[0];
    assert_eq!(rdp["dst"], json!(["shop", "prod"]));
    assert_eq!(rdp["ports"], json!(["3389/tcp"]));
    assert_eq!(
        rdp["sources"],
        json!({"intra": 0, "apps": 0, "ip_lists": 1, "any": false, "internet": false})
    );
    assert_eq!(rdp["exposed_flows"], 1);
    assert_eq!(
        rdp["explain"],
        json!([
            "traffic",
            "explain",
            "--dst",
            "app=shop,env=prod",
            "--port",
            "3389",
            "--proto",
            "tcp"
        ])
    );
    let ssh = &rows[1];
    assert_eq!(ssh["sources"]["any"], true);
    assert_eq!(ssh["sources"]["internet"], true);
    assert!(ssh.get("explain").is_none(), "only exposed rows name one");
    assert_eq!(rows[2]["sources"]["intra"], 1);
    let services = data["services"].as_array().unwrap();
    assert_eq!(services.len(), 15, "every service of the list");
    assert_eq!(
        services[0],
        json!({"service": "rdp", "destinations": 1, "exposed": 1,
               "potentially_blocked": 0, "blocked": 0, "intra": 0})
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn the_table_prints_the_summary_the_rows_and_the_explain_footer() {
    let server = MockServer::start().await;
    mount(&server, 1).await;
    let home = tempfile::tempdir().unwrap();
    let a = cmd(&server, home.path())
        .args(["exposure", "--table"])
        .assert()
        .success();
    let text = String::from_utf8_lossy(&a.get_output().stdout).to_string();
    assert!(
        text.contains(
            "rdp: 1 destination · 1 exposed (allowed from other apps) · 0 potentially blocked · 0 blocked"
        ),
        "{text}"
    );
    assert!(
        text.contains("exposure · 1d · 3 rows · 1 exposed"),
        "{text}"
    );
    assert!(
        text.contains(
            "illumio-tui traffic explain --dst app=shop,env=prod --port 3389 --proto tcp"
        ),
        "{text}"
    );
    assert!(
        !text.contains("telnet:"),
        "no line for a service without flows"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn fail_on_exposed_exits_8_after_the_report() {
    let server = MockServer::start().await;
    mount(&server, 1).await;
    let home = tempfile::tempdir().unwrap();
    let a = cmd(&server, home.path())
        .args(["exposure", "--json", "--fail-on-exposed"])
        .assert()
        .code(8);
    let out = schema_check::envelope(&a.get_output().stdout);
    assert_eq!(out["data"]["rows"][0]["status"], "exposed");
    let err: Value = serde_json::from_slice(&a.get_output().stderr).unwrap();
    assert_eq!(err["error"]["kind"], "exposed");
    assert_eq!(err["error"]["exit_code"], 8);
}

#[tokio::test(flavor = "multi_thread")]
async fn service_narrows_the_query_and_only_the_rows() {
    let server = MockServer::start().await;
    mount(&server, 1).await;
    let home = tempfile::tempdir().unwrap();
    let a = cmd(&server, home.path())
        .args([
            "exposure",
            "--json",
            "--service",
            "ssh,postgres",
            "--only",
            "blocked",
            "--fail-on-exposed",
            "--src",
            "ipl:Corp",
        ])
        .assert()
        .success();
    let out = schema_check::envelope(&a.get_output().stdout);
    let reqs = server.received_requests().await.unwrap();
    let bodies = post_bodies(&reqs);
    assert_eq!(include_ports(&bodies[0]), ["22/tcp", "5432/tcp"]);
    assert_eq!(
        bodies[0]["sources"]["include"][0][0]["ip_list"]["href"],
        "/orgs/5/sec_policy/draft/ip_lists/21",
        "--src as on traffic"
    );
    let rows = out["data"]["rows"].as_array().unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["service"], "ssh");
    assert_eq!(out["data"]["services"].as_array().unwrap().len(), 2);
    assert_eq!(
        out["data"]["services"][1]["intra"], 1,
        "the summary counts every row"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn csv_prints_the_rows_and_fields_pick() {
    let server = MockServer::start().await;
    mount(&server, 1).await;
    let home = tempfile::tempdir().unwrap();
    let a = cmd(&server, home.path())
        .args(["exposure", "--csv"])
        .assert()
        .success();
    let text = String::from_utf8_lossy(&a.get_output().stdout).to_string();
    let mut lines = text.lines();
    assert_eq!(
        lines.next().unwrap(),
        "status,service,ports,dst,intra,apps,ip_lists,any,internet,flows,connections,allowed,potentially_blocked,blocked,exposed_flows,allowed_conns,pot_blocked_conns,blocked_conns,unknown_conns,draft_allowed,draft_blocked,explain"
    );
    let first = lines.next().unwrap();
    assert!(
        first.starts_with("exposed,rdp,3389/tcp,shop/prod,0,0,1,"),
        "{first}"
    );
    assert!(
        first.ends_with(
            "illumio-tui traffic explain --dst app=shop,env=prod --port 3389 --proto tcp\""
        ),
        "{first}"
    );

    let server = MockServer::start().await;
    mount(&server, 1).await;
    let a = cmd(&server, home.path())
        .args(["exposure", "--json", "--fields", "rows.service,rows.status"])
        .assert()
        .success();
    let out = schema_check::picked(&a.get_output().stdout);
    assert_eq!(
        out["data"]["rows"][0],
        json!({"service": "rdp", "status": "exposed"})
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_malformed_list_or_an_unknown_service_is_refused_before_any_request() {
    let server = MockServer::start().await;
    mount(&server, 0).await;
    let home = tempfile::tempdir().unwrap();
    let a = cmd(&server, home.path())
        .args(["exposure", "--json", "--service", "rpd"])
        .assert()
        .code(2);
    let err = String::from_utf8_lossy(&a.get_output().stderr).to_string();
    assert!(err.contains("unknown risky service \\\"rpd\\\""), "{err}");

    let dir = home.path().join("illumio-tui");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("config.toml"),
        "default_profile = \"lab\"\n[profiles.lab]\n[profiles.lab.exposure]\nextra = { nfs = [\"2049\"] }\n",
    )
    .unwrap();
    let a = cmd(&server, home.path())
        .args(["exposure", "--json"])
        .assert()
        .code(2);
    let err: Value = serde_json::from_slice(&a.get_output().stderr).unwrap();
    assert_eq!(err["error"]["kind"], "config");
    assert_eq!(
        err["error"]["message"],
        "exposure.extra.nfs: \"2049\": give a port and a protocol, e.g. 3389/tcp"
    );
    // Other commands still run with that profile.
    cmd(&server, home.path())
        .args(["schema", "exposure"])
        .assert()
        .success();
    assert!(server.received_requests().await.unwrap().is_empty());
}
