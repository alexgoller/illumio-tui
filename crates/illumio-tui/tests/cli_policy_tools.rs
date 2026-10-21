//! The Policy tools on the CLI: `policy search` (#51), the impact's
//! workload list `policy rules impact --list` (#53), and the help of
//! `enable`/`disable` (#52). Reads only; never more than one read per list.
use assert_cmd::Command;
use serde_json::{Value, json};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const TEST_SECRET: &str = "s3cr3t-cli-test-9f2a";

fn no_config_dir() -> std::path::PathBuf {
    static N: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = N.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    std::env::temp_dir().join(format!(
        "illumio-tui-no-config-tools-{}-{n}",
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
        .env("XDG_CONFIG_HOME", no_config_dir());
    c
}

fn stdout_json(assert: assert_cmd::assert::Assert) -> Value {
    serde_json::from_slice(&assert.get_output().stdout).expect("stdout is JSON")
}

fn stdout(assert: &assert_cmd::assert::Assert) -> String {
    String::from_utf8_lossy(&assert.get_output().stdout).to_string()
}

fn stderr(assert: &assert_cmd::assert::Assert) -> String {
    String::from_utf8_lossy(&assert.get_output().stderr).to_string()
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

/// The four lists behind the name index and both rulesets lists.
async fn mount_policy(server: &MockServer) {
    mount_policy_with(
        server,
        include_str!("../../../fixtures/ip_lists.json"),
        include_str!("../../../fixtures/rule_sets.json"),
    )
    .await;
}

/// `mount_policy` with these IP lists and draft rulesets.
async fn mount_policy_with(server: &MockServer, ip_lists: &str, rule_sets: &str) {
    for (p, body) in [
        (
            "/api/v2/orgs/5/labels",
            include_str!("../../../fixtures/labels.json"),
        ),
        (
            "/api/v2/orgs/5/sec_policy/draft/label_groups",
            include_str!("../../../fixtures/label_groups.json"),
        ),
        ("/api/v2/orgs/5/sec_policy/draft/ip_lists", ip_lists),
        (
            "/api/v2/orgs/5/sec_policy/draft/services",
            include_str!("../../../fixtures/services.json"),
        ),
        ("/api/v2/orgs/5/sec_policy/draft/rule_sets", rule_sets),
        ("/api/v2/orgs/5/sec_policy/active/rule_sets", "[]"),
    ] {
        mount(server, p, body).await;
    }
}

async fn mount_workloads(server: &MockServer) {
    mount(
        server,
        "/api/v2/orgs/5/workloads",
        include_str!("../../../fixtures/workloads.json"),
    )
    .await;
}

/// GETs of `p` so far.
async fn gets(server: &MockServer, p: &str) -> usize {
    server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .filter(|r| r.method.as_str() == "GET" && r.url.path() == p)
        .count()
}

fn hrefs(out: &Value) -> Vec<String> {
    out["data"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| {
            r["href"]
                .as_str()
                .unwrap()
                .rsplit('/')
                .next()
                .unwrap()
                .to_string()
        })
        .collect()
}

#[test]
fn the_new_commands_document_every_flag() {
    for args in [
        vec!["policy", "search", "--help"],
        vec!["policy", "rules", "enable", "--help"],
        vec!["policy", "rules", "disable", "--help"],
        vec!["policy", "rulesets", "enable", "--help"],
        vec!["policy", "rulesets", "disable", "--help"],
        vec!["policy", "rules", "impact", "--help"],
    ] {
        let out = Command::cargo_bin("illumio-tui")
            .unwrap()
            .env_clear()
            .args(&args)
            .output()
            .unwrap();
        assert!(out.status.success(), "{args:?}");
        let text = String::from_utf8_lossy(&out.stdout);
        let lines: Vec<&str> = text.lines().collect();
        for (i, line) in lines.iter().enumerate() {
            let t = line.trim_start();
            if t.starts_with("--") {
                let inline = t.split_once("  ").map(|(_, d)| d.trim()).unwrap_or("");
                let next = lines.get(i + 1).map(|l| l.trim()).unwrap_or("");
                let wrapped = !next.is_empty() && !next.starts_with('-');
                assert!(!inline.is_empty() || wrapped, "{args:?}: no help for {t:?}");
            }
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn policy_search_flags_and_terms_and_with_one_rulesets_read() {
    let server = MockServer::start().await;
    mount_policy(&server).await;
    let out = stdout_json(
        cmd(&server)
            .args([
                "--json",
                "policy",
                "search",
                "--label",
                "role=web,db",
                "--deny",
            ])
            .assert()
            .success(),
    );
    assert_eq!(hrefs(&out), ["8"]);
    let row = &out["data"][0];
    assert_eq!(row["ruleset"], "Shop-Prod");
    assert_eq!(row["kind"], "override_deny");
    assert_eq!(row["scopes"], json!(["app=shop & env=prod"]));
    assert_eq!(
        gets(&server, "/api/v2/orgs/5/sec_policy/draft/rule_sets").await,
        1
    );
    // The TUI's terms, the same search.
    let out = stdout_json(
        cmd(&server)
            .args(["--json", "policy", "search", "role=web,db", "deny"])
            .assert()
            .success(),
    );
    assert_eq!(hrefs(&out), ["8"]);
    // A label group holding the label counts; ruleset order, then evaluation order.
    let out = stdout_json(
        cmd(&server)
            .args(["--json", "policy", "search", "--label", "env=prod"])
            .assert()
            .success(),
    );
    assert_eq!(hrefs(&out), ["8", "1", "2", "7", "3"]);
    let out = stdout_json(
        cmd(&server)
            .args([
                "--json",
                "policy",
                "search",
                "--service",
                "443/tcp",
                "--ip-list",
                "corp",
            ])
            .assert()
            .success(),
    );
    assert_eq!(hrefs(&out), ["2"]);
    let out = stdout_json(
        cmd(&server)
            .args([
                "--json",
                "policy",
                "search",
                "--service",
                "All Services",
                "--disabled",
            ])
            .assert()
            .success(),
    );
    assert_eq!(hrefs(&out), ["3"]);
    assert_eq!(
        out["data"][0]["enabled"], true,
        "the rule; its ruleset is off"
    );
    assert_eq!(out["data"][0]["ruleset_enabled"], false);
    // Active: nothing there.
    let out = stdout_json(
        cmd(&server)
            .args(["--json", "policy", "search", "deny", "--active"])
            .assert()
            .success(),
    );
    assert_eq!(out["data"], json!([]));
    assert_eq!(gets(&server, "/api/v2/orgs/5/workloads").await, 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn policy_search_prints_table_and_csv() {
    let server = MockServer::start().await;
    mount_policy(&server).await;
    let a = cmd(&server)
        .args(["--table", "policy", "search", "--href", "sec_rules/2"])
        .assert()
        .success();
    let text = stdout(&a);
    assert!(
        text.contains("ruleset") && text.contains("Shop-Prod"),
        "{text}"
    );
    assert!(text.contains("ipl:Corp"), "{text}");
    let a = cmd(&server)
        .args(["--csv", "policy", "search", "--href", "sec_rules/2"])
        .assert()
        .success();
    let text = stdout(&a);
    let mut lines = text.lines();
    assert_eq!(
        lines.next().unwrap(),
        "ruleset,kind,enabled,scope,consumers,providers,services,href"
    );
    assert_eq!(
        lines.next().unwrap(),
        "Shop-Prod,allow,on,app=shop & env=prod,ipl:Corp,role=web,443/tcp; 8000-8099/tcp,/orgs/5/sec_policy/draft/rule_sets/100/sec_rules/2"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn policy_search_refuses_no_term_bad_terms_and_unknown_names() {
    let server = MockServer::start().await;
    for args in [
        vec!["policy", "search"],
        vec!["policy", "search", "shop"],
        vec!["policy", "search", "--label", "shop"],
        vec!["policy", "search", "--label", "!app=shop"],
        vec!["policy", "search", "-"],
        vec!["policy", "search", "app=shop", "--x"],
        vec!["policy", "search", "scope:all"],
    ] {
        let a = cmd(&server).args(&args).assert().code(2);
        assert!(!stderr(&a).is_empty(), "{args:?}");
    }
    assert!(server.received_requests().await.unwrap().is_empty());
    mount_policy(&server).await;
    let a = cmd(&server)
        .args([
            "policy",
            "search",
            "--ip-list",
            "Crop",
            "--label",
            "app=shpo",
        ])
        .assert()
        .code(2);
    let e = stderr(&a);
    assert!(
        e.contains("unknown value app=shpo") && e.contains("unknown IP list Crop (closest: Corp)"),
        "{e}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn policy_search_names_take_a_unique_prefix_and_refuse_several() {
    let server = MockServer::start().await;
    mount_policy(&server).await;
    // `Any` is the `Any (0.0.0.0/0 and ::/0)` list; `post` PostgreSQL.
    let out = stdout_json(
        cmd(&server)
            .args(["--json", "policy", "search", "--ip-list", "Any"])
            .assert()
            .success(),
    );
    assert_eq!(hrefs(&out), ["7"]);
    let out = stdout_json(
        cmd(&server)
            .args(["--json", "policy", "search", "--service", "post"])
            .assert()
            .success(),
    );
    assert_eq!(hrefs(&out), ["1"]);
    // A protocol: every tcp entry (and All Services).
    let out = stdout_json(
        cmd(&server)
            .args(["--json", "policy", "search", "--service", "tcp"])
            .assert()
            .success(),
    );
    assert_eq!(hrefs(&out), ["8", "1", "2", "7", "3"]);

    let server = MockServer::start().await;
    let lists = r#"[
      {"href":"/orgs/5/sec_policy/draft/ip_lists/20","name":"Any (0.0.0.0/0 and ::/0)","ip_ranges":[]},
      {"href":"/orgs/5/sec_policy/draft/ip_lists/22","name":"Any IPv4","ip_ranges":[]}
    ]"#;
    mount_policy_with(
        &server,
        lists,
        include_str!("../../../fixtures/rule_sets.json"),
    )
    .await;
    let a = cmd(&server)
        .args(["policy", "search", "--ip-list", "any"])
        .assert()
        .code(2);
    assert!(
        stderr(&a).contains("IP list any matches several: Any (0.0.0.0/0 and ::/0), Any IPv4"),
        "{}",
        stderr(&a)
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn policy_search_enabled_and_the_deny_warning() {
    let server = MockServer::start().await;
    mount_policy(&server).await;
    let a = cmd(&server)
        .args(["--csv", "policy", "search", "--href", "sec_rules/3"])
        .assert()
        .success();
    let text = stdout(&a);
    // Rule 3 is on; its ruleset Global-Infra is not.
    assert!(
        text.lines()
            .nth(1)
            .unwrap()
            .starts_with("Global-Infra,allow,on (ruleset off),"),
        "{text}"
    );
    // Ruleset 100 lists deny rules inline: no warning.
    let out = stdout_json(
        cmd(&server)
            .args(["--json", "policy", "search", "--deny"])
            .assert()
            .success(),
    );
    assert_eq!(out["meta"]["warnings"], Value::Null);

    // No ruleset lists them inline: they may be kept apart.
    let server = MockServer::start().await;
    let mut sets: Value =
        serde_json::from_str(include_str!("../../../fixtures/rule_sets.json")).unwrap();
    for rs in sets.as_array_mut().unwrap() {
        rs["deny_rules"] = json!([]);
    }
    mount_policy_with(
        &server,
        include_str!("../../../fixtures/ip_lists.json"),
        &sets.to_string(),
    )
    .await;
    let out = stdout_json(
        cmd(&server)
            .args(["--json", "policy", "search", "--deny"])
            .assert()
            .success(),
    );
    assert_eq!(out["data"], json!([]));
    let w = out["meta"]["warnings"][0].as_str().unwrap();
    assert!(
        w.starts_with("2 rulesets may hold deny rules not loaded"),
        "{w}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn policy_search_by_workload_name_reads_the_workloads_once() {
    let server = MockServer::start().await;
    mount_policy(&server).await;
    mount_workloads(&server).await;
    let out = stdout_json(
        cmd(&server)
            .args(["--json", "policy", "search", "--workload", "host-1.example"])
            .assert()
            .success(),
    );
    // No fixture rule names a workload.
    assert_eq!(out["data"], json!([]));
    assert_eq!(gets(&server, "/api/v2/orgs/5/workloads").await, 1);
}

const DENY_8: &str = "/orgs/5/sec_policy/draft/rule_sets/100/deny_rules/8";
const RULE_2: &str = "/orgs/5/sec_policy/draft/rule_sets/100/sec_rules/2";

#[tokio::test(flavor = "multi_thread")]
async fn impact_list_prints_one_sides_workloads_from_one_workloads_read() {
    let server = MockServer::start().await;
    mount_policy(&server).await;
    mount_workloads(&server).await;
    let out = stdout_json(
        cmd(&server)
            .args([
                "--json",
                "policy",
                "rules",
                "impact",
                DENY_8,
                "--list",
                "providers",
            ])
            .assert()
            .success(),
    );
    assert_eq!(out["data"]["side"], "providers");
    assert_eq!(out["data"]["count"], 1);
    assert_eq!(out["data"]["note"], Value::Null);
    let w = &out["data"]["workloads"][0];
    assert_eq!(w["hostname"], "host-2.example");
    assert_eq!(w["enforcement"], "visibility_only");
    assert_eq!(w["online"], false);
    assert_eq!(w["labels"], json!(["app=shop", "env=prod", "role=db"]));
    assert_eq!(gets(&server, "/api/v2/orgs/5/workloads").await, 1);
    let a = cmd(&server)
        .args([
            "--csv",
            "policy",
            "rules",
            "impact",
            DENY_8,
            "--list",
            "consumers",
        ])
        .assert()
        .success();
    let text = stdout(&a);
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines[0], "hostname,name,labels,enforcement,online,href");
    assert_eq!(
        lines[1],
        "host-1.example,web01,app=shop; env=prod; role=web,full,true,/orgs/5/workloads/aaaa-1"
    );
    assert_eq!(lines.len(), 3);
    assert_eq!(gets(&server, "/api/v2/orgs/5/workloads").await, 2);
}

#[tokio::test(flavor = "multi_thread")]
async fn impact_list_explains_an_ip_list_side() {
    let server = MockServer::start().await;
    mount_policy(&server).await;
    mount_workloads(&server).await;
    let out = stdout_json(
        cmd(&server)
            .args([
                "--json",
                "policy",
                "rules",
                "impact",
                RULE_2,
                "--list",
                "consumers",
            ])
            .assert()
            .success(),
    );
    assert_eq!(out["data"]["workloads"], json!([]));
    assert_eq!(
        out["data"]["note"],
        "the consumers are IP lists (Corp): addresses, not workloads; nothing to list"
    );
    let a = cmd(&server)
        .args([
            "--table",
            "policy",
            "rules",
            "impact",
            RULE_2,
            "--list",
            "consumers",
        ])
        .assert()
        .success();
    assert!(stderr(&a).contains("note: the consumers are IP lists (Corp)"));
    // Without --list the impact prints no table: --csv is refused before any request.
    let before = server.received_requests().await.unwrap().len();
    cmd(&server)
        .args(["--csv", "policy", "rules", "impact", RULE_2])
        .assert()
        .code(2);
    cmd(&server)
        .args(["policy", "rules", "impact", RULE_2, "--list", "both"])
        .assert()
        .code(2);
    assert_eq!(server.received_requests().await.unwrap().len(), before);
}

/// The issue's examples' world: a payment ruleset and a global one.
async fn mount_examples(server: &MockServer) {
    let label = |id: u32, key: &str, value: &str| json!({"href": format!("/orgs/5/labels/{id}"), "key": key, "value": value});
    let labels = json!([
        label(1, "role", "web"),
        label(2, "role", "db"),
        label(3, "app", "payment"),
        label(4, "env", "prod")
    ]);
    let l = |id: u32| json!({"label": {"href": format!("/orgs/5/labels/{id}")}});
    let rule = |id: u32, consumers: Value, providers: Value, port: u16, unscoped: bool| {
        json!({
            "href": format!("/orgs/5/sec_policy/draft/rule_sets/300/sec_rules/{id}"),
            "enabled": true, "consumers": consumers, "providers": providers,
            "ingress_services": [{"port": port, "proto": 6}], "unscoped_consumers": unscoped
        })
    };
    let ams = json!([{"actors": "ams"}]);
    let rule_sets = json!([
        {"href": "/orgs/5/sec_policy/draft/rule_sets/300", "name": "Payment", "enabled": true,
         "scopes": [[l(3)]],
         "rules": [
            rule(11, json!([l(1)]), json!([l(2)]), 5432, true),
            rule(12, json!([l(1)]), json!([l(2)]), 443, false),
            rule(13, json!([{"label": {"href": "/orgs/5/labels/4"}, "exclusion": true}]), json!([l(2)]), 22, false)
         ],
         "deny_rules": [
            {"href": "/orgs/5/sec_policy/draft/rule_sets/300/deny_rules/14", "enabled": true,
             "consumers": ams, "providers": [l(2)], "ingress_services": [{"port": 23, "proto": 6}]}
         ]},
        {"href": "/orgs/5/sec_policy/draft/rule_sets/301", "name": "Global", "enabled": true,
         "scopes": [[]],
         "rules": [
            rule(15, ams.clone(), json!([l(1)]), 443, false),
            rule(16, json!([l(4)]), ams, 443, false)
         ]}
    ]);
    for (p, body) in [
        ("/api/v2/orgs/5/labels", labels.to_string()),
        ("/api/v2/orgs/5/sec_policy/draft/label_groups", "[]".into()),
        (
            "/api/v2/orgs/5/sec_policy/draft/ip_lists",
            include_str!("../../../fixtures/ip_lists.json").into(),
        ),
        (
            "/api/v2/orgs/5/sec_policy/draft/services",
            include_str!("../../../fixtures/services.json").into(),
        ),
        (
            "/api/v2/orgs/5/sec_policy/draft/rule_sets",
            rule_sets.to_string(),
        ),
    ] {
        mount(server, p, &body).await;
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn policy_search_runs_the_issue_examples() {
    let server = MockServer::start().await;
    mount_examples(&server).await;
    let search = |terms: &[&str]| {
        let mut args = vec!["--json", "policy", "search"];
        args.extend_from_slice(terms);
        stdout_json(cmd(&server).args(&args).assert().success())
    };
    // Extra-scope rules from web to db: 12 is intra.
    let out = search(&["src:role=web", "dst:role=db", "extra"]);
    assert_eq!(hrefs(&out), ["11"]);
    assert_eq!(out["meta"]["matched"], json!({"matched": 1, "loaded": 6}));
    // Payment rules not covering 443: allow rules, then the deny rule.
    assert_eq!(
        hrefs(&search(&["scope:app=payment", "-svc:443/tcp"])),
        ["11", "13", "14"]
    );
    // Consumers that say `All except env=prod`; 16 names env=prod itself.
    assert_eq!(hrefs(&search(&["src:!env=prod"])), ["13"]);
    assert_eq!(hrefs(&search(&["src:env=prod"])), ["16"]);
    // Allow rules open to All Workloads: deny rule 14 is left out.
    assert_eq!(hrefs(&search(&["src:all", "-deny"])), ["15"]);
    assert_eq!(hrefs(&search(&["src:all"])), ["14", "15"]);
    assert_eq!(hrefs(&search(&["scope:global"])), ["15", "16"]);
    assert_eq!(gets(&server, "/api/v2/orgs/5/workloads").await, 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn policy_search_takes_dash_terms_as_terms_and_flags_as_flags() {
    let server = MockServer::start().await;
    mount_examples(&server).await;
    // A negated term first, then flags; flags before; after `--`.
    for args in [
        vec!["policy", "search", "-deny", "--json", "--label", "role=db"],
        vec!["--json", "policy", "search", "--label", "role=db", "-deny"],
        vec![
            "--json",
            "policy",
            "search",
            "--label=role=db",
            "--",
            "-deny",
        ],
        vec!["--json", "policy", "search", "role=db", "-deny", "-src:all"],
    ] {
        let out = stdout_json(cmd(&server).args(&args).assert().success());
        assert_eq!(hrefs(&out), ["11", "12", "13"], "{args:?}");
    }
    let a = cmd(&server)
        .args(["--table", "policy", "search", "-scope:global", "-deny"])
        .assert()
        .success();
    assert!(
        stderr(&a).contains(" · 3 of 6 rules matched"),
        "{}",
        stderr(&a)
    );
    // Two dashes are a flag's: an unknown one is refused.
    for args in [
        vec!["policy", "search", "--deny-x"],
        vec!["policy", "search", "app=payment", "--nope"],
    ] {
        let a = cmd(&server).args(&args).assert().code(2);
        assert!(!stderr(&a).is_empty(), "{args:?}");
    }
}
