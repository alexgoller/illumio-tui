//! Traffic commands apply marks.toml: squelched records left out (or kept
//! with `--show-squelched`), `mark` on rows, `meta.squelched`, an invalid
//! file a warning; `gen` only counts them. The 4-flow fixture through a
//! mock PCE, marks in a temp `XDG_CONFIG_HOME`. The helpers repeat those of
//! cli.rs (test files share no code without a module).
#[path = "common/schema_check.rs"]
mod schema_check;
use assert_cmd::Command;
use serde_json::{Value, json};
use std::path::Path;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const QHREF: &str = "/orgs/5/traffic_flows/async_queries/q-1";

/// The fixture's flows: web → db 5432 (allowed), Corp → web 8443
/// (potentially blocked), unmanaged-nas → db icmp (blocked), web → an
/// FQDN 443 (allowed). Corp is squelched, both flows into db are
/// critical, 443 is reviewed.
const MARKS: &str = r#"
[[mark]]
id = "m-sqcorp"
action = "squelch"
src = { ip_list = "Corp" }
dst = "any"
comment = "scanner"

[[mark]]
id = "m-crit01"
action = "critical"
src = "any"
dst = { ip = "10.0.2.20" }
comment = "the database"

[[mark]]
id = "m-rev443"
action = "reviewed"
src = "any"
dst = "any"
service = "443/tcp"
comment = "egress, known"
"#;

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
        .env("API_SECRET", "s3cr3t-marks-test")
        .env("XDG_CONFIG_HOME", home);
    c
}

fn home_with(marks: &str) -> tempfile::TempDir {
    let home = tempfile::tempdir().unwrap();
    let dir = home.path().join("illumio-tui");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("marks.toml"), marks).unwrap();
    home
}

fn json_out(a: assert_cmd::assert::Assert) -> Value {
    schema_check::envelope(&a.get_output().stdout)
}

/// The object lists, the traffic query (any POST), the draft rulesets.
async fn mount(server: &MockServer) {
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
    Mock::given(method("POST"))
        .and(path("/api/v2/orgs/5/traffic_flows/async_queries"))
        .respond_with(
            ResponseTemplate::new(202).set_body_json(json!({"href": QHREF, "status": "queued"})),
        )
        .mount(server)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("/api/v2{QHREF}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "status": "completed",
            "result": {"href": format!("{QHREF}/download")},
            "matches_count": 4
        })))
        .mount(server)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("/api/v2{QHREF}/download")))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(include_str!("../../../fixtures/traffic_flows.json"))
                .insert_header("Content-Type", "application/json"),
        )
        .mount(server)
        .await;
}

async fn requests(server: &MockServer) -> usize {
    server.received_requests().await.unwrap().len()
}

#[tokio::test(flavor = "multi_thread")]
async fn traffic_query_leaves_out_squelched_flows_and_marks_the_rest() {
    let server = MockServer::start().await;
    mount(&server).await;
    let home = home_with(MARKS);
    let q = ["traffic", "query", "--window", "1h"];
    let out = json_out(cmd(&server, home.path()).args(q).assert().success());
    let marked_run = requests(&server).await;
    let flows = out["data"].as_array().unwrap();
    assert_eq!(flows.len(), 3, "Corp's flow is squelched");
    let marks: Vec<&Value> = flows.iter().map(|f| &f["mark"]).collect();
    assert_eq!(
        marks,
        vec![
            &json!({"action": "critical", "id": "m-crit01"}),
            &json!({"action": "critical", "id": "m-crit01"}),
            &json!({"action": "reviewed", "id": "m-rev443"}),
        ]
    );
    assert_eq!(
        flows[0]["src"]["hostname"], "host-1.example",
        "the flow itself, flattened"
    );
    assert_eq!(out["meta"]["squelched"], json!({"rows": 1, "flows": 1}));
    assert_eq!(out["meta"]["marks"]["active"], 3);
    assert!(
        out["meta"]["marks"]["file"]
            .as_str()
            .unwrap()
            .ends_with("marks.toml")
    );

    let shown = json_out(
        cmd(&server, home.path())
            .args(q)
            .arg("--show-squelched")
            .assert()
            .success(),
    );
    let flows = shown["data"].as_array().unwrap();
    assert_eq!(flows.len(), 4);
    assert_eq!(
        flows[1]["mark"],
        json!({"action": "squelch", "id": "m-sqcorp"})
    );
    assert_eq!(shown["meta"]["squelched"], json!({"rows": 1, "flows": 1}));

    let before = requests(&server).await;
    let off = json_out(
        cmd(&server, home.path())
            .args(q)
            .arg("--no-marks")
            .assert()
            .success(),
    );
    assert_eq!(
        requests(&server).await - before,
        marked_run,
        "marks add no request"
    );
    assert_eq!(off["data"].as_array().unwrap().len(), 4);
    assert!(off["data"][0].get("mark").is_none());
    assert!(off["meta"].get("squelched").is_none() && off["meta"].get("marks").is_none());

    let table = cmd(&server, home.path())
        .args(q)
        .arg("--table")
        .assert()
        .success();
    let text = String::from_utf8_lossy(&table.get_output().stdout).to_string();
    let header = text.lines().nth(1).unwrap();
    assert!(header.trim_end().ends_with("mark"), "{header}");
    assert!(text.contains("critical"), "{text}");
    let err = String::from_utf8_lossy(&table.get_output().stderr).to_string();
    assert!(err.contains("1 rows (1 flows) squelched"), "{err}");
}

#[tokio::test(flavor = "multi_thread")]
async fn traffic_groups_by_in_omits_squelched_peer_rows_and_counts_them() {
    let server = MockServer::start().await;
    mount(&server).await;
    let home = home_with(MARKS);
    let out = json_out(
        cmd(&server, home.path())
            .args(["traffic", "groups", "--window", "1h", "--by", "in"])
            .assert()
            .success(),
    );
    let shop = &out["data"]["sections"][0];
    assert_eq!(shop["app"], json!(["shop", "prod"]));
    let rows = shop["rows"].as_array().unwrap();
    assert_eq!(rows.len(), 2, "Corp's 8443 row is gone");
    assert!(rows.iter().all(|r| r["peer"] != json!(["ipl:Corp", "-"])));
    for r in rows {
        assert_eq!(r["mark"]["action"], "critical", "{r}");
        assert_eq!(r["mark"]["id"], "m-crit01");
        assert_eq!(r["marks"], json!({"critical": 1}));
    }
    assert_eq!(out["meta"]["squelched"], json!({"rows": 1, "flows": 1}));
    // Pairs, by service: the same squelched record, its own row gone.
    let pairs = json_out(
        cmd(&server, home.path())
            .args(["traffic", "groups", "--window", "1h", "--services"])
            .assert()
            .success(),
    );
    assert_eq!(pairs["data"].as_array().unwrap().len(), 3);
    assert_eq!(pairs["meta"]["squelched"], json!({"rows": 1, "flows": 1}));
    let mesh = json_out(
        cmd(&server, home.path())
            .args(["traffic", "mesh", "--window", "1h"])
            .assert()
            .success(),
    );
    assert_eq!(mesh["data"].as_array().unwrap().len(), 3);
    assert_eq!(mesh["meta"]["squelched"]["flows"], 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn an_active_file_without_hits_reports_zeros_and_an_invalid_one_warns() {
    let server = MockServer::start().await;
    mount(&server).await;
    let none = home_with(
        "[[mark]]\naction = \"squelch\"\nsrc = { ip_list = \"Nowhere\" }\ndst = \"any\"\ncomment = \"x\"\n",
    );
    let out = json_out(
        cmd(&server, none.path())
            .args(["traffic", "query", "--window", "1h"])
            .assert()
            .success(),
    );
    assert_eq!(out["meta"]["squelched"], json!({"rows": 0, "flows": 0}));
    assert_eq!(out["data"].as_array().unwrap().len(), 4);

    let bad = home_with("[[mark]]\naction = \"squelch\"\nsrc = \"any\"\n");
    let a = cmd(&server, bad.path())
        .args(["traffic", "query", "--window", "1h"])
        .assert()
        .success();
    let err = String::from_utf8_lossy(&a.get_output().stderr).to_string();
    assert!(
        err.contains(
            "marks.toml invalid: 1 problem, marks off (illumio-tui marks list shows them)"
        ),
        "{err}"
    );
    let out = json_out(a);
    assert_eq!(
        out["data"].as_array().unwrap().len(),
        4,
        "runs without marks"
    );
    assert!(out["meta"].get("squelched").is_none());
    assert_eq!(
        out["meta"]["warnings"],
        json!(["marks.toml invalid: 1 problem, marks off (illumio-tui marks list shows them)"])
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn app_peers_leave_out_squelched_rows() {
    let server = MockServer::start().await;
    mount(&server).await;
    Mock::given(method("GET"))
        .and(path("/api/v2/orgs/5/workloads"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(include_str!("../../../fixtures/workloads.json"))
                .insert_header("Content-Type", "application/json"),
        )
        .mount(&server)
        .await;
    let home = home_with(MARKS);
    let out = json_out(
        cmd(&server, home.path())
            .args(["app", "shop/prod", "--window", "1h", "--direction", "in"])
            .assert()
            .success(),
    );
    let inbound = out["data"]["inbound"].as_array().unwrap();
    assert!(
        inbound
            .iter()
            .all(|p| p["peer"] != json!(["ipl:Corp", "-"])),
        "{inbound:?}"
    );
    assert!(inbound.iter().all(|p| p["mark"]["action"] == "critical"));
    assert_eq!(out["meta"]["squelched"], json!({"rows": 1, "flows": 1}));
}

#[tokio::test(flavor = "multi_thread")]
async fn gen_ignores_marks_for_its_rules_and_only_counts_them() {
    let server = MockServer::start().await;
    mount(&server).await;
    let home = home_with(MARKS);
    let args = ["gen", "shop/prod", "--window", "1h"];
    let with = json_out(cmd(&server, home.path()).args(args).assert().success());
    let without = json_out(
        cmd(&server, home.path())
            .args(args)
            .arg("--no-marks")
            .assert()
            .success(),
    );
    assert_eq!(
        serde_json::to_string(&with["data"]).unwrap(),
        serde_json::to_string(&without["data"]).unwrap(),
        "the proposal is byte-identical"
    );
    assert_eq!(with["meta"]["squelched"], json!({"flows": 1}));
    assert!(without["meta"].get("squelched").is_none());
}

/// A status that changes on every poll (`updated_at`), as the PCE's does
/// while it evaluates the draft rules (repeats cli.rs's `Ticking`).
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

/// `mount` for `--draft`, mounted first so these win: the status reports
/// the rules completed, `update_rules` answers, the download carries draft
/// decisions (Corp's potentially blocked flow would be allowed).
async fn mount_draft(server: &MockServer) {
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
        .mount(server)
        .await;
    let mut flows: Vec<Value> =
        serde_json::from_str(include_str!("../../../fixtures/traffic_flows.json")).unwrap();
    for f in &mut flows {
        let d = match f["policy_decision"].as_str().unwrap() {
            "potentially_blocked" => "allowed",
            d => d,
        }
        .to_string();
        f["draft_policy_decision"] = json!(d);
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
    mount(server).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn draft_decision_marks_apply_only_with_draft_and_are_counted_as_waiting() {
    let server = MockServer::start().await;
    mount_draft(&server).await;
    let home = home_with(
        "[[mark]]\nid = \"m-fixed1\"\naction = \"revisit\"\nsrc = \"any\"\ndst = \"any\"\n\
         decision = \"potentially_blocked\"\ndraft_decision = \"allowed\"\ncomment = \"the draft fixes it\"\n",
    );
    let q = ["traffic", "query", "--window", "1h"];
    let off = json_out(cmd(&server, home.path()).args(q).assert().success());
    assert!(
        off["data"]
            .as_array()
            .unwrap()
            .iter()
            .all(|f| f.get("mark").is_none())
    );
    assert_eq!(off["meta"]["marks"]["active"], 0);
    assert_eq!(off["meta"]["marks"]["waiting_draft"], 1);
    assert!(off["meta"].get("squelched").is_none());
    let on = json_out(
        cmd(&server, home.path())
            .args(q)
            .arg("--draft")
            .assert()
            .success(),
    );
    let marked: Vec<&Value> = on["data"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|f| f.get("mark").is_some())
        .collect();
    assert_eq!(marked.len(), 1, "{}", on["data"]);
    assert_eq!(
        marked[0]["mark"],
        json!({"action": "revisit", "id": "m-fixed1"})
    );
    assert_eq!(on["meta"]["marks"]["active"], 1);
    assert!(on["meta"]["marks"].get("waiting_draft").is_none());
}

#[tokio::test(flavor = "multi_thread")]
async fn by_both_with_app_counts_a_squelched_record_once_and_an_emptied_app_says_so() {
    let server = MockServer::start().await;
    mount(&server).await;
    let home = home_with(MARKS);
    let both = json_out(
        cmd(&server, home.path())
            .args([
                "traffic",
                "groups",
                "--window",
                "1h",
                "--by",
                "both",
                "--app",
                "shop/prod",
            ])
            .assert()
            .success(),
    );
    let sections = both["data"]["sections"].as_array().unwrap();
    assert!(!sections.is_empty());
    assert!(
        sections
            .iter()
            .flat_map(|s| s["rows"].as_array().unwrap())
            .all(|r| r["peer"] != json!(["ipl:Corp", "-"]))
    );
    assert_eq!(both["meta"]["squelched"]["flows"], 1, "{}", both["meta"]);

    let all = home_with(
        "[[mark]]\naction = \"squelch\"\nsrc = \"any\"\ndst = \"any\"\ncomment = \"everything\"\n",
    );
    let groups = |show: bool| {
        let mut c = cmd(&server, all.path());
        c.args([
            "traffic",
            "groups",
            "--window",
            "1h",
            "--by",
            "in",
            "--app",
            "shop/prod",
        ]);
        if show {
            c.arg("--show-squelched");
        }
        c.assert().success()
    };
    let a = groups(false);
    let err = String::from_utf8_lossy(&a.get_output().stderr).to_string();
    assert!(
        err.contains("all of \"shop/prod\"'s flows are squelched (--show-squelched shows them)"),
        "{err}"
    );
    assert!(!err.contains("no app matches"), "{err}");
    let a = groups(true);
    let err = String::from_utf8_lossy(&a.get_output().stderr).to_string();
    assert!(
        !err.contains("squelched (--show") && !err.contains("no app"),
        "{err}"
    );
    assert!(
        !json_out(a)["data"]["sections"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}
