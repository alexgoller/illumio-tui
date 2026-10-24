//! `profile add`: a profile from flags, an env file or a stdin paste, written
//! atomically, mode 0600, never printing the secret. Every run gets its own
//! temporary config and cache directories and an explicit `--config`.
#[path = "common/schema_check.rs"]
mod schema_check;
use assert_cmd::Command;
use serde_json::Value;
use std::path::{Path, PathBuf};

/// Distinctive so a leak check cannot trip on prose.
const SECRET: &str = "s3cr3t-profile-add-7c1e";

struct Home {
    dir: tempfile::TempDir,
}

impl Home {
    fn new() -> Self {
        Home {
            dir: tempfile::tempdir().unwrap(),
        }
    }
    fn cfg(&self) -> PathBuf {
        self.dir.path().join("conf").join("config.toml")
    }
    fn text(&self) -> String {
        std::fs::read_to_string(self.cfg()).unwrap()
    }
    fn env_file(&self, body: &str) -> PathBuf {
        let p = self.dir.path().join("pce.env");
        std::fs::write(&p, body).unwrap();
        p
    }
    fn add(&self, args: &[&str]) -> Command {
        let mut c = Command::cargo_bin("illumio-tui").unwrap();
        c.env_clear()
            .env("PATH", std::env::var("PATH").unwrap_or_default())
            .env("HOME", self.dir.path())
            .env("XDG_CONFIG_HOME", self.dir.path().join("xdg-config"))
            .env("XDG_CACHE_HOME", self.dir.path().join("xdg-cache"))
            .args(["--config", self.cfg().to_str().unwrap(), "profile", "add"])
            .args(args);
        c
    }
}

fn json(assert: &assert_cmd::assert::Assert) -> Value {
    schema_check::envelope(&assert.get_output().stdout)
}

fn assert_no_secret(assert: &assert_cmd::assert::Assert) {
    let out = assert.get_output();
    for (name, bytes) in [("stdout", &out.stdout), ("stderr", &out.stderr)] {
        let text = String::from_utf8_lossy(bytes);
        assert!(!text.contains(SECRET), "{name} leaks the secret: {text}");
    }
}

fn full_env() -> String {
    format!(
        "PCE_HOST=pce.example\nPCE_PORT=8443\nPCE_ORG_ID=3\nAPI_KEY=api_1\nAPI_SECRET={SECRET}\nOTHER=ignored\n"
    )
}

fn arg(p: &Path) -> &str {
    p.to_str().unwrap()
}

#[test]
fn imports_an_env_file_and_hides_the_secret() {
    let h = Home::new();
    let env = h.env_file(&full_env());
    let a = h
        .add(&["demo", "--from-env-file", arg(&env)])
        .assert()
        .success();
    assert_no_secret(&a);
    let out = json(&a);
    let d = &out["data"];
    assert_eq!(d["name"], "demo");
    assert_eq!(d["host"], "pce.example");
    assert_eq!(d["port"], 8443);
    assert_eq!(d["org_id"], 3);
    assert_eq!(d["api_key"], "set");
    assert_eq!(d["api_secret"], "set");
    assert_eq!(d["is_default"], true, "the first profile is the default");
    assert_eq!(d["config_path"], arg(&h.cfg()));
    let text = h.text();
    assert!(text.contains("default_profile = \"demo\""), "{text}");
    assert!(text.contains("host = \"pce.example\""), "{text}");
    assert!(text.contains("port = 8443"), "{text}");
    assert!(text.contains("api_key = \"api_1\""), "{text}");
    assert!(
        text.contains(&format!("api_secret = \"{SECRET}\"")),
        "{text}"
    );
    assert!(!text.contains("OTHER"), "other keys are ignored: {text}");

    // The table shows the same, still without the secret.
    let env2 = h.env_file(&full_env());
    let a = h
        .add(&["demo", "--from-env-file", arg(&env2), "--force", "--table"])
        .assert()
        .success();
    assert_no_secret(&a);
    let out = String::from_utf8_lossy(&a.get_output().stdout).to_string();
    assert!(out.contains("pce.example") && out.contains("set"), "{out}");
}

#[cfg(unix)]
#[test]
fn file_mode_0600_and_dir_0700() {
    use std::os::unix::fs::PermissionsExt;
    let h = Home::new();
    h.add(&["demo", "--host", "h.example", "--org-id", "1"])
        .assert()
        .success();
    let mode = std::fs::metadata(h.cfg()).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o600);
    let dir = std::fs::metadata(h.cfg().parent().unwrap())
        .unwrap()
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(dir, 0o700);
    // An existing file of a looser mode is tightened on rewrite.
    std::fs::set_permissions(h.cfg(), std::fs::Permissions::from_mode(0o644)).unwrap();
    h.add(&["two", "--host", "h2.example", "--org-id", "2"])
        .assert()
        .success();
    let mode = std::fs::metadata(h.cfg()).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o600);
    // No temporary file left behind.
    let names: Vec<_> = std::fs::read_dir(h.cfg().parent().unwrap())
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert_eq!(names, vec![std::ffi::OsString::from("config.toml")]);
}

#[test]
fn stdin_paste_of_the_env_file() {
    let h = Home::new();
    let a = h
        .add(&["demo", "--from-env-file", "-"])
        .write_stdin(full_env())
        .assert()
        .success();
    assert_no_secret(&a);
    assert_eq!(json(&a)["data"]["api_secret"], "set");
    assert!(h.text().contains(SECRET));
}

#[test]
fn flags_override_imported_values() {
    let h = Home::new();
    let env = h.env_file(&full_env());
    let a = h
        .add(&[
            "demo",
            "--from-env-file",
            arg(&env),
            "--host",
            "flag.example",
            "--org-id",
            "9",
            "--port",
            "9443",
            "--api-key",
            "flag_key",
            "--scheme",
            "http",
            "--insecure",
            "--writes",
        ])
        .assert()
        .success();
    let d = &json(&a)["data"];
    assert_eq!(d["host"], "flag.example");
    assert_eq!(d["org_id"], 9);
    assert_eq!(d["port"], 9443);
    let text = h.text();
    assert!(text.contains("api_key = \"flag_key\""), "{text}");
    assert!(text.contains("scheme = \"http\""), "{text}");
    assert!(text.contains("tls_verify = false"), "{text}");
    assert!(text.contains("writes = true"), "{text}");
    assert!(!text.contains("pce.example"), "{text}");
    // --pce-port (the global spelling) works as well.
    h.add(&["p2", "--host", "h", "--org-id", "1", "--pce-port", "8444"])
        .assert()
        .success();
    assert!(h.text().contains("port = 8444"));
}

#[test]
fn secret_sources() {
    // --api-secret-stdin: one line, trimmed, over the env file's API_SECRET.
    let h = Home::new();
    let env = h.env_file(&full_env());
    let a = h
        .add(&["demo", "--from-env-file", arg(&env), "--api-secret-stdin"])
        .write_stdin("  from-stdin-secret \nsecond line\n")
        .assert()
        .success();
    assert_no_secret(&a);
    let text = h.text();
    assert!(
        text.contains("api_secret = \"from-stdin-secret\""),
        "{text}"
    );
    assert!(
        !text.contains(SECRET) && !text.contains("second line"),
        "{text}"
    );

    // --api-secret: plaintext from a flag, over the env file.
    let h = Home::new();
    let env = h.env_file("PCE_HOST=h\nPCE_ORG_ID=1\nAPI_SECRET=from-file\n");
    let a = h
        .add(&["demo", "--from-env-file", arg(&env), "--api-secret", SECRET])
        .assert()
        .success();
    assert_no_secret(&a);
    assert_eq!(json(&a)["data"]["api_secret"], "set");
    assert!(h.text().contains(&format!("api_secret = \"{SECRET}\"")));
    assert!(!h.text().contains("from-file"));

    // --api-secret-cmd beats the env file's API_SECRET: no plaintext written.
    let h = Home::new();
    let env = h.env_file(&full_env());
    let a = h
        .add(&[
            "demo",
            "--from-env-file",
            arg(&env),
            "--api-secret-cmd",
            "op read op://vault/pce/secret",
        ])
        .assert()
        .success();
    assert_eq!(json(&a)["data"]["api_secret"], "cmd");
    let text = h.text();
    assert!(!text.contains(SECRET), "{text}");
    assert!(!text.contains("api_secret ="), "{text}");
    assert!(
        text.contains("api_secret_cmd = \"op read op://vault/pce/secret\""),
        "{text}"
    );

    // No secret at all.
    let h = Home::new();
    let a = h
        .add(&["demo", "--host", "h", "--org-id", "1"])
        .assert()
        .success();
    assert_eq!(json(&a)["data"]["api_secret"], "none");
    assert_eq!(json(&a)["data"]["api_key"], "none");
}

#[test]
fn conflicting_secret_sources_are_refused() {
    let h = Home::new();
    for args in [
        &[
            "demo",
            "--host",
            "h",
            "--org-id",
            "1",
            "--api-secret",
            SECRET,
            "--api-secret-stdin",
        ][..],
        &[
            "demo",
            "--host",
            "h",
            "--org-id",
            "1",
            "--api-secret",
            SECRET,
            "--api-secret-cmd",
            "x",
        ],
        &[
            "demo",
            "--host",
            "h",
            "--org-id",
            "1",
            "--api-secret-stdin",
            "--api-secret-cmd",
            "x",
        ],
        &["demo", "--from-env-file", "-", "--api-secret-stdin"],
    ] {
        let a = h.add(args).write_stdin(full_env()).assert().code(2);
        assert_no_secret(&a);
        assert!(!h.cfg().exists(), "{args:?} wrote the file");
    }
}

#[test]
fn existing_profile_needs_force_and_others_survive() {
    let h = Home::new();
    std::fs::create_dir_all(h.cfg().parent().unwrap()).unwrap();
    std::fs::write(
        h.cfg(),
        "default_profile = \"a\"\ntop_unknown = 1\n\n[profiles.a]\nhost = \"a.example\"\norg_id = 1\nunknown_key = \"kept\"\n\n[profiles.demo]\nhost = \"old.example\"\norg_id = 2\n",
    )
    .unwrap();
    let before = h.text();
    let a = h
        .add(&["demo", "--host", "new.example", "--org-id", "3"])
        .assert()
        .code(2);
    let err = String::from_utf8_lossy(&a.get_output().stdout).to_string()
        + &String::from_utf8_lossy(&a.get_output().stderr);
    assert!(
        err.contains("already exists") && err.contains("--force"),
        "{err}"
    );
    assert_eq!(h.text(), before, "nothing changed");

    let a = h
        .add(&["demo", "--host", "new.example", "--org-id", "3", "--force"])
        .assert()
        .success();
    let d = &json(&a)["data"];
    assert_eq!(d["replaced"], true);
    assert_eq!(d["is_default"], false, "not the first profile");
    let text = h.text();
    assert!(
        text.contains("new.example") && !text.contains("old.example"),
        "{text}"
    );
    assert!(text.contains("default_profile = \"a\""), "{text}");
    assert!(text.contains("host = \"a.example\""), "{text}");
    assert!(text.contains("unknown_key = \"kept\""), "{text}");
    assert!(text.contains("top_unknown = 1"), "{text}");

    // --default makes a later profile the default.
    h.add(&["c", "--host", "c.example", "--org-id", "4", "--default"])
        .assert()
        .success();
    assert!(h.text().contains("default_profile = \"c\""));
}

#[test]
fn validation_errors() {
    let h = Home::new();
    let cases: &[(&[&str], &str)] = &[
        (&["demo", "--org-id", "1"], "no PCE host"),
        (&["demo", "--host", "h"], "no org id"),
        (
            &["demo", "--host", "https://h.example", "--org-id", "1"],
            "hostname only",
        ),
        (
            &["demo", "--host", "h", "--org-id", "1", "--port", "0"],
            "0",
        ),
        (
            &["demo", "--host", "h", "--org-id", "1", "--port", "70000"],
            "70000",
        ),
        (
            &[
                "demo",
                "--host",
                "h",
                "--org-id",
                "1",
                "--env-file",
                "x.env",
            ],
            "--from-env-file",
        ),
    ];
    for (args, want) in cases {
        let a = h.add(args).assert().code(2);
        let err = String::from_utf8_lossy(&a.get_output().stdout).to_string()
            + &String::from_utf8_lossy(&a.get_output().stderr);
        assert!(err.contains(want), "{args:?}: {err}");
        assert!(!h.cfg().exists(), "{args:?} wrote the file");
    }
    // Bad values imported from the env file are named, never quoted.
    for (body, want) in [
        ("PCE_HOST=h\nPCE_ORG_ID=1\nPCE_PORT=99999zz\n", "PCE_PORT"),
        ("PCE_HOST=h\nPCE_ORG_ID=1\nPCE_PORT=0\n", "PCE_PORT"),
        ("PCE_HOST=h\nPCE_ORG_ID=one99zz\n", "PCE_ORG_ID"),
        ("PCE_ORG_ID=1\n", "no PCE host"),
    ] {
        let env = h.env_file(body);
        let a = h
            .add(&["demo", "--from-env-file", arg(&env)])
            .assert()
            .code(2);
        let err = String::from_utf8_lossy(&a.get_output().stdout).to_string()
            + &String::from_utf8_lossy(&a.get_output().stderr);
        assert!(err.contains(want), "{body}: {err}");
        assert!(!err.contains("99zz"), "{body}: {err}");
        assert!(!h.cfg().exists());
    }
}

#[test]
fn malformed_env_line_is_not_echoed() {
    let h = Home::new();
    let body = format!("PCE_HOST=h\nPCE_ORG_ID=1\nAPI_SECRET=\"{SECRET} unterminated\n");
    let env = h.env_file(&body);
    let a = h
        .add(&["demo", "--from-env-file", arg(&env)])
        .assert()
        .code(2);
    assert_no_secret(&a);
    let err = String::from_utf8_lossy(&a.get_output().stdout).to_string()
        + &String::from_utf8_lossy(&a.get_output().stderr);
    assert!(err.contains("bad line"), "{err}");
    assert!(!h.cfg().exists());
    // The same through stdin.
    let a = h
        .add(&["demo", "--from-env-file", "-"])
        .write_stdin(body)
        .assert()
        .code(2);
    assert_no_secret(&a);
    assert!(!h.cfg().exists());
}

#[test]
fn the_written_profile_is_usable_and_listed() {
    let h = Home::new();
    let env = h.env_file(&full_env());
    h.add(&["demo", "--from-env-file", arg(&env)])
        .assert()
        .success();
    let a = Command::cargo_bin("illumio-tui")
        .unwrap()
        .env_clear()
        .args(["--config", arg(&h.cfg()), "profile", "list"])
        .assert()
        .success();
    let rows = json(&a)["data"].as_array().unwrap().clone();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["name"], "demo");
    assert_eq!(rows[0]["is_default"], true);
}

#[test]
fn add_args_debug_redacts_the_secret() {
    use clap::Parser;
    let cli = illumio_tui::cli::Cli::try_parse_from([
        "illumio-tui",
        "profile",
        "add",
        "demo",
        "--api-secret",
        SECRET,
    ])
    .unwrap();
    let debug = format!("{cli:?}");
    assert!(debug.contains("<redacted>"), "{debug}");
    assert!(!debug.contains(SECRET), "{debug}");
}

fn output(a: &assert_cmd::assert::Assert) -> String {
    String::from_utf8_lossy(&a.get_output().stdout).to_string()
        + &String::from_utf8_lossy(&a.get_output().stderr)
}

const RICH: &str = r#"default_profile = "prod"

[profiles.prod]
host = "old.example"
port = 8443
org_id = 2
scheme = "https"
api_key = "old_key"
api_secret_cmd = "op read op://old"
writes = true
completion_cache = false
mystery = "kept"

[profiles.prod.limits]
traffic_max_results = 5000
requests_per_second = 1.5
future_limit = 7

[profiles.prod.labels]
scope_keys = ["app", "env", "loc"]
tier_key = "role"
label_future = "x"

[profiles.prod.generator]
ruleset_name = "RS-{app}"
gen_future = true

[profiles.prod.custom_sub]
anything = 1
"#;

#[test]
fn force_keeps_what_it_is_not_given() {
    let h = Home::new();
    std::fs::create_dir_all(h.cfg().parent().unwrap()).unwrap();
    std::fs::write(h.cfg(), RICH).unwrap();
    let a = h
        .add(&[
            "prod",
            "--host",
            "new.example",
            "--org-id",
            "3",
            "--api-secret-stdin",
            "--force",
        ])
        .write_stdin(format!("{SECRET}\n"))
        .assert()
        .success();
    assert_no_secret(&a);
    let d = &json(&a)["data"];
    assert_eq!(d["replaced"], true);
    assert_eq!(d["port"], 8443, "the old port is kept");
    assert_eq!(d["api_key"], "set");
    assert_eq!(d["api_secret"], "set");
    assert_eq!(d["is_default"], true);
    let text = h.text();
    let v: toml::Table = toml::from_str(&text).unwrap();
    let p = &v["profiles"]["prod"];
    assert_eq!(p["host"].as_str(), Some("new.example"));
    assert_eq!(p["org_id"].as_integer(), Some(3));
    assert_eq!(p["port"].as_integer(), Some(8443));
    assert_eq!(p["api_key"].as_str(), Some("old_key"));
    assert_eq!(p["api_secret"].as_str(), Some(SECRET));
    assert!(
        p.get("api_secret_cmd").is_none(),
        "a new plaintext secret clears the cmd: {text}"
    );
    assert_eq!(p["writes"].as_bool(), Some(true));
    assert_eq!(p["completion_cache"].as_bool(), Some(false));
    assert_eq!(p["mystery"].as_str(), Some("kept"));
    assert_eq!(p["limits"]["traffic_max_results"].as_integer(), Some(5000));
    assert_eq!(p["limits"]["requests_per_second"].as_float(), Some(1.5));
    assert_eq!(p["limits"]["future_limit"].as_integer(), Some(7));
    assert_eq!(p["labels"]["scope_keys"].as_array().unwrap().len(), 3);
    assert_eq!(p["labels"]["label_future"].as_str(), Some("x"));
    assert_eq!(p["generator"]["ruleset_name"].as_str(), Some("RS-{app}"));
    assert_eq!(p["generator"]["gen_future"].as_bool(), Some(true));
    assert_eq!(p["custom_sub"]["anything"].as_integer(), Some(1));

    // And back to a cmd: the plaintext secret goes.
    h.add(&[
        "prod",
        "--host",
        "new.example",
        "--org-id",
        "3",
        "--api-secret-cmd",
        "op read op://new",
        "--force",
    ])
    .assert()
    .success();
    let text = h.text();
    assert!(!text.contains(SECRET), "{text}");
    assert!(
        text.contains("api_secret_cmd = \"op read op://new\""),
        "{text}"
    );
}

#[test]
fn unknown_keys_in_subtables_survive_profile_use() {
    let h = Home::new();
    std::fs::create_dir_all(h.cfg().parent().unwrap()).unwrap();
    std::fs::write(
        h.cfg(),
        format!("{RICH}\n[profiles.other]\nhost = \"o.example\"\norg_id = 1\n"),
    )
    .unwrap();
    Command::cargo_bin("illumio-tui")
        .unwrap()
        .env_clear()
        .args(["--config", arg(&h.cfg()), "profile", "use", "other"])
        .assert()
        .success();
    let text = h.text();
    for kept in [
        "future_limit = 7",
        "label_future = \"x\"",
        "gen_future = true",
        "anything = 1",
        "mystery = \"kept\"",
        "default_profile = \"other\"",
    ] {
        assert!(text.contains(kept), "{kept}: {text}");
    }
}

#[test]
fn bom_and_line_numbers() {
    let h = Home::new();
    let body = format!("\u{feff}{}", full_env());
    let env = h.env_file(&body);
    h.add(&["file", "--from-env-file", arg(&env)])
        .assert()
        .success();
    h.add(&["paste", "--from-env-file", "-"])
        .write_stdin(body)
        .assert()
        .success();
    let text = h.text();
    assert!(text.contains("[profiles.file]") && text.contains("[profiles.paste]"));
    assert_eq!(text.matches("host = \"pce.example\"").count(), 2, "{text}");

    let env = h.env_file(&format!(
        "\u{feff}# comment\nPCE_HOST=h\n\nAPI_SECRET=\"{SECRET} unterminated\nmore\n"
    ));
    let a = h
        .add(&["bad", "--from-env-file", arg(&env)])
        .assert()
        .code(2);
    assert_no_secret(&a);
    let err = output(&a);
    assert!(err.contains("bad line 4 in"), "{err}");
    assert!(
        !err.contains("unterminated") && !err.contains("more"),
        "{err}"
    );
}

#[test]
fn dollar_in_an_imported_secret_warns_without_the_value() {
    let h = Home::new();
    let env = h.env_file("PCE_HOST=h\nPCE_ORG_ID=1\nAPI_KEY=k\nAPI_SECRET=ab$cd-s3cr3t\n");
    let a = h
        .add(&["d", "--from-env-file", arg(&env)])
        .assert()
        .success();
    let err = String::from_utf8_lossy(&a.get_output().stderr).to_string();
    assert!(
        err.contains("API_SECRET") && err.contains("single-quote"),
        "{err}"
    );
    assert!(!err.contains("s3cr3t") && !err.contains("API_KEY"), "{err}");

    // Single-quoted: literal, no warning.
    let env = h.env_file("PCE_HOST=h\nPCE_ORG_ID=1\nAPI_SECRET='ab$cd'\n");
    let a = h
        .add(&["e", "--from-env-file", arg(&env)])
        .assert()
        .success();
    assert!(!output(&a).contains("warning"), "{}", output(&a));
    assert!(h.text().contains("api_secret = \"ab$cd\""), "{}", h.text());
    // A flag secret over the file: no warning about the file's value.
    let env = h.env_file("PCE_HOST=h\nPCE_ORG_ID=1\nAPI_SECRET=x$y\n");
    let a = h
        .add(&[
            "f",
            "--from-env-file",
            arg(&env),
            "--api-secret-cmd",
            "true",
        ])
        .assert()
        .success();
    assert!(!output(&a).contains("warning"), "{}", output(&a));
}

#[test]
fn host_with_a_port_is_refused() {
    let h = Home::new();
    let a = h
        .add(&["d", "--host", "pce.example:8443", "--org-id", "1"])
        .assert()
        .code(2);
    assert!(output(&a).contains("--port"), "{}", output(&a));
    let env = h.env_file("PCE_HOST=pce.example:8443\nPCE_ORG_ID=1\n");
    h.add(&["d", "--from-env-file", arg(&env)]).assert().code(2);
    assert!(!h.cfg().exists());
}

#[cfg(unix)]
#[test]
fn a_write_failure_says_write() {
    use std::os::unix::fs::PermissionsExt;
    let h = Home::new();
    let dir = h.cfg().parent().unwrap().to_path_buf();
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o500)).unwrap();
    let a = h
        .add(&["d", "--host", "h", "--org-id", "1"])
        .assert()
        .failure();
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();
    let err = output(&a);
    assert!(err.contains("cannot write"), "{err}");
    assert!(!err.contains("cannot read"), "{err}");
    assert!(!h.cfg().exists());
}

#[test]
fn a_secret_may_start_with_a_hyphen() {
    let h = Home::new();
    let a = h
        .add(&[
            "d",
            "--host",
            "h",
            "--org-id",
            "1",
            "--api-secret",
            "-starts-with-hyphen",
        ])
        .assert()
        .success();
    assert_eq!(json(&a)["data"]["api_secret"], "set");
    assert!(h.text().contains("api_secret = \"-starts-with-hyphen\""));
}

#[test]
fn help_says_the_environment_is_ignored() {
    let a = Command::cargo_bin("illumio-tui")
        .unwrap()
        .env_clear()
        .args(["profile", "add", "--help"])
        .assert()
        .success();
    let text = output(&a);
    assert!(text.contains("real environment"), "{text}");
    assert!(text.contains("shell history"), "{text}");
}
