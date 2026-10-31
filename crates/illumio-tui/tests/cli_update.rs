//! `update check` and `news` (#91) against a mock release list
//! (`ILLUMIO_TUI_UPDATE_URL`): every request pinned with `.expect`, the
//! cache under a temp `XDG_CACHE_HOME`, the JSON checked against its
//! schema, exit 10 (`--fail-if-outdated`) and 5 (GitHub unreachable). No
//! PCE is configured: neither command needs one.
#[path = "common/schema_check.rs"]
mod schema_check;
use assert_cmd::Command;
use illumio_tui::updates::{Cache, Release, Version};
use serde_json::{Value, json};
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

const RELEASES: &str = "/repos/alexgoller/illumio-tui/releases";

struct Env {
    home: tempfile::TempDir,
}

impl Env {
    fn new() -> Self {
        Self {
            home: tempfile::tempdir().unwrap(),
        }
    }

    fn cache_file(&self) -> std::path::PathBuf {
        self.home.path().join("cache/illumio-tui/update.json")
    }

    fn cmd(&self, url: &str) -> Command {
        let mut c = Command::cargo_bin("illumio-tui").unwrap();
        c.env_clear()
            .env("PATH", std::env::var("PATH").unwrap_or_default())
            .env("ILLUMIO_TUI_UPDATE_URL", url)
            .env("XDG_CONFIG_HOME", self.home.path().join("config"))
            .env("XDG_CACHE_HOME", self.home.path().join("cache"))
            .env("XDG_STATE_HOME", self.home.path().join("state"))
            .env("XDG_DATA_HOME", self.home.path().join("data"));
        c
    }
}

fn current() -> Version {
    Version::current()
}

/// The next patch release after this binary.
fn next() -> String {
    let c = current();
    Version(c.0, c.1, c.2 + 1).to_string()
}

fn release(version: &str, body: &str) -> Value {
    json!({"tag_name": format!("v{version}"), "body": body, "draft": false,
           "prerelease": false, "published_at": "2026-11-02T09:00:00Z"})
}

async fn github(list: Value, times: u64) -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(RELEASES))
        .and(query_param("per_page", "30"))
        .respond_with(ResponseTemplate::new(200).set_body_json(list))
        .expect(times)
        .mount(&server)
        .await;
    server
}

/// A server that fails the test if anything reaches it.
async fn silent() -> MockServer {
    let server = MockServer::start().await;
    Mock::given(wiremock::matchers::any())
        .respond_with(ResponseTemplate::new(500))
        .expect(0)
        .mount(&server)
        .await;
    server
}

fn stdout(a: &assert_cmd::assert::Assert) -> String {
    String::from_utf8_lossy(&a.get_output().stdout).into_owned()
}

fn stderr(a: &assert_cmd::assert::Assert) -> String {
    String::from_utf8_lossy(&a.get_output().stderr).into_owned()
}

#[tokio::test]
async fn check_names_the_newer_release_and_how_to_upgrade() {
    let env = Env::new();
    let list = json!([
        release(&next(), "## New\n\n- something"),
        {"tag_name": "v99.0.0", "body": "draft", "draft": true, "prerelease": false},
        {"tag_name": "v98.0.0", "body": "pre", "draft": false, "prerelease": true},
        release(illumio_tui::updates::CURRENT, "this one"),
    ]);
    let server = github(list, 3).await;
    let a = env
        .cmd(&server.uri())
        .args(["update", "check", "--table"])
        .assert()
        .success();
    let text = stdout(&a);
    assert!(
        text.starts_with(&format!(
            "illumio-tui {} is available (you have {})\nupgrade: ",
            next(),
            current()
        )),
        "{text}"
    );
    // Run from the build tree: no package manager, the release page.
    assert!(
        text.contains(&format!(
            "https://github.com/alexgoller/illumio-tui/releases/tag/v{}",
            next()
        )),
        "{text}"
    );
    let cache = Cache::load(&env.cache_file()).expect("the cache is written");
    assert_eq!(cache.latest.as_deref(), Some(next().as_str()));
    assert_eq!(cache.releases.len(), 2, "every release read");

    let a = env
        .cmd(&server.uri())
        .args(["update", "check", "--json"])
        .assert()
        .success();
    let v = schema_check::envelope(&a.get_output().stdout);
    assert_eq!(v["meta"]["schema"], "update-check");
    assert_eq!(v["meta"]["api_calls"], 0, "GitHub is no PCE call");
    assert_eq!(v["data"]["outdated"], true);
    assert_eq!(v["data"]["latest"], next());
    assert_eq!(v["data"]["current"], current().to_string());
    assert_eq!(v["data"]["install"], "other");

    let a = env
        .cmd(&server.uri())
        .args(["update", "check", "--json", "--fail-if-outdated"])
        .assert()
        .code(10);
    schema_check::envelope(&a.get_output().stdout);
    let err: Value = serde_json::from_str(stderr(&a).trim()).unwrap();
    assert_eq!(err["error"]["kind"], "outdated");
    assert_eq!(err["error"]["exit_code"], 10);
}

#[tokio::test]
async fn check_up_to_date_is_the_latest_and_fields_pick() {
    let env = Env::new();
    let server = github(json!([release(illumio_tui::updates::CURRENT, "x")]), 2).await;
    let a = env
        .cmd(&server.uri())
        .args(["update", "check", "--table", "--fail-if-outdated"])
        .assert()
        .success();
    assert_eq!(
        stdout(&a),
        format!("illumio-tui {} is the latest\n", current())
    );
    let a = env
        .cmd(&server.uri())
        .args(["update", "check", "--json", "--fields", "latest,outdated"])
        .assert()
        .success();
    let v = schema_check::picked(&a.get_output().stdout);
    assert_eq!(
        v["data"],
        json!({"latest": current().to_string(), "outdated": false})
    );
}

#[tokio::test]
async fn check_without_github_exits_4_and_bad_flags_send_nothing() {
    let env = Env::new();
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(503))
        .expect(1)
        .mount(&server)
        .await;
    let a = env
        .cmd(&server.uri())
        .args(["update", "check"])
        .assert()
        .code(4);
    let err: Value = serde_json::from_str(stderr(&a).trim()).unwrap();
    assert_eq!(err["error"]["kind"], "network");
    assert!(
        err["error"]["message"]
            .as_str()
            .unwrap()
            .contains("GitHub answered HTTP 503"),
        "{err}"
    );
    // Nothing listens there.
    let a = env
        .cmd("http://127.0.0.1:9")
        .args(["update", "check"])
        .assert()
        .code(4);
    assert!(stderr(&a).contains("cannot reach GitHub"), "{}", stderr(&a));
    // GitHub's rate limit: exit 5, said plainly.
    let limited = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(403).insert_header("x-ratelimit-remaining", "0"))
        .expect(1)
        .mount(&limited)
        .await;
    let a = env
        .cmd(&limited.uri())
        .args(["update", "check"])
        .assert()
        .code(5);
    let err: Value = serde_json::from_str(stderr(&a).trim()).unwrap();
    assert_eq!(err["error"]["kind"], "rate_limited");
    assert_eq!(
        err["error"]["message"],
        "GitHub rate limit reached; try again later"
    );
    let quiet = silent().await;
    env.cmd(&quiet.uri())
        .args(["update", "check", "--csv"])
        .assert()
        .code(2);
    env.cmd(&quiet.uri())
        .args(["update", "check", "--json", "--fields", "nope"])
        .assert()
        .code(2);
}

#[tokio::test]
async fn news_lists_the_newer_releases_and_reads_the_cache_a_day() {
    let env = Env::new();
    let server = github(
        json!([
            release(&next(), "## New\n\n- **News** (#91)"),
            release(illumio_tui::updates::CURRENT, "old")
        ]),
        1,
    )
    .await;
    // The first asks GitHub once; the second reads the fresh cache.
    for _ in 0..2 {
        let a = env.cmd(&server.uri()).arg("news").assert().success();
        assert_eq!(
            stdout(&a),
            format!("# v{} — 2026-11-02\n\n## New\n\n- **News** (#91)\n", next())
        );
    }
    let a = env
        .cmd(&server.uri())
        .args(["news", "--json"])
        .assert()
        .success();
    let v = schema_check::envelope(&a.get_output().stdout);
    assert_eq!(v["meta"]["schema"], "news");
    assert_eq!(v["meta"]["cache"], "hit");
    assert_eq!(v["data"][0]["version"], next());
    assert_eq!(v["data"].as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn news_of_this_version_and_older_needs_no_request() {
    let env = Env::new();
    let quiet = silent().await;
    // Up to date (a fresh cache with nothing newer): this version's notes.
    Cache::from_fetch(
        &[Release {
            version: current().to_string(),
            date: None,
            body: String::new(),
        }],
        jiff::Timestamp::now(),
    )
    .save(&env.cache_file())
    .unwrap();
    let a = env.cmd(&quiet.uri()).arg("news").assert().success();
    assert!(
        stdout(&a).starts_with(&format!("# v{} — ", current())),
        "{}",
        stdout(&a)
    );
    let a = env
        .cmd(&quiet.uri())
        .args(["news", "--offline", "--json"])
        .assert()
        .success();
    let v = schema_check::envelope(&a.get_output().stdout);
    assert_eq!(v["data"][0]["version"], current().to_string());
    assert_eq!(v["meta"]["cache"], "none");
    // A closed range up to this version: the built-in notes alone.
    let a = env
        .cmd(&quiet.uri())
        .args(["news", "0.5.6..0.5.8", "--json", "--fields", "version"])
        .assert()
        .success();
    let v = schema_check::picked(&a.get_output().stdout);
    assert_eq!(
        v["data"],
        json!([{"version": "0.5.8"}, {"version": "0.5.7"}])
    );
    let a = env
        .cmd(&quiet.uri())
        .args(["news", "0.5.6..", "--offline"])
        .assert()
        .success();
    assert!(
        stderr(&a).contains("--offline: the built-in notes only"),
        "{}",
        stderr(&a)
    );
    assert!(
        stdout(&a).contains("# v0.5.7 — 2026-10-26"),
        "{}",
        stdout(&a)
    );
    assert!(!stdout(&a).contains("# v0.5.6 "), "after 0.5.6, not from");
    for bad in [
        vec!["news", "0.5.8"],
        vec!["news", "0.5.8..0.5.6"],
        vec!["news", "--csv"],
    ] {
        env.cmd(&quiet.uri()).args(&bad).assert().code(2);
    }
}

#[tokio::test]
async fn news_without_github_falls_back_with_a_warning() {
    let env = Env::new();
    let a = env
        .cmd("http://127.0.0.1:9")
        .args(["news", "--json"])
        .assert()
        .success();
    let v = schema_check::envelope(&a.get_output().stdout);
    assert_eq!(v["data"][0]["version"], current().to_string());
    let w = v["meta"]["warnings"][0].as_str().unwrap();
    assert!(
        w.contains("cannot reach GitHub") && w.contains("built-in notes"),
        "{w}"
    );
}

#[test]
fn the_schemas_print() {
    let env = Env::new();
    for (words, title) in [
        (vec!["update", "check"], "update-check"),
        (vec!["news"], "news"),
    ] {
        let a = env
            .cmd("http://127.0.0.1:9")
            .arg("schema")
            .args(&words)
            .assert()
            .success();
        let doc: Value = serde_json::from_slice(&a.get_output().stdout).unwrap();
        assert_eq!(doc["title"], format!("illumio-tui {title}"));
    }
}
