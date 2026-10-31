//! Compare mode (`--baseline`, #77): the recent query, then the baseline,
//! the same filter over exactly `[start - N, start)`, each pinned by its
//! body; rows marked new, changed, gone or same; `--changes`; `--since`;
//! the dry run; the cost guard per query; truncation per side; marks on
//! both sides. The helpers repeat those of cli.rs (test files share no
//! code without a module).
#[path = "common/schema_check.rs"]
mod schema_check;
use assert_cmd::Command;
use serde_json::{Value, json};
use std::path::Path;
use wiremock::matchers::{body_partial_json, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

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
        .env("API_SECRET", "s3cr3t-compare-test")
        // A time without an offset is local: the tests' zone is fixed.
        .env("TZ", "Europe/Berlin")
        .env("XDG_CONFIG_HOME", home);
    c
}

fn json_out(a: assert_cmd::assert::Assert) -> Value {
    schema_check::envelope(&a.get_output().stdout)
}

fn stderr(a: &assert_cmd::assert::Assert) -> String {
    String::from_utf8_lossy(&a.get_output().stderr).to_string()
}

fn stdout(a: &assert_cmd::assert::Assert) -> String {
    String::from_utf8_lossy(&a.get_output().stdout).to_string()
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

const RECENT_START: &str = "2026-10-10T06:00:00Z";
const RECENT_END: &str = "2026-10-10T08:00:00Z";
/// Seven days before the recent start.
const BASE_START: &str = "2026-10-03T06:00:00Z";

/// One query pinned by its window and cap: exactly `expect` POSTs, its
/// status (`flows_count`: at the cap it is truncated) and its download.
#[allow(clippy::too_many_arguments)]
async fn mount_window(
    server: &MockServer,
    name: &str,
    start: &str,
    end: &str,
    cap: u64,
    flows_count: u64,
    body: &Value,
    expect: u64,
) {
    let href = format!("/orgs/5/traffic_flows/async_queries/{name}");
    Mock::given(method("POST"))
        .and(path("/api/v2/orgs/5/traffic_flows/async_queries"))
        .and(body_partial_json(
            json!({"start_date": start, "end_date": end, "max_results": cap}),
        ))
        .respond_with(
            ResponseTemplate::new(202).set_body_json(json!({"href": href, "status": "queued"})),
        )
        .expect(expect)
        .mount(server)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("/api/v2{href}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "status": "completed",
            "result": {"href": format!("{href}/download")},
            "flows_count": flows_count
        })))
        .mount(server)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("/api/v2{href}/download")))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(body.to_string())
                .insert_header("Content-Type", "application/json"),
        )
        .mount(server)
        .await;
}

fn fixture() -> Vec<Value> {
    serde_json::from_str(include_str!("../../../fixtures/traffic_flows.json")).unwrap()
}

/// The fixture: web → db 5432 allowed, Corp → web 8443 potentially
/// blocked, nas → db icmp blocked, web → FQDN 443 allowed.
fn recent() -> Value {
    Value::Array(fixture())
}

/// The baseline: 5432 the very same record (it spans the cut-off: same),
/// 8443 allowed then (changed), icmp (same), no 443 (new), and a 22/tcp
/// flow not seen since (gone).
fn baseline() -> Value {
    let f = fixture();
    let mut changed = f[1].clone();
    changed["policy_decision"] = json!("allowed");
    let mut gone = f[0].clone();
    gone["service"]["port"] = json!(22);
    gone["service"]["process_name"] = json!("sshd");
    json!([f[0], changed, f[2], gone])
}

const WINDOW: [&str; 4] = ["--from", RECENT_START, "--to", RECENT_END];

/// Both windows mounted at cap 10000, one POST each.
async fn compare_server() -> MockServer {
    let server = MockServer::start().await;
    mount_objects(&server).await;
    mount_window(
        &server,
        "recent",
        RECENT_START,
        RECENT_END,
        10_000,
        4,
        &recent(),
        1,
    )
    .await;
    mount_window(
        &server,
        "base",
        BASE_START,
        RECENT_START,
        10_000,
        4,
        &baseline(),
        1,
    )
    .await;
    server
}

fn changes(out: &Value) -> Vec<(Option<u64>, String, Value)> {
    out["data"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| {
            (
                f["port"].as_u64(),
                f["change"].as_str().unwrap().to_string(),
                f["baseline_decision"].clone(),
            )
        })
        .collect()
}

#[tokio::test(flavor = "multi_thread")]
async fn query_compares_the_recent_window_with_the_baseline_before_it() {
    let server = compare_server().await;
    let home = tempfile::tempdir().unwrap();
    let out = json_out(
        cmd(&server, home.path())
            .args(["traffic", "query"])
            .args(WINDOW)
            .args(["--baseline", "7d"])
            .assert()
            .success(),
    );
    // The default --changes: new, changed, gone; the recent rows first.
    assert_eq!(
        changes(&out),
        vec![
            (Some(8443), "changed".into(), json!("allowed")),
            (Some(443), "new".into(), Value::Null),
            (Some(22), "gone".into(), json!("allowed")),
        ]
    );
    let gone = &out["data"][2];
    assert_eq!(gone["process_name"], "sshd", "the baseline's values");
    let c = &out["meta"]["compare"];
    assert_eq!(c["length"], "7d");
    assert_eq!(c["changes"], "new,changed,gone");
    assert_eq!(c["recent"]["start"], RECENT_START);
    assert_eq!(c["recent"]["end"], RECENT_END);
    assert_eq!(c["baseline"]["start"], BASE_START);
    assert_eq!(
        c["baseline"]["end"], RECENT_START,
        "it ends where recent starts"
    );
    assert_eq!(c["baseline"]["truncated"], false);
    assert_eq!(
        c["counts"],
        json!({"new": 1, "changed": 1, "gone": 1, "same": 2})
    );
    assert_eq!(out["meta"]["queries"], 2);
    assert!(out["meta"]["cost"].as_str().unwrap().contains("baseline"));
    // Name index (4 lists) + 2 queries of POST, status, download.
    assert_eq!(out["meta"]["api_calls"], 4 + 2 * 3);
    server.verify().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn changes_picks_the_rows_and_needs_a_baseline() {
    let server = MockServer::start().await;
    mount_objects(&server).await;
    for (name, start, end, body) in [
        ("recent", RECENT_START, RECENT_END, recent()),
        ("base", BASE_START, RECENT_START, baseline()),
    ] {
        mount_window(&server, name, start, end, 10_000, 4, &body, 2).await;
    }
    let home = tempfile::tempdir().unwrap();
    let out = json_out(
        cmd(&server, home.path())
            .args(["traffic", "query"])
            .args(WINDOW)
            .args(["--baseline", "7d", "--changes", "same"])
            .assert()
            .success(),
    );
    let ports: Vec<Option<u64>> = changes(&out).into_iter().map(|c| c.0).collect();
    assert_eq!(ports, vec![Some(5432), None]);
    assert_eq!(out["meta"]["compare"]["changes"], "same");
    let out = json_out(
        cmd(&server, home.path())
            .args(["traffic", "query"])
            .args(WINDOW)
            .args(["--baseline", "7d", "--changes", "new,changed,gone,same"])
            .assert()
            .success(),
    );
    assert_eq!(out["data"].as_array().unwrap().len(), 5);
    server.verify().await;

    let quiet = MockServer::start().await;
    for args in [
        &["--changes", "new"][..],
        &["--baseline", "7d", "--changes", "old"],
        &["--baseline", "soon"],
    ] {
        let a = cmd(&quiet, home.path())
            .args(["traffic", "query"])
            .args(args)
            .assert()
            .code(2);
        let err = stderr(&a);
        assert!(
            err.contains("--baseline") || err.contains("--changes: unknown change"),
            "{args:?}: {err}"
        );
    }
    assert!(quiet.received_requests().await.unwrap().is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_dry_run_prints_both_bodies_and_since_ends_now() {
    let quiet = MockServer::start().await;
    let home = tempfile::tempdir().unwrap();
    let out = json_out(
        cmd(&quiet, home.path())
            .args(["traffic", "query", "--since", RECENT_START])
            .args(["--baseline", "30d", "--dry-run"])
            .assert()
            .success(),
    );
    let bodies = out["data"].as_array().unwrap();
    assert_eq!(bodies.len(), 2, "the recent body, then the baseline's");
    assert_eq!(bodies[0]["start_date"], RECENT_START);
    let end: jiff::Timestamp = bodies[0]["end_date"].as_str().unwrap().parse().unwrap();
    let ago = jiff::Timestamp::now().duration_since(end);
    assert!(ago.as_secs() < 120, "--since ends now: {end}");
    assert_eq!(bodies[1]["start_date"], "2026-09-10T06:00:00Z");
    assert_eq!(bodies[1]["end_date"], RECENT_START);
    let (mut a, mut b) = (bodies[0].clone(), bodies[1].clone());
    for v in [&mut a, &mut b] {
        v.as_object_mut().unwrap().remove("start_date");
        v.as_object_mut().unwrap().remove("end_date");
    }
    assert_eq!(a, b, "the same filters");
    // --since takes --to too; it is --from's alias, so not both.
    let out = json_out(
        cmd(&quiet, home.path())
            .args(["traffic", "query", "--since", RECENT_START])
            .args(["--to", RECENT_END, "--dry-run"])
            .assert()
            .success(),
    );
    assert_eq!(out["data"]["end_date"], RECENT_END);
    // A date and time without an offset is local (TZ Europe/Berlin, CEST
    // on 2026-10-10); so is a date alone for --since, while --from and
    // --to keep a date in UTC.
    let out = json_out(
        cmd(&quiet, home.path())
            .args(["traffic", "query", "--since", "2026-10-10T08:00"])
            .args(["--to", "2026-10-10T10:00", "--dry-run"])
            .assert()
            .success(),
    );
    assert_eq!(out["data"]["start_date"], RECENT_START);
    assert_eq!(out["data"]["end_date"], RECENT_END);
    let out = json_out(
        cmd(&quiet, home.path())
            .args(["traffic", "query", "--since", "2026-10-10", "--dry-run"])
            .assert()
            .success(),
    );
    assert_eq!(out["data"]["start_date"], "2026-10-09T22:00:00Z");
    let out = json_out(
        cmd(&quiet, home.path())
            .args([
                "traffic",
                "query",
                "--from",
                "2026-10-10",
                "--to",
                "2026-10-10",
            ])
            .arg("--dry-run")
            .assert()
            .success(),
    );
    assert_eq!(out["data"]["start_date"], "2026-10-10T00:00:00Z");
    assert_eq!(out["data"]["end_date"], "2026-10-10T23:59:59Z");
    cmd(&quiet, home.path())
        .args(["traffic", "query", "--since", RECENT_START])
        .args(WINDOW)
        .assert()
        .code(2);
    assert!(quiet.received_requests().await.unwrap().is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn the_cost_guard_judges_the_baseline_on_its_own_and_yes_covers_both() {
    let quiet = MockServer::start().await;
    let home = tempfile::tempdir().unwrap();
    let a = cmd(&quiet, home.path())
        .args(["traffic", "query"])
        .args(WINDOW)
        .args(["--baseline", "30d"])
        .assert()
        .code(2);
    let err = stderr(&a);
    assert!(err.contains("expensive baseline query"), "{err}");
    assert!(err.contains("30d"), "{err}");
    assert!(quiet.received_requests().await.unwrap().is_empty());

    let server = MockServer::start().await;
    mount_objects(&server).await;
    mount_window(
        &server,
        "recent",
        RECENT_START,
        RECENT_END,
        10_000,
        4,
        &recent(),
        1,
    )
    .await;
    mount_window(
        &server,
        "base",
        "2026-09-10T06:00:00Z",
        RECENT_START,
        10_000,
        4,
        &baseline(),
        1,
    )
    .await;
    let out = json_out(
        cmd(&server, home.path())
            .args(["traffic", "query"])
            .args(WINDOW)
            .args(["--baseline", "30d", "--yes"])
            .assert()
            .success(),
    );
    assert_eq!(out["meta"]["compare"]["length"], "30d");
    server.verify().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_truncated_baseline_warns_and_backfill_raises_both_sides() {
    let server = MockServer::start().await;
    mount_objects(&server).await;
    mount_window(
        &server,
        "recent",
        RECENT_START,
        RECENT_END,
        4,
        4,
        &recent(),
        1,
    )
    .await;
    mount_window(
        &server,
        "base",
        BASE_START,
        RECENT_START,
        4,
        4,
        &baseline(),
        1,
    )
    .await;
    let home = tempfile::tempdir().unwrap();
    let a = cmd(&server, home.path())
        .args(["traffic", "query"])
        .args(WINDOW)
        .args(["--baseline", "7d", "--max-results", "4"])
        .assert()
        .success();
    let err = stderr(&a);
    assert!(
        err.contains("warning: baseline truncated, capped at 4: new/gone over-report"),
        "{err}"
    );
    assert!(err.contains("warning: recent truncated"), "{err}");
    let out = json_out(a);
    assert_eq!(out["meta"]["compare"]["baseline"]["truncated"], true);
    assert_eq!(out["meta"]["compare"]["recent"]["truncated"], true);
    let warnings = out["meta"]["warnings"].as_array().unwrap();
    assert_eq!(warnings.len(), 2, "{warnings:?}");
    server.verify().await;

    // --backfill: each side climbs on its own.
    let server = MockServer::start().await;
    mount_objects(&server).await;
    mount_window(&server, "r4", RECENT_START, RECENT_END, 4, 3, &recent(), 1).await;
    mount_window(
        &server,
        "b4",
        BASE_START,
        RECENT_START,
        4,
        4,
        &baseline(),
        1,
    )
    .await;
    mount_window(
        &server,
        "b1",
        BASE_START,
        RECENT_START,
        10_000,
        4,
        &baseline(),
        1,
    )
    .await;
    let a = cmd(&server, home.path())
        .args(["traffic", "query"])
        .args(WINDOW)
        .args(["--baseline", "7d", "--max-results", "4", "--backfill"])
        .assert()
        .success();
    let err = stderr(&a);
    assert!(
        err.contains("baseline backfill raises the cap to 10000"),
        "{err}"
    );
    assert!(!err.contains("warning"), "{err}");
    let out = json_out(a);
    assert_eq!(out["meta"]["compare"]["baseline"]["cap"], 10_000);
    assert_eq!(out["meta"]["compare"]["recent"]["cap"], 4);
    // Each side's cap steps are counted: 1 recent, 2 baseline.
    assert_eq!(
        out["meta"]["compare"]["baseline"]["cap_steps"],
        json!([4, 10_000])
    );
    assert_eq!(out["meta"]["compare"]["recent"]["cap_steps"], json!([4]));
    assert_eq!(out["meta"]["queries"], 3);
    let cost = out["meta"]["cost"].as_str().unwrap();
    assert!(
        cost.starts_with("3 Explorer queries: 1 for the recent window, 2 for the baseline"),
        "{cost}"
    );
    server.verify().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn groups_mesh_and_sections_carry_the_change_and_csv_has_its_columns() {
    let server = MockServer::start().await;
    mount_objects(&server).await;
    for (name, start, end, body) in [
        ("recent", RECENT_START, RECENT_END, recent()),
        ("base", BASE_START, RECENT_START, baseline()),
    ] {
        mount_window(&server, name, start, end, 10_000, 4, &body, 5).await;
    }
    let home = tempfile::tempdir().unwrap();
    let run = |extra: &[&str]| {
        cmd(&server, home.path())
            .args(["traffic"])
            .args(extra)
            .args(WINDOW)
            .args(["--baseline", "7d", "--keys", "app,env"])
            .assert()
            .success()
    };
    // Pairs by service: shop/prod → shop/prod 22/tcp is gone; Corp →
    // shop/prod 8443 changed; the FQDN 443 new.
    let out = json_out(run(&["groups", "--services"]));
    let rows: Vec<(String, String)> = out["data"]
        .as_array()
        .unwrap()
        .iter()
        .map(|g| {
            (
                g["service"].as_str().unwrap().to_string(),
                g["change"].as_str().unwrap().to_string(),
            )
        })
        .collect();
    assert!(rows.contains(&("22/tcp".into(), "gone".into())), "{rows:?}");
    assert!(
        rows.contains(&("8443/tcp".into(), "changed".into())),
        "{rows:?}"
    );
    assert!(rows.contains(&("443/tcp".into(), "new".into())), "{rows:?}");
    assert_eq!(rows.len(), 3, "same rows left out: {rows:?}");
    // Sections per destination app.
    let out = json_out(run(&["groups", "--by", "in"]));
    let peers: Vec<String> = out["data"]["sections"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|s| s["rows"].as_array().unwrap().clone())
        .map(|r| {
            format!(
                "{} {}",
                r["service"].as_str().unwrap(),
                r["change"].as_str().unwrap()
            )
        })
        .collect();
    assert!(peers.contains(&"22/tcp gone".to_string()), "{peers:?}");
    // The mesh.
    let out = json_out(run(&["mesh"]));
    let mesh: Vec<String> = out["data"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["change"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(mesh.len(), 3, "{mesh:?}");
    // CSV: the two columns at the end.
    let a = run(&["groups", "--services", "--csv"]);
    let text = stdout(&a);
    let header = text.lines().next().unwrap();
    assert!(header.ends_with(",change,baseline_decision"), "{header}");
    assert!(
        text.lines().any(|l| l.ends_with(",changed,allowed")),
        "{text}"
    );
    // --fields picks them.
    let a = run(&[
        "groups",
        "--services",
        "--csv",
        "--fields",
        "service,change",
    ]);
    let text = stdout(&a);
    assert_eq!(text.lines().next(), Some("service,change"));
    assert!(text.lines().any(|l| l == "22/tcp,gone"), "{text}");
    server.verify().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn squelched_flows_are_left_out_of_both_sides_and_never_gone() {
    let server = compare_server().await;
    let home = tempfile::tempdir().unwrap();
    let dir = home.path().join("illumio-tui");
    std::fs::create_dir_all(&dir).unwrap();
    // The 22/tcp flow (only in the baseline) and the Corp flow (changed)
    // are squelched: neither counts.
    std::fs::write(
        dir.join("marks.toml"),
        r#"
[[mark]]
id = "m-sq22"
action = "squelch"
src = "any"
dst = "any"
service = "22/tcp"

[[mark]]
id = "m-sqcorp"
action = "squelch"
src = { ip_list = "Corp" }
dst = "any"
"#,
    )
    .unwrap();
    let out = json_out(
        cmd(&server, home.path())
            .args(["traffic", "query"])
            .args(WINDOW)
            .args(["--baseline", "7d"])
            .assert()
            .success(),
    );
    assert_eq!(
        out["meta"]["compare"]["counts"],
        json!({"new": 1, "changed": 0, "gone": 0, "same": 2})
    );
    server.verify().await;
}

/// A decision filter picks rows after each key is judged on every
/// decision: the 8443 flow, allowed then and potentially blocked now, is
/// `changed` in the blocked sections, not new.
#[tokio::test(flavor = "multi_thread")]
async fn show_blocked_reports_a_decision_change_as_changed() {
    let server = MockServer::start().await;
    mount_objects(&server).await;
    for (name, start, end, body) in [
        ("recent", RECENT_START, RECENT_END, recent()),
        ("base", BASE_START, RECENT_START, baseline()),
    ] {
        mount_window(&server, name, start, end, 10_000, 4, &body, 2).await;
    }
    let home = tempfile::tempdir().unwrap();
    for argv in [
        &["groups", "--by", "in", "--show", "blocked"][..],
        &["query", "--blocked"],
    ] {
        let out = json_out(
            cmd(&server, home.path())
                .arg("traffic")
                .args(argv)
                .args(WINDOW)
                .args(["--baseline", "7d", "--changes", "new,changed"])
                .assert()
                .success(),
        );
        let rows: Vec<(String, String, Value)> = out["data"]["sections"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|s| s["rows"].as_array().unwrap().clone())
            .map(|r| {
                (
                    r["service"].as_str().unwrap().to_string(),
                    r["change"].as_str().unwrap().to_string(),
                    r["baseline_decision"].clone(),
                )
            })
            .collect();
        assert_eq!(
            rows,
            vec![("8443/tcp".into(), "changed".into(), json!("allowed"))],
            "{argv:?}"
        );
    }
    server.verify().await;
}

/// Plain pairs compare their service keys: shop/prod → shop/prod keeps
/// 5432 and lost 22 (gone), so the pair is changed; the counts are the
/// pair rows' (#79).
#[tokio::test(flavor = "multi_thread")]
async fn a_plain_pair_is_judged_by_its_service_keys() {
    let server = compare_server().await;
    let home = tempfile::tempdir().unwrap();
    let out = json_out(
        cmd(&server, home.path())
            .args(["traffic", "groups", "--keys", "app,env"])
            .args(WINDOW)
            .args(["--baseline", "7d", "--changes", "new,changed,gone,same"])
            .assert()
            .success(),
    );
    let pair = out["data"]
        .as_array()
        .unwrap()
        .iter()
        .find(|g| g["src"] == json!(["shop", "prod"]) && g["dst"] == json!(["shop", "prod"]))
        .unwrap()
        .clone();
    assert_eq!(pair["change"], "changed");
    assert_eq!(pair["baseline_decision"], "allowed");
    // shop/prod → shop/prod changed (22 gone), Corp → shop/prod changed
    // (8443), nas icmp the same, the FQDN 443 new: pair rows.
    assert_eq!(
        out["meta"]["compare"]["counts"],
        json!({"new": 1, "changed": 2, "gone": 0, "same": 1})
    );
    server.verify().await;
}

/// #79: `meta.compare.counts` counts the rows printed: pair rows without
/// `--services`, service rows with it, under every `--changes`.
#[tokio::test(flavor = "multi_thread")]
async fn groups_counts_equal_the_rows_printed_with_and_without_services() {
    let server = MockServer::start().await;
    mount_objects(&server).await;
    let sets = ["new", "changed", "gone", "new,changed,gone,same"];
    let runs = 2 * sets.len() as u64;
    for (name, start, end, body) in [
        ("recent", RECENT_START, RECENT_END, recent()),
        ("base", BASE_START, RECENT_START, baseline()),
    ] {
        mount_window(&server, name, start, end, 10_000, 4, &body, runs).await;
    }
    let home = tempfile::tempdir().unwrap();
    for services in [false, true] {
        let mut first: Option<Value> = None;
        for set in sets {
            let mut c = cmd(&server, home.path());
            c.args(["traffic", "groups", "--keys", "app,env"])
                .args(WINDOW)
                .args(["--baseline", "7d", "--changes", set]);
            if services {
                c.arg("--services");
            }
            let out = json_out(c.assert().success());
            let counts = out["meta"]["compare"]["counts"].clone();
            let what = format!("services={services} --changes {set}");
            // `--changes` picks rows, never the counts.
            match &first {
                Some(f) => assert_eq!(&counts, f, "{what}"),
                None => first = Some(counts.clone()),
            }
            let rows = out["data"].as_array().unwrap();
            for change in ["new", "changed", "gone", "same"] {
                let printed = rows.iter().filter(|r| r["change"] == change).count() as u64;
                let want = if set.split(',').any(|s| s == change) {
                    counts[change].as_u64().unwrap()
                } else {
                    0
                };
                assert_eq!(printed, want, "{what}: {change}");
            }
        }
        let want = if services {
            // 5432 same, 8443 changed, icmp same, 443 new, 22 gone.
            json!({"new": 1, "changed": 1, "gone": 1, "same": 2})
        } else {
            json!({"new": 1, "changed": 2, "gone": 0, "same": 1})
        };
        assert_eq!(first.unwrap(), want, "services={services}");
    }
    server.verify().await;
}

/// #90: `--app SHOP/Prod` finds the app `shop/prod`: the same two queries
/// (a side each) with the labels' hrefs, and the flows kept by the PCE's
/// spelling.
#[tokio::test(flavor = "multi_thread")]
async fn app_in_another_case_sends_the_labels_and_keeps_the_flows() {
    let server = MockServer::start().await;
    mount_objects(&server).await;
    mount_window(
        &server,
        "recent",
        RECENT_START,
        RECENT_END,
        10_000,
        4,
        &recent(),
        4,
    )
    .await;
    let home = tempfile::tempdir().unwrap();
    let run = |app: &str| {
        json_out(
            cmd(&server, home.path())
                .args(["traffic", "query", "--group", "app,env", "--app", app])
                .args(WINDOW)
                .assert()
                .success(),
        )
    };
    let exact = run("shop/prod");
    assert!(!exact["data"].as_array().unwrap().is_empty());
    let posts = |v: Vec<wiremock::Request>| -> Vec<Value> {
        v.iter()
            .filter(|r| r.method.as_str() == "POST")
            .map(|r| serde_json::from_slice(&r.body).unwrap())
            .collect()
    };
    let first = posts(server.received_requests().await.unwrap());
    assert_eq!(first.len(), 2);
    let other = run("SHOP/Prod");
    assert_eq!(other["data"], exact["data"]);
    let all = posts(server.received_requests().await.unwrap());
    assert_eq!(all.len(), 4);
    for i in 0..2 {
        assert_eq!(all[i]["sources"], all[i + 2]["sources"]);
        assert_eq!(all[i]["destinations"], all[i + 2]["destinations"]);
    }
    server.verify().await;
}

/// #90: `--src app=WEB` with two labels `Web` and `web` is exit 2 naming
/// both, before any traffic request.
#[tokio::test(flavor = "multi_thread")]
async fn an_ambiguous_label_in_src_is_exit_2_and_sends_no_traffic_query() {
    let server = MockServer::start().await;
    mount_objects(&server).await;
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
        .with_priority(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/v2/orgs/5/traffic_flows/async_queries"))
        .respond_with(ResponseTemplate::new(500))
        .expect(0)
        .mount(&server)
        .await;
    let home = tempfile::tempdir().unwrap();
    for side in ["--src", "--dst"] {
        let a = cmd(&server, home.path())
            .args(["traffic", "query", side, "app=WEB"])
            .args(WINDOW)
            .assert()
            .code(2);
        assert!(
            stderr(&a).contains("app=WEB matches app=Web and app=web; write one exactly"),
            "{}",
            stderr(&a)
        );
    }
    server.verify().await;
}

/// `--app` with `--baseline`: one query per side for each window, 4 POSTs,
/// pinned; `meta.queries` counts them; the baseline guard names the runs.
#[tokio::test(flavor = "multi_thread")]
async fn app_and_baseline_run_one_query_per_side_for_each_window() {
    let server = MockServer::start().await;
    mount_objects(&server).await;
    mount_window(
        &server,
        "recent",
        RECENT_START,
        RECENT_END,
        10_000,
        4,
        &recent(),
        2,
    )
    .await;
    mount_window(
        &server,
        "base",
        BASE_START,
        RECENT_START,
        10_000,
        4,
        &baseline(),
        2,
    )
    .await;
    let home = tempfile::tempdir().unwrap();
    let out = json_out(
        cmd(&server, home.path())
            .args([
                "traffic",
                "query",
                "--group",
                "app,env",
                "--app",
                "shop/prod",
            ])
            .args(WINDOW)
            .args(["--baseline", "7d"])
            .assert()
            .success(),
    );
    assert_eq!(out["meta"]["queries"], 4);
    assert_eq!(out["meta"]["compare"]["recent"]["queries"], 2);
    assert_eq!(out["meta"]["compare"]["baseline"]["queries"], 2);
    let posts: Vec<Value> = server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .filter(|r| r.method.as_str() == "POST")
        .map(|r| serde_json::from_slice(&r.body).unwrap())
        .collect();
    assert_eq!(posts.len(), 4);
    for p in &posts[2..] {
        assert_eq!(p["start_date"], BASE_START);
        assert_eq!(p["end_date"], RECENT_START);
    }
    // The same labels on the same side in both windows.
    for i in 0..2 {
        assert_eq!(posts[i]["sources"], posts[i + 2]["sources"]);
        assert_eq!(posts[i]["destinations"], posts[i + 2]["destinations"]);
    }
    server.verify().await;

    let quiet = MockServer::start().await;
    let a = cmd(&quiet, home.path())
        .args(["traffic", "query", "--app", "shop/prod"])
        .args(WINDOW)
        .args(["--baseline", "30d"])
        .assert()
        .code(2);
    let err = stderr(&a);
    assert!(
        err.contains("expensive baseline query") && err.contains("(--app runs it as 2 queries)"),
        "{err}"
    );
}

/// #80: plain pairs carry `change_detail`, what changed on a changed pair
/// (shop/prod → shop/prod lost 22, Corp → shop/prod's 8443 went allowed →
/// potentially blocked), empty on the others; in JSON (schema-checked),
/// CSV and the table. Pairs by service have none.
#[tokio::test(flavor = "multi_thread")]
async fn plain_pairs_carry_what_changed_in_change_detail() {
    let server = MockServer::start().await;
    mount_objects(&server).await;
    for (name, start, end, body) in [
        ("recent", RECENT_START, RECENT_END, recent()),
        ("base", BASE_START, RECENT_START, baseline()),
    ] {
        mount_window(&server, name, start, end, 10_000, 4, &body, 4).await;
    }
    let home = tempfile::tempdir().unwrap();
    let run = |extra: &[&str]| {
        cmd(&server, home.path())
            .args(["traffic", "groups"])
            .args(extra)
            .args(WINDOW)
            .args([
                "--baseline",
                "7d",
                "--keys",
                "app,env",
                "--changes",
                "new,changed,gone,same",
            ])
            .assert()
            .success()
    };
    let out = json_out(run(&[]));
    let mut rows: Vec<(String, String, String)> = out["data"]
        .as_array()
        .unwrap()
        .iter()
        .map(|g| {
            (
                g["src"].as_array().unwrap()[0]
                    .as_str()
                    .unwrap()
                    .to_string(),
                g["change"].as_str().unwrap().to_string(),
                g["change_detail"].to_string(),
            )
        })
        .collect();
    rows.sort();
    let detail = |change: &str| -> Vec<String> {
        rows.iter()
            .filter(|r| r.1 == change)
            .map(|r| r.2.clone())
            .collect()
    };
    let mut changed = detail("changed");
    changed.sort();
    assert_eq!(changed, vec!["\"allow→pot.bl\"", "\"−1 svc\""], "{rows:?}");
    // Fix round 1: null on every other pair row.
    for other in ["new", "same"] {
        let d = detail(other);
        assert!(
            !d.is_empty() && d.iter().all(|d| d == "null"),
            "{other}: {rows:?}"
        );
    }
    // Pairs by service: no change_detail.
    let out = json_out(run(&["--services"]));
    assert!(
        out["data"]
            .as_array()
            .unwrap()
            .iter()
            .all(|g| g.get("change_detail").is_none()),
        "{out}"
    );
    // CSV: last, after baseline_decision.
    let a = run(&["--csv"]);
    let text = stdout(&a);
    let header = text.lines().next().unwrap();
    assert!(
        header.ends_with(",change,baseline_decision,change_detail"),
        "{header}"
    );
    assert!(
        text.lines().any(|l| l.ends_with(",changed,allowed,−1 svc")),
        "{text}"
    );
    // Empty on the other rows: the new pair's, the same ones'.
    assert!(text.lines().any(|l| l.ends_with(",new,,")), "{text}");
    assert!(
        text.lines().any(|l| l.ends_with(",same,blocked,")),
        "{text}"
    );
    // The table.
    let a = cmd(&server, home.path())
        .args(["traffic", "groups"])
        .args(WINDOW)
        .args(["--baseline", "7d", "--keys", "app,env", "--table"])
        .assert()
        .success();
    let text = stdout(&a);
    assert!(text.contains("change_detail"), "{text}");
    assert!(
        text.contains("allow→pot.bl") && text.contains("−1 svc"),
        "{text}"
    );
    server.verify().await;
}

/// What `gen` reads besides the traffic: the draft rulesets.
async fn mount_rule_sets(server: &MockServer) {
    Mock::given(method("GET"))
        .and(path("/api/v2/orgs/5/sec_policy/draft/rule_sets"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(include_str!("../../../fixtures/rule_sets.json"))
                .insert_header("Content-Type", "application/json"),
        )
        .mount(server)
        .await;
}

fn peers(out: &Value) -> Vec<String> {
    out["data"]["rows"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["peer"].as_str().unwrap().to_string())
        .collect()
}

/// `gen --baseline` (#81): the app's `in` query for the recent window,
/// then for the baseline, one POST each; only the new keys feed the rows
/// and rules, the rest count as `seen_before`; `--changes new,changed`
/// feeds a key whose decision changed too.
#[tokio::test(flavor = "multi_thread")]
async fn gen_baseline_proposes_only_for_the_new_traffic() {
    let server = compare_server().await;
    mount_rule_sets(&server).await;
    let home = tempfile::tempdir().unwrap();
    let out = json_out(
        cmd(&server, home.path())
            .args(["gen", "shop/prod"])
            .args(WINDOW)
            .args(["--baseline", "7d"])
            .assert()
            .success(),
    );
    // Inbound: 5432 and icmp the same, 8443 changed: nothing is new.
    assert_eq!(peers(&out), Vec::<String>::new(), "rows without flows go");
    assert_eq!(out["data"]["rules"], json!([]));
    let p = &out["data"]["preview"];
    assert_eq!(p["seen_before_flows"], 3, "left out, not uncovered");
    assert_eq!(p["uncovered_flows"], 0);
    assert_eq!(p["covered_flows"], 0);
    let c = &out["data"]["compare"];
    assert_eq!(c["length"], "7d");
    assert_eq!(c["changes"], "new");
    assert_eq!(c["seen_before"], 3);
    assert_eq!(c["recent"]["start"], RECENT_START);
    assert_eq!(c["baseline"]["start"], BASE_START);
    assert_eq!(c["baseline"]["end"], RECENT_START);
    assert_eq!(c["baseline"]["queries"], 1);
    assert_eq!(
        c["counts"],
        json!({"new": 0, "changed": 1, "gone": 1, "same": 2})
    );
    assert_eq!(out["meta"]["queries"], 2, "both sides counted");
    assert!(
        out["meta"]["cost"]
            .as_str()
            .unwrap()
            .starts_with("2 Explorer queries: 1 for the recent window, 1 for the baseline"),
        "{}",
        out["meta"]["cost"]
    );
    assert!(out["meta"].get("compare").is_none(), "data.compare only");
    server.verify().await;

    let server = compare_server().await;
    mount_rule_sets(&server).await;
    let out = json_out(
        cmd(&server, home.path())
            .args(["--no-cache", "gen", "shop/prod"])
            .args(WINDOW)
            .args(["--baseline", "7d", "--changes", "new,changed"])
            .assert()
            .success(),
    );
    assert_eq!(peers(&out), vec!["IP list Corp · 1 IP"], "8443 changed");
    assert_eq!(out["data"]["preview"]["seen_before_flows"], 2);
    assert_eq!(out["data"]["compare"]["changes"], "new,changed");
    server.verify().await;

    // `--table`: the new-since line on stderr; `changed,new` in either order.
    let server = compare_server().await;
    mount_rule_sets(&server).await;
    let a = cmd(&server, home.path())
        .args(["--no-cache", "gen", "shop/prod", "--table"])
        .args(WINDOW)
        .args(["--baseline", "7d", "--changes", "changed,new"])
        .assert()
        .success();
    let err = stderr(&a);
    assert!(
        err.contains("new since 7d before: 2 flows seen before"),
        "{err}"
    );
    server.verify().await;
    let server = compare_server().await;
    mount_rule_sets(&server).await;
    let a = cmd(&server, home.path())
        .args(["--no-cache", "gen", "shop/prod", "--csv"])
        .args(WINDOW)
        .args(["--baseline", "7d"])
        .assert()
        .success();
    let err = stderr(&a);
    assert!(
        err.contains("nothing new since 7d before: 3 flows seen before"),
        "{err}"
    );
    server.verify().await;
}

/// A capped recent side exits 2 before the baseline is sent.
#[tokio::test(flavor = "multi_thread")]
async fn gen_on_a_capped_recent_side_sends_no_baseline() {
    let server = MockServer::start().await;
    mount_objects(&server).await;
    mount_rule_sets(&server).await;
    mount_window(
        &server,
        "recent",
        RECENT_START,
        RECENT_END,
        4,
        4,
        &recent(),
        1,
    )
    .await;
    mount_window(
        &server,
        "base",
        BASE_START,
        RECENT_START,
        4,
        4,
        &baseline(),
        0,
    )
    .await;
    let home = tempfile::tempdir().unwrap();
    let a = cmd(&server, home.path())
        .args(["gen", "shop/prod", "--max-results", "4"])
        .args(WINDOW)
        .args(["--baseline", "7d"])
        .assert()
        .code(2);
    assert!(stderr(&a).contains("traffic capped at 4 for shop/prod"));
    assert!(a.get_output().stdout.is_empty());
    server.verify().await;
}

/// `--changes` needs `--baseline`, and takes new or new,changed only:
/// exit 2 before any request.
#[tokio::test(flavor = "multi_thread")]
async fn gen_changes_without_a_baseline_or_with_gone_exits_2_unsent() {
    let quiet = MockServer::start().await;
    let home = tempfile::tempdir().unwrap();
    for args in [
        &["--changes", "new"][..],
        &["--baseline", "7d", "--changes", "new,gone"],
        &["--baseline", "7d", "--changes", "same"],
        &["--baseline", "7d", "--changes", "changed"],
        &["--baseline", "soon"],
    ] {
        let a = cmd(&quiet, home.path())
            .args(["gen", "shop/prod"])
            .args(WINDOW)
            .args(args)
            .assert()
            .code(2);
        let err = stderr(&a);
        assert!(
            err.contains("--baseline") || err.contains("--changes"),
            "{args:?}: {err}"
        );
    }
    assert!(quiet.received_requests().await.unwrap().is_empty());
    // `--fields` reaches the new fields through the schema.
    let wanted = ["compare.seen_before", "preview.seen_before_flows"].map(String::from);
    assert!(illumio_tui::cli::schema::check_fields("gen", None, &wanted).is_ok());
}

/// A truncated baseline warns (new over-reports) and still proposes;
/// `--outbound` runs both sides of both windows.
#[tokio::test(flavor = "multi_thread")]
async fn gen_warns_on_a_truncated_baseline_and_outbound_runs_four_queries() {
    let server = MockServer::start().await;
    mount_objects(&server).await;
    mount_rule_sets(&server).await;
    mount_window(
        &server,
        "recent",
        RECENT_START,
        RECENT_END,
        4,
        3,
        &recent(),
        2,
    )
    .await;
    mount_window(
        &server,
        "base",
        BASE_START,
        RECENT_START,
        4,
        4,
        &baseline(),
        2,
    )
    .await;
    let home = tempfile::tempdir().unwrap();
    let a = cmd(&server, home.path())
        .args(["gen", "shop/prod", "--outbound", "--max-results", "4"])
        .args(WINDOW)
        .args(["--baseline", "7d"])
        .assert()
        .success();
    let err = stderr(&a);
    assert!(
        err.contains("warning: baseline truncated, capped at 4: new over-reports"),
        "{err}"
    );
    let out = json_out(a);
    assert_eq!(out["meta"]["queries"], 4);
    assert_eq!(out["data"]["compare"]["baseline"]["truncated"], true);
    assert_eq!(
        peers(&out),
        vec!["93.184.216.34"],
        "only the outbound 443 is new"
    );
    assert!(
        out["meta"]["warnings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|w| w.as_str().unwrap().contains("new over-reports"))
    );
    server.verify().await;
}
