//! `illumio-tui readiness` (#54) against a mock PCE: the request sequence
//! and its cost (one draft-evaluated query, two with `--app`, then the
//! workloads, pending changes and draft rulesets), the JSON shape, the
//! `--app` narrowing, marks counting as accepted, and the cost guard. The
//! helpers repeat those of cli.rs (test files share no code without a
//! module).
use assert_cmd::Command;
use serde_json::{Value, json};
use std::path::Path;
use wiremock::matchers::{method, path, query_param};
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
        .env("API_SECRET", "s3cr3t-readiness-test")
        .env("XDG_CONFIG_HOME", home);
    c
}

/// A config home with `marks` as marks.toml (none when empty).
fn home_with(marks: &str) -> tempfile::TempDir {
    let home = tempfile::tempdir().unwrap();
    if !marks.is_empty() {
        let dir = home.path().join("illumio-tui");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("marks.toml"), marks).unwrap();
    }
    home
}

fn json_out(a: assert_cmd::assert::Assert) -> Value {
    serde_json::from_slice(&a.get_output().stdout).expect("stdout is JSON")
}

fn answer(body: &str) -> ResponseTemplate {
    ResponseTemplate::new(200)
        .set_body_string(body)
        .insert_header("Content-Type", "application/json")
}

/// The fixture's flows after `update_rules`: web → db 5432 allowed, Corp
/// → web 8443 allowed in draft (potentially blocked now), unmanaged-nas →
/// db icmp blocked, web → an FQDN 443 blocked in draft (allowed now).
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

/// A status answer whose `updated_at` moves on every call, as the PCE's
/// does while it works: a draft `completed` after the PUT is then new.
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

/// The object lists; `posts` traffic queries and as many PUTs, each
/// answered with the draft flows; the workloads (`workloads_query`: the
/// `labels` parameter expected, or none), the pending changes and the draft
/// rulesets, once each.
async fn mount(server: &MockServer, posts: u64, workloads_labels: Option<&str>) {
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
        .and(query_param("label_based_rules", "false"))
        .respond_with(ResponseTemplate::new(204))
        .expect(posts)
        .mount(server)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("/api/v2{QHREF}/download")))
        .respond_with(answer(&draft_flows()))
        .expect(posts)
        .mount(server)
        .await;
    let wl = Mock::given(method("GET")).and(path("/api/v2/orgs/5/workloads"));
    let wl = match workloads_labels {
        Some(l) => wl.and(query_param("labels", l)),
        None => wl,
    };
    let once = u64::from(posts > 0);
    wl.respond_with(answer(include_str!("../../../fixtures/workloads.json")))
        .expect(once)
        .mount(server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v2/orgs/5/sec_policy/pending"))
        .respond_with(answer(include_str!("../../../fixtures/pending_list.json")))
        .expect(once)
        .mount(server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v2/orgs/5/sec_policy/draft/rule_sets"))
        .respond_with(answer(include_str!("../../../fixtures/rule_sets.json")))
        .expect(once)
        .mount(server)
        .await;
}

/// Every request after the object lists, in order: `METHOD path`.
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
        .filter(|s| {
            !["/labels", "/label_groups", "/ip_lists", "/services"]
                .iter()
                .any(|o| s.ends_with(o))
        })
        .collect()
}

fn posts(server_requests: &[wiremock::Request]) -> Vec<Value> {
    server_requests
        .iter()
        .filter(|r| r.method.as_str() == "POST")
        .map(|r| serde_json::from_slice(&r.body).unwrap())
        .collect()
}

#[tokio::test(flavor = "multi_thread")]
async fn readiness_runs_one_draft_query_then_the_cached_reads_and_judges_each_app() {
    let server = MockServer::start().await;
    mount(&server, 1, None).await;
    let home = home_with("");
    let out = json_out(
        cmd(&server, home.path())
            .args(["readiness", "--json"])
            .assert()
            .success(),
    );
    assert_eq!(
        sequence(&server).await,
        vec![
            "POST /orgs/5/traffic_flows/async_queries".to_string(),
            format!("GET {QHREF}"),
            format!("PUT {QHREF}/update_rules"),
            format!("GET {QHREF}"),
            format!("GET {QHREF}/download"),
            "GET /orgs/5/workloads".into(),
            "GET /orgs/5/sec_policy/pending".into(),
            "GET /orgs/5/sec_policy/draft/rule_sets".into(),
        ]
    );
    // 4 object lists, 5 traffic requests, 3 reads.
    assert_eq!(out["meta"]["api_calls"], 12);
    assert_eq!(out["meta"]["draft"]["evaluated"], true);
    assert_eq!(out["meta"]["cap"], 10_000);
    let reqs = server.received_requests().await.unwrap();
    let body = &posts(&reqs)[0];
    assert_eq!(
        body["destinations"]["exclude"],
        json!([{"transmission": "broadcast"}, {"transmission": "multicast"}]),
        "unicast only"
    );
    let start: jiff::Timestamp = body["start_date"].as_str().unwrap().parse().unwrap();
    let end: jiff::Timestamp = body["end_date"].as_str().unwrap().parse().unwrap();
    assert_eq!(
        end.duration_since(start).as_secs(),
        7 * 86_400,
        "7d by default"
    );

    let data = &out["data"];
    assert_eq!(data["window"], "7d");
    assert_eq!(data["draft"], true);
    let apps = data["apps"].as_array().unwrap();
    assert_eq!(apps.len(), 1, "shop/prod is the only labelled app");
    let shop = &apps[0];
    assert_eq!(shop["app"], "shop/prod");
    assert_eq!(shop["verdict"], "needs_rules");
    assert!(shop.get("gaps").is_none(), "gaps only with --app");
    assert_eq!(shop["evidence"]["flows"], 4);
    assert_eq!(shop["evidence"]["little"], true);
    let c = &shop["coverage"];
    assert_eq!(
        c["inbound"]["connections"], 325,
        "5432 inside once, 8443, icmp"
    );
    assert_eq!(c["outbound"]["connections"], 5);
    assert_eq!(c["total"]["allowed"], 322);
    assert_eq!(c["total"]["would_block"], 8);
    assert_eq!(c["total"]["would_block_flows"], 2);
    assert_eq!(shop["workloads"]["total"], 2);
    assert_eq!(shop["workloads"]["managed"], 2);
    assert_eq!(shop["workloads"]["enforcement"]["enforced"], 1);
    assert_eq!(shop["workloads"]["enforcement"]["visibility"], 1);
    // Global-Infra (every workload) and Corp (used by Shop-Prod's rule).
    assert_eq!(shop["pending"]["count"], 2);
    assert!(
        shop["warnings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|w| w["kind"] == "little_evidence")
    );
    server.verify().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn readiness_app_narrows_to_two_queries_and_prints_the_gaps() {
    let server = MockServer::start().await;
    mount(
        &server,
        2,
        Some(r#"[["/orgs/5/labels/3","/orgs/5/labels/4"]]"#),
    )
    .await;
    let home = home_with("");
    let out = json_out(
        cmd(&server, home.path())
            .args([
                "readiness",
                "--app",
                "shop/prod",
                "--window",
                "1d",
                "--json",
            ])
            .assert()
            .success(),
    );
    let seq = sequence(&server).await;
    assert_eq!(
        seq.iter().filter(|s| s.starts_with("PUT")).count(),
        2,
        "one update_rules per side: {seq:?}"
    );
    assert_eq!(
        seq[seq.len() - 3..],
        [
            "GET /orgs/5/workloads",
            "GET /orgs/5/sec_policy/pending",
            "GET /orgs/5/sec_policy/draft/rule_sets",
        ]
    );
    let reqs = server.received_requests().await.unwrap();
    let bodies = posts(&reqs);
    let shop =
        json!([[{"label": {"href": "/orgs/5/labels/3"}}, {"label": {"href": "/orgs/5/labels/4"}}]]);
    assert_eq!(
        bodies[0]["destinations"]["include"], shop,
        "the app as destination"
    );
    assert_eq!(bodies[0]["sources"]["include"], json!([[]]));
    assert_eq!(bodies[1]["sources"]["include"], shop, "then as source");
    // 4 object lists, 2 × 5 traffic requests, 3 reads.
    assert_eq!(out["meta"]["api_calls"], 17);
    assert_eq!(out["meta"]["queries"], 2);
    let app = &out["data"]["apps"][0];
    assert_eq!(
        app["evidence"]["flows"], 4,
        "the sides merged without duplicates"
    );
    assert_eq!(app["evidence"]["window"], "1d");
    let gaps: Vec<(String, String, String, u64)> = app["gaps"]
        .as_array()
        .unwrap()
        .iter()
        .map(|g| {
            (
                g["direction"].as_str().unwrap().into(),
                g["peer"].as_array().unwrap()[0].as_str().unwrap().into(),
                g["service"].as_str().unwrap().into(),
                g["connections"].as_u64().unwrap(),
            )
        })
        .collect();
    assert_eq!(
        gaps,
        vec![
            ("out".into(), "fqdn-1.example".into(), "443/tcp".into(), 5),
            ("in".into(), "unmanaged-nas".into(), "icmp".into(), 3),
        ]
    );
    let pending: Vec<&str> = app["pending"]["changes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["href"].as_str().unwrap())
        .collect();
    assert_eq!(
        pending,
        [
            "/orgs/5/sec_policy/draft/rule_sets/101",
            "/orgs/5/sec_policy/draft/ip_lists/21"
        ]
    );
    server.verify().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn readiness_counts_squelched_and_accept_blocked_flows_as_accepted() {
    let server = MockServer::start().await;
    mount(
        &server,
        2,
        Some(r#"[["/orgs/5/labels/3","/orgs/5/labels/4"]]"#),
    )
    .await;
    let home = home_with(
        r#"
[[mark]]
id = "m-nas-ok"
action = "accept-blocked"
src = { ip = "10.0.3.30" }
dst = "any"
decision = "blocked"
comment = "the NAS pings, harmless"

[[mark]]
id = "m-egress"
action = "squelch"
src = "any"
dst = "any"
service = "443/tcp"
comment = "egress is handled elsewhere"
"#,
    );
    let out = json_out(
        cmd(&server, home.path())
            .args([
                "readiness",
                "--app",
                "shop/prod",
                "--window",
                "1d",
                "--json",
            ])
            .assert()
            .success(),
    );
    let app = &out["data"]["apps"][0];
    assert_eq!(app["coverage"]["total"]["accepted"], 8);
    assert_eq!(app["coverage"]["total"]["would_block"], 0);
    assert_eq!(app["coverage"]["ratio"], 1.0);
    assert_eq!(app["gaps"], json!([]));
    assert_eq!(
        app["verdict"], "needs_provision",
        "Global-Infra and Corp are pending"
    );
    assert_eq!(out["meta"]["marks"]["active"], 2);
}

#[tokio::test(flavor = "multi_thread")]
async fn readiness_table_and_csv() {
    let server = MockServer::start().await;
    mount(
        &server,
        2,
        Some(r#"[["/orgs/5/labels/3","/orgs/5/labels/4"]]"#),
    )
    .await;
    let home = home_with("");
    let a = cmd(&server, home.path())
        .args(["readiness", "--app", "shop/prod", "--table"])
        .assert()
        .success();
    let text = String::from_utf8_lossy(&a.get_output().stdout).to_string();
    for want in [
        "readiness · 7d · 1 app",
        "needs rules",
        "97.5%",
        "gaps · shop/prod · 2",
        "unmanaged-nas/-",
        "pending · shop/prod · 2",
    ] {
        assert!(text.contains(want), "{want:?} in {text}");
    }
    let server = MockServer::start().await;
    mount(&server, 1, None).await;
    let a = cmd(&server, home.path())
        .args(["readiness", "--csv"])
        .assert()
        .success();
    let text = String::from_utf8_lossy(&a.get_output().stdout).to_string();
    let mut lines = text.lines();
    assert!(
        lines
            .next()
            .unwrap()
            .starts_with("app,workloads,managed,unmanaged,enforced,"),
        "{text}"
    );
    let row = lines.next().unwrap();
    assert!(row.starts_with("shop/prod,2,2,0,1,0,1,0,"), "{row}");
    assert!(row.contains(",0.9758,2,needs_rules,"), "{row}");
}

#[tokio::test(flavor = "multi_thread")]
async fn readiness_refuses_an_expensive_query_without_yes_before_any_request() {
    let server = MockServer::start().await;
    mount(&server, 0, None).await;
    let home = home_with("");
    for args in [
        vec!["readiness", "--window", "8d"],
        vec!["readiness", "--app", "shop/prod", "--window", "30d"],
        vec!["readiness", "--max-results", "20000"],
    ] {
        let a = cmd(&server, home.path()).args(&args).assert().code(2);
        let err = String::from_utf8_lossy(&a.get_output().stderr).to_string();
        assert!(err.contains("--yes"), "{args:?}: {err}");
    }
    let a = cmd(&server, home.path())
        .args(["readiness", "--app", "shop/prod", "--window", "30d"])
        .assert()
        .code(2);
    assert!(
        String::from_utf8_lossy(&a.get_output().stderr).contains("2 queries"),
        "the guard names the two queries"
    );
    assert!(server.received_requests().await.unwrap().is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn readiness_unknown_app_exits_2_before_any_traffic_request() {
    let server = MockServer::start().await;
    mount(&server, 0, None).await;
    let home = home_with("");
    let a = cmd(&server, home.path())
        .args(["readiness", "--app", "shp/prod"])
        .assert()
        .code(2);
    assert!(String::from_utf8_lossy(&a.get_output().stderr).contains("closest: shop"));
    assert!(
        sequence(&server).await.is_empty(),
        "only the object lists were read"
    );
}
