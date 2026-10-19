//! `completions <shell>` and the hidden `__complete` helper, run as the
//! binary: scripts for every shell, profile names from the config file
//! only (no request, no secret command), nothing without a config, and
//! the bash script completing through the helper in a real bash.
use assert_cmd::Command;
use std::net::TcpListener;
use std::path::Path;
use std::time::{Duration, Instant};

/// The binary with an empty environment: no PCE variables at all.
fn bin(xdg: &Path) -> Command {
    let mut c = Command::cargo_bin("illumio-tui").unwrap();
    c.env_clear()
        .env("PATH", std::env::var("PATH").unwrap_or_default())
        .env("HOME", xdg)
        .env("XDG_CONFIG_HOME", xdg);
    c
}

fn stdout(c: &mut Command) -> String {
    let out = c.assert().success().get_output().clone();
    String::from_utf8(out.stdout).unwrap()
}

/// A config with two profiles whose PCE is a local listener and whose
/// secret command would leave a marker file: either one being touched
/// would mean the helper did more than read the file.
fn config_with_trap(dir: &Path) -> (TcpListener, std::path::PathBuf) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let port = listener.local_addr().unwrap().port();
    let marker = dir.join("secret-cmd-ran");
    std::fs::create_dir_all(dir.join("illumio-tui")).unwrap();
    std::fs::write(
        dir.join("illumio-tui").join("config.toml"),
        format!(
            "default_profile = \"lab\"\n\
             [profiles.lab]\nhost = \"127.0.0.1\"\nport = {port}\norg_id = 1\nscheme = \"http\"\n\
             api_key = \"k\"\napi_secret_cmd = \"touch {}\"\n\
             [profiles.prod-eu]\nhost = \"127.0.0.1\"\nport = {port}\norg_id = 1\n",
            marker.display()
        ),
    )
    .unwrap();
    (listener, marker)
}

#[test]
fn completions_prints_a_script_per_shell() {
    let dir = tempfile::tempdir().unwrap();
    let top = [
        "health",
        "labels",
        "label-groups",
        "iplists",
        "services",
        "events",
        "workloads",
        "app",
        "gen",
        "policy",
        "traffic",
        "profile",
        "marks",
        "completions",
    ];
    for shell in ["bash", "zsh", "fish"] {
        let s = stdout(bin(dir.path()).args(["completions", shell]));
        for name in top
            .iter()
            .chain(&["rulesets", "deny-rules", "provision", "import"])
        {
            assert!(s.contains(name), "{shell} script misses {name}");
        }
        assert!(s.contains("illumio-tui __complete profiles"), "{shell}");
    }
    bin(dir.path())
        .args(["completions", "powershell"])
        .assert()
        .failure();
}

#[test]
fn helper_lists_profiles_without_any_request() {
    let dir = tempfile::tempdir().unwrap();
    let (listener, marker) = config_with_trap(dir.path());
    let start = Instant::now();
    let s = stdout(bin(dir.path()).args(["__complete", "profiles"]));
    let took = start.elapsed();
    assert_eq!(s, "lab\nprod-eu\n");
    assert_eq!(
        stdout(bin(dir.path()).args(["__complete", "profiles", "pr"])),
        "prod-eu\n"
    );
    // Generous for a loaded CI runner; locally a few milliseconds.
    assert!(took < Duration::from_secs(2), "took {took:?}");
    assert!(
        listener.accept().is_err(),
        "the helper connected to the PCE"
    );
    assert!(!marker.exists(), "the helper ran the secret command");
}

#[test]
fn helper_prints_nothing_without_a_config_or_cache() {
    let dir = tempfile::tempdir().unwrap();
    for kind in [
        "profiles", "labels", "apps", "rulesets", "iplists", "services",
    ] {
        assert_eq!(stdout(bin(dir.path()).args(["__complete", kind])), "");
    }
    assert_eq!(stdout(bin(dir.path()).arg("__complete")), "");
}

/// The bash script, sourced in a clean bash, completes subcommands,
/// enum values and profile names (through the helper on PATH).
#[test]
fn bash_script_completes_through_the_helper() {
    if !Path::new("/bin/bash").exists() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let (_listener, _marker) = config_with_trap(dir.path());
    let script = dir.path().join("illumio-tui.bash");
    std::fs::write(
        &script,
        stdout(bin(dir.path()).args(["completions", "bash"])),
    )
    .unwrap();
    let driver = r#"source "$1"
t() { COMP_WORDS=("$@"); COMP_CWORD=$((${#COMP_WORDS[@]}-1)); COMPREPLY=()
  _illumio-tui__dynamic illumio-tui "${COMP_WORDS[COMP_CWORD]}" "${COMP_WORDS[COMP_CWORD-1]}"
  echo "${COMPREPLY[*]}"; }
t illumio-tui --profile ""
t illumio-tui traffic --profile = p
t illumio-tui profile use ""
t illumio-tui pol
t illumio-tui policy de
t illumio-tui events --severity w
"#;
    let bin_dir = assert_cmd::cargo::cargo_bin("illumio-tui")
        .parent()
        .unwrap()
        .to_path_buf();
    let out = std::process::Command::new("/bin/bash")
        .args(["--norc", "--noprofile", "-c", driver, "driver"])
        .arg(&script)
        .env_clear()
        .env("PATH", format!("{}:/usr/bin:/bin", bin_dir.display()))
        .env("HOME", dir.path())
        .env("XDG_CONFIG_HOME", dir.path())
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let lines: Vec<String> = String::from_utf8(out.stdout)
        .unwrap()
        .lines()
        .map(String::from)
        .collect();
    assert_eq!(
        lines,
        [
            "lab prod-eu",
            "prod-eu",
            "lab prod-eu",
            "policy",
            "deny-rules",
            "warning"
        ]
    );
}
