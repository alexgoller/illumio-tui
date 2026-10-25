//! `pairing-profiles` (#76): list, create, delete. Every request is pinned
//! with `.expect`: the pairing profile list, the labels list (only when a
//! label needs it) and the one POST or DELETE.
#[path = "common/schema_check.rs"]
mod schema_check;
use assert_cmd::Command;
use serde_json::{Value, json};
use wiremock::matchers::{body_json, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const TEST_SECRET: &str = "s3cr3t-cli-test-9f2a";
const PROFILES: &str = include_str!("../../../fixtures/pairing_profiles.json");
const LABELS: &str = include_str!("../../../fixtures/labels.json");
const LIST: &str = "/api/v2/orgs/5/pairing_profiles";

fn no_config_dir() -> std::path::PathBuf {
    static N: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = N.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    std::env::temp_dir().join(format!(
        "illumio-tui-no-config-pairing-{}-{n}",
        std::process::id()
    ))
}

/// Credentials, no config file, writes not enabled.
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

/// Writes enabled, the audit log under `state`.
fn writer(server: &MockServer, state: &std::path::Path) -> Command {
    let mut c = cmd(server);
    c.env("ILLUMIO_WRITES", "1").env("XDG_STATE_HOME", state);
    c
}

fn stdout(assert: &assert_cmd::assert::Assert) -> String {
    String::from_utf8_lossy(&assert.get_output().stdout).to_string()
}

fn stderr(assert: &assert_cmd::assert::Assert) -> String {
    String::from_utf8_lossy(&assert.get_output().stderr).to_string()
}

fn audit_lines(state: &std::path::Path) -> Vec<Value> {
    match std::fs::read_to_string(state.join("illumio-tui").join("writes.jsonl")) {
        Ok(t) => t
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect(),
        Err(_) => Vec::new(),
    }
}

/// One GET of `p` answered with `body`, expected `times` times.
async fn expect_get(server: &MockServer, p: &str, body: &str, times: u64) {
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

/// No request of `m` reaches the server.
async fn expect_none(server: &MockServer, m: &str) {
    Mock::given(method(m))
        .respond_with(ResponseTemplate::new(500))
        .expect(0)
        .mount(server)
        .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn list_prints_every_profile_with_label_names() {
    let server = MockServer::start().await;
    expect_get(&server, LIST, PROFILES, 2).await;
    expect_get(&server, "/api/v2/orgs/5/labels", LABELS, 2).await;
    let a = cmd(&server)
        .args(["--json", "pairing-profiles", "list"])
        .assert()
        .success();
    let v = schema_check::envelope(&a.get_output().stdout);
    assert_eq!(v["meta"]["schema"], "pairing-profiles");
    assert_eq!(v["data"].as_array().unwrap().len(), 2);
    assert_eq!(v["data"][1]["name"], "shop-prod");
    // As the PCE sent it, unknown fields too.
    assert_eq!(v["data"][0]["is_default"], true);
    assert_eq!(v["data"][1]["key_lifespan"], 86400);
    // Labels named from the labels list read for them.
    assert_eq!(
        v["data"][1]["labels"][0],
        json!({"href": "/orgs/5/labels/1", "key": "role", "value": "web"})
    );
    let labels: Vec<Value> = serde_json::from_str(LABELS).unwrap();
    let name_of = |href: &str| {
        let l = labels.iter().find(|l| l["href"] == href).unwrap();
        format!(
            "{}={}",
            l["key"].as_str().unwrap(),
            l["value"].as_str().unwrap()
        )
    };
    let a = cmd(&server)
        .args(["pp", "list", "--table"])
        .assert()
        .success();
    let out = stdout(&a);
    assert!(out.contains("shop-prod"), "{out}");
    assert!(out.contains(&name_of("/orgs/5/labels/1")), "{out}");
    assert!(out.contains("role,app,env,enforcement"), "{out}");
    assert!(out.contains("1d"), "{out}");
}

#[tokio::test(flavor = "multi_thread")]
async fn list_without_labels_reads_no_labels() {
    let server = MockServer::start().await;
    let one: Vec<Value> = serde_json::from_str(PROFILES).unwrap();
    let body = serde_json::to_string(&one[..1]).unwrap();
    expect_get(&server, LIST, &body, 1).await;
    expect_get(&server, "/api/v2/orgs/5/labels", "[]", 0).await;
    cmd(&server)
        .args(["--json", "pairing-profiles", "list"])
        .assert()
        .success();
}

#[tokio::test(flavor = "multi_thread")]
async fn list_csv_and_fields() {
    let server = MockServer::start().await;
    expect_get(&server, LIST, PROFILES, 3).await;
    expect_get(&server, "/api/v2/orgs/5/labels", LABELS, 3).await;
    let a = cmd(&server)
        .args(["--csv", "pairing-profiles", "list"])
        .assert()
        .success();
    let out = stdout(&a);
    let mut lines = out.lines();
    assert_eq!(
        lines.next().unwrap(),
        "name,enabled,labels,enforcement,visibility,ven_type,uses_per_key,key_lifespan,locks,href"
    );
    let shop = lines.find(|l| l.starts_with("shop-prod")).unwrap();
    assert!(shop.contains(",false,"), "{shop}");
    assert!(
        shop.contains(",86400,role; app; env; enforcement,"),
        "{shop}"
    );
    let a = cmd(&server)
        .args(["--csv", "--fields", "name,href", "pairing-profiles", "list"])
        .assert()
        .success();
    assert_eq!(stdout(&a).lines().next(), Some("name,href"));
    let a = cmd(&server)
        .args([
            "--json",
            "--fields",
            "name,enabled",
            "pairing-profiles",
            "list",
        ])
        .assert()
        .success();
    let v = schema_check::picked(&a.get_output().stdout);
    assert_eq!(v["data"][1], json!({"name": "shop-prod", "enabled": false}));
    // An unknown field fails before any request.
    let a = cmd(&server)
        .args(["--json", "--fields", "nope", "pairing-profiles", "list"])
        .assert()
        .code(2);
    assert!(
        stderr(&a).contains("no field \\\"nope\\\" in pairing-profiles"),
        "{}",
        stderr(&a)
    );
}

/// The POST of exactly `body`, answered with `answer`, once.
async fn expect_post(server: &MockServer, body: Value, answer: Value) {
    Mock::given(method("POST"))
        .and(path(LIST))
        .and(body_json(body))
        .respond_with(ResponseTemplate::new(201).set_body_json(answer))
        .expect(1)
        .mount(server)
        .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn create_sends_the_name_and_only_the_flags_given() {
    let server = MockServer::start().await;
    expect_get(&server, LIST, PROFILES, 1).await;
    expect_get(&server, "/api/v2/orgs/5/labels", LABELS, 0).await;
    let answer = json!({
        "href": "/orgs/5/pairing_profiles/9",
        "name": "pp1",
        "enabled": false,
        "labels": [],
        "total_use_count": 0
    });
    expect_post(
        &server,
        json!({"name": "pp1", "enabled": false}),
        answer.clone(),
    )
    .await;
    let state = tempfile::tempdir().unwrap();
    let a = writer(&server, state.path())
        .args([
            "--json",
            "pairing-profiles",
            "create",
            "pp1",
            "--disabled",
            "--yes",
        ])
        .assert()
        .success();
    let v = schema_check::envelope(&a.get_output().stdout);
    assert_eq!(v["meta"]["schema"], "pairing-profiles-create");
    assert_eq!(v["data"], answer, "the profile as the PCE returned it");
    assert_eq!(v["meta"]["write"]["method"], "POST");
    assert_eq!(v["meta"]["write"]["status"], 201);
    let audit = audit_lines(state.path());
    assert_eq!(audit.len(), 1);
    assert_eq!(audit[0]["path"], "/orgs/5/pairing_profiles");
    assert_eq!(audit[0]["body"], json!({"name": "pp1", "enabled": false}));
}

#[tokio::test(flavor = "multi_thread")]
async fn create_maps_every_flag_and_resolves_labels() {
    let server = MockServer::start().await;
    expect_get(&server, LIST, PROFILES, 1).await;
    expect_get(&server, "/api/v2/orgs/5/labels", LABELS, 1).await;
    let labels: Vec<Value> = serde_json::from_str(LABELS).unwrap();
    let live: Vec<&Value> = labels.iter().filter(|l| l["deleted"] != true).collect();
    let (a, b) = (
        live[0],
        live.iter().find(|l| l["key"] != live[0]["key"]).unwrap(),
    );
    let spec = |l: &Value| {
        format!(
            "{}={}",
            l["key"].as_str().unwrap(),
            l["value"].as_str().unwrap()
        )
    };
    let body = json!({
        "name": "shop",
        "description": "shop web tier",
        "labels": [{"href": a["href"]}, {"href": b["href"]}],
        "enforcement_mode": "selective",
        "visibility_level": "flow_summary",
        "ven_type": "specified_during_activation",
        "allowed_uses_per_key": 3,
        "key_lifespan": 3600,
        "role_label_lock": true,
        "app_label_lock": true,
        "log_traffic_lock": true,
        "env_label_lock": false,
        "loc_label_lock": false
    });
    let mut answer = body.clone();
    answer["href"] = json!("/orgs/5/pairing_profiles/10");
    expect_post(&server, body, answer).await;
    let state = tempfile::tempdir().unwrap();
    let a = writer(&server, state.path())
        .args([
            "pairing-profiles",
            "create",
            "shop",
            "--description",
            "shop web tier",
        ])
        .args(["--label", &spec(a), "--label", &spec(b)])
        .args([
            "--enforcement",
            "selective",
            "--visibility",
            "blocked-allowed",
        ])
        .args(["--ven-type", "specified-during-activation", "--uses", "3"])
        .args([
            "--lifespan",
            "1h",
            "--lock",
            "log-traffic,role",
            "--lock",
            "app",
        ])
        .args(["--unlock", "env,loc"])
        .args(["--table", "--yes"])
        .assert()
        .success();
    let out = stdout(&a);
    assert!(
        out.contains("shop") && out.contains("/orgs/5/pairing_profiles/10"),
        "{out}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_taken_name_or_an_unknown_label_sends_nothing() {
    let server = MockServer::start().await;
    expect_get(&server, LIST, PROFILES, 2).await;
    expect_get(&server, "/api/v2/orgs/5/labels", LABELS, 1).await;
    expect_none(&server, "POST").await;
    let state = tempfile::tempdir().unwrap();
    let a = writer(&server, state.path())
        .args(["pairing-profiles", "create", "shop-prod", "--yes"])
        .assert()
        .code(2);
    assert!(stderr(&a).contains("shop-prod"), "{}", stderr(&a));
    assert!(
        stderr(&a).contains("/orgs/5/pairing_profiles/7"),
        "{}",
        stderr(&a)
    );
    let a = writer(&server, state.path())
        .args([
            "pairing-profiles",
            "create",
            "new",
            "--label",
            "nosuchkey=x",
            "--yes",
        ])
        .assert()
        .code(2);
    assert!(
        stderr(&a).contains("unknown label nosuchkey=x"),
        "{}",
        stderr(&a)
    );
    // Bad flag values: refused before any request.
    for flags in [
        &["--lock", "colour"][..],
        &["--label", "app=a", "--label", "app=b"],
        &["--uses", "0"],
        &["--enforcement", "enforced"],
        &["--lock", "env", "--unlock", "env"],
    ] {
        writer(&server, state.path())
            .args(["pairing-profiles", "create", "new", "--yes"])
            .args(flags)
            .assert()
            .code(2);
    }
    assert!(audit_lines(state.path()).is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn writes_need_the_profile_switch_even_for_a_dry_run() {
    let server = MockServer::start().await;
    expect_none(&server, "GET").await;
    expect_none(&server, "POST").await;
    expect_none(&server, "DELETE").await;
    let state = tempfile::tempdir().unwrap();
    for argv in [
        &["pairing-profiles", "create", "pp1", "--dry-run"][..],
        &["pairing-profiles", "create", "pp1", "--yes"],
        &["pp", "delete", "shop-prod", "--dry-run"],
    ] {
        cmd(&server)
            .env("XDG_STATE_HOME", state.path())
            .args(argv)
            .assert()
            .code(6);
    }
    // Writes enabled but neither --yes nor --dry-run: exit 2, no request.
    writer(&server, state.path())
        .args(["pairing-profiles", "create", "pp1"])
        .assert()
        .code(2);
    // --csv prints no table for a write.
    writer(&server, state.path())
        .args(["--csv", "pairing-profiles", "create", "pp1", "--dry-run"])
        .assert()
        .code(2);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_dry_run_prints_the_request_and_sends_nothing() {
    let server = MockServer::start().await;
    expect_get(&server, LIST, PROFILES, 2).await;
    expect_none(&server, "POST").await;
    expect_none(&server, "DELETE").await;
    let state = tempfile::tempdir().unwrap();
    let a = writer(&server, state.path())
        .args([
            "--json",
            "pairing-profiles",
            "create",
            "pp1",
            "--uses",
            "unlimited",
        ])
        .args(["--lifespan", "unlimited", "--dry-run"])
        .assert()
        .success();
    let v = schema_check::envelope(&a.get_output().stdout);
    assert_eq!(v["meta"]["schema"], "write");
    assert_eq!(v["data"]["method"], "POST");
    assert_eq!(v["data"]["path"], "/orgs/5/pairing_profiles");
    assert_eq!(
        v["data"]["body"],
        json!({"name": "pp1", "allowed_uses_per_key": "unlimited", "key_lifespan": "unlimited"})
    );
    let a = writer(&server, state.path())
        .args(["--json", "pp", "delete", "shop-prod", "--dry-run"])
        .assert()
        .success();
    let v = schema_check::envelope(&a.get_output().stdout);
    assert_eq!(v["data"]["method"], "DELETE");
    assert_eq!(v["data"]["path"], "/orgs/5/pairing_profiles/7");
    assert!(audit_lines(state.path()).is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn delete_resolves_the_name_and_deletes_the_href() {
    let server = MockServer::start().await;
    expect_get(&server, LIST, PROFILES, 3).await;
    Mock::given(method("DELETE"))
        .and(path("/api/v2/orgs/5/pairing_profiles/7"))
        .respond_with(ResponseTemplate::new(204))
        .expect(2)
        .mount(&server)
        .await;
    let state = tempfile::tempdir().unwrap();
    let a = writer(&server, state.path())
        .args([
            "--table",
            "pairing-profiles",
            "delete",
            "shop-prod",
            "--yes",
        ])
        .assert()
        .success();
    assert_eq!(stdout(&a), "deleted shop-prod\n");
    let a = writer(&server, state.path())
        .args(["--json", "pairing-profiles", "delete", "shop-prod", "--yes"])
        .assert()
        .success();
    let v = schema_check::envelope(&a.get_output().stdout);
    assert_eq!(v["meta"]["schema"], "pairing-profiles-delete");
    assert_eq!(
        v["data"],
        json!({"deleted": "shop-prod", "href": "/orgs/5/pairing_profiles/7"})
    );
    assert_eq!(v["meta"]["write"]["method"], "DELETE");
    assert_eq!(audit_lines(state.path()).len(), 2);
    // Unknown: exit 2 with what is like it, nothing sent.
    let a = writer(&server, state.path())
        .args(["pairing-profiles", "delete", "shop-prd", "--yes"])
        .assert()
        .code(2);
    assert!(stderr(&a).contains("similar: shop-prod"), "{}", stderr(&a));
}

#[tokio::test(flavor = "multi_thread")]
async fn delete_by_href_reads_no_list() {
    let server = MockServer::start().await;
    expect_get(&server, LIST, PROFILES, 0).await;
    Mock::given(method("DELETE"))
        .and(path("/api/v2/orgs/5/pairing_profiles/7"))
        .respond_with(ResponseTemplate::new(204))
        .expect(1)
        .mount(&server)
        .await;
    let state = tempfile::tempdir().unwrap();
    let a = writer(&server, state.path())
        .args([
            "--json",
            "pp",
            "delete",
            "/orgs/5/pairing_profiles/7",
            "--yes",
        ])
        .assert()
        .success();
    let v = schema_check::envelope(&a.get_output().stdout);
    assert_eq!(v["data"]["deleted"], "/orgs/5/pairing_profiles/7");
    // Another org's or another kind's href: refused, nothing sent.
    for href in ["/orgs/6/pairing_profiles/7", "/orgs/5/labels/7"] {
        writer(&server, state.path())
            .args(["pp", "delete", href, "--yes"])
            .assert()
            .code(2);
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_rejected_create_is_exit_4_and_audited() {
    let server = MockServer::start().await;
    expect_get(&server, LIST, PROFILES, 1).await;
    Mock::given(method("POST"))
        .and(path(LIST))
        .respond_with(
            ResponseTemplate::new(406)
                .set_body_json(json!([{"token": "invalid_value", "message": "bad ven_type"}])),
        )
        .expect(1)
        .mount(&server)
        .await;
    let state = tempfile::tempdir().unwrap();
    writer(&server, state.path())
        .args(["pairing-profiles", "create", "pp1", "--yes"])
        .assert()
        .code(4);
    let audit = audit_lines(state.path());
    assert_eq!(audit.len(), 1);
    assert_eq!(audit[0]["status"], 406);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_name_two_profiles_share_is_refused_with_both_hrefs() {
    let server = MockServer::start().await;
    let twins = json!([
        {"href": "/orgs/5/pairing_profiles/1", "name": "dup"},
        {"href": "/orgs/5/pairing_profiles/2", "name": "dup"}
    ]);
    expect_get(&server, LIST, &twins.to_string(), 1).await;
    expect_none(&server, "DELETE").await;
    let state = tempfile::tempdir().unwrap();
    let a = writer(&server, state.path())
        .args(["pairing-profiles", "delete", "dup", "--yes"])
        .assert()
        .code(2);
    let err = stderr(&a);
    assert!(
        err.contains("/orgs/5/pairing_profiles/1") && err.contains("/orgs/5/pairing_profiles/2"),
        "{err}"
    );
}

#[test]
fn schema_takes_the_alias() {
    let out = Command::cargo_bin("illumio-tui")
        .unwrap()
        .env_clear()
        .env("XDG_CONFIG_HOME", no_config_dir())
        .args(["schema", "pp", "list"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let doc: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(doc["title"], "illumio-tui pairing-profiles");
    assert!(doc["$comment"].as_str().unwrap().contains("key_lifespan"));
}

#[test]
fn schema_lists_the_commands() {
    let out = Command::cargo_bin("illumio-tui")
        .unwrap()
        .env_clear()
        .env("XDG_CONFIG_HOME", no_config_dir())
        .args(["schema", "pairing-profiles", "list"])
        .output()
        .unwrap();
    let doc: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert!(
        doc["$comment"]
            .as_str()
            .unwrap()
            .contains("uses_per_key, key_lifespan, locks, href"),
        "{doc}"
    );
}
