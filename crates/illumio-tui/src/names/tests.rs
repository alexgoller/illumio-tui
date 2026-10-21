use super::record::Recorder;
use super::*;
use pce::ListObserver;

fn names() -> Names {
    Names {
        version: VERSION,
        written_at: None,
        host: None,
        org_id: None,
        labels: vec![
            "app=crm".into(),
            "app=shop".into(),
            "env=prod".into(),
            "loc=new york".into(),
        ],
        label_keys: vec!["app".into(), "env".into(), "loc".into()],
        apps: vec!["crm/prod".into(), "shop/prod".into()],
        hostnames: vec!["db 1".into(), "web1".into()],
        services: vec!["HTTPS".into(), "SSH".into()],
        ip_lists: vec!["Any (0.0.0.0/0 and ::/0)".into(), "Corp".into()],
        label_groups: vec!["Prod envs".into()],
    }
}

/// The PCE the test files are written for.
fn lab() -> Origin {
    Origin {
        host: "pce.lab".into(),
        org_id: 1,
    }
}

fn c(kind: Kind, word: &str) -> Vec<String> {
    complete(kind, &names(), word)
}

#[test]
fn each_kind_completes_its_names_by_prefix() {
    assert_eq!(c(Kind::Labels, "app="), ["app=crm", "app=shop"]);
    assert_eq!(c(Kind::Labels, "loc=new"), ["loc=new york"]);
    assert_eq!(c(Kind::Labels, "x"), Vec::<String>::new());
    assert_eq!(c(Kind::LabelKeys, "e"), ["env"]);
    assert_eq!(c(Kind::Apps, "s"), ["shop/prod"]);
    assert_eq!(c(Kind::Hostnames, "db"), ["db 1"]);
    assert_eq!(c(Kind::Services, ""), ["HTTPS", "SSH"]);
    assert_eq!(c(Kind::IpLists, "C"), ["Corp"]);
    assert_eq!(c(Kind::LabelGroups, "P"), ["Prod envs"]);
    assert_eq!(c(Kind::ServiceTerms, "svc:S"), ["svc:SSH"]);
    assert_eq!(c(Kind::ServiceTerms, "22"), Vec::<String>::new());
    assert_eq!(c(Kind::Traffic, "wl:w"), ["wl:web1"]);
    assert_eq!(c(Kind::Traffic, "ipl:C"), ["ipl:Corp"]);
    assert_eq!(c(Kind::Traffic, "e"), ["env=prod"]);
    assert_eq!(c(Kind::Scope, "group:"), ["group:Prod envs"]);
    assert_eq!(c(Kind::Actors, "!workload:d"), ["!workload:db 1"]);
    assert_eq!(c(Kind::Actors, "iplist:C"), ["iplist:Corp"]);
    assert_eq!(c(Kind::Search, "svc:H"), ["svc:HTTPS"]);
    assert_eq!(c(Kind::Search, "wl:"), ["wl:db 1", "wl:web1"]);
}

#[test]
fn lists_complete_their_last_term_and_a_flag_prefix_is_dropped() {
    assert_eq!(c(Kind::Labels, "app=shop,e"), ["app=shop,env=prod"]);
    assert_eq!(c(Kind::Traffic, "app=shop,wl:w"), ["app=shop,wl:web1"]);
    assert_eq!(c(Kind::Scope, "env=prod,g"), ["env=prod,group:Prod envs"]);
    // A label filter's later terms are more values of its key.
    assert_eq!(c(Kind::LabelFilter, "app=shop,c"), ["app=shop,crm"]);
    assert_eq!(c(Kind::LabelFilter, "!app=s"), ["!app=shop"]);
    assert_eq!(
        c(Kind::LabelFilter, "!app=shop,"),
        ["!app=shop,crm", "!app=shop,shop"]
    );
    // Not a list: the comma is part of the name.
    assert_eq!(c(Kind::Hostnames, "a,b"), Vec::<String>::new());
    assert_eq!(c(Kind::Labels, "--label=app=c"), ["app=crm"]);
    assert_eq!(c(Kind::Labels, "!app"), Vec::<String>::new());
}

#[test]
fn search_completes_after_a_side_a_bang_and_a_dash() {
    assert_eq!(c(Kind::Search, "scope:app=s"), ["scope:app=shop"]);
    assert_eq!(c(Kind::Search, "scope:g"), ["scope:global"]);
    assert_eq!(c(Kind::Search, "src:!env="), ["src:!env=prod"]);
    assert_eq!(c(Kind::Search, "dst:ipl:C"), ["dst:ipl:Corp"]);
    assert_eq!(c(Kind::Search, "consumer:wl:w"), ["consumer:wl:web1"]);
    assert_eq!(
        c(Kind::Search, "dst:a"),
        ["dst:app=crm", "dst:app=shop", "dst:all"]
    );
    assert_eq!(c(Kind::Search, "-scope:e"), ["-scope:env=prod"]);
    assert_eq!(c(Kind::Search, "-svc:S"), ["-svc:SSH"]);
    assert_eq!(c(Kind::Search, "!app=c"), ["!app=crm"]);
    // A bare word: the side prefixes and the property words too.
    assert_eq!(
        c(Kind::Search, "s"),
        ["scope:", "src:", "svc:HTTPS", "svc:SSH"]
    );
    assert_eq!(c(Kind::Search, "in"), ["intra"]);
    assert_eq!(c(Kind::Search, "-e"), ["-enabled", "-extra", "-env=prod"]);
    assert_eq!(c(Kind::Search, "d"), ["dst:", "disabled", "deny"]);
    // `scope:` takes labels only; `!` labels only.
    assert_eq!(c(Kind::Search, "scope:ipl:"), Vec::<String>::new());
    assert_eq!(c(Kind::Search, "src:!ipl:"), Vec::<String>::new());
    assert_eq!(c(Kind::Search, "src:!a"), ["src:!app=crm", "src:!app=shop"]);
}

#[test]
fn kinds_round_trip_by_name() {
    for k in Kind::ALL {
        assert_eq!(Kind::parse(k.name()), Some(k));
    }
    assert_eq!(Kind::parse("profiles"), None);
}

#[test]
fn the_path_is_per_profile_under_the_cache_dir() {
    let p = path_from(Some(Path::new("/x")), Some(Path::new("/h")), "lab");
    assert_eq!(p.unwrap(), Path::new("/x/illumio-tui/lab/names.json"));
    let p = path_from(None, Some(Path::new("/h")), "a/../b");
    assert_eq!(
        p.unwrap(),
        Path::new("/h/.cache/illumio-tui/a%2F%2E%2E%2Fb/names.json")
    );
    assert_eq!(
        path_from(None, Some(Path::new("/h")), "..").unwrap(),
        Path::new("/h/.cache/illumio-tui/%2E%2E/names.json")
    );
    assert_eq!(path_from(None, None, "lab"), None);
    // Lossless: names that differ get directories that differ.
    let names = ["prod lab", "prod_lab", "prod%20lab", "", "%", "é"];
    let dirs: std::collections::BTreeSet<String> = names.iter().map(|n| dir_name(n)).collect();
    assert_eq!(dirs.len(), names.len(), "{dirs:?}");
    assert_eq!(dir_name("prod-eu_1"), "prod-eu_1");
    assert_eq!(dir_name("prod lab"), "prod%20lab");
}

#[test]
fn a_missing_corrupt_or_other_version_file_reads_as_none() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("names.json");
    assert_eq!(load(&path), None);
    std::fs::write(&path, "{not json").unwrap();
    assert_eq!(load(&path), None);
    std::fs::write(&path, r#"{"version": 99, "labels": ["app=x"]}"#).unwrap();
    assert_eq!(load(&path), None);
    std::fs::write(&path, r#"{"version": 1, "labels": ["app=x"]}"#).unwrap();
    assert_eq!(load(&path).unwrap().labels, ["app=x"]);
}

#[test]
fn a_whole_list_replaces_its_sets_a_part_adds_and_other_sets_stay() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("p").join("names.json");
    let now: Timestamp = "2026-10-21T10:00:00Z".parse().unwrap();
    let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
    record(
        &path,
        &lab(),
        Update::Services(s(&["SSH", "HTTPS", "SSH"])),
        true,
        now,
    )
    .unwrap();
    record(
        &path,
        &lab(),
        Update::Labels {
            labels: s(&["app=shop"]),
            keys: s(&["app"]),
        },
        true,
        now,
    )
    .unwrap();
    let n = load(&path).unwrap();
    assert_eq!(n.services, ["HTTPS", "SSH"], "sorted, deduplicated");
    assert_eq!(n.labels, ["app=shop"]);
    assert_eq!(n.written_at, Some(now));
    // A filtered read adds; a whole one replaces; services untouched.
    record(
        &path,
        &lab(),
        Update::Workloads {
            hostnames: s(&["web1"]),
            apps: s(&["shop/prod"]),
        },
        true,
        now,
    )
    .unwrap();
    record(
        &path,
        &lab(),
        Update::Workloads {
            hostnames: s(&["db1"]),
            apps: vec![],
        },
        false,
        now,
    )
    .unwrap();
    let n = load(&path).unwrap();
    assert_eq!(n.hostnames, ["db1", "web1"]);
    assert_eq!(n.apps, ["shop/prod"]);
    assert_eq!(n.services, ["HTTPS", "SSH"]);
    record(
        &path,
        &lab(),
        Update::Workloads {
            hostnames: s(&["app1"]),
            apps: vec![],
        },
        true,
        now,
    )
    .unwrap();
    let n = load(&path).unwrap();
    assert_eq!(n.hostnames, ["app1"]);
    assert!(n.apps.is_empty());
    assert_eq!(n.labels, ["app=shop"]);
    // Another PCE's file is started over, not merged into.
    let other = Origin {
        host: "pce.other".into(),
        org_id: 1,
    };
    record(&path, &other, Update::IpLists(s(&["Other"])), false, now).unwrap();
    let n = load(&path).unwrap();
    assert_eq!(n.ip_lists, ["Other"]);
    assert!(n.labels.is_empty() && n.hostnames.is_empty(), "{n:?}");
    assert!(n.is_of(&other) && !n.is_of(&lab()));
    // A corrupt file is started over, not an error.
    std::fs::write(&path, "garbage").unwrap();
    record(&path, &lab(), Update::IpLists(s(&["Corp"])), true, now).unwrap();
    assert_eq!(load(&path).unwrap().ip_lists, ["Corp"]);
    assert!(clear(&path).unwrap());
    assert!(!clear(&path).unwrap());
}

#[cfg(unix)]
#[test]
fn the_file_is_private() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let path = dir
        .path()
        .join("illumio-tui")
        .join("lab")
        .join("names.json");
    record(
        &path,
        &lab(),
        Update::Services(vec!["SSH".into()]),
        true,
        Timestamp::now(),
    )
    .unwrap();
    let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode(&path), 0o600);
    assert_eq!(mode(path.parent().unwrap()), 0o700);
    let leftovers: Vec<_> = std::fs::read_dir(path.parent().unwrap())
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert_eq!(leftovers, ["names.json"], "no temp file left");
}

#[test]
fn the_recorder_keeps_live_names_and_apps_of_the_scope_keys() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("names.json");
    let r = Recorder::new(path.clone(), lab(), vec!["app".into(), "env".into()]);
    let labels: Vec<pce::models::Label> = serde_json::from_value(serde_json::json!([
        {"href": "/l/1", "key": "app", "value": "shop"},
        {"href": "/l/2", "key": "env", "value": "prod"},
        {"href": "/l/3", "key": "env", "value": "old", "deleted": true},
    ]))
    .unwrap();
    r.labels(&labels, true);
    let services: Vec<pce::models::Service> = serde_json::from_value(serde_json::json!([
        {"href": "/s/1", "name": "SSH"},
        {"href": "/s/2", "name": "Gone", "update_type": "delete"},
    ]))
    .unwrap();
    r.services(&services, true);
    let wls: Vec<pce::models::Workload> = serde_json::from_value(serde_json::json!([
        {"href": "/w/1", "hostname": "web1", "labels": [
            {"href": "/l/1", "key": "app", "value": "shop"},
            {"href": "/l/2", "key": "env", "value": "prod"}]},
        {"href": "/w/2", "name": "nas", "hostname": "", "labels": [
            {"href": "/l/1", "key": "app", "value": "shop"}]},
        {"href": "/w/3", "hostname": "gone", "deleted": true},
    ]))
    .unwrap();
    r.workloads(&wls, true);
    // A name the helper could not print on one line is left out.
    let odd: Vec<pce::models::IpList> = serde_json::from_value(serde_json::json!([
        {"href": "/i/1", "name": "Corp"},
        {"href": "/i/2", "name": "two\nlines"},
        {"href": "/i/3", "name": "cr\r"},
    ]))
    .unwrap();
    r.ip_lists(&odd, true);
    r.flush();
    let n = load(&path).unwrap();
    assert!(n.is_of(&lab()));
    assert_eq!(n.ip_lists, ["Corp"]);
    assert_eq!(n.labels, ["app=shop", "env=prod"]);
    assert_eq!(n.label_keys, ["app", "env"]);
    assert_eq!(n.services, ["SSH"]);
    assert_eq!(
        n.hostnames,
        ["nas", "web1"],
        "the name when the hostname is empty"
    );
    assert_eq!(
        n.apps,
        ["shop/prod"],
        "an app missing a scope label is left out"
    );
}

#[test]
fn completion_cache_false_gives_no_path_to_read_or_write() {
    let f: ConfigFile =
        toml::from_str("default_profile = \"lab\"\ncompletion_cache = false\n[profiles.lab]\n")
            .unwrap();
    assert_eq!(path_for(&f, "lab"), None);
}

/// The completion helper's side must not reach client code: this module
/// reads and writes one file, and the recorder only hears of reads.
#[test]
fn this_module_depends_on_no_client_code() {
    let src = include_str!("mod.rs");
    let code: String = src
        .lines()
        .filter(|l| !l.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n");
    for banned in [
        "pce::",
        "PceClient",
        "Context",
        "resolve(",
        "identity(",
        "tokio",
        "Command::new",
    ] {
        assert!(!code.contains(banned), "names/mod.rs uses {banned}");
    }
}

/// Updates are written in the order they came, on the recorder's thread,
/// and all of them before the recorder is gone.
#[test]
fn the_recorder_writes_in_order_and_finishes_when_dropped() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("names.json");
    let services = |names: &[&str]| -> Vec<pce::models::Service> {
        names
            .iter()
            .enumerate()
            .map(|(i, n)| {
                serde_json::from_value(serde_json::json!({"href": format!("/s/{i}"), "name": n}))
                    .unwrap()
            })
            .collect()
    };
    let r = Recorder::new(path.clone(), lab(), Vec::new());
    for i in 0..50 {
        r.services(&services(&[&format!("S{i:02}")]), i == 0);
    }
    drop(r);
    assert_eq!(load(&path).unwrap().services.len(), 50);
    let r = Recorder::new(path.clone(), lab(), Vec::new());
    r.services(&services(&["A"]), false);
    r.services(&services(&["B"]), true);
    drop(r);
    assert_eq!(
        load(&path).unwrap().services,
        ["B"],
        "the whole read came last"
    );
}
