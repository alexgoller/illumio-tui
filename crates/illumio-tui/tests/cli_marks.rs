//! `illumio-tui marks`: the local marks file through the CLI, in a temp
//! `XDG_CONFIG_HOME`. No PCE: none of these commands sends a request.
#[path = "common/schema_check.rs"]
mod schema_check;
use assert_cmd::Command;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

fn marks(home: &Path) -> Command {
    let mut c = Command::cargo_bin("illumio-tui").unwrap();
    c.env_clear()
        .env("PATH", std::env::var("PATH").unwrap_or_default())
        .env("XDG_CONFIG_HOME", home)
        .arg("marks");
    c
}

fn file(home: &Path) -> PathBuf {
    home.join("illumio-tui").join("marks.toml")
}

fn json_out(a: assert_cmd::assert::Assert) -> Value {
    schema_check::envelope(&a.get_output().stdout)
}

fn stderr(a: &assert_cmd::assert::Assert) -> String {
    String::from_utf8_lossy(&a.get_output().stderr).to_string()
}

fn add(home: &Path, args: &[&str]) -> String {
    let out = json_out(marks(home).arg("add").args(args).assert().success());
    out["data"]["id"].as_str().unwrap().to_string()
}

#[test]
fn add_list_rm_round_trip_without_a_profile() {
    let home = tempfile::tempdir().unwrap();
    let a = add(
        home.path(),
        &[
            "--action",
            "squelch",
            "--src",
            "app=monitoring",
            "--dst",
            "any",
            "--service",
            "5666/tcp",
            "--comment",
            "Nagios NRPE, known",
        ],
    );
    assert!(a.starts_with("m-") && a.len() == 8, "{a}");
    let b = add(
        home.path(),
        &[
            "--action",
            "critical",
            "--src",
            "any",
            "--dst",
            "wl:db-01",
            "--pce",
            "pce.example.com",
            "--org",
            "1",
            "--comment",
            "crown jewels",
        ],
    );
    let out = json_out(marks(home.path()).arg("list").assert().success());
    let rows = out["data"].as_array().unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0]["id"], a.as_str());
    assert_eq!(rows[0]["src"], "app=monitoring");
    assert_eq!(rows[0]["service"], json!(["5666/tcp"]));
    assert_eq!(rows[0]["active"], true);
    assert!(
        rows[0]["expires"].as_str().is_some(),
        "squelch defaults to 90d"
    );
    assert_eq!(rows[1]["dst"], "wl:db-01");
    assert_eq!(rows[1]["pce"], "pce.example.com");
    assert_eq!(rows[1]["org"], 1);
    assert_eq!(
        rows[1]["expires"],
        Value::Null,
        "critical defaults to never"
    );
    assert_eq!(out["meta"]["api_calls"], 0);
    assert_eq!(
        out["meta"]["marks"]["active"], 2,
        "no profile: scope not judged"
    );
    let only = json_out(
        marks(home.path())
            .args(["list", "--action", "critical"])
            .assert()
            .success(),
    );
    assert_eq!(only["data"].as_array().unwrap().len(), 1);
    let text = std::fs::read_to_string(file(home.path())).unwrap();
    assert!(text.starts_with("# illumio-tui marks."), "{text}");
    assert!(
        text.contains("src = { labels = { app = \"monitoring\" } }"),
        "{text}"
    );
    marks(home.path())
        .args(["rm", &b, "m-nope00"])
        .assert()
        .code(2);
    let out = json_out(marks(home.path()).args(["rm", &b]).assert().success());
    assert_eq!(out["data"]["removed"], json!([b]));
    let out = json_out(marks(home.path()).arg("list").assert().success());
    assert_eq!(out["data"].as_array().unwrap().len(), 1);
}

#[test]
fn add_refuses_without_a_comment_and_names_the_bad_field() {
    let home = tempfile::tempdir().unwrap();
    marks(home.path())
        .args(["add", "--action", "squelch", "--src", "any", "--dst", "any"])
        .assert()
        .code(2);
    let a = marks(home.path())
        .args([
            "add",
            "--action",
            "squelch",
            "--src",
            "app=",
            "--dst",
            "any",
            "--comment",
            "x",
        ])
        .assert()
        .code(2);
    assert!(
        stderr(&a).contains("--src: app: empty label value"),
        "{}",
        stderr(&a)
    );
    let a = marks(home.path())
        .args([
            "add",
            "--action",
            "critical",
            "--src",
            "any",
            "--dst",
            "wl:db-01",
            "--comment",
            "x",
        ])
        .assert()
        .code(2);
    assert!(
        stderr(&a).contains("--pce: workload needs pce"),
        "{}",
        stderr(&a)
    );
    let a = marks(home.path())
        .args([
            "add",
            "--action",
            "accept-blocked",
            "--src",
            "any",
            "--dst",
            "any",
            "--comment",
            "x",
        ])
        .assert()
        .code(2);
    assert!(
        stderr(&a).contains("--decision: accept-blocked needs decision"),
        "{}",
        stderr(&a)
    );
    let a = marks(home.path())
        .args([
            "add",
            "--action",
            "squelch",
            "--src",
            "any",
            "--dst",
            "any",
            "--org",
            "1",
            "--comment",
            "x",
        ])
        .assert()
        .code(2);
    assert!(
        stderr(&a).contains("--pce"),
        "--org needs --pce: {}",
        stderr(&a)
    );
    let a = marks(home.path())
        .args([
            "add",
            "--action",
            "squelch",
            "--src",
            "any",
            "--dst",
            "any",
            "--pce",
            "--comment",
            "x",
        ])
        .assert()
        .code(2);
    assert!(stderr(&a).contains("no profile"), "{}", stderr(&a));
    assert!(!file(home.path()).exists(), "nothing was written");
}

#[test]
fn pce_without_a_host_takes_the_profiles_host() {
    let home = tempfile::tempdir().unwrap();
    let id = json_out(
        marks(home.path())
            .env("PCE_HOST", "pce.lab.example")
            .env("PCE_ORG_ID", "3")
            .args([
                "add",
                "--action",
                "revisit",
                "--src",
                "ip:10.0.0.0/8",
                "--dst",
                "any",
                "--pce",
                "--comment",
                "check",
            ])
            .assert()
            .success(),
    )["data"]["id"]
        .as_str()
        .unwrap()
        .to_string();
    let out = json_out(
        marks(home.path())
            .env("PCE_HOST", "other.example")
            .env("PCE_ORG_ID", "3")
            .arg("list")
            .assert()
            .success(),
    );
    assert_eq!(out["data"][0]["id"], id.as_str());
    assert_eq!(out["data"][0]["pce"], "pce.lab.example");
    assert_eq!(out["data"][0]["created_on"], "pce.lab.example");
    assert_eq!(out["data"][0]["active"], false, "scoped to another PCE");
}

#[test]
fn export_then_import_round_trips_and_import_is_refused_atomically() {
    let home = tempfile::tempdir().unwrap();
    add(
        home.path(),
        &[
            "--action",
            "accept-blocked",
            "--src",
            "ipl:rfc1918",
            "--dst",
            "app=shop env=prod|dr",
            "--decision",
            "blocked",
            "--comment",
            "CHG-1182",
        ],
    );
    let out = marks(home.path()).arg("export").assert().success();
    let exported: Value = serde_json::from_slice(&out.get_output().stdout).unwrap();
    let entry = &exported["marks"][0];
    assert_eq!(entry["action"], "accept-blocked");
    assert_eq!(entry["src"], json!({"ip_list": "rfc1918"}));
    assert_eq!(
        entry["dst"],
        json!({"labels": {"app": "shop", "env": ["prod", "dr"]}})
    );
    let toml_out = marks(home.path())
        .args(["export", "--format", "toml"])
        .assert()
        .success();
    let toml_text = String::from_utf8_lossy(&toml_out.get_output().stdout).to_string();
    assert_eq!(
        toml_text,
        std::fs::read_to_string(file(home.path())).unwrap()
    );

    let other = tempfile::tempdir().unwrap();
    let path = other.path().join("marks.json");
    std::fs::write(&path, serde_json::to_string(&exported).unwrap()).unwrap();
    let res = json_out(
        marks(other.path())
            .arg("import")
            .arg(&path)
            .assert()
            .success(),
    );
    assert_eq!(
        res["data"],
        json!({"added": 1, "replaced": 0, "skipped": []})
    );
    let again = json_out(
        marks(other.path())
            .arg("import")
            .arg(&path)
            .assert()
            .success(),
    );
    assert_eq!(again["data"]["skipped"], json!([entry["id"]]));
    let replaced = json_out(
        marks(other.path())
            .arg("import")
            .arg(&path)
            .arg("--replace")
            .assert()
            .success(),
    );
    assert_eq!(replaced["data"]["replaced"], 1);

    // One bad mark refuses the whole import; nothing is written.
    let before = std::fs::read_to_string(file(other.path())).unwrap();
    let bad = other.path().join("bad.json");
    std::fs::write(
        &bad,
        json!({"marks": [
            {"action": "reviewed", "src": "any", "dst": "any", "comment": "fine"},
            {"action": "reviewed", "src": {"workload": "x"}, "dst": "any", "comment": "no pce"}
        ]})
        .to_string(),
    )
    .unwrap();
    let a = marks(other.path()).arg("import").arg(&bad).assert().code(2);
    assert!(
        stderr(&a).contains("mark 2: workload needs pce"),
        "{}",
        stderr(&a)
    );
    assert_eq!(std::fs::read_to_string(file(other.path())).unwrap(), before);
}

#[test]
fn an_invalid_file_refuses_every_write_and_a_held_lock_exits_3() {
    let home = tempfile::tempdir().unwrap();
    let path = file(home.path());
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let bad = "[[mark]]\naction = \"squelch\"\nsrc = \"any\"\ndst = { workload = \"db\" }\ncomment = \"x\"\n";
    std::fs::write(&path, bad).unwrap();
    let args = [
        "add",
        "--action",
        "squelch",
        "--src",
        "any",
        "--dst",
        "any",
        "--comment",
        "x",
    ];
    let a = marks(home.path()).args(args).assert().code(2);
    assert!(
        stderr(&a).contains("mark 1: workload needs pce"),
        "{}",
        stderr(&a)
    );
    marks(home.path()).arg("list").assert().code(2);
    marks(home.path())
        .args(["rm", "--expired"])
        .assert()
        .code(2);
    assert_eq!(std::fs::read_to_string(&path).unwrap(), bad);
    std::fs::remove_file(&path).unwrap();
    std::fs::write(path.with_file_name("marks.toml.lock"), "1").unwrap();
    let a = marks(home.path()).args(args).assert().code(3);
    assert!(
        stderr(&a).contains("locked by another illumio-tui"),
        "{}",
        stderr(&a)
    );
}

#[test]
fn import_checks_ids_and_comments_and_reads_upper_case_extensions() {
    let home = tempfile::tempdir().unwrap();
    let dir = tempfile::tempdir().unwrap();
    // Two marks with one id: refused, nothing written.
    let dup = dir.path().join("dup.JSON");
    std::fs::write(
        &dup,
        json!({"marks": [
            {"id": "m-aaaaaa", "action": "reviewed", "src": "any", "dst": "any", "comment": "a"},
            {"id": "m-aaaaaa", "action": "revisit", "src": "any", "dst": "any", "comment": "b"}
        ]})
        .to_string(),
    )
    .unwrap();
    let a = marks(home.path()).arg("import").arg(&dup).assert().code(2);
    assert!(
        stderr(&a).contains("mark 2 (m-aaaaaa): id used by mark 1 too"),
        "{}",
        stderr(&a)
    );
    assert!(!file(home.path()).exists());
    // A mark without a comment imports with a warning; list says so.
    let bare = dir.path().join("bare.Toml");
    std::fs::write(
        &bare,
        "[[mark]]\naction = \"reviewed\"\nsrc = \"ANY\"\ndst = \"any\"\n",
    )
    .unwrap();
    let a = marks(home.path())
        .arg("import")
        .arg(&bare)
        .assert()
        .success();
    assert!(stderr(&a).contains("mark 1: no comment"), "{}", stderr(&a));
    let t = marks(home.path())
        .args(["list", "--table"])
        .assert()
        .success();
    let text = String::from_utf8_lossy(&t.get_output().stdout).to_string();
    assert!(text.contains("(no comment)"), "{text}");
    // A TOML syntax error names its line and column.
    let broken = dir.path().join("broken.toml");
    std::fs::write(&broken, "[[mark]]\naction = \"reviewed\"\nsrc = = 1\n").unwrap();
    let a = marks(home.path())
        .arg("import")
        .arg(&broken)
        .assert()
        .code(2);
    assert!(stderr(&a).contains("at line 3, column"), "{}", stderr(&a));
}
