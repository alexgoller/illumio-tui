//! Saved filters on the command line: `--save-filter` writes the config
//! file and never builds a client (the profile has no credentials, so any
//! attempt would fail), `--filter` is the base the flags replace fields
//! of, `filters list|show|rm`. Every run gets its own temporary config and
//! cache directories and an explicit `--config`.
use assert_cmd::Command;
use serde_json::Value;
use std::path::PathBuf;

/// Distinctive so a leak check cannot trip on prose.
const SECRET: &str = "s3cr3t-cli-filters-70";

fn config() -> String {
    format!(
        r#"default_profile = "demo"
future_top = "kept"

[profiles.demo]
host = "pce.invalid"
org_id = 3

[profiles.demo.filters.traffic.blocked]
service = "443/tcp"
decisions = ["blocked"]
window = "7d"
max_results = 500

[profiles.demo.filters.traffic.broken]
src = 3

[profiles.demo.filters.policy]
deny-prod = "dst:env=prod deny"

[profiles.creds]
host = "127.0.0.1"
port = 9
org_id = 3
api_key = "api_1"
api_secret = "{SECRET}"
future_key = "kept too"

[profiles.creds.filters.traffic.blocked]
service = "443/tcp"
decisions = ["blocked"]
window = "7d"
max_results = 500
"#
    )
}

struct Home {
    dir: tempfile::TempDir,
}

impl Home {
    fn new() -> Self {
        let h = Home {
            dir: tempfile::tempdir().unwrap(),
        };
        std::fs::write(h.cfg(), config()).unwrap();
        h
    }
    fn cfg(&self) -> PathBuf {
        self.dir.path().join("config.toml")
    }
    fn text(&self) -> String {
        std::fs::read_to_string(self.cfg()).unwrap()
    }
    fn run(&self, args: &[&str]) -> Command {
        let mut c = Command::cargo_bin("illumio-tui").unwrap();
        c.env_clear()
            .env("PATH", std::env::var("PATH").unwrap_or_default())
            .env("HOME", self.dir.path())
            .env("XDG_CONFIG_HOME", self.dir.path().join("xdg-config"))
            .env("XDG_CACHE_HOME", self.dir.path().join("xdg-cache"))
            .args(["--config", self.cfg().to_str().unwrap()])
            .args(args);
        c
    }
}

fn json(assert: &assert_cmd::assert::Assert) -> Value {
    serde_json::from_slice(&assert.get_output().stdout).expect("stdout is JSON")
}

fn stderr(assert: &assert_cmd::assert::Assert) -> String {
    String::from_utf8_lossy(&assert.get_output().stderr).into_owned()
}

#[test]
fn save_filter_writes_the_file_without_a_client_and_refuses_an_existing_name() {
    let h = Home::new();
    let a = h
        .run(&[
            "--table",
            "traffic",
            "groups",
            "--src",
            "app=web",
            "--src",
            "app=api",
            "--exclude-dst",
            "app=backup",
            "--save-filter",
            "web",
        ])
        .assert()
        .success();
    let out = String::from_utf8_lossy(&a.get_output().stdout).into_owned();
    assert_eq!(
        out.trim(),
        format!("saved traffic filter web to {}", h.cfg().display())
    );
    let text = h.text();
    assert!(
        text.contains("[profiles.demo.filters.traffic.web]"),
        "{text}"
    );
    assert!(text.contains("src = \"app=web | app=api\""), "{text}");
    assert!(text.contains("dst = \"!app=backup\""), "{text}");
    for kept in [
        "future_top",
        "future_key",
        SECRET,
        "deny-prod",
        "[profiles.demo.filters.traffic.broken]",
    ] {
        assert!(text.contains(kept), "{kept} lost: {text}");
    }
    // Again: refused, then replaced with --force (JSON says so).
    let e = h
        .run(&[
            "traffic",
            "query",
            "--src",
            "app=db",
            "--save-filter",
            "web",
        ])
        .assert()
        .code(2);
    assert!(stderr(&e).contains("--force"), "{}", stderr(&e));
    let a = h
        .run(&[
            "--json",
            "traffic",
            "query",
            "--src",
            "app=db",
            "--save-filter",
            "web",
            "--force",
        ])
        .assert()
        .success();
    let v = json(&a);
    assert_eq!(v["data"]["replaced"], true);
    assert_eq!(v["data"]["filter"]["src"], "app=db");
    assert_eq!(v["meta"]["api_calls"], 0);
}

#[test]
fn save_filter_takes_the_saved_base_and_says_what_it_cannot_hold() {
    let h = Home::new();
    let a = h
        .run(&[
            "--json",
            "traffic",
            "mesh",
            "--filter",
            "blocked",
            "--window",
            "1d",
            "--save-filter",
            "blocked-1d",
        ])
        .assert()
        .success();
    let f = &json(&a)["data"]["filter"];
    assert_eq!(f["window"], "1d", "the flag replaces the saved window");
    assert_eq!(f["service"], "443/tcp");
    assert_eq!(f["decisions"], serde_json::json!(["blocked"]));
    assert_eq!(f["max_results"], 500);
    let e = h
        .run(&[
            "traffic",
            "query",
            "--process",
            "nginx",
            "--save-filter",
            "p",
        ])
        .assert()
        .code(2);
    assert!(stderr(&e).contains("--process"), "{}", stderr(&e));
    let e = h
        .run(&[
            "traffic",
            "query",
            "--filter",
            "broken",
            "--save-filter",
            "p",
        ])
        .assert()
        .code(2);
    let err = stderr(&e);
    assert!(
        err.contains("profile demo")
            && err.contains("traffic filter broken")
            && err.contains("src"),
        "{err}"
    );
    let e = h
        .run(&["traffic", "query", "--save-filter", "a b"])
        .assert()
        .code(2);
    assert!(stderr(&e).contains("not allowed"), "{}", stderr(&e));
}

#[test]
fn policy_search_adds_terms_to_the_saved_query_and_negated_terms_still_parse() {
    let h = Home::new();
    let a = h
        .run(&[
            "--json",
            "policy",
            "search",
            "--filter",
            "deny-prod",
            "-scope:env=dev",
            "--save-filter",
            "deny-prod-not-dev",
        ])
        .assert()
        .success();
    let q = json(&a)["data"]["filter"].as_str().unwrap().to_string();
    assert!(
        q.contains("dst:env=prod") && q.contains("deny") && q.contains("-scope:env=dev"),
        "{q}"
    );
}

#[test]
fn filter_runs_the_saved_filter_as_the_base_of_a_dry_run() {
    let h = Home::new();
    let a = h
        .run(&[
            "--json",
            "--profile",
            "creds",
            "traffic",
            "query",
            "--filter",
            "blocked",
            "--max-results",
            "50",
            "--dry-run",
        ])
        .assert()
        .success();
    let body = &json(&a)["data"];
    assert_eq!(body["max_results"], 50, "the flag wins: {body}");
    assert_eq!(
        body["policy_decisions"],
        serde_json::json!(["blocked"]),
        "{body}"
    );
    let text = body.to_string();
    assert!(text.contains("443"), "{text}");
    let e = h
        .run(&[
            "--profile",
            "creds",
            "traffic",
            "query",
            "--filter",
            "nope",
            "--dry-run",
        ])
        .assert()
        .code(2);
    assert!(stderr(&e).contains("saved: blocked"), "{}", stderr(&e));
}

#[test]
fn filters_list_show_rm_and_csv() {
    let h = Home::new();
    let a = h.run(&["--json", "filters", "list"]).assert().success();
    let rows = json(&a)["data"].as_array().unwrap().clone();
    let names: Vec<(String, String)> = rows
        .iter()
        .map(|r| {
            (
                r["screen"].as_str().unwrap().into(),
                r["name"].as_str().unwrap().into(),
            )
        })
        .collect();
    assert_eq!(
        names,
        vec![
            ("traffic".into(), "blocked".into()),
            ("traffic".into(), "broken".into()),
            ("policy".into(), "deny-prod".into())
        ]
    );
    assert!(rows[1]["error"].as_str().unwrap().contains("src"));
    let a = h
        .run(&["--csv", "filters", "list", "--screen", "policy"])
        .assert()
        .success();
    let csv = String::from_utf8_lossy(&a.get_output().stdout).into_owned();
    assert_eq!(
        csv.lines().collect::<Vec<_>>(),
        vec!["screen,name,summary", "policy,deny-prod,dst:env=prod deny"]
    );
    let a = h
        .run(&["--json", "filters", "show", "traffic", "blocked"])
        .assert()
        .success();
    assert_eq!(json(&a)["data"]["filter"]["window"], "7d");
    h.run(&["--json", "filters", "rm", "policy", "deny-prod"])
        .assert()
        .success();
    assert!(!h.text().contains("deny-prod"));
    assert!(h.text().contains(SECRET));
    let e = h
        .run(&["filters", "rm", "policy", "deny-prod"])
        .assert()
        .code(2);
    assert!(stderr(&e).contains("no policy filter"), "{}", stderr(&e));
    h.run(&["--profile", "creds", "--json", "filters", "list"])
        .assert()
        .success()
        .stdout(predicates::str::contains("\"blocked\""));
}

#[test]
fn an_env_file_or_env_only_connection_has_no_saved_filters() {
    let h = Home::new();
    let env = h.dir.path().join("pce.env");
    std::fs::write(&env, "PCE_HOST=x\nPCE_ORG_ID=1\nAPI_KEY=k\nAPI_SECRET=s\n").unwrap();
    let e = h
        .run(&[
            "--env-file",
            env.to_str().unwrap(),
            "events",
            "--filter",
            "x",
        ])
        .assert()
        .code(2);
    assert!(stderr(&e).contains("--env-file"), "{}", stderr(&e));
    let e = h
        .run(&["--profile", "ghost", "events", "--save-filter", "x"])
        .assert()
        .code(2);
    let err = stderr(&e);
    assert!(err.contains("ghost") && err.contains("is not in"), "{err}");
    // --csv with --save-filter: no table to print.
    h.run(&["--csv", "events", "--save-filter", "x"])
        .assert()
        .code(2);
}
