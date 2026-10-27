use super::diff::{DiffSection, End, check_same, diff};
use super::*;
use serde_json::json;

fn parse<T: serde::de::DeserializeOwned>(text: &str) -> T {
    serde_json::from_str(text).unwrap()
}

/// A snapshot of the fixtures, every section but readiness.
pub(crate) fn fixture(at: &str) -> Snapshot {
    let workloads: Vec<Workload> = parse(include_str!("../../../../fixtures/workloads.json"));
    Snapshot {
        version: VERSION,
        profile: "demo".into(),
        pce: "pce.example".into(),
        org: 5,
        taken_at: at.parse().unwrap(),
        label: None,
        truncated: Vec::new(),
        sections: Sections {
            policy: Some(PolicySection {
                draft_rule_sets: parse(include_str!("../../../../fixtures/rule_sets.json")),
                active_rule_sets: Vec::new(),
                label_groups: parse(include_str!("../../../../fixtures/label_groups.json")),
                ip_lists: parse(include_str!("../../../../fixtures/ip_lists.json")),
                services: parse(include_str!("../../../../fixtures/services.json")),
                enforcement_boundaries: parse(include_str!(
                    "../../../../fixtures/enforcement_boundaries.json"
                )),
            }),
            labels: Some(parse(include_str!("../../../../fixtures/labels.json"))),
            workloads: Some(workloads.into_iter().map(trim_workload).collect()),
            readiness: None,
        },
    }
}

fn end(id: &str, s: &Snapshot) -> End {
    End {
        id: id.into(),
        taken_at: s.taken_at,
        label: s.label.clone(),
    }
}

fn run(a: &Snapshot, b: &Snapshot, wanted: &[DiffSection]) -> diff::SnapshotDiff {
    diff(
        (a, end("A", a)),
        (b, end("B", b)),
        wanted,
        &KeyOrder::default(),
    )
}

use pce::KeyOrder;

#[test]
fn the_root_is_under_the_data_home_else_home() {
    assert_eq!(
        root_from(Some(Path::new("/x")), Some(Path::new("/h"))),
        Some(PathBuf::from("/x/illumio-tui/snapshots"))
    );
    assert_eq!(
        root_from(None, Some(Path::new("/h"))),
        Some(PathBuf::from("/h/.local/share/illumio-tui/snapshots"))
    );
    assert_eq!(root_from(None, None), None);
    assert_eq!(
        profile_dir(Path::new("/r"), "a b/.."),
        PathBuf::from("/r/a%20b%2F%2E%2E")
    );
}

#[test]
fn a_workload_keeps_its_labels_mode_and_agent_status_only() {
    let w = &fixture("2026-10-01T00:00:00Z").sections.workloads.unwrap()[0];
    assert!(w.interfaces.is_empty());
    assert_eq!(w.agent_status().as_deref(), Some("active"));
    assert_eq!(w.agent_version(), Some("23.2.31-3113"));
    assert_eq!(w.agent.as_ref().unwrap().as_object().unwrap().len(), 1);
    assert_eq!(w.labels.len(), 3);
    assert_eq!(w.enforcement_mode.as_deref(), Some("full"));
}

#[test]
fn save_list_find_rm_and_prune() {
    let dir = tempfile::tempdir().unwrap();
    let d = dir.path().join("demo");
    assert!(list(&d).unwrap().is_empty(), "no directory: none");
    let mut a = fixture("2026-10-01T03:15:00Z");
    a.label = Some("before".into());
    let sa = save(&d, &a).unwrap();
    assert_eq!(sa.id, "2026-10-01T031500Z");
    assert_eq!(sa.path, d.join("2026-10-01T031500Z.json"));
    // The same second again: a suffix, never an overwrite.
    let again = save(&d, &a).unwrap();
    assert_eq!(again.id, "2026-10-01T031500Z-1");
    let mut b = fixture("2026-10-08T03:15:00Z");
    b.sections.workloads = None;
    save(&d, &b).unwrap();
    std::fs::write(d.join("notes.txt"), "not a snapshot").unwrap();

    let metas = list(&d).unwrap();
    let ids: Vec<&str> = metas.iter().map(|m| m.id.as_str()).collect();
    assert_eq!(
        ids,
        [
            "2026-10-01T031500Z",
            "2026-10-01T031500Z-1",
            "2026-10-08T031500Z"
        ]
    );
    assert_eq!(metas[0].label.as_deref(), Some("before"));
    assert_eq!(metas[0].sections, ["policy", "labels", "workloads"]);
    assert_eq!(metas[2].sections, ["policy", "labels"]);
    assert_eq!(metas[0].bytes, sa.bytes);
    assert!(metas.iter().all(|m| m.problem.is_none()));
    assert_eq!(load(&metas[0].path).unwrap(), a);

    assert_eq!(find(&metas, "latest").unwrap().id, "2026-10-08T031500Z");
    assert_eq!(find(&metas, "previous").unwrap().id, "2026-10-01T031500Z-1");
    assert_eq!(find(&metas, "2026-10-08").unwrap().id, "2026-10-08T031500Z");
    assert_eq!(
        find(&metas, "2026-10-01T031500Z").unwrap().id,
        "2026-10-01T031500Z",
        "an exact id wins over the prefix it is of another"
    );
    let e = find(&metas, "2026-10").unwrap_err();
    assert!(e.contains("names 3 snapshots"), "{e}");
    assert!(find(&metas, "2025").unwrap_err().contains("no snapshot"));
    assert!(
        find(&metas[..1], "previous")
            .unwrap_err()
            .contains("2 needed")
    );

    remove(&metas[1]).unwrap();
    let metas = list(&d).unwrap();
    assert_eq!(metas.len(), 2);
    let gone = prune(&metas, 1).unwrap();
    assert_eq!(gone.len(), 1);
    assert_eq!(gone[0].id, "2026-10-01T031500Z");
    assert_eq!(list(&d).unwrap().len(), 1);
    assert!(prune(&list(&d).unwrap(), 5).unwrap().is_empty());
}

#[test]
fn a_corrupt_or_newer_file_says_so() {
    let dir = tempfile::tempdir().unwrap();
    let bad = dir.path().join("2026-10-01T000000Z.json");
    std::fs::write(&bad, "{not json").unwrap();
    let e = load(&bad).unwrap_err();
    assert!(e.contains("not a snapshot file"), "{e}");
    let newer = dir.path().join("2026-10-02T000000Z.json");
    let mut v = serde_json::to_value(fixture("2026-10-02T00:00:00Z")).unwrap();
    v["version"] = json!(VERSION + 1);
    std::fs::write(&newer, v.to_string()).unwrap();
    let e = load(&newer).unwrap_err();
    assert!(e.contains("newer than this illumio-tui reads"), "{e}");
    let metas = list(dir.path()).unwrap();
    assert!(
        metas[0]
            .problem
            .as_deref()
            .unwrap()
            .contains("not a snapshot")
    );
    assert!(metas[1].problem.as_deref().unwrap().contains("upgrade"));
    assert_eq!(metas[1].version, Some(VERSION + 1));
}

#[test]
fn identical_snapshots_differ_in_nothing() {
    let a = fixture("2026-10-01T00:00:00Z");
    let b = fixture("2026-10-02T00:00:00Z");
    let d = run(&a, &b, &[]);
    assert!(!d.changed);
    assert_eq!(d.sections.len(), 7);
    assert!(d.not_compared.is_empty());
}

#[test]
fn workloads_report_added_removed_labels_mode_and_ven() {
    let a = fixture("2026-10-01T00:00:00Z");
    let mut b = fixture("2026-10-02T00:00:00Z");
    let ws = b.sections.workloads.as_mut().unwrap();
    let removed = ws.remove(ws.len() - 1);
    let w = &mut ws[0];
    w.labels.retain(|l| l.key.as_deref() != Some("env"));
    w.labels.push(pce::models::WorkloadLabel {
        href: "/orgs/5/labels/9".into(),
        key: Some("env".into()),
        value: Some("dev".into()),
    });
    w.enforcement_mode = Some("visibility_only".into());
    w.agent = Some(json!({"status": {"status": "stopped", "agent_version": "23.5.0"}}));
    let mut new = ws[1].clone();
    new.href = "/orgs/5/workloads/new-1".into();
    new.hostname = Some("new-1.example".into());
    ws.push(new);
    let d = run(&a, &b, &[DiffSection::Workloads]);
    assert_eq!(d.sections.len(), 1);
    let c = &d.sections[0];
    assert_eq!((c.added, c.removed, c.changed), (1, 1, 1));
    let changed = d.changes.iter().find(|c| c.change == "changed").unwrap();
    assert_eq!(changed.name, "host-1.example");
    assert_eq!(
        changed.fields,
        ["labels", "enforcement_mode", "ven_status", "ven_version"]
    );
    assert!(changed.details.contains(&"+label env=dev".to_string()));
    assert!(changed.details.contains(&"-label env=prod".to_string()));
    assert!(
        changed
            .details
            .contains(&"-enforcement_mode: full".to_string())
    );
    assert!(
        changed
            .details
            .contains(&"+ven_version: 23.5.0".to_string())
    );
    let gone = d.changes.iter().find(|c| c.change == "removed").unwrap();
    assert_eq!(gone.href, removed.href);
    let added = d.changes.iter().find(|c| c.change == "added").unwrap();
    assert_eq!(added.name, "new-1.example");
    assert!(added.fields.is_empty());
}

#[test]
fn policy_sections_reuse_the_policy_diff() {
    let a = fixture("2026-10-01T00:00:00Z");
    let mut b = fixture("2026-10-02T00:00:00Z");
    let p = b.sections.policy.as_mut().unwrap();
    p.ip_lists[1]
        .ip_ranges
        .push(parse(r#"{"from_ip": "10.9.0.0/16"}"#));
    p.services.remove(0);
    p.draft_rule_sets[0].rules[0].enabled = false;
    p.enforcement_boundaries.as_mut().unwrap()[0].name = "Block prod from test".into();
    p.label_groups[0].name.push_str(" (old)");
    let d = run(&a, &b, &[]);
    let one = |s: &str| {
        let v: Vec<_> = d.changes.iter().filter(|c| c.section == s).collect();
        assert_eq!(v.len(), 1, "{s}: {v:#?}");
        v[0].clone()
    };
    let ip = one("iplists");
    assert_eq!(
        (ip.change, ip.fields.as_slice()),
        ("changed", &["ranges".to_string()][..])
    );
    assert_eq!(ip.details, ["+range 10.9.0.0/16"]);
    assert_eq!(one("services").change, "removed");
    let rs = one("rulesets");
    assert_eq!(rs.scope, Some("draft"));
    assert_eq!(rs.fields, ["rules"]);
    assert_eq!(rs.details.len(), 2, "{:?}", rs.details);
    assert!(rs.details[1].ends_with("(disabled)"), "{:?}", rs.details);
    let b_ = one("boundaries");
    assert_eq!(b_.fields, ["name"]);
    assert_eq!(
        b_.details,
        ["-name: Block prod from dev", "+name: Block prod from test"]
    );
    assert_eq!(one("label-groups").fields, ["name"]);
    assert!(d.changes.iter().all(|c| c.section != "labels"));
}

#[test]
fn labels_report_value_changes_and_deletes() {
    let a = fixture("2026-10-01T00:00:00Z");
    let mut b = fixture("2026-10-02T00:00:00Z");
    let l = b.sections.labels.as_mut().unwrap();
    l[0].value = "frontend".into();
    l[1].deleted = Some(true);
    let d = run(&a, &b, &[DiffSection::Labels]);
    assert_eq!(d.changes.len(), 2);
    assert_eq!(d.changes[0].fields, ["value"]);
    assert_eq!(d.changes[0].name, "role=frontend");
    assert_eq!(d.changes[1].details, ["-deleted: no", "+deleted: yes"]);
}

#[test]
fn a_section_one_side_lacks_is_not_compared() {
    let a = fixture("2026-10-01T00:00:00Z");
    let mut b = fixture("2026-10-02T00:00:00Z");
    b.sections.workloads = None;
    let d = run(&a, &b, &[DiffSection::Workloads, DiffSection::Labels]);
    assert_eq!(d.sections.len(), 1);
    assert_eq!(d.not_compared.len(), 1);
    assert_eq!(d.not_compared[0].reason, "no workloads section in B");
}

#[test]
fn snapshots_of_another_profile_or_pce_are_refused() {
    let a = fixture("2026-10-01T00:00:00Z");
    let mut b = fixture("2026-10-02T00:00:00Z");
    assert!(check_same(&a, "A", &b, "B").is_ok());
    b.pce = "other.example".into();
    let e = check_same(&a, "A", &b, "B").unwrap_err();
    assert!(
        e.contains("pce.example") && e.contains("other.example"),
        "{e}"
    );
    b.pce = a.pce.clone();
    b.profile = "prod".into();
    assert!(
        check_same(&a, "A", &b, "B")
            .unwrap_err()
            .contains("\"prod\"")
    );
}

#[test]
fn only_snapshot_ids_are_snapshots_and_they_sort_by_time_then_number() {
    assert_eq!(
        parse_id("2026-10-01T031500Z"),
        Some(("2026-10-01T031500Z", 0))
    );
    assert_eq!(
        parse_id("2026-10-01T031500Z-12"),
        Some(("2026-10-01T031500Z", 12))
    );
    for bad in [
        "notes",
        "backup-2026",
        "2026-10-01T031500",
        "2026-10-01T031500Z-",
        "2026-10-01T031500Z-x",
        "2026-10-01 031500Z",
        "2026-10-01T03:15:00Z",
    ] {
        assert_eq!(parse_id(bad), None, "{bad}");
    }
    let dir = tempfile::tempdir().unwrap();
    let a = fixture("2026-10-01T03:15:00Z");
    for _ in 0..11 {
        save(dir.path(), &a).unwrap();
    }
    std::fs::write(dir.path().join("notes.json"), "{}").unwrap();
    let ids: Vec<String> = list(dir.path())
        .unwrap()
        .into_iter()
        .map(|m| m.id)
        .collect();
    assert_eq!(ids.len(), 11, "notes.json is no snapshot");
    assert_eq!(ids[2], "2026-10-01T031500Z-2");
    assert_eq!(ids[10], "2026-10-01T031500Z-10", "-10 after -2");
}

#[test]
fn latest_previous_and_prune_skip_unreadable_files() {
    let dir = tempfile::tempdir().unwrap();
    let d = dir.path();
    save(d, &fixture("2026-10-01T00:00:00Z")).unwrap();
    save(d, &fixture("2026-10-02T00:00:00Z")).unwrap();
    save(d, &fixture("2026-10-03T00:00:00Z")).unwrap();
    std::fs::write(d.join("2026-10-04T000000Z.json"), "{broken").unwrap();
    let mut v = serde_json::to_value(fixture("2026-10-05T00:00:00Z")).unwrap();
    v["version"] = json!(VERSION + 1);
    std::fs::write(d.join("2026-10-05T000000Z.json"), v.to_string()).unwrap();
    let metas = list(d).unwrap();
    assert_eq!(metas.len(), 5);
    assert_eq!(find(&metas, "latest").unwrap().id, "2026-10-03T000000Z");
    assert_eq!(find(&metas, "previous").unwrap().id, "2026-10-02T000000Z");
    // Named by its time, an unreadable file can still be picked (rm).
    assert_eq!(find(&metas, "2026-10-04").unwrap().id, "2026-10-04T000000Z");
    let gone = prune(&metas, 1).unwrap();
    let ids: Vec<&str> = gone.iter().map(|m| m.id.as_str()).collect();
    assert_eq!(ids, ["2026-10-01T000000Z", "2026-10-02T000000Z"]);
    let left: Vec<String> = list(d).unwrap().into_iter().map(|m| m.id).collect();
    assert_eq!(
        left,
        [
            "2026-10-03T000000Z",
            "2026-10-04T000000Z",
            "2026-10-05T000000Z"
        ]
    );
}

#[test]
fn save_never_overwrites_and_leaves_no_temporary_file() {
    let dir = tempfile::tempdir().unwrap();
    let d = dir.path();
    // Another writer took the name first.
    std::fs::write(d.join("2026-10-01T000000Z.json"), "theirs").unwrap();
    // A crashed save's leftover, two hours old.
    let stale = d.join(".2026-09-01T000000Z.1-0.tmp");
    // A hidden temp file of someone else's: never touched.
    let theirs = d.join(".notes.tmp");
    std::fs::write(&theirs, "keep").unwrap();
    std::fs::write(&stale, "x").unwrap();
    let old = std::time::SystemTime::now() - std::time::Duration::from_secs(7200);
    std::fs::File::options()
        .write(true)
        .open(&stale)
        .unwrap()
        .set_modified(old)
        .unwrap();
    std::fs::File::options()
        .write(true)
        .open(&theirs)
        .unwrap()
        .set_modified(old)
        .unwrap();
    let s = save(d, &fixture("2026-10-01T00:00:00Z")).unwrap();
    assert_eq!(s.id, "2026-10-01T000000Z-1");
    assert!(theirs.exists());
    assert!(!stale.exists());
    assert_eq!(
        std::fs::read_to_string(d.join("2026-10-01T000000Z.json")).unwrap(),
        "theirs"
    );
    let names: Vec<String> = std::fs::read_dir(d)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert!(
        names
            .iter()
            .all(|n| !n.ends_with(".tmp") || n == ".notes.tmp"),
        "{names:?}"
    );
}

#[test]
fn unread_boundaries_are_not_compared_not_all_removed() {
    let a = fixture("2026-10-01T00:00:00Z");
    let mut b = fixture("2026-10-02T00:00:00Z");
    b.sections.policy.as_mut().unwrap().enforcement_boundaries = None;
    let d = run(&a, &b, &[]);
    assert!(d.changes.iter().all(|c| c.section != "boundaries"));
    assert_eq!(d.not_compared.len(), 1);
    assert_eq!(d.not_compared[0].section, "boundaries");
    assert_eq!(d.not_compared[0].reason, "boundaries not read in B");
    // And the file round-trips without the field.
    let text = serde_json::to_string(&b).unwrap();
    assert!(!text.contains("enforcement_boundaries"));
    let back: Snapshot = serde_json::from_str(&text).unwrap();
    assert!(
        back.sections
            .policy
            .unwrap()
            .enforcement_boundaries
            .is_none()
    );
}

#[test]
fn active_rulesets_boundary_actors_and_deleted_deny_rules_are_reported() {
    let mut a = fixture("2026-10-01T00:00:00Z");
    let pa = a.sections.policy.as_mut().unwrap();
    pa.active_rule_sets = pa.draft_rule_sets.clone();
    let mut b = a.clone();
    b.taken_at = "2026-10-02T00:00:00Z".parse().unwrap();
    let pb = b.sections.policy.as_mut().unwrap();
    pb.active_rule_sets[0].enabled = false;
    // Every deny rule deleted: the list no longer inlines any.
    for rs in &mut pb.draft_rule_sets {
        rs.deny_rules.clear();
    }
    pb.enforcement_boundaries.as_mut().unwrap()[0].consumers =
        vec![parse(r#"{"label": {"href": "/orgs/5/labels/3"}}"#)];
    let d = run(&a, &b, &[DiffSection::Rulesets, DiffSection::Boundaries]);
    let active: Vec<_> = d
        .changes
        .iter()
        .filter(|c| c.scope == Some("active"))
        .collect();
    assert_eq!(active.len(), 1);
    assert_eq!(active[0].fields, ["enabled"]);
    let draft = d
        .changes
        .iter()
        .find(|c| c.scope == Some("draft"))
        .expect("the deleted deny rules");
    assert_eq!(draft.fields, ["rules"]);
    assert!(
        draft.details.iter().all(|l| l.starts_with('-')) && draft.details.len() == 2,
        "{:?}",
        draft.details
    );
    let bnd = d
        .changes
        .iter()
        .find(|c| c.section == "boundaries")
        .unwrap();
    assert_eq!(bnd.fields, ["consumers"]);
    assert_eq!(bnd.details, ["+consumer app=shop", "-consumer loc=eu"]);
}

#[test]
fn a_large_workload_diff_is_linear() {
    let base = fixture("2026-10-01T00:00:00Z")
        .sections
        .workloads
        .unwrap()
        .remove(0);
    let many = |n: usize, mode: &str| -> Vec<Workload> {
        (0..n)
            .map(|i| {
                let mut w = base.clone();
                w.href = format!("/orgs/5/workloads/w-{i}");
                w.enforcement_mode = Some(mode.into());
                w
            })
            .collect()
    };
    let mut a = fixture("2026-10-01T00:00:00Z");
    let mut b = fixture("2026-10-02T00:00:00Z");
    a.sections.workloads = Some(many(50_000, "full"));
    let mut wb = many(50_000, "full");
    wb[49_999].enforcement_mode = Some("visibility_only".into());
    b.sections.workloads = Some(wb);
    let t = std::time::Instant::now();
    let d = run(&a, &b, &[DiffSection::Workloads]);
    assert_eq!(d.changes.len(), 1);
    // Quadratic would be 2.5e9 comparisons: minutes, not this.
    assert!(
        t.elapsed() < std::time::Duration::from_secs(20),
        "{:?}",
        t.elapsed()
    );
}

#[test]
fn the_identity_holds_host_port_and_org() {
    let cfg = pce::ClientConfig::new("pce.example", 7, pce::Credentials::new("k", "s"));
    let mut cfg2 = cfg.clone();
    cfg2.port = 8443;
    let a = Identity::of("p", &pce::PceClient::new(cfg).unwrap());
    let b = Identity::of("p", &pce::PceClient::new(cfg2).unwrap());
    assert_eq!(a.pce, "pce.example:443");
    assert_eq!(b.pce, "pce.example:8443");
    assert_eq!(a.org, 7);
}

#[test]
fn without_hard_links_the_name_is_reserved_then_renamed_over() {
    let dir = tempfile::tempdir().unwrap();
    let d = dir.path();
    let tmp = d.join(".2026-10-01T000000Z.1-0.tmp");
    std::fs::write(&tmp, "body").unwrap();
    let taken = d.join("2026-10-01T000000Z.json");
    std::fs::write(&taken, "theirs").unwrap();
    let e = reserve_and_rename(&tmp, &taken).unwrap_err();
    assert_eq!(e.kind(), std::io::ErrorKind::AlreadyExists);
    assert_eq!(std::fs::read_to_string(&taken).unwrap(), "theirs");
    let free = d.join("2026-10-01T000000Z-1.json");
    reserve_and_rename(&tmp, &free).unwrap();
    assert_eq!(std::fs::read_to_string(&free).unwrap(), "body");
    assert!(!tmp.exists());
}

#[test]
fn only_saves_own_temp_files_are_its_own() {
    assert!(is_own_tmp(".2026-10-01T000000Z.123-0.tmp"));
    for other in [
        ".notes.tmp",
        "2026-10-01T000000Z.123-0.tmp",
        ".2026-10-01T000000Z.123.tmp",
        ".2026-10-01T000000Z-1.123-0.tmp",
        ".2026-10-01T000000Z.x-0.tmp",
    ] {
        assert!(!is_own_tmp(other), "{other}");
    }
}
