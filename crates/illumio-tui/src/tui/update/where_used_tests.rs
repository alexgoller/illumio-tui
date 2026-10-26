//! #75: `W` / `:used` on the Objects screen, the where-used overlay. It
//! reads what is loaded; what is missing is loaded once, on the key press,
//! through the cache (`CacheMode::Use`), and reopening it loads nothing.
use super::*;
use crate::tui::commands;
use crate::tui::model::{Model, Overlay, Screen};
use crate::tui::msg::{Cmd, Msg};
use crate::tui::test_support::{
    model_with_ip_lists, model_with_objects, on_ip_lists_tab, with_pairing_profiles, with_policy,
    with_workloads,
};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use pce::where_used::{RefKind, WorkloadCount};

const PROD: &str = "/orgs/5/labels/4";

fn press(m: &mut Model, code: KeyCode) -> Vec<Cmd> {
    update(m, Msg::Key(KeyEvent::new(code, KeyModifiers::NONE)))
}

/// The Objects screen on the Labels tab with nothing but the objects (and
/// the name index) loaded.
fn on_labels() -> Model {
    let mut m = model_with_objects();
    m.screen = Screen::Objects;
    m
}

fn select_label(m: &mut Model, href: &str) {
    m.objects.labels.select_where(|r| r.href == href);
    assert_eq!(m.objects.labels.selected_item().unwrap().href, href);
}

/// The overlay's rows as `section reference [via]` lines.
fn rows(m: &Model) -> Vec<String> {
    let v = m.objects.where_used.as_ref().expect("the overlay is open");
    v.table
        .visible()
        .iter()
        .map(|&i| {
            let r = &v.table.rows()[i];
            format!("{} {}", r.kind.as_str(), r.line())
        })
        .collect()
}

fn rule_sets() -> Vec<pce::models::RuleSet> {
    serde_json::from_str(include_str!("../../../../../fixtures/rule_sets.json")).unwrap()
}

fn rule_sets_loaded(scope: PolicyScope) -> Msg {
    Msg::RuleSetsLoaded {
        scope,
        result: Ok((rule_sets(), 0, false)),
    }
}

#[test]
fn w_on_a_label_with_everything_loaded_asks_for_nothing_and_leaves_u_alone() {
    let mut m = with_pairing_profiles(with_workloads(with_policy(on_labels())));
    select_label(&mut m, PROD);
    let cmds = press(&mut m, KeyCode::Char('W'));
    assert_eq!(cmds, Vec::<Cmd>::new());
    assert_eq!(m.overlay, Overlay::WhereUsed);
    assert_eq!(
        rows(&m),
        [
            "scope Shop-Prod",
            "rule Global-Infra · #1 · consumer · allow · pending create disabled · via All prod envs",
            "label_group All prod envs",
            "workloads 2 workloads",
        ]
    );
    let v = m.objects.where_used.as_ref().unwrap();
    assert_eq!(v.total(), 3);
    assert_eq!(v.requests, 0);
    assert_eq!(
        v.sections(),
        "Rulesets 1 · Rules 1 · Label groups 1 · Workloads 2"
    );
    assert!(
        m.status
            .text
            .contains("where used: env=prod · 3 references")
    );
    assert!(!m.objects.unused_only, "u is not touched");
    assert!(!m.objects.pce_usage_loading);
}

#[test]
fn what_is_missing_loads_once_through_the_cache_and_reopening_loads_nothing() {
    let mut m = on_labels();
    select_label(&mut m, PROD);
    let cmds = press(&mut m, KeyCode::Char('W'));
    assert_eq!(
        cmds,
        vec![
            Cmd::LoadRuleSets {
                scope: PolicyScope::Draft,
                mode: CacheMode::Use
            },
            Cmd::LoadPairingProfiles(CacheMode::Use),
            Cmd::CountLabelWorkloads {
                href: PROD.into(),
                mode: CacheMode::Use
            },
        ]
    );
    let v = m.objects.where_used.as_ref().unwrap();
    assert_eq!(v.requests, 3, "the api counter's share, shown in the title");
    assert_eq!(
        v.waiting,
        ["rulesets", "pairing profiles", "workload count"]
    );
    assert!(
        m.status
            .text
            .contains("loading rulesets, pairing profiles, workload count…")
    );
    // Only the group is known before the rulesets land.
    assert_eq!(rows(&m), ["label_group All prod envs"]);
    // A second W while they are on their way sends nothing more.
    assert_eq!(press(&mut m, KeyCode::Esc), Vec::<Cmd>::new());
    assert_eq!(press(&mut m, KeyCode::Char('W')), Vec::<Cmd>::new());
    assert_eq!(
        m.objects.where_used.as_ref().unwrap().requests,
        3,
        "still waiting for all three: the title keeps counting them"
    );

    assert_eq!(
        update(&mut m, rule_sets_loaded(PolicyScope::Draft)),
        Vec::<Cmd>::new()
    );
    assert_eq!(rows(&m).len(), 3);
    let profiles: Vec<pce::models::PairingProfile> = serde_json::from_str(include_str!(
        "../../../../../fixtures/pairing_profiles.json"
    ))
    .unwrap();
    assert_eq!(
        update(&mut m, Msg::PairingProfilesLoaded(Ok(profiles))),
        Vec::<Cmd>::new()
    );
    let cmds = update(
        &mut m,
        Msg::LabelWorkloadsCounted {
            href: PROD.into(),
            result: Ok(WorkloadCount::from_probe(1)),
        },
    );
    assert_eq!(cmds, Vec::<Cmd>::new());
    assert_eq!(rows(&m).last().unwrap(), "workloads ≥1 workloads");
    assert!(m.objects.where_used.as_ref().unwrap().waiting.is_empty());
    assert!(m.status.text.contains("3 references"), "{}", m.status.text);

    // Reopened: everything is in hand, nothing is asked again.
    press(&mut m, KeyCode::Esc);
    assert_eq!(m.overlay, Overlay::None);
    assert!(m.objects.where_used.is_none());
    assert_eq!(press(&mut m, KeyCode::Char('W')), Vec::<Cmd>::new());
    assert_eq!(rows(&m).len(), 4);
    // Another label reuses the rulesets: only its own probe.
    press(&mut m, KeyCode::Esc);
    select_label(&mut m, "/orgs/5/labels/2");
    assert_eq!(
        press(&mut m, KeyCode::Char('W')),
        vec![Cmd::CountLabelWorkloads {
            href: "/orgs/5/labels/2".into(),
            mode: CacheMode::Use
        }]
    );
}

#[test]
fn a_workload_list_on_its_way_is_waited_for_instead_of_a_probe() {
    let mut m = with_pairing_profiles(with_policy(on_labels()));
    m.workloads.loading = true;
    select_label(&mut m, PROD);
    assert_eq!(press(&mut m, KeyCode::Char('W')), Vec::<Cmd>::new());
    assert_eq!(
        m.objects.where_used.as_ref().unwrap().waiting,
        ["workload count"]
    );
    let raw: Vec<pce::models::Workload> =
        serde_json::from_str(include_str!("../../../../../fixtures/workloads.json")).unwrap();
    update(&mut m, Msg::WorkloadsLoaded(Ok((raw, 0, false))));
    assert_eq!(rows(&m).last().unwrap(), "workloads 2 workloads");
}

#[test]
fn a_failed_probe_or_rulesets_load_is_said_and_w_again_asks() {
    let mut m = with_policy(on_labels());
    select_label(&mut m, PROD);
    press(&mut m, KeyCode::Char('W'));
    update(
        &mut m,
        Msg::LabelWorkloadsCounted {
            href: PROD.into(),
            result: Err("HTTP 500".into()),
        },
    );
    let v = m.objects.where_used.as_ref().unwrap();
    assert!(v.failed.as_deref().unwrap().contains("W again asks"));
    press(&mut m, KeyCode::Esc);
    assert_eq!(
        press(&mut m, KeyCode::Char('W')),
        vec![Cmd::CountLabelWorkloads {
            href: PROD.into(),
            mode: CacheMode::Use
        }]
    );

    let mut m = on_labels();
    select_label(&mut m, "/orgs/5/labels/2");
    m.workloads.loading = true;
    press(&mut m, KeyCode::Char('W'));
    update(
        &mut m,
        Msg::RuleSetsLoaded {
            scope: PolicyScope::Draft,
            result: Err("HTTP 503".into()),
        },
    );
    let v = m.objects.where_used.as_ref().unwrap();
    assert!(
        v.failed
            .as_deref()
            .unwrap()
            .contains("rulesets are not loaded")
    );
}

#[test]
fn the_active_policy_on_screen_loads_the_drafts_apart() {
    let mut m = on_labels();
    m.policy.scope = PolicyScope::Active;
    m.workloads.loading = true;
    m.objects.kind = ObjectKind::Services;
    assert_eq!(
        press(&mut m, KeyCode::Char('W')),
        vec![Cmd::LoadRuleSets {
            scope: PolicyScope::Draft,
            mode: CacheMode::Use
        }]
    );
    assert!(m.policy.form_drafts_loading);
    update(&mut m, rule_sets_loaded(PolicyScope::Draft));
    assert!(m.policy.loaded_at.is_none(), "the screen's list untouched");
    // All Services (the first row): the disabled Global-Infra rule.
    assert_eq!(
        rows(&m),
        ["rule Global-Infra · #1 · service · allow · pending create disabled"]
    );
}

#[test]
fn w_on_every_tab_and_used_as_a_command() {
    let mut m = with_policy(on_labels());
    m.objects.kind = ObjectKind::LabelGroups;
    assert_eq!(press(&mut m, KeyCode::Char('W')), Vec::<Cmd>::new());
    assert_eq!(
        rows(&m),
        ["rule Global-Infra · #1 · consumer · allow · pending create disabled"]
    );
    press(&mut m, KeyCode::Esc);

    m.objects.kind = ObjectKind::Services;
    m.objects.services.select_where(|s| s.name == "PostgreSQL");
    let cmds = update(&mut m, Msg::Command(commands::parse("used").unwrap()));
    assert_eq!(cmds, Vec::<Cmd>::new());
    assert_eq!(rows(&m), ["rule Shop-Prod · #2 · service · allow"]);
    press(&mut m, KeyCode::Esc);

    let mut m = model_with_ip_lists();
    on_ip_lists_tab(&mut m);
    m.objects
        .ip_lists
        .table
        .select_where(|r| r.list.name == "Corp");
    assert_eq!(press(&mut m, KeyCode::Char('W')), Vec::<Cmd>::new());
    assert_eq!(
        rows(&m),
        ["rule Shop-Prod · #3 · consumer · allow · pending update"]
    );
    let screen = crate::tui::test_support::render_to_string(&m, 100, 30);
    assert!(screen.contains("where used: IP list Corp"), "{screen}");

    // Off the Objects screen `:used` says where it works.
    let mut m = with_policy(on_labels());
    m.screen = Screen::Policy;
    update(&mut m, Msg::Command(commands::parse("used").unwrap()));
    assert_eq!(m.overlay, Overlay::None);
    assert!(m.status.text.contains("Objects screen"));
}

#[test]
fn nothing_found_says_what_is_not_searched() {
    let mut m = with_workloads(with_policy(on_labels()));
    select_label(&mut m, "/orgs/5/labels/5");
    press(&mut m, KeyCode::Char('W'));
    assert_eq!(rows(&m), ["workloads 0 workloads"]);
    let screen = crate::tui::test_support::render_to_string(&m, 100, 30);
    assert!(
        screen.contains("not searched: virtual services"),
        "{screen}"
    );
    assert!(screen.contains("where used: loc=eu"), "{screen}");
}

#[test]
fn enter_on_a_rule_opens_it_selected_on_policy() {
    let mut m = with_workloads(with_policy(on_labels()));
    select_label(&mut m, "/orgs/5/labels/1");
    press(&mut m, KeyCode::Char('W'));
    // role=web: #2 consumer (rule 1), #3 provider (rule 2).
    press(&mut m, KeyCode::Down);
    let cmds = press(&mut m, KeyCode::Enter);
    assert_eq!(m.screen, Screen::Policy);
    assert_eq!(m.overlay, Overlay::None);
    assert!(m.objects.where_used.is_none());
    assert_eq!(m.policy.rulesets.selected_item().unwrap().name, "Shop-Prod");
    assert_eq!(
        m.policy.rules.selected_item().unwrap().href,
        "/orgs/5/sec_policy/draft/rule_sets/100/sec_rules/2"
    );
    assert_eq!(m.policy.pane, PolicyPane::Rules);
    // The policy is loaded and carries its deny rules inline: no request.
    assert_eq!(cmds, Vec::<Cmd>::new());
}

#[test]
fn enter_on_a_scope_selects_the_ruleset() {
    let mut m = with_workloads(with_policy(on_labels()));
    select_label(&mut m, "/orgs/5/labels/3");
    press(&mut m, KeyCode::Char('W'));
    assert_eq!(rows(&m)[0], "scope Shop-Prod");
    press(&mut m, KeyCode::Enter);
    assert_eq!(m.screen, Screen::Policy);
    assert_eq!(m.policy.pane, PolicyPane::RuleSets);
    assert_eq!(
        m.policy.rulesets.selected_item().unwrap().href,
        "/orgs/5/sec_policy/draft/rule_sets/100"
    );
}

#[test]
fn enter_on_a_group_opens_the_label_groups_tab_on_it() {
    let mut m = with_workloads(with_policy(on_labels()));
    select_label(&mut m, PROD);
    press(&mut m, KeyCode::Char('W'));
    let v = m.objects.where_used.as_mut().unwrap();
    v.table.select_where(|r| r.kind == RefKind::LabelGroup);
    assert_eq!(press(&mut m, KeyCode::Enter), Vec::<Cmd>::new());
    assert_eq!(m.screen, Screen::Objects);
    assert_eq!(m.objects.kind, ObjectKind::LabelGroups);
    assert_eq!(
        m.objects.groups.selected_item().unwrap().name,
        "All prod envs"
    );
}

#[test]
fn enter_on_the_workloads_line_filters_workloads_by_the_label() {
    let mut m = with_workloads(with_policy(on_labels()));
    select_label(&mut m, PROD);
    press(&mut m, KeyCode::Char('W'));
    let v = m.objects.where_used.as_mut().unwrap();
    v.table.select_where(|r| r.kind == RefKind::Workloads);
    assert_eq!(press(&mut m, KeyCode::Enter), Vec::<Cmd>::new());
    assert_eq!(m.screen, Screen::Workloads);
    assert_eq!(m.workloads.labels.to_string(), "env=prod");
    assert_eq!(m.workloads.table.len(), 2);
}

#[test]
fn slash_searches_within_the_overlay() {
    let mut m = with_workloads(with_policy(on_labels()));
    select_label(&mut m, PROD);
    press(&mut m, KeyCode::Char('W'));
    press(&mut m, KeyCode::Char('/'));
    for c in "infra".chars() {
        press(&mut m, KeyCode::Char(c));
    }
    press(&mut m, KeyCode::Enter);
    assert_eq!(m.overlay, Overlay::WhereUsed, "Enter ends the search only");
    assert_eq!(rows(&m).len(), 1);
    assert!(rows(&m)[0].starts_with("rule Global-Infra"));
    // q types into a search; out of it, q closes.
    press(&mut m, KeyCode::Char('q'));
    assert_eq!(m.overlay, Overlay::None);
}

#[test]
fn a_write_or_the_workload_list_drops_the_kept_probes() {
    let mut m = with_policy(on_labels());
    m.objects
        .workload_probes
        .insert(PROD.into(), WorkloadCount::from_probe(1));
    let raw: Vec<pce::models::Workload> =
        serde_json::from_str(include_str!("../../../../../fixtures/workloads.json")).unwrap();
    update(&mut m, Msg::WorkloadsLoaded(Ok((raw, 0, false))));
    assert!(m.objects.workload_probes.is_empty());
}

#[test]
fn w_before_the_name_index_waits() {
    let mut m = on_labels();
    m.connection = crate::tui::model::Connection::Connected {
        index: None,
        index_age_secs: 0,
    };
    assert_eq!(press(&mut m, KeyCode::Char('W')), Vec::<Cmd>::new());
    assert_eq!(m.overlay, Overlay::None);
    assert!(m.status.text.contains("still loading"));
}

#[test]
fn enter_reaches_a_rule_and_a_ruleset_hidden_by_a_policy_table_search() {
    let mut m = with_workloads(with_policy(on_labels()));
    // `/Global` on the rulesets pane, `/deny` on the rules pane.
    m.policy.rulesets.start_search();
    for c in "Global".chars() {
        m.policy.rulesets.search_push(c);
    }
    m.policy.rulesets.search_commit();
    assert_eq!(m.policy.rulesets.visible().len(), 1);
    m.policy.rules.start_search();
    for c in "deny".chars() {
        m.policy.rules.search_push(c);
    }
    m.policy.rules.search_commit();
    select_label(&mut m, "/orgs/5/labels/1");
    press(&mut m, KeyCode::Char('W'));
    press(&mut m, KeyCode::Down);
    assert_eq!(press(&mut m, KeyCode::Enter), Vec::<Cmd>::new());
    assert_eq!(m.screen, Screen::Policy);
    assert_eq!(m.policy.rulesets.selected_item().unwrap().name, "Shop-Prod");
    assert_eq!(
        m.policy.rules.selected_item().unwrap().href,
        "/orgs/5/sec_policy/draft/rule_sets/100/sec_rules/2"
    );
    assert_eq!(m.policy.pane, PolicyPane::Rules);
    assert!(!m.status.text.contains("reload"), "{}", m.status.text);

    // A scope row's ruleset behind a search.
    let mut m = with_workloads(with_policy(on_labels()));
    m.policy.rulesets.start_search();
    for c in "Global".chars() {
        m.policy.rulesets.search_push(c);
    }
    m.policy.rulesets.search_commit();
    select_label(&mut m, "/orgs/5/labels/3");
    press(&mut m, KeyCode::Char('W'));
    press(&mut m, KeyCode::Enter);
    assert_eq!(
        m.policy.rulesets.selected_item().unwrap().href,
        "/orgs/5/sec_policy/draft/rule_sets/100"
    );
    assert_eq!(m.status.text, "Shop-Prod");
}

#[test]
fn enter_while_the_active_policy_loads_keeps_the_overlay() {
    let mut m = with_workloads(with_policy(on_labels()));
    select_label(&mut m, "/orgs/5/labels/1");
    press(&mut m, KeyCode::Char('W'));
    m.policy.scope = PolicyScope::Active;
    m.policy.loading = true;
    assert_eq!(press(&mut m, KeyCode::Enter), Vec::<Cmd>::new());
    assert_eq!(
        m.overlay,
        Overlay::WhereUsed,
        "still open: Enter again works"
    );
    assert_eq!(m.screen, Screen::Objects);
    assert!(m.status.text.contains("Enter again"));
}

#[test]
fn enter_on_the_workloads_line_before_the_list_says_it_is_loading() {
    let mut m = with_policy(on_labels());
    m.objects
        .workload_probes
        .insert(PROD.into(), WorkloadCount::from_probe(1));
    select_label(&mut m, PROD);
    press(&mut m, KeyCode::Char('W'));
    let v = m.objects.where_used.as_mut().unwrap();
    v.table.select_where(|r| r.kind == RefKind::Workloads);
    let cmds = press(&mut m, KeyCode::Enter);
    assert_eq!(cmds, vec![Cmd::LoadWorkloads(CacheMode::Use)]);
    assert_eq!(m.screen, Screen::Workloads);
    assert_eq!(m.status.text, "labels env=prod · loading workloads…");
}

#[test]
fn the_footer_shows_the_overlay_keys_and_not_searched_stays_whole() {
    let mut m = with_pairing_profiles(with_workloads(with_policy(on_labels())));
    select_label(&mut m, "/orgs/5/labels/5");
    press(&mut m, KeyCode::Char('W'));
    let screen = crate::tui::test_support::render_to_string(&m, 80, 24);
    assert!(
        screen.contains("Enter open  / search  Esc back"),
        "{screen}"
    );
    assert!(!screen.contains("Tab type"), "{screen}");
    assert!(
        screen.contains("not searched: virtual services, boundaries, container profiles"),
        "{screen}"
    );
    // With the deny note too, both show.
    m.objects.where_used.as_mut().unwrap().deny_unread = 2;
    let screen = crate::tui::test_support::render_to_string(&m, 120, 40);
    assert!(screen.contains("container workload profiles"), "{screen}");
    assert!(
        screen.contains("2 rulesets may hold deny rules"),
        "{screen}"
    );
}

#[test]
fn w_on_a_label_reads_the_pairing_profiles_once_and_lists_them_with_locked() {
    // role=web: given by shop-prod, which locks role and is disabled.
    let mut m = with_workloads(with_policy(on_labels()));
    select_label(&mut m, "/orgs/5/labels/1");
    assert_eq!(
        press(&mut m, KeyCode::Char('W')),
        vec![Cmd::LoadPairingProfiles(CacheMode::Use)]
    );
    let v = m.objects.where_used.as_ref().unwrap();
    assert_eq!(v.requests, 1);
    assert_eq!(v.waiting, ["pairing profiles"]);
    // A second W, or a reopen, while it is on its way: nothing more.
    assert_eq!(press(&mut m, KeyCode::Char('W')), Vec::<Cmd>::new());
    press(&mut m, KeyCode::Esc);
    assert_eq!(press(&mut m, KeyCode::Char('W')), Vec::<Cmd>::new());
    assert_eq!(m.objects.where_used.as_ref().unwrap().requests, 1);
    let profiles: Vec<pce::models::PairingProfile> = serde_json::from_str(include_str!(
        "../../../../../fixtures/pairing_profiles.json"
    ))
    .unwrap();
    update(&mut m, Msg::PairingProfilesLoaded(Ok(profiles)));
    assert_eq!(
        rows(&m),
        [
            "rule Shop-Prod · #2 · consumer · allow",
            "rule Shop-Prod · #3 · provider · allow · pending update",
            "pairing_profile shop-prod · locked · disabled",
            "workloads 1 workload",
        ]
    );
    let v = m.objects.where_used.as_ref().unwrap();
    assert_eq!(v.total(), 3);
    assert!(
        v.sections().contains("Pairing profiles 1"),
        "{}",
        v.sections()
    );
    assert!(m.status.text.contains("3 references"), "{}", m.status.text);
    let screen = crate::tui::test_support::render_to_string(&m, 80, 24);
    assert!(
        screen.contains("pairing profile shop-prod · locked · disabled"),
        "{screen}"
    );

    // Reopened, and on another label: held, never read again.
    press(&mut m, KeyCode::Esc);
    assert_eq!(press(&mut m, KeyCode::Char('W')), Vec::<Cmd>::new());
    press(&mut m, KeyCode::Esc);
    select_label(&mut m, "/orgs/5/labels/3");
    assert_eq!(press(&mut m, KeyCode::Char('W')), Vec::<Cmd>::new());
    assert!(rows(&m).contains(&"pairing_profile shop-prod · locked · disabled".to_string()));

    // Other kinds never read them.
    let mut m = with_policy(on_labels());
    let lists: Vec<pce::models::IpList> =
        serde_json::from_str(include_str!("../../../../../fixtures/ip_lists.json")).unwrap();
    m.objects.ip_lists.set_lists(lists, &m.policy);
    m.objects.ip_lists.loaded_at = Some(std::time::Instant::now());
    for kind in [
        ObjectKind::LabelGroups,
        ObjectKind::Services,
        ObjectKind::IpLists,
    ] {
        m.objects.kind = kind;
        assert_eq!(
            press(&mut m, KeyCode::Char('W')),
            Vec::<Cmd>::new(),
            "{kind:?}"
        );
        assert_eq!(
            m.overlay,
            Overlay::WhereUsed,
            "{kind:?}: the overlay opened"
        );
        press(&mut m, KeyCode::Esc);
    }
    assert!(!m.objects.pairing_loading && m.objects.pairing_profiles.is_none());
}

#[test]
fn a_failed_pairing_profiles_read_is_said_and_w_again_asks() {
    let mut m = with_workloads(with_policy(on_labels()));
    select_label(&mut m, PROD);
    press(&mut m, KeyCode::Char('W'));
    update(&mut m, Msg::PairingProfilesLoaded(Err("HTTP 403".into())));
    let v = m.objects.where_used.as_ref().unwrap();
    assert_eq!(
        v.failed.as_deref(),
        Some("pairing profiles: HTTP 403 (W again asks)"),
        "the cause is shown"
    );
    let screen = crate::tui::test_support::render_to_string(&m, 100, 30);
    assert!(screen.contains("pairing profiles: HTTP 403"), "{screen}");
    press(&mut m, KeyCode::Esc);
    assert_eq!(
        press(&mut m, KeyCode::Char('W')),
        vec![Cmd::LoadPairingProfiles(CacheMode::Use)]
    );
    assert!(m.objects.pairing_error.is_none(), "asked again");

    // Both the profiles and the probe failing: both are said.
    let mut m = with_policy(on_labels());
    select_label(&mut m, PROD);
    press(&mut m, KeyCode::Char('W'));
    update(&mut m, Msg::PairingProfilesLoaded(Err("HTTP 403".into())));
    update(
        &mut m,
        Msg::LabelWorkloadsCounted {
            href: PROD.into(),
            result: Err("HTTP 500".into()),
        },
    );
    assert_eq!(
        m.objects.where_used.as_ref().unwrap().failed.as_deref(),
        Some(
            "pairing profiles: HTTP 403 (W again asks) · workload count not loaded (W again asks)"
        )
    );
}

#[test]
fn enter_on_a_pairing_profile_names_the_command_and_keeps_the_overlay() {
    let mut m = with_pairing_profiles(with_workloads(with_policy(on_labels())));
    select_label(&mut m, "/orgs/5/labels/1");
    press(&mut m, KeyCode::Char('W'));
    let v = m.objects.where_used.as_mut().unwrap();
    v.table.select_where(|r| r.kind == RefKind::PairingProfile);
    assert_eq!(press(&mut m, KeyCode::Enter), Vec::<Cmd>::new());
    assert_eq!(m.overlay, Overlay::WhereUsed);
    assert_eq!(m.screen, Screen::Objects);
    assert!(m.objects.where_used.is_some());
    assert_eq!(
        m.status.text,
        "pairing profile shop-prod: illumio-tui pairing-profiles list"
    );
}
