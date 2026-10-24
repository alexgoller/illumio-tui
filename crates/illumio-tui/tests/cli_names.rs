//! The PCE names cache shell completion reads: filled as a side effect of
//! list commands against a mock PCE, by `completions refresh` (one read of
//! each list), deleted by `completions refresh --clear` (no request), and
//! never written with `completion_cache = false` or for a PCE a flag or
//! the environment names. A temp `XDG_CACHE_HOME`; the PCE is the config
//! file's profile `lab` (only the credentials come from the environment).
#[path = "common/schema_check.rs"]
mod schema_check;
use assert_cmd::Command;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// The binary against `server` through the config file's profile `lab`
/// (written unless there is a config), its config and cache under `home`.
fn cmd(server: &MockServer, home: &Path) -> Command {
    let url = url::Url::parse(&server.uri()).unwrap();
    let file = home.join("config").join("illumio-tui").join("config.toml");
    if !file.exists() {
        config(
            home,
            &format!(
                "default_profile = \"lab\"\n[profiles.lab]\nhost = \"{}\"\nport = {}\n\
                 org_id = 5\nscheme = \"http\"\n",
                url.host_str().unwrap(),
                url.port().unwrap()
            ),
        );
    }
    let mut c = Command::cargo_bin("illumio-tui").unwrap();
    c.env_clear()
        .env("PATH", std::env::var("PATH").unwrap_or_default())
        .env("API_KEY", "key")
        .env("API_SECRET", "secret")
        .env("XDG_CONFIG_HOME", home.join("config"))
        .env("XDG_CACHE_HOME", home.join("cache"))
        .arg("--json");
    c
}

fn names_file(home: &Path, profile: &str) -> PathBuf {
    home.join("cache")
        .join("illumio-tui")
        .join(profile)
        .join("names.json")
}

fn names(home: &Path) -> Value {
    serde_json::from_slice(&std::fs::read(names_file(home, "lab")).unwrap()).unwrap()
}

fn config(home: &Path, text: &str) {
    let dir = home.join("config").join("illumio-tui");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("config.toml"), text).unwrap();
}

async fn mount(server: &MockServer, api: &str, body: Value) {
    Mock::given(method("GET"))
        .and(path(format!("/api/v2/orgs/5/{api}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .mount(server)
        .await;
}

async fn mount_all(server: &MockServer) {
    mount(
        server,
        "labels",
        json!([
            {"href": "/orgs/5/labels/1", "key": "app", "value": "shop"},
            {"href": "/orgs/5/labels/2", "key": "env", "value": "prod"},
            {"href": "/orgs/5/labels/3", "key": "env", "value": "gone", "deleted": true}
        ]),
    )
    .await;
    mount(
        server,
        "sec_policy/draft/label_groups",
        json!([{"href": "/orgs/5/sec_policy/draft/label_groups/1", "name": "Prod envs", "key": "env"}]),
    )
    .await;
    mount(
        server,
        "sec_policy/draft/ip_lists",
        json!([{"href": "/orgs/5/sec_policy/draft/ip_lists/1", "name": "Corp"}]),
    )
    .await;
    mount(
        server,
        "sec_policy/draft/services",
        json!([{"href": "/orgs/5/sec_policy/draft/services/1", "name": "SSH"}]),
    )
    .await;
    mount(
        server,
        "workloads",
        json!([{"href": "/orgs/5/workloads/1", "hostname": "web1", "labels": [
            {"href": "/orgs/5/labels/1", "key": "app", "value": "shop"},
            {"href": "/orgs/5/labels/2", "key": "env", "value": "prod"}]}]),
    )
    .await;
}

#[tokio::test]
async fn list_commands_fill_the_cache_each_their_own_part() {
    let server = MockServer::start().await;
    mount_all(&server).await;
    let home = tempfile::tempdir().unwrap();
    let h = home.path();
    cmd(&server, h).arg("labels").assert().success();
    let n = names(h);
    assert_eq!(n["version"], 1);
    assert_eq!(n["host"], "127.0.0.1");
    assert_eq!(n["org_id"], 5);
    assert_eq!(n["labels"], json!(["app=shop", "env=prod"]));
    assert_eq!(n["label_keys"], json!(["app", "env"]));
    assert!(n["written_at"].is_string());
    cmd(&server, h).arg("services").assert().success();
    cmd(&server, h).arg("iplists").assert().success();
    cmd(&server, h).arg("label-groups").assert().success();
    cmd(&server, h).arg("workloads").assert().success();
    let n = names(h);
    assert_eq!(n["labels"], json!(["app=shop", "env=prod"]), "kept");
    assert_eq!(n["services"], json!(["SSH"]));
    assert_eq!(n["ip_lists"], json!(["Corp"]));
    assert_eq!(n["label_groups"], json!(["Prod envs"]));
    assert_eq!(n["hostnames"], json!(["web1"]));
    assert_eq!(n["apps"], json!(["shop/prod"]), "the profile's scope keys");
    // A filtered list adds what it saw; the whole list replaces.
    std::fs::write(
        names_file(h, "lab"),
        serde_json::to_vec(
            &json!({"version": 1, "host": "127.0.0.1", "org_id": 5, "hostnames": ["old"]}),
        )
        .unwrap(),
    )
    .unwrap();
    cmd(&server, h)
        .args(["workloads", "--hostname", "web"])
        .assert()
        .success();
    assert_eq!(names(h)["hostnames"], json!(["old", "web1"]));
    cmd(&server, h).arg("workloads").assert().success();
    assert_eq!(names(h)["hostnames"], json!(["web1"]));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let file = names_file(h, "lab");
        let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&file), 0o600);
        assert_eq!(mode(file.parent().unwrap()), 0o700);
    }
}

#[tokio::test]
async fn completion_cache_false_writes_nothing_and_refresh_reads_nothing() {
    let server = MockServer::start().await;
    mount_all(&server).await;
    let home = tempfile::tempdir().unwrap();
    let h = home.path();
    cmd(&server, h); // writes the config
    let text = std::fs::read_to_string(h.join("config/illumio-tui/config.toml")).unwrap();
    config(h, &format!("completion_cache = false\n{text}"));
    cmd(&server, h).arg("labels").assert().success();
    cmd(&server, h).arg("workloads").assert().success();
    assert!(!h.join("cache").exists(), "nothing on disk");
    let before = server.received_requests().await.unwrap().len();
    let out = cmd(&server, h)
        .args(["completions", "refresh"])
        .assert()
        .code(2);
    let err = String::from_utf8_lossy(&out.get_output().stderr).to_string();
    assert!(err.contains("completion_cache = false"), "{err}");
    assert_eq!(server.received_requests().await.unwrap().len(), before);
    assert!(!h.join("cache").exists());
}

#[tokio::test]
async fn a_cache_that_cannot_be_written_fails_no_list_command() {
    let server = MockServer::start().await;
    mount_all(&server).await;
    let home = tempfile::tempdir().unwrap();
    // The cache directory is a file: every write fails.
    std::fs::write(home.path().join("cache"), "not a directory").unwrap();
    let out = cmd(&server, home.path()).arg("labels").assert().success();
    let v: Value = schema_check::envelope(&out.get_output().stdout);
    assert_eq!(v["data"].as_array().unwrap().len(), 2);
}

#[tokio::test]
async fn refresh_reads_each_list_once_and_clear_sends_nothing() {
    let server = MockServer::start().await;
    mount_all(&server).await;
    let home = tempfile::tempdir().unwrap();
    let h = home.path();
    let out = cmd(&server, h)
        .args(["completions", "refresh"])
        .assert()
        .success();
    let v: Value = schema_check::envelope(&out.get_output().stdout);
    assert_eq!(
        v["data"]["counts"],
        json!({"labels": 2, "label_keys": 2, "apps": 1, "hostnames": 1,
               "services": 1, "ip_lists": 1, "label_groups": 1})
    );
    assert_eq!(v["data"]["age_seconds"], 0);
    assert_eq!(v["meta"]["api_calls"], 5, "one read per list");
    let paths: Vec<String> = server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .map(|r| r.url.path().to_string())
        .collect();
    assert_eq!(paths.len(), 5, "{paths:?}");
    assert!(names_file(h, "lab").exists());
    // The table names the age.
    let out = cmd(&server, h)
        .args(["completions", "refresh", "--table"])
        .assert()
        .success();
    let table = String::from_utf8_lossy(&out.get_output().stdout).to_string();
    assert!(
        table.contains("age") && table.contains("hostnames"),
        "{table}"
    );

    // --clear: no PCE in the environment at all, and no request.
    let clear = |h: &Path| {
        let mut c = Command::cargo_bin("illumio-tui").unwrap();
        c.env_clear()
            .env("PATH", std::env::var("PATH").unwrap_or_default())
            .env("XDG_CONFIG_HOME", h.join("config"))
            .env("XDG_CACHE_HOME", h.join("cache"))
            .args(["--json", "completions", "refresh", "--clear"]);
        let out = c.assert().success();
        serde_json::from_slice::<Value>(&out.get_output().stdout).unwrap()
    };
    let requests = server.received_requests().await.unwrap().len();
    assert_eq!(clear(h)["data"]["cleared"], true);
    assert!(!names_file(h, "lab").exists());
    assert_eq!(clear(h)["data"]["cleared"], false);
    assert_eq!(server.received_requests().await.unwrap().len(), requests);
}

/// Host, port or org id from a flag or the environment: the profile's
/// cache would get another PCE's names, so nothing is written, and
/// `refresh` says why before any request.
#[tokio::test]
async fn a_pce_named_by_flags_or_environment_writes_no_cache() {
    let server = MockServer::start().await;
    mount_all(&server).await;
    let home = tempfile::tempdir().unwrap();
    let h = home.path();
    let url = url::Url::parse(&server.uri()).unwrap();
    let host = url.host_str().unwrap().to_string();
    let port = url.port().unwrap().to_string();
    cmd(&server, h)
        .env("PCE_HOST", &host)
        .arg("labels")
        .assert()
        .success();
    cmd(&server, h)
        .args(["--org-id", "5", "labels"])
        .assert()
        .success();
    cmd(&server, h)
        .args(["--pce-port", &port, "labels"])
        .assert()
        .success();
    let env_file = h.join("pce.env");
    std::fs::write(&env_file, "PCE_ORG_ID=5\n").unwrap();
    cmd(&server, h)
        .arg("--env-file")
        .arg(&env_file)
        .arg("labels")
        .assert()
        .success();
    assert!(!h.join("cache").exists(), "nothing on disk");
    let before = server.received_requests().await.unwrap().len();
    let out = cmd(&server, h)
        .env("PCE_HOST", &host)
        .args(["completions", "refresh"])
        .assert()
        .code(2);
    let err = String::from_utf8_lossy(&out.get_output().stderr).to_string();
    assert!(
        err.contains("the completion cache needs a profile from the config file"),
        "{err}"
    );
    assert_eq!(server.received_requests().await.unwrap().len(), before);
    // No config file at all: no profile, no cache.
    let bare = tempfile::tempdir().unwrap();
    let mut c = Command::cargo_bin("illumio-tui").unwrap();
    c.env_clear()
        .env("PATH", std::env::var("PATH").unwrap_or_default())
        .env("PCE_SCHEME", "http")
        .env("PCE_HOST", &host)
        .env("PCE_PORT", &port)
        .env("PCE_ORG_ID", "5")
        .env("API_KEY", "key")
        .env("API_SECRET", "secret")
        .env("XDG_CONFIG_HOME", bare.path().join("config"))
        .env("XDG_CACHE_HOME", bare.path().join("cache"))
        .args(["--json", "labels"])
        .assert()
        .success();
    assert!(!bare.path().join("cache").exists());
}

/// The profile's host changed since the file was written: the next write
/// starts it over rather than mixing two PCEs' names.
#[tokio::test]
async fn a_file_of_another_pce_is_replaced_not_merged() {
    let server = MockServer::start().await;
    mount_all(&server).await;
    let home = tempfile::tempdir().unwrap();
    let h = home.path();
    let file = names_file(h, "lab");
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    std::fs::write(
        &file,
        serde_json::to_vec(&json!({"version": 1, "host": "old.example", "org_id": 5,
            "labels": ["app=old"], "hostnames": ["old1"]}))
        .unwrap(),
    )
    .unwrap();
    cmd(&server, h).arg("services").assert().success();
    let n = names(h);
    assert_eq!(n["host"], "127.0.0.1");
    assert_eq!(n["services"], json!(["SSH"]));
    assert_eq!(n["labels"], json!([]), "the old PCE's names are gone");
    assert_eq!(n["hostnames"], json!([]));
}
