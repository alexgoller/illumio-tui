//! #83: `:hygiene`, the unused objects and the workloads missing labels.
//! It reads what is loaded; what is missing is loaded once, on the
//! command, through the cache (`CacheMode::Use`), and reopening it loads
//! nothing.
use super::*;
use crate::tui::commands;
use crate::tui::model::PceUsage;
use crate::tui::model::{Model, Overlay, Screen};
use crate::tui::msg::{Cmd, Msg, ObjectsPayload};
use crate::tui::test_support::{
    model_fixture, model_with_objects, with_pairing_profiles, with_policy, with_workloads,
};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use pce::hygiene::Kind;

fn press(m: &mut Model, code: KeyCode) -> Vec<Cmd> {
    update(m, Msg::Key(KeyEvent::new(code, KeyModifiers::NONE)))
}

fn hygiene(m: &mut Model) -> Vec<Cmd> {
    update(m, Msg::Command(commands::parse("hygiene").unwrap()))
}

/// Everything the report reads is loaded, from a PCE that answered the
/// usage ask without flags (an older one): the scan decides.
fn loaded() -> Model {
    let mut m = with_pairing_profiles(with_workloads(with_policy(model_with_objects())));
    m.objects.pce_usage = PceUsage::Unsupported;
    m
}

fn usage_labels() -> Vec<pce::models::Label> {
    serde_json::from_str(include_str!("../../../../../fixtures/labels_usage.json")).unwrap()
}

/// The overlay's rows as `kind name · reason` lines.
fn rows(m: &Model) -> Vec<String> {
    let v = m.objects.hygiene.as_ref().expect("the overlay is open");
    v.table
        .visible()
        .iter()
        .map(|&i| {
            let r = &v.table.rows()[i];
            format!("{} {} · {}", r.kind.as_str(), r.name, r.reason)
        })
        .collect()
}

/// The cursor on the row of `name`.
fn select(m: &mut Model, name: &str) {
    let v = m.objects.hygiene.as_mut().unwrap();
    v.table.select_where(|r| r.name == name);
    assert_eq!(v.table.selected_item().unwrap().name, name);
}

fn rule_sets() -> Vec<pce::models::RuleSet> {
    serde_json::from_str(include_str!("../../../../../fixtures/rule_sets.json")).unwrap()
}

#[test]
fn with_everything_loaded_it_asks_for_nothing() {
    let mut m = loaded();
    m.screen = Screen::Traffic;
    assert_eq!(hygiene(&mut m), Vec::<Cmd>::new());
    assert_eq!(m.overlay, Overlay::Hygiene);
    assert_eq!(
        rows(&m),
        [
            "label loc=eu · unused",
            "service Web · unused",
            "workload host-1.example · missing labels: loc",
            "workload host-2.example · missing labels: loc",
            "workload unmanaged-nas · missing labels: role,app,env,loc",
            "workload host-4.example · missing labels: role,app,env,loc",
        ]
    );
    let v = m.objects.hygiene.as_ref().unwrap();
    assert_eq!(v.requests, 0);
    assert_eq!(
        v.unused_line(),
        "Unused: labels 1 · label groups 0 · IP lists 0 · services 1"
    );
    assert_eq!(
        v.missing_line(),
        "Missing labels: role 2 · app 2 · env 2 · loc 4 (of 4 workloads)"
    );
    assert_eq!(
        m.status.text,
        "hygiene · 2 unused objects · 4 workloads missing labels"
    );
}

#[test]
fn what_is_missing_loads_once_through_the_cache_and_reopening_loads_nothing() {
    let mut m = model_fixture();
    let cmds = hygiene(&mut m);
    assert_eq!(
        cmds,
        vec![
            Cmd::LoadRuleSets {
                scope: PolicyScope::Draft,
                mode: CacheMode::Use
            },
            Cmd::LoadPairingProfiles(CacheMode::Use),
            Cmd::LoadWorkloads(CacheMode::Use),
            Cmd::LoadLabelUsage(CacheMode::Use),
        ]
    );
    let v = m.objects.hygiene.as_ref().unwrap();
    assert_eq!(v.requests, 4);
    assert_eq!(
        v.waiting,
        ["rulesets", "label usage", "pairing profiles", "workloads"]
    );
    assert!(rows(&m).is_empty(), "no candidate before every list is in");
    assert!(
        m.status
            .text
            .contains("loading rulesets, label usage, pairing profiles, workloads…")
    );
    // Closed and reopened while they are on their way: nothing more.
    press(&mut m, KeyCode::Esc);
    assert_eq!(m.overlay, Overlay::None);
    assert_eq!(hygiene(&mut m), Vec::<Cmd>::new());
    assert_eq!(m.objects.hygiene.as_ref().unwrap().requests, 4);

    update(
        &mut m,
        Msg::RuleSetsLoaded {
            scope: PolicyScope::Draft,
            result: Ok((rule_sets(), 0, false)),
        },
    );
    let raw: Vec<pce::models::Workload> =
        serde_json::from_str(include_str!("../../../../../fixtures/workloads.json")).unwrap();
    update(&mut m, Msg::WorkloadsLoaded(Ok((raw, 0, false))));
    update(&mut m, Msg::LabelUsageLoaded(Ok(usage_labels())));
    // The workloads are checked already; the objects wait for the profiles.
    assert_eq!(rows(&m).len(), 4);
    assert_eq!(
        m.objects.hygiene.as_ref().unwrap().waiting,
        ["pairing profiles"]
    );
    let profiles: Vec<pce::models::PairingProfile> = serde_json::from_str(include_str!(
        "../../../../../fixtures/pairing_profiles.json"
    ))
    .unwrap();
    update(&mut m, Msg::PairingProfilesLoaded(Ok(profiles)));
    // The lists from the name index: labels, groups, IP lists, services.
    // loc=eu: the PCE says a pairing profile and a permission hold it.
    assert_eq!(rows(&m)[..1], ["service Web · unused"]);
    assert!(
        m.status.text.contains("1 unused object"),
        "{}",
        m.status.text
    );

    press(&mut m, KeyCode::Esc);
    assert!(m.objects.hygiene.is_none());
    assert_eq!(hygiene(&mut m), Vec::<Cmd>::new(), "reopened: all in hand");
    assert_eq!(rows(&m).len(), 5);
}

#[test]
fn the_pces_usage_flags_settle_labels_even_on_a_cut_short_workload_list() {
    let mut m = loaded();
    m.objects.pce_usage = PceUsage::from_labels(usage_labels());
    m.workloads.truncated = true;
    hygiene(&mut m);
    let v = m.objects.hygiene.as_ref().unwrap();
    // loc=eu is held (pairing profile, permission); none is unused by
    // the flags among the fixture's labels.
    assert!(v.withheld.is_empty());
    assert_eq!(rows(&m)[..1], ["service Web · unused"]);
    // Without flags, the same cut-short list withholds the labels.
    let mut m = loaded();
    m.workloads.truncated = true;
    hygiene(&mut m);
    let v = m.objects.hygiene.as_ref().unwrap();
    assert_eq!(v.withheld, [Kind::Label]);
    assert!(
        v.warnings()
            .iter()
            .any(|w| w.starts_with("not listed: labels (workloads not read in full")),
        "{:?}",
        v.warnings()
    );
}

#[test]
fn unread_deny_rules_mark_rows_uncertain_and_every_warning_shows() {
    let mut m = loaded();
    for rs in &mut m.policy.raw {
        rs.deny_rules.clear();
    }
    m.workloads.truncated = true;
    hygiene(&mut m);
    let v = m.objects.hygiene.as_ref().unwrap();
    assert!(v.deny_unread > 0);
    assert_eq!(v.uncertain, [Kind::LabelGroup, Kind::IpList, Kind::Service]);
    let web = v.table.rows().iter().find(|r| r.name == "Web").unwrap();
    assert!(!web.certain);
    let w = v.warnings();
    assert!(
        w.iter().any(|w| w.starts_with("not listed: labels")),
        "{w:?}"
    );
    assert!(w.iter().any(|w| w.starts_with("uncertain: ")), "{w:?}");
    assert!(
        w.iter()
            .any(|w| w.starts_with("the workload list was cut short")),
        "{w:?}"
    );
}

#[test]
fn a_failed_load_is_said_and_lists_no_candidate() {
    let mut m = with_workloads(with_policy(model_fixture()));
    m.objects.pce_usage = PceUsage::Unsupported;
    hygiene(&mut m);
    update(&mut m, Msg::PairingProfilesLoaded(Err("HTTP 403".into())));
    let v = m.objects.hygiene.as_ref().unwrap();
    assert_eq!(
        v.failed.as_deref(),
        Some("pairing profiles: HTTP 403 (:hygiene again asks)")
    );
    // Labels may sit in the profiles not read: withheld; the rest listed.
    assert_eq!(v.withheld, [Kind::Label]);
    assert!(rows(&m).contains(&"service Web · unused".to_string()));
    assert_eq!(
        rows(&m)
            .iter()
            .filter(|r| r.starts_with("workload"))
            .count(),
        4,
        "the workloads are still checked"
    );
    assert!(m.status.text.contains("HTTP 403"), "{}", m.status.text);
}

#[test]
fn the_profiles_label_model_sets_the_keys() {
    let mut m = loaded();
    m.limits.scope_keys = vec!["app".into(), "env".into()];
    m.limits.tier_key = "tier".into();
    hygiene(&mut m);
    let v = m.objects.hygiene.as_ref().unwrap();
    assert_eq!(v.keys, ["app", "env", "tier"]);
    assert!(
        rows(&m).contains(&"workload host-1.example · missing labels: tier".to_string()),
        "{:?}",
        rows(&m)
    );
}

#[test]
fn enter_on_an_object_selects_it_on_its_tab() {
    let mut m = loaded();
    m.screen = Screen::Traffic;
    hygiene(&mut m);
    select(&mut m, "Web");
    assert_eq!(press(&mut m, KeyCode::Enter), Vec::<Cmd>::new());
    assert_eq!(m.overlay, Overlay::None);
    assert!(m.objects.hygiene.is_none());
    assert_eq!(m.screen, Screen::Objects);
    assert_eq!(m.objects.kind, ObjectKind::Services);
    assert_eq!(m.objects.services.selected_item().unwrap().name, "Web");
    assert_eq!(m.status.text, "Web · x deletes it (with the usual confirm)");

    hygiene(&mut m);
    select(&mut m, "loc=eu");
    press(&mut m, KeyCode::Enter);
    assert_eq!(m.objects.kind, ObjectKind::Labels);
    assert_eq!(
        m.objects.labels.selected_item().unwrap().href,
        "/orgs/5/labels/5"
    );
}

#[test]
fn enter_on_an_object_whose_tab_is_loading_selects_it_when_it_lands() {
    let mut m = with_pairing_profiles(with_workloads(with_policy(model_fixture())));
    m.objects.pce_usage = PceUsage::Unsupported;
    hygiene(&mut m);
    select(&mut m, "loc=eu");
    let cmds = press(&mut m, KeyCode::Enter);
    // The Objects screen loads its tabs as always (cache hits after the index).
    assert!(cmds.contains(&Cmd::LoadObjects {
        kind: ObjectKind::Labels,
        mode: CacheMode::Use
    }));
    assert_eq!(m.objects.kind, ObjectKind::Labels);
    assert!(
        m.status
            .text
            .contains("loading Labels: loc=eu is selected then")
    );
    let labels: Vec<pce::models::Label> =
        serde_json::from_str(include_str!("../../../../../fixtures/labels.json")).unwrap();
    update(
        &mut m,
        Msg::ObjectsLoaded {
            kind: ObjectKind::Labels,
            result: Ok(ObjectsPayload::Labels(labels, 0)),
        },
    );
    assert_eq!(
        m.objects.labels.selected_item().unwrap().href,
        "/orgs/5/labels/5"
    );
    assert!(m.objects.select_on_load.is_none());
    assert!(
        m.status.text.starts_with("loc=eu · x deletes it"),
        "{}",
        m.status.text
    );
}

#[test]
fn enter_on_a_hidden_object_says_so() {
    let mut m = loaded();
    hygiene(&mut m);
    // The Labels tab shows the role key only.
    m.objects.label_keys = vec!["role".into()];
    let order = m.limits.key_order();
    m.objects.rebuild_labels(&order);
    select(&mut m, "loc=eu");
    press(&mut m, KeyCode::Enter);
    assert_eq!(m.status.kind, crate::tui::model::StatusKind::Warn);
    assert!(
        m.status.text.contains("is not on the Labels tab"),
        "{}",
        m.status.text
    );
}

#[test]
fn enter_on_a_workload_opens_its_detail() {
    let mut m = loaded();
    hygiene(&mut m);
    select(&mut m, "unmanaged-nas");
    press(&mut m, KeyCode::Enter);
    assert_eq!(m.screen, Screen::Workloads);
    assert!(m.workloads.show_detail);
    assert_eq!(
        m.workloads.selected_row().unwrap().workload.href,
        "/orgs/5/workloads/aaaa-3"
    );
}

#[test]
fn w_checks_an_unused_object_with_where_used_and_esc_comes_back() {
    let mut m = loaded();
    m.screen = Screen::Traffic;
    hygiene(&mut m);
    select(&mut m, "loc=eu");
    assert_eq!(press(&mut m, KeyCode::Char('W')), Vec::<Cmd>::new());
    assert_eq!(m.overlay, Overlay::WhereUsed);
    let v = m.objects.where_used.as_ref().unwrap();
    assert_eq!(v.target.href, "/orgs/5/labels/5");
    assert_eq!(v.name, "loc=eu");
    assert!(v.from_hygiene);
    // Nothing names it.
    assert_eq!(v.total(), 0);
    press(&mut m, KeyCode::Esc);
    assert_eq!(m.overlay, Overlay::Hygiene);
    assert_eq!(rows(&m).len(), 6);
    assert!(m.status.text.starts_with("hygiene · "), "{}", m.status.text);
    // On a workload, W says what it is for.
    select(&mut m, "host-1.example");
    press(&mut m, KeyCode::Char('W'));
    assert_eq!(m.overlay, Overlay::Hygiene);
    assert!(m.status.text.contains("Enter opens the workload"));
    // Where-used from Objects keeps closing as before.
    press(&mut m, KeyCode::Esc);
    m.screen = Screen::Objects;
    press(&mut m, KeyCode::Char('W'));
    assert_eq!(m.overlay, Overlay::WhereUsed);
    press(&mut m, KeyCode::Esc);
    assert_eq!(m.overlay, Overlay::None);
}

#[test]
fn before_the_name_index_it_waits() {
    let mut m = model_fixture();
    m.connection = crate::tui::model::Connection::Connected {
        index: None,
        index_age_secs: 0,
    };
    assert_eq!(hygiene(&mut m), Vec::<Cmd>::new());
    assert_eq!(m.overlay, Overlay::None);
    assert_eq!(m.status.text, "objects are still loading");
}

#[test]
fn a_truncated_workload_list_is_said() {
    let mut m = loaded();
    m.workloads.truncated = true;
    hygiene(&mut m);
    let v = m.objects.hygiene.as_ref().unwrap();
    assert!(v.workloads_truncated);
    assert!(v.missing_line().ends_with("(of 4+ workloads)"));
    assert_eq!(
        v.table
            .rows()
            .iter()
            .filter(|r| r.kind == Kind::Workload)
            .count(),
        4
    );
}

#[test]
fn search_and_help_keys_work_in_the_overlay() {
    let mut m = loaded();
    hygiene(&mut m);
    press(&mut m, KeyCode::Char('/'));
    for c in "nas".chars() {
        press(&mut m, KeyCode::Char(c));
    }
    assert_eq!(
        rows(&m),
        ["workload unmanaged-nas · missing labels: role,app,env,loc"]
    );
    press(&mut m, KeyCode::Enter);
    assert_eq!(m.overlay, Overlay::Hygiene, "Enter ends the search only");
    press(&mut m, KeyCode::Char('?'));
    assert_eq!(m.overlay, Overlay::Help);
    press(&mut m, KeyCode::Esc);
    assert_eq!(m.overlay, Overlay::Hygiene);
}
