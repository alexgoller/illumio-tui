//! Saved filters on the command line: `--save-filter` writes the config
//! file and never builds a client (the profile has no credentials, so any
//! attempt would fail), `--filter` is the base the flags replace fields
//! of, `filters list|show|rm|default` and `--default-filter`. Every run gets its own temporary config and
//! cache directories and an explicit `--config`.
#[path = "common/schema_check.rs"]
mod schema_check;
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

[profiles.demo.filters.traffic.baddec]
decisions = ["nope"]

[profiles.demo.filters.policy]
deny-prod = "dst:env=prod deny"
badq = "dst:"

[profiles.demo.filters.events.badtype]
type = "!"

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
    schema_check::envelope(&assert.get_output().stdout)
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
            ("traffic".into(), "baddec".into()),
            ("traffic".into(), "blocked".into()),
            ("traffic".into(), "broken".into()),
            ("policy".into(), "badq".into()),
            ("policy".into(), "deny-prod".into()),
            ("events".into(), "badtype".into())
        ]
    );
    assert!(rows[2]["error"].as_str().unwrap().contains("src"));
    let a = h
        .run(&["--csv", "filters", "list", "--screen", "policy"])
        .assert()
        .success();
    let csv = String::from_utf8_lossy(&a.get_output().stdout).into_owned();
    assert_eq!(
        csv.lines().collect::<Vec<_>>(),
        vec![
            "screen,name,default,summary",
            "policy,badq,,dst:",
            "policy,deny-prod,,dst:env=prod deny"
        ]
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

/// Workloads filters with the VEN health fields (#72), one broken.
const VEN_FILTERS: &str = r#"
[profiles.demo.filters.workloads.badhealth]
health = ["warn"]

[profiles.demo.filters.workloads.unhealthy]
health = "warning, error,offline"
ven_version = "23.2.*"
"#;

impl Home {
    fn with_ven_filters() -> Self {
        let h = Home::new();
        std::fs::write(h.cfg(), config() + VEN_FILTERS).unwrap();
        h
    }
}

#[test]
fn every_broken_saved_field_names_profile_screen_filter_and_field() {
    let h = Home::with_ven_filters();
    for (args, want) in [
        (
            vec![
                "traffic",
                "query",
                "--filter",
                "baddec",
                "--save-filter",
                "x",
            ],
            "profile demo, traffic filter baddec: field decisions",
        ),
        (
            vec!["policy", "search", "--filter", "badq", "--save-filter", "x"],
            "profile demo, policy filter badq: query",
        ),
        (
            vec!["events", "--filter", "badtype", "--save-filter", "x"],
            "profile demo, events filter badtype: field type",
        ),
        (
            vec!["workloads", "--filter", "badhealth", "--save-filter", "x"],
            "profile demo: workloads filter badhealth: field health",
        ),
        (
            vec![
                "workloads",
                "ven-summary",
                "--filter",
                "badhealth",
                "--save-filter",
                "x",
            ],
            "profile demo: workloads filter badhealth: field health",
        ),
    ] {
        let e = h.run(&args).assert().code(2);
        let err = stderr(&e);
        assert!(err.contains(want), "{args:?}: {err}");
    }
}

#[test]
fn a_wrong_shaped_screen_key_breaks_only_that_screen() {
    let h = Home::new();
    let text = h
        .text()
        .replace("[profiles.demo.filters.events.badtype]\ntype = \"!\"\n", "");
    let text = text.replace(
        "[profiles.demo.filters.traffic.blocked]",
        "[profiles.demo.filters]\nevents = \"oops\"\n\n[profiles.demo.filters.traffic.blocked]",
    );
    std::fs::write(h.cfg(), text).unwrap();
    h.run(&["--json", "profile", "list"]).assert().success();
    h.run(&["--json", "--profile", "creds", "filters", "list"])
        .assert()
        .success();
    let a = h
        .run(&["--json", "filters", "list", "--screen", "events"])
        .assert()
        .success();
    let rows = json(&a)["data"].clone();
    assert!(
        rows[0]["error"]
            .as_str()
            .unwrap()
            .contains("filters.events"),
        "{rows}"
    );
    let e = h
        .run(&["events", "--filter", "x", "--save-filter", "y"])
        .assert()
        .code(2);
    let err = stderr(&e);
    assert!(
        err.contains("profile demo") && err.contains("filters.events"),
        "{err}"
    );
    h.run(&[
        "traffic",
        "query",
        "--filter",
        "blocked",
        "--save-filter",
        "b2",
    ])
    .assert()
    .success();
    assert!(h.text().contains("events = \"oops\""), "kept as it was");
}

#[test]
fn json_save_carries_screen_name_and_path_and_a_leading_dash_is_refused() {
    let h = Home::new();
    let a = h
        .run(&["--json", "events", "--type", "agent", "--save-filter", "ag"])
        .assert()
        .success();
    let d = &json(&a)["data"];
    assert_eq!(d["screen"], "events");
    assert_eq!(d["name"], "ag");
    assert_eq!(d["config_path"], h.cfg().to_str().unwrap());
    let e = h.run(&["events", "--save-filter=-x"]).assert().code(2);
    assert!(stderr(&e).contains("starts with -"), "{}", stderr(&e));
}

#[test]
fn an_events_actor_saves_as_the_editor_text_and_shows() {
    let h = Home::new();
    h.run(&[
        "events",
        "--actor",
        "user:*",
        "--exclude-actor",
        "system,=alex@ryte.de",
        "--save-filter",
        "people",
    ])
    .assert()
    .success();
    assert!(
        h.text().contains(
            "[profiles.demo.filters.events.people]\nactor = \"user:*, !system, !=alex@ryte.de\"\n"
        ),
        "{}",
        h.text()
    );
    let a = h
        .run(&["--json", "filters", "list", "--screen", "events"])
        .assert()
        .success();
    let rows = json(&a)["data"].as_array().unwrap().clone();
    assert!(
        rows.iter().any(|r| r["name"] == "people"
            && r["summary"]
                .as_str()
                .is_some_and(|s| s == "actor user:*, !system, !=alex@ryte.de")),
        "{rows:?}"
    );
    let e = h
        .run(&["events", "--actor", "a !b", "--save-filter", "bad"])
        .assert()
        .code(2);
    assert!(stderr(&e).contains("actor"), "{}", stderr(&e));
}

/// The dry-run body of `traffic query` on profile `creds` with `extra`.
fn creds_body(h: &Home, extra: &[&str]) -> Value {
    let mut args = vec![
        "--json",
        "--profile",
        "creds",
        "traffic",
        "query",
        "--dry-run",
        // A fixed window: two runs a second apart give the same body.
        "--from",
        "2026-01-01T00:00:00Z",
        "--to",
        "2026-01-02T00:00:00Z",
    ];
    args.extend_from_slice(extra);
    let a = h.run(&args).assert().success();
    json(&a)["data"].clone()
}

#[test]
fn a_default_applies_only_with_default_filter_never_implicitly() {
    let h = Home::new();
    let plain = creds_body(&h, &[]);
    let a = h
        .run(&[
            "--json",
            "--profile",
            "creds",
            "filters",
            "default",
            "traffic",
            "blocked",
        ])
        .assert()
        .success();
    let d = &json(&a)["data"];
    assert_eq!(
        (d["screen"].as_str(), d["default"].as_str()),
        (Some("traffic"), Some("blocked"))
    );
    assert!(d["previous"].is_null(), "{d}");
    assert!(
        h.text().contains("[profiles.creds.filter_defaults]"),
        "{}",
        h.text()
    );
    assert!(h.text().contains(SECRET) && h.text().contains("future_key"));
    // Without the flag: the same body as before the default was set.
    assert_eq!(creds_body(&h, &[]), plain);
    let with = creds_body(&h, &["--default-filter", "--max-results", "50"]);
    assert_eq!(with["max_results"], 50, "a flag still wins: {with}");
    assert_eq!(
        with["policy_decisions"],
        serde_json::json!(["blocked"]),
        "{with}"
    );
    assert_eq!(
        with,
        creds_body(&h, &["--filter", "blocked", "--max-results", "50"])
    );
    // It conflicts with --filter.
    let e = h
        .run(&[
            "--profile",
            "creds",
            "traffic",
            "query",
            "--dry-run",
            "--default-filter",
            "--filter",
            "blocked",
        ])
        .assert()
        .code(2);
    assert!(stderr(&e).contains("cannot be used with"), "{}", stderr(&e));
    // A screen without a default: an error, nothing sent.
    let e = h
        .run(&["--profile", "creds", "events", "--default-filter"])
        .assert()
        .code(2);
    assert!(
        stderr(&e).contains("profile creds has no default events filter")
            && stderr(&e).contains("filters default events NAME"),
        "{}",
        stderr(&e)
    );
}

#[test]
fn default_filter_keeps_the_policy_search_term_rewrite() {
    let h = Home::new();
    h.run(&["filters", "default", "policy", "deny-prod"])
        .assert()
        .success();
    let a = h
        .run(&[
            "--json",
            "policy",
            "search",
            "--default-filter",
            "-scope:env=dev",
            "--save-filter",
            "from-default",
        ])
        .assert()
        .success();
    let q = json(&a)["data"]["filter"].as_str().unwrap().to_string();
    assert!(
        q.contains("dst:env=prod") && q.contains("-scope:env=dev"),
        "{q}"
    );
}

#[test]
fn filters_default_sets_unsets_lists_and_goes_with_rm() {
    let h = Home::new();
    let e = h
        .run(&["filters", "default", "traffic", "nope"])
        .assert()
        .code(2);
    assert!(
        stderr(&e).contains("saved: baddec, blocked, broken"),
        "{}",
        stderr(&e)
    );
    h.run(&["filters", "default", "traffic"]).assert().code(2);
    h.run(&["filters", "default", "traffic", "blocked", "--unset"])
        .assert()
        .code(2);
    let a = h
        .run(&["--table", "filters", "default", "policy", "deny-prod"])
        .assert()
        .success();
    let out = String::from_utf8_lossy(&a.get_output().stdout).into_owned();
    assert!(out.contains("deny-prod"), "{out}");
    // list: a default column in JSON, CSV and the table.
    let a = h
        .run(&["--json", "filters", "list", "--screen", "policy"])
        .assert()
        .success();
    let rows = json(&a)["data"].as_array().unwrap().clone();
    let defaults: Vec<(&str, bool)> = rows
        .iter()
        .map(|r| (r["name"].as_str().unwrap(), r["default"].as_bool().unwrap()))
        .collect();
    assert_eq!(defaults, vec![("badq", false), ("deny-prod", true)]);
    let a = h
        .run(&["--csv", "filters", "list", "--screen", "policy"])
        .assert()
        .success();
    let csv = String::from_utf8_lossy(&a.get_output().stdout).into_owned();
    assert!(
        csv.contains("policy,deny-prod,yes,dst:env=prod deny"),
        "{csv}"
    );
    let a = h
        .run(&["--table", "filters", "list", "--screen", "policy"])
        .assert()
        .success();
    let table = String::from_utf8_lossy(&a.get_output().stdout).into_owned();
    assert!(table.lines().next().unwrap().contains("default"), "{table}");
    // rm of the default clears it and says so.
    let a = h
        .run(&["--json", "filters", "rm", "policy", "deny-prod"])
        .assert()
        .success();
    assert_eq!(json(&a)["data"]["default_cleared"], true);
    assert!(!h.text().contains("filter_defaults"), "{}", h.text());
    h.run(&["filters", "default", "traffic", "blocked"])
        .assert()
        .success();
    let a = h
        .run(&["--table", "filters", "rm", "traffic", "blocked"])
        .assert()
        .success();
    let out = String::from_utf8_lossy(&a.get_output().stdout).into_owned();
    assert!(
        out.contains("traffic filter blocked and the traffic default"),
        "{out}"
    );
    // --unset.
    h.run(&["filters", "default", "events", "badtype"])
        .assert()
        .success();
    let a = h
        .run(&["--json", "filters", "default", "events", "--unset"])
        .assert()
        .success();
    let d = &json(&a)["data"];
    assert!(d["default"].is_null() && d["previous"] == "badtype", "{d}");
    h.run(&["--csv", "filters", "default", "events", "--unset"])
        .assert()
        .code(2);
}

#[test]
fn a_hand_edited_default_that_does_not_resolve_is_said_where_used() {
    let h = Home::new();
    let text = h.text().replace(
        "[profiles.demo.filters.traffic.blocked]",
        "[profiles.demo.filter_defaults]\ntraffic = \"broken\"\npolicy = \"gone\"\nevents = 4\n\n\
         [profiles.demo.filters.traffic.blocked]",
    );
    std::fs::write(h.cfg(), text).unwrap();
    let e = h
        .run(&["traffic", "query", "--default-filter", "--save-filter", "x"])
        .assert()
        .code(2);
    assert!(
        stderr(&e).contains("--default-filter: profile demo: traffic filter broken: field src"),
        "{}",
        stderr(&e)
    );
    let e = h
        .run(&["policy", "search", "--default-filter", "--save-filter", "x"])
        .assert()
        .code(2);
    assert!(
        stderr(&e).contains("--default-filter: profile demo: no policy filter")
            && stderr(&e).contains("gone"),
        "{}",
        stderr(&e)
    );
    let e = h
        .run(&["events", "--default-filter", "--save-filter", "x"])
        .assert()
        .code(2);
    assert!(
        stderr(&e).contains("filter_defaults.events is an integer"),
        "{}",
        stderr(&e)
    );
    // list shows each, other commands are not stopped.
    let a = h.run(&["--json", "filters", "list"]).assert().success();
    let rows = json(&a)["data"].as_array().unwrap().clone();
    let gone = rows.iter().find(|r| r["name"] == "gone").unwrap();
    assert!(
        gone["default"].as_bool().unwrap() && gone["error"].as_str().unwrap().contains("ignored")
    );
    let broken = rows.iter().find(|r| r["name"] == "broken").unwrap();
    assert!(broken["default"].as_bool().unwrap(), "{broken}");
    assert!(
        rows.iter()
            .any(|r| r["screen"] == "events" && r["name"] == "-" && r["default"] == true)
    );
    h.run(&[
        "traffic",
        "query",
        "--filter",
        "blocked",
        "--save-filter",
        "y",
    ])
    .assert()
    .success();
}

#[test]
fn wrong_shaped_filters_or_defaults_stop_nothing_else() {
    let h = Home::new();
    let text = h.text().replace(
        "future_key = \"kept too\"",
        "future_key = \"kept too\"\nfilters = \"x\"\nfilter_defaults = 3",
    );
    let text = text.replace(
        "[profiles.creds.filters.traffic.blocked]\nservice = \"443/tcp\"\ndecisions = [\"blocked\"]\nwindow = \"7d\"\nmax_results = 500\n",
        "",
    );
    assert!(text.contains("filters = \"x\""), "{text}");
    std::fs::write(h.cfg(), &text).unwrap();
    h.run(&["--json", "profile", "list"]).assert().success();
    h.run(&["--json", "filters", "list"]).assert().success();
    let a = h
        .run(&["--json", "--profile", "creds", "filters", "list"])
        .assert()
        .success();
    let rows = json(&a)["data"].as_array().unwrap().clone();
    let errors: Vec<&str> = rows.iter().map(|r| r["error"].as_str().unwrap()).collect();
    assert_eq!(
        errors,
        vec![
            "filters is a string, not a table of screens",
            "filter_defaults is an integer, not a table of screen = filter name"
        ]
    );
    let e = h
        .run(&[
            "--profile",
            "creds",
            "traffic",
            "query",
            "--dry-run",
            "--filter",
            "x",
        ])
        .assert()
        .code(2);
    assert!(
        stderr(&e).contains("profile creds: filters is a string"),
        "{}",
        stderr(&e)
    );
    let e = h
        .run(&[
            "--profile",
            "creds",
            "traffic",
            "query",
            "--dry-run",
            "--default-filter",
        ])
        .assert()
        .code(2);
    assert!(
        stderr(&e).contains("filter_defaults is an integer"),
        "{}",
        stderr(&e)
    );
    // Unchanged, and a write elsewhere keeps both values.
    h.run(&[
        "traffic",
        "query",
        "--filter",
        "blocked",
        "--save-filter",
        "z",
    ])
    .assert()
    .success();
    let after = h.text();
    assert!(
        after.contains("filters = \"x\"") && after.contains("filter_defaults = 3"),
        "{after}"
    );
    h.run(&["--profile", "creds", "traffic", "query", "--dry-run"])
        .assert()
        .success();
}

#[test]
fn workloads_health_and_ven_version_save_and_load_list_and_ven_summary_alike() {
    let h = Home::with_ven_filters();
    h.run(&[
        "workloads",
        "--health",
        "error,warning",
        "--ven-version",
        "23.*",
        "--save-filter",
        "mine",
    ])
    .assert()
    .success();
    let text = h.text();
    assert!(
        text.contains("[profiles.demo.filters.workloads.mine]"),
        "{text}"
    );
    let mine = &text[text.find("workloads.mine]").unwrap()..];
    let health: String = mine.split_whitespace().collect();
    assert!(
        health.contains(r#"health=["warning","error",]"#),
        "the known order: {text}"
    );
    assert!(text.contains(r#"ven_version = "23.*""#), "{text}");
    // The saved base under ven-summary; a flag replaces its field.
    h.run(&[
        "workloads",
        "ven-summary",
        "--filter",
        "unhealthy",
        "--ven-version",
        "22.*",
        "--save-filter",
        "copy",
    ])
    .assert()
    .success();
    let a = h
        .run(&["--json", "filters", "show", "workloads", "copy"])
        .assert()
        .success();
    let out = json(&a);
    let body = out["data"].to_string();
    assert!(body.contains("offline"), "{body}");
    assert!(body.contains("22.*"), "the flag wins: {body}");
    assert!(!body.contains("23.2.*"), "{body}");
}
