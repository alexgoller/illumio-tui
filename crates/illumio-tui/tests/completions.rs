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
    let s = stdout(bin(dir.path()).args(["completions", "powershell"]));
    assert!(s.contains("Register-ArgumentCompleter -Native -CommandName 'illumio-tui'"));
    for name in top
        .iter()
        .chain(&["rulesets", "deny-rules", "provision", "import"])
    {
        assert!(s.contains(name), "powershell script misses {name}");
    }
    bin(dir.path())
        .args(["completions", "elvish"])
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

/// A names cache for `profile` under `dir` (as `XDG_CACHE_HOME`).
fn write_names(dir: &Path, profile: &str, body: &str) -> std::path::PathBuf {
    let path = dir
        .join("cache")
        .join("illumio-tui")
        .join(profile)
        .join("names.json");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, body).unwrap();
    path
}

const NAMES: &str = r#"{
  "version": 1,
  "host": "127.0.0.1",
  "org_id": 1,
  "written_at": "2026-10-21T10:00:00Z",
  "labels": ["app=crm", "app=shop", "env=prod", "loc=new york"],
  "label_keys": ["app", "env", "loc"],
  "apps": ["crm/prod", "shop/prod"],
  "hostnames": ["db 1", "web1"],
  "services": ["HTTPS", "SSH"],
  "ip_lists": ["Any (0.0.0.0/0 and ::/0)", "Corp"],
  "label_groups": ["Prod envs"]
}"#;

fn helper(dir: &Path, args: &[&str]) -> String {
    stdout(
        bin(dir)
            .env("XDG_CACHE_HOME", dir.join("cache"))
            .arg("__complete")
            .args(args),
    )
}

#[test]
fn helper_offers_cached_names_per_kind_without_any_request() {
    let dir = tempfile::tempdir().unwrap();
    let (listener, marker) = config_with_trap(dir.path());
    write_names(dir.path(), "lab", NAMES);
    let h = |args: &[&str]| helper(dir.path(), args);
    let start = Instant::now();
    assert_eq!(h(&["labels", "--", "app="]), "app=crm\napp=shop\n");
    let took = start.elapsed();
    assert_eq!(h(&["labels", "app=shop,e"]), "app=shop,env=prod\n");
    assert_eq!(h(&["label-filter", "--", "!app=s"]), "!app=shop\n");
    assert_eq!(h(&["label-keys", "e"]), "env\n");
    assert_eq!(h(&["apps", "s"]), "shop/prod\n");
    assert_eq!(h(&["hostnames", "db"]), "db 1\n");
    assert_eq!(h(&["services"]), "HTTPS\nSSH\n");
    assert_eq!(h(&["iplists", "C"]), "Corp\n");
    assert_eq!(h(&["label-groups"]), "Prod envs\n");
    assert_eq!(h(&["service-terms", "svc:S"]), "svc:SSH\n");
    assert_eq!(h(&["traffic", "wl:"]), "wl:db 1\nwl:web1\n");
    assert_eq!(h(&["traffic", "--", "--src=ipl:C"]), "ipl:Corp\n");
    assert_eq!(h(&["scope", "group:"]), "group:Prod envs\n");
    assert_eq!(
        h(&["actors", "iplist:A"]),
        "iplist:Any (0.0.0.0/0 and ::/0)\n"
    );
    assert_eq!(h(&["search", "svc:H"]), "svc:HTTPS\n");
    assert_eq!(h(&["rulesets"]), "", "an unknown kind");
    // Another profile's file, named on the command line or in the env.
    write_names(
        dir.path(),
        "prod-eu",
        r#"{"version": 1, "host": "127.0.0.1", "org_id": 1, "labels": ["app=eu"]}"#,
    );
    assert_eq!(h(&["labels", "--profile", "prod-eu"]), "app=eu\n");
    assert_eq!(
        stdout(
            bin(dir.path())
                .env("XDG_CACHE_HOME", dir.path().join("cache"))
                .env("ILLUMIO_PROFILE", "prod-eu")
                .args(["__complete", "labels"])
        ),
        "app=eu\n"
    );
    assert!(took < Duration::from_secs(2), "took {took:?}");
    assert!(
        listener.accept().is_err(),
        "the helper connected to the PCE"
    );
    assert!(!marker.exists(), "the helper ran the secret command");
}

#[test]
fn helper_offers_nothing_from_a_missing_corrupt_or_old_cache() {
    let dir = tempfile::tempdir().unwrap();
    let (_listener, _marker) = config_with_trap(dir.path());
    assert_eq!(helper(dir.path(), &["labels"]), "", "no file");
    for body in ["{not json", r#"{"version": 99, "labels": ["app=x"]}"#, ""] {
        write_names(dir.path(), "lab", body);
        assert_eq!(helper(dir.path(), &["labels"]), "", "{body}");
    }
    // No config file at all: no profile, so no cache of one.
    let bare = tempfile::tempdir().unwrap();
    write_names(bare.path(), "default", NAMES);
    assert_eq!(helper(bare.path(), &["label-groups"]), "");
    // The profile's PCE changed since the file was written.
    write_names(
        dir.path(),
        "lab",
        &NAMES.replace("\"org_id\": 1", "\"org_id\": 2"),
    );
    assert_eq!(helper(dir.path(), &["label-groups"]), "", "another org");
    write_names(dir.path(), "lab", &NAMES.replace("127.0.0.1", "pce.old"));
    assert_eq!(helper(dir.path(), &["label-groups"]), "", "another host");
    write_names(dir.path(), "lab", NAMES);
    assert_eq!(helper(dir.path(), &["label-groups"]), "Prod envs\n");
}

#[test]
fn completion_cache_false_keeps_the_helper_off_the_file() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("illumio-tui")).unwrap();
    let config = dir.path().join("illumio-tui").join("config.toml");
    write_names(dir.path(), "lab", NAMES);
    for (text, want) in [
        (
            "default_profile = \"lab\"\n[profiles.lab]\nhost = \"127.0.0.1\"\norg_id = 1\n",
            "Any (0.0.0.0/0 and ::/0)\nCorp\n",
        ),
        (
            "default_profile = \"lab\"\ncompletion_cache = false\n[profiles.lab]\nhost = \"127.0.0.1\"\norg_id = 1\n",
            "",
        ),
        (
            "default_profile = \"lab\"\n[profiles.lab]\nhost = \"127.0.0.1\"\norg_id = 1\ncompletion_cache = false\n",
            "",
        ),
    ] {
        std::fs::write(&config, text).unwrap();
        assert_eq!(helper(dir.path(), &["iplists"]), want, "{text}");
    }
}

/// The bash script, sourced in a clean bash, completes PCE names from the
/// cache: `key=value` and `wl:HOST` split by bash at `=` and `:`, names
/// with spaces escaped, `--flag=value`, positionals, the current profile.
#[test]
fn bash_script_completes_cached_names() {
    if !Path::new("/bin/bash").exists() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let (_listener, _marker) = config_with_trap(dir.path());
    write_names(dir.path(), "lab", NAMES);
    write_names(
        dir.path(),
        "prod-eu",
        r#"{"version": 1, "host": "127.0.0.1", "org_id": 1, "labels": ["app=eu"]}"#,
    );
    let script = dir.path().join("illumio-tui.bash");
    std::fs::write(
        &script,
        stdout(bin(dir.path()).args(["completions", "bash"])),
    )
    .unwrap();
    // `t LINE`: complete at the end of LINE, as bash would split it.
    let driver = r#"source "$1"
t() { COMP_LINE="$1"; COMP_POINT=${#COMP_LINE}
  local IFS=$' \t\n'
  local -a w; read -ra w <<< "$COMP_LINE"
  [[ "$COMP_LINE" == *" " ]] && w+=("")
  COMP_WORDS=("${w[@]}"); COMP_CWORD=$((${#COMP_WORDS[@]}-1)); COMPREPLY=()
  _illumio-tui__dynamic illumio-tui "${COMP_WORDS[COMP_CWORD]}" "${COMP_WORDS[COMP_CWORD-1]}"
  local IFS='|'; echo "${COMPREPLY[*]}"; }
t "illumio-tui workloads --label app=s"
t "illumio-tui workloads --label=app="
t "illumio-tui workloads relabel --set app=shop,e"
t "illumio-tui traffic query --src wl:d"
t "illumio-tui traffic query --exclude-service svc:"
t "illumio-tui enforce s"
t "illumio-tui workloads update "
t "illumio-tui policy rules add --provider iplist:A"
t "illumio-tui --profile prod-eu workloads --label "
t "illumio-tui workloads --hostname db\ "
t "illumio-tui enforce --"
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
        .env("XDG_CACHE_HOME", dir.path().join("cache"))
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
            "shop",
            "crm|shop",
            "shop,env=prod",
            "db\\ 1",
            "HTTPS|SSH",
            "shop/prod",
            "db\\ 1|web1",
            "Any\\ \\(0.0.0.0/0\\ and\\ ::/0\\)",
            "app=eu",
            "db\\ 1",
            // A flag being typed: the generated function's options.
            lines[10].as_str(),
        ]
    );
    assert!(lines[10].contains("--to"), "{}", lines[10]);
}

/// Runs `driver` (a zsh script given the arguments `dir`) in zsh with
/// the binary on PATH and `dir` as config and cache; killed after 30 s.
fn run_zsh(driver: &str, dir: &Path) -> std::process::Output {
    let bin_dir = assert_cmd::cargo::cargo_bin("illumio-tui")
        .parent()
        .unwrap()
        .to_path_buf();
    let mut child = std::process::Command::new("/bin/zsh")
        .args(["-f", "-c", driver, "driver"])
        .arg(dir)
        .env_clear()
        .env("PATH", format!("{}:/usr/bin:/bin", bin_dir.display()))
        .env("HOME", dir)
        .env("TERM", "xterm")
        .env("XDG_CONFIG_HOME", dir)
        .env("XDG_CACHE_HOME", dir.join("cache"))
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let start = Instant::now();
    while child.try_wait().unwrap().is_none() {
        if start.elapsed() > Duration::from_secs(30) {
            let _ = child.kill();
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    child.wait_with_output().unwrap()
}

/// The zsh script, loaded by compinit in an interactive zsh on a pty
/// (zsh/zpty), completes PCE names through `_arguments`, as a user's Tab
/// does: each line is typed with a Tab, then a widget records the buffer.
#[test]
fn zsh_script_completes_cached_names_through_arguments() {
    if !Path::new("/bin/zsh").exists() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let (_listener, _marker) = config_with_trap(dir.path());
    write_names(dir.path(), "lab", NAMES);
    write_names(
        dir.path(),
        "prod-eu",
        r#"{"version": 1, "host": "127.0.0.1", "org_id": 1, "labels": ["app=eu"]}"#,
    );
    let fpath = dir.path().join("fn");
    std::fs::create_dir_all(&fpath).unwrap();
    std::fs::write(
        fpath.join("_illumio-tui"),
        stdout(bin(dir.path()).args(["completions", "zsh"])),
    )
    .unwrap();
    let driver = r#"zmodload zsh/zpty || exit 3
out=$1/buffers
zpty z "zsh -f -i"
zpty -w z "PS1='READY> '; fpath=($1/fn \$fpath); autoload -Uz compinit; compinit -u -D; show() { print -r -- \"<\$BUFFER>\" >> $out; BUFFER=; zle accept-line }; zle -N show; bindkey '^T' show"
for line in \
    "illumio-tui workloads --label app=s" \
    "illumio-tui workloads --label loc=n" \
    "illumio-tui workloads --hostname d" \
    "illumio-tui workloads --label=app=c" \
    "illumio-tui traffic query --src wl:w" \
    "illumio-tui enforce s" \
    "illumio-tui workloads --profile prod-eu --label app=e"; do
  zpty -r -m z x '*READY> '
  zpty -w -n z "$line"$'\t\x14'
done
# Typed input is handled in order: the last buffer is written before this.
zpty -w z "print -r -- FIN''ISHED"
zpty -r -m z x '*FINISHED*'
zpty -d z
cat $out
"#;
    let out = run_zsh(driver, dir.path());
    if out.status.code() == Some(3) {
        eprintln!("zsh/zpty not available; skipped");
        return;
    }
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        String::from_utf8(out.stdout).unwrap(),
        "<illumio-tui workloads --label app=shop >\n\
         <illumio-tui workloads --label loc=new\\ york >\n\
         <illumio-tui workloads --hostname db\\ 1 >\n\
         <illumio-tui workloads --label=app=crm >\n\
         <illumio-tui traffic query --src wl:web1 >\n\
         <illumio-tui enforce shop/prod >\n\
         <illumio-tui workloads --profile prod-eu --label app=eu >\n"
    );
}

/// A profile name is never expanded by bash: `$(...)` stays text.
#[test]
fn bash_profile_names_are_quoted_not_expanded() {
    if !Path::new("/bin/bash").exists() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("illumio-tui")).unwrap();
    let marker = dir.path().join("ran");
    std::fs::write(
        dir.path().join("illumio-tui").join("config.toml"),
        format!(
            "[profiles.\"$(touch {})\"]\n[profiles.\"a b\"]\n",
            marker.display()
        ),
    )
    .unwrap();
    let script = dir.path().join("illumio-tui.bash");
    std::fs::write(
        &script,
        stdout(bin(dir.path()).args(["completions", "bash"])),
    )
    .unwrap();
    let driver = r#"source "$1"
COMP_WORDS=(illumio-tui --profile ""); COMP_CWORD=2; COMPREPLY=()
_illumio-tui__dynamic illumio-tui "" --profile
printf '%s\n' "${COMPREPLY[@]}"
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
    let text = String::from_utf8(out.stdout).unwrap();
    assert_eq!(
        text,
        format!("\\$\\(touch\\ {}\\)\na\\ b\n", marker.display())
    );
    assert!(!marker.exists(), "a profile name ran a command");
}
