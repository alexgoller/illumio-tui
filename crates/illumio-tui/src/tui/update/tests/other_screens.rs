use super::*;

// ---- Policy screen (Task 10) ----
#[test]
fn opening_policy_loads_rulesets_once_and_d_toggles_scope() {
    use pce::models::PolicyScope;
    let mut m = model_fixture();
    let cmds = update(&mut m, key('2'));
    assert_eq!(
        cmds,
        vec![Cmd::LoadRuleSets {
            scope: PolicyScope::Draft,
            mode: pce::CacheMode::Use
        }]
    );
    assert!(m.policy.loading);
    let rs: Vec<pce::models::RuleSet> =
        serde_json::from_str(include_str!("../../../../../../fixtures/rule_sets.json")).unwrap();
    update(
        &mut m,
        Msg::RuleSetsLoaded {
            scope: PolicyScope::Draft,
            result: Ok((rs, 0, false)),
        },
    );
    assert!(!m.policy.loading);
    assert_eq!(m.policy.rulesets.len(), 2);
    assert_eq!(
        m.policy.rules.len(),
        4,
        "allow + inline deny rules of the first ruleset"
    );
    let kinds: Vec<_> = m.policy.rules.rows().iter().map(|r| r.kind).collect();
    assert_eq!(
        kinds,
        vec![
            pce::RuleKind::OverrideDeny,
            pce::RuleKind::Allow,
            pce::RuleKind::Allow,
            pce::RuleKind::Deny
        ],
        "evaluation order: override deny, allow, deny"
    );
    assert!(update(&mut m, key('1')).is_empty());
    assert!(
        update(&mut m, key('2')).is_empty(),
        "already loaded: no request"
    );
    let cmds = update(&mut m, key('d'));
    assert_eq!(
        cmds,
        vec![Cmd::LoadRuleSets {
            scope: PolicyScope::Active,
            mode: pce::CacheMode::Use
        }]
    );
    assert_eq!(m.policy.scope, PolicyScope::Active);
    update(
        &mut m,
        Msg::RuleSetsLoaded {
            scope: PolicyScope::Active,
            result: Ok((Vec::new(), 0, false)),
        },
    );
    let cmds = update(&mut m, key('r'));
    assert_eq!(
        cmds,
        vec![Cmd::LoadRuleSets {
            scope: PolicyScope::Active,
            mode: pce::CacheMode::Bypass
        }]
    );
}

#[test]
fn selecting_a_ruleset_rebuilds_rules_and_enter_opens_detail() {
    use crate::tui::model::Overlay;
    use crate::tui::test_support::model_with_policy;
    let mut m = model_with_policy();
    update(&mut m, key('2'));
    assert_eq!(m.policy.rules.len(), 4);
    update(&mut m, kc(KeyCode::Down));
    assert_eq!(m.policy.rules.len(), 1, "second ruleset has one rule");
    update(&mut m, kc(KeyCode::Tab));
    update(&mut m, kc(KeyCode::Enter));
    match &m.overlay {
        Overlay::RuleDetail(r) => assert_eq!(r.providers[0].name, "All workloads"),
        other => panic!("{other:?}"),
    }
    update(&mut m, kc(KeyCode::Esc));
    assert!(matches!(m.overlay, Overlay::None));
}

#[test]
fn p_loads_pending_and_shows_the_panel() {
    use crate::tui::model::Overlay;
    use crate::tui::test_support::model_with_policy;
    let mut m = model_with_policy();
    update(&mut m, key('2'));
    let cmds = update(&mut m, key('p'));
    assert_eq!(cmds, vec![Cmd::LoadPending(pce::CacheMode::Use)]);
    let pending = pce::models::parse_pending(
        serde_json::from_str(include_str!("../../../../../../fixtures/pending_list.json")).unwrap(),
    );
    update(&mut m, Msg::PendingLoaded(Ok(pending)));
    assert!(matches!(m.overlay, Overlay::Pending));
    assert_eq!(m.policy.pending.as_ref().unwrap().len(), 2);
}

#[test]
fn empty_inline_deny_rules_fall_back_to_the_endpoint() {
    use pce::models::PolicyScope;
    let mut m = model_fixture();
    update(&mut m, key('2'));
    let mut rs: Vec<pce::models::RuleSet> =
        serde_json::from_str(include_str!("../../../../../../fixtures/rule_sets.json")).unwrap();
    rs[0].deny_rules.clear();
    let cmds = update(
        &mut m,
        Msg::RuleSetsLoaded {
            scope: PolicyScope::Draft,
            result: Ok((rs, 0, false)),
        },
    );
    assert_eq!(
        cmds,
        vec![Cmd::LoadDenyRules {
            href: "/orgs/5/sec_policy/draft/rule_sets/100".into(),
            mode: pce::CacheMode::Use
        }]
    );
    let deny: Vec<pce::models::DenyRule> =
        serde_json::from_str(include_str!("../../../../../../fixtures/deny_rules.json")).unwrap();
    update(
        &mut m,
        Msg::DenyRulesLoaded {
            href: "/orgs/5/sec_policy/draft/rule_sets/100".into(),
            result: Ok(deny),
        },
    );
    assert_eq!(m.policy.rules.len(), 4);
}

#[test]
fn fallback_is_requested_once_per_ruleset_even_when_revisited() {
    use crate::tui::test_support::model_with_policy;
    let mut m = model_with_policy();
    update(&mut m, key('2'));
    // the second ruleset has no inline deny rules: first visit requests, revisit does not
    let first = update(&mut m, kc(KeyCode::Down));
    assert_eq!(
        first,
        vec![Cmd::LoadDenyRules {
            href: "/orgs/5/sec_policy/draft/rule_sets/101".into(),
            mode: pce::CacheMode::Use
        }]
    );
    update(&mut m, kc(KeyCode::Up));
    assert!(update(&mut m, kc(KeyCode::Down)).is_empty());
    // a "not found" answer is remembered as empty
    update(
        &mut m,
        Msg::DenyRulesLoaded {
            href: "/orgs/5/sec_policy/draft/rule_sets/101".into(),
            result: Err("not found".into()),
        },
    );
    update(&mut m, kc(KeyCode::Up));
    assert!(update(&mut m, kc(KeyCode::Down)).is_empty());
}

#[test]
fn result_for_the_other_scope_is_ignored_and_keeps_loading() {
    use pce::models::PolicyScope;
    let mut m = model_fixture();
    update(&mut m, key('2'));
    let rs: Vec<pce::models::RuleSet> =
        serde_json::from_str(include_str!("../../../../../../fixtures/rule_sets.json")).unwrap();
    update(
        &mut m,
        Msg::RuleSetsLoaded {
            scope: PolicyScope::Draft,
            result: Ok((rs.clone(), 0, false)),
        },
    );
    update(&mut m, key('d')); // Active now loading
    // A late draft answer (e.g. a duplicate) must not land in the active scope.
    update(
        &mut m,
        Msg::RuleSetsLoaded {
            scope: PolicyScope::Draft,
            result: Ok((rs, 0, false)),
        },
    );
    assert!(m.policy.loading, "the active-scope load is still in flight");
    assert!(m.policy.rulesets.is_empty());
}

#[test]
fn enter_without_a_rule_does_nothing_and_search_captures_keys() {
    use crate::tui::model::Overlay;
    use crate::tui::test_support::model_with_policy;
    let mut m = model_with_policy();
    update(&mut m, key('2'));
    update(&mut m, key('/'));
    for c in "qd2".chars() {
        assert!(update(&mut m, key(c)).is_empty());
    }
    assert!(!m.quit, "q is typed into the search");
    assert_eq!(m.screen, Screen::Policy);
    assert_eq!(m.policy.scope, pce::models::PolicyScope::Draft);
    assert!(m.policy.rulesets.is_empty(), "nothing matches qd2");
    assert!(m.policy.rules.is_empty());
    update(&mut m, kc(KeyCode::Esc));
    assert_eq!(m.policy.rulesets.len(), 2);
    update(&mut m, kc(KeyCode::Tab));
    m.policy.rules.set_rows(Vec::new());
    update(&mut m, kc(KeyCode::Enter));
    assert!(matches!(m.overlay, Overlay::None));
}

#[test]
fn policy_without_an_index_requests_nothing_until_connected() {
    let mut m = model_fixture();
    m.connection = Connection::Connecting;
    assert!(update(&mut m, key('2')).is_empty());
    assert!(!m.policy.loading);
}

#[test]
fn policy_auto_refresh_obeys_the_guards() {
    use crate::tui::model::Overlay;
    use crate::tui::test_support::model_with_policy;
    use std::time::Duration;
    let mut m = model_with_policy();
    update(&mut m, key('2'));
    let now = Instant::now() + Duration::from_secs(120);
    m.policy.auto.last_run = Some(Instant::now());
    assert!(update(&mut m, Msg::Tick(now)).is_empty(), "opt-in only");
    update(&mut m, key('a'));
    assert!(m.policy.auto.enabled);
    let early = Instant::now() + Duration::from_secs(10);
    assert!(
        update(&mut m, Msg::Tick(early)).is_empty(),
        "minimum interval"
    );
    m.focused = false;
    assert!(update(&mut m, Msg::Tick(now)).is_empty(), "not unfocused");
    m.focused = true;
    m.overlay = Overlay::Help;
    assert!(
        update(&mut m, Msg::Tick(now)).is_empty(),
        "not under an overlay"
    );
    m.overlay = Overlay::None;
    m.policy.loading = true;
    assert!(
        update(&mut m, Msg::Tick(now)).is_empty(),
        "not while loading"
    );
    m.policy.loading = false;
    assert_eq!(
        update(&mut m, Msg::Tick(now)),
        vec![Cmd::LoadRuleSets {
            scope: pce::models::PolicyScope::Draft,
            mode: pce::CacheMode::Bypass
        }]
    );
    m.screen = Screen::Traffic;
    m.policy.loading = false;
    m.policy.auto.last_run = Some(Instant::now());
    assert!(
        update(&mut m, Msg::Tick(now)).is_empty(),
        "only on its own screen"
    );
}

fn objects_fixture_msgs() -> Vec<Msg> {
    use crate::tui::model::ObjectKind;
    use crate::tui::msg::ObjectsPayload;
    let labels =
        serde_json::from_str(include_str!("../../../../../../fixtures/labels.json")).unwrap();
    let groups =
        serde_json::from_str(include_str!("../../../../../../fixtures/label_groups.json")).unwrap();
    let services =
        serde_json::from_str(include_str!("../../../../../../fixtures/services.json")).unwrap();
    vec![
        Msg::ObjectsLoaded {
            kind: ObjectKind::Labels,
            result: Ok(ObjectsPayload::Labels(labels, 120)),
        },
        Msg::ObjectsLoaded {
            kind: ObjectKind::LabelGroups,
            result: Ok(ObjectsPayload::LabelGroups(groups, 120)),
        },
        Msg::ObjectsLoaded {
            kind: ObjectKind::Services,
            result: Ok(ObjectsPayload::Services(services, 120)),
        },
    ]
}

#[test]
fn opening_objects_loads_all_three_once_and_tab_cycles() {
    use crate::tui::model::ObjectKind;
    use crate::tui::msg::ObjectsPayload;
    let mut m = model_fixture();
    let cmds = update(&mut m, key('3'));
    assert_eq!(cmds.len(), 3);
    assert!(cmds.iter().all(|c| matches!(
        c,
        Cmd::LoadObjects {
            mode: pce::CacheMode::Use,
            ..
        }
    )));
    let labels: Vec<pce::models::Label> =
        serde_json::from_str(include_str!("../../../../../../fixtures/labels.json")).unwrap();
    update(
        &mut m,
        Msg::ObjectsLoaded {
            kind: ObjectKind::Labels,
            result: Ok(ObjectsPayload::Labels(labels, 120)),
        },
    );
    assert_eq!(m.objects.labels.len(), 6);
    assert_eq!(m.objects.ages[0], Some(120));
    assert!(update(&mut m, key('1')).is_empty());
    assert!(
        update(&mut m, key('3')).is_empty(),
        "no reload while the other kinds are still loading"
    );
    update(&mut m, kc(KeyCode::Tab));
    assert_eq!(m.objects.kind, ObjectKind::LabelGroups);
    update(&mut m, kc(KeyCode::BackTab));
    assert_eq!(m.objects.kind, ObjectKind::Labels);
    let cmds = update(&mut m, key('r'));
    assert_eq!(
        cmds,
        vec![Cmd::LoadObjects {
            kind: ObjectKind::Labels,
            mode: pce::CacheMode::Bypass
        }]
    );
    assert!(
        update(&mut m, key('r')).is_empty(),
        "a second r while the kind is loading adds no request"
    );
}

#[test]
fn objects_load_only_what_is_missing_and_never_before_the_index() {
    use crate::tui::model::ObjectKind;
    let mut m = model_fixture();
    let Connection::Connected { index, .. } = &mut m.connection else {
        unreachable!()
    };
    let idx = index.take();
    assert!(
        update(&mut m, key('3')).is_empty(),
        "the startup load is still running: no duplicate requests"
    );
    assert!(m.objects.loading.iter().all(|l| !l));
    update(
        &mut m,
        Msg::IndexLoaded {
            index: idx.unwrap(),
            age_secs: 0,
        },
    );
    assert!(m.objects.loading.iter().all(|l| *l));
    // a failed kind is retried on re-entry, loaded kinds are not
    for msg in objects_fixture_msgs().into_iter().take(2) {
        update(&mut m, msg);
    }
    update(
        &mut m,
        Msg::ObjectsLoaded {
            kind: ObjectKind::Services,
            result: Err("boom".into()),
        },
    );
    assert_eq!(m.status.kind, StatusKind::Error);
    assert!(m.objects.ages[2].is_none());
    update(&mut m, key('1'));
    assert_eq!(
        update(&mut m, key('3')),
        vec![Cmd::LoadObjects {
            kind: ObjectKind::Services,
            mode: pce::CacheMode::Use
        }]
    );
}

#[test]
fn a_late_result_for_another_kind_still_fills_its_table() {
    use crate::tui::model::ObjectKind;
    let mut m = model_fixture();
    update(&mut m, key('3'));
    update(&mut m, kc(KeyCode::Tab)); // LabelGroups is current
    let msgs = objects_fixture_msgs();
    update(&mut m, msgs.into_iter().nth(2).unwrap()); // Services arrives
    assert_eq!(m.objects.kind, ObjectKind::LabelGroups);
    assert_eq!(m.objects.services.len(), 3);
    assert_eq!(m.objects.ages[2], Some(120));
    assert!(!m.objects.loading[2]);
    assert!(m.objects.loading[1]);
}

#[test]
fn objects_search_and_detail_are_local() {
    use crate::tui::test_support::model_with_objects;
    let mut m = model_with_objects();
    assert!(update(&mut m, key('3')).is_empty());
    assert!(update(&mut m, key('/')).is_empty());
    for c in "role".chars() {
        assert!(update(&mut m, key(c)).is_empty());
    }
    assert_eq!(m.objects.labels.len(), 2);
    assert!(update(&mut m, kc(KeyCode::Enter)).is_empty());
    assert!(update(&mut m, kc(KeyCode::Enter)).is_empty());
    assert!(m.objects.show_detail);
}

#[test]
fn objects_search_captures_keys_including_tab() {
    use crate::tui::model::ObjectKind;
    use crate::tui::test_support::model_with_objects;
    let mut m = model_with_objects();
    update(&mut m, key('3'));
    update(&mut m, key('/'));
    for c in "qr3".chars() {
        assert!(update(&mut m, key(c)).is_empty());
    }
    update(&mut m, kc(KeyCode::Tab));
    assert!(!m.quit);
    assert_eq!(m.screen, Screen::Objects);
    assert_eq!(
        m.objects.kind,
        ObjectKind::Labels,
        "Tab does not switch while searching"
    );
    assert!(m.objects.labels.is_empty(), "nothing matches qr3");
    update(&mut m, kc(KeyCode::Esc));
    assert_eq!(m.objects.labels.len(), 6);
}

#[test]
fn enter_toggles_detail_only_with_a_selection_and_tab_closes_it() {
    use crate::tui::test_support::model_with_objects;
    let mut m = model_with_objects();
    update(&mut m, key('3'));
    m.objects.labels.set_rows(Vec::new());
    update(&mut m, kc(KeyCode::Enter));
    assert!(!m.objects.show_detail, "nothing selected");
    let mut m = model_with_objects();
    update(&mut m, key('3'));
    update(&mut m, kc(KeyCode::Enter));
    assert!(m.objects.show_detail);
    update(&mut m, kc(KeyCode::Tab));
    assert!(!m.objects.show_detail);
    update(&mut m, kc(KeyCode::Enter));
    assert!(m.objects.show_detail);
    update(&mut m, kc(KeyCode::Enter));
    assert!(!m.objects.show_detail);
}

#[test]
fn objects_auto_refresh_obeys_the_guards_and_reloads_only_the_current_kind() {
    use crate::tui::model::{ObjectKind, Overlay};
    use crate::tui::test_support::model_with_objects;
    use std::time::Duration;
    let mut m = model_with_objects();
    update(&mut m, key('3'));
    update(&mut m, kc(KeyCode::Tab)); // LabelGroups
    let now = Instant::now() + Duration::from_secs(120);
    m.objects.auto.last_run = Some(Instant::now());
    assert!(update(&mut m, Msg::Tick(now)).is_empty(), "opt-in only");
    update(&mut m, key('a'));
    assert!(m.objects.auto.enabled);
    let early = Instant::now() + Duration::from_secs(10);
    assert!(
        update(&mut m, Msg::Tick(early)).is_empty(),
        "minimum interval"
    );
    m.focused = false;
    assert!(update(&mut m, Msg::Tick(now)).is_empty(), "not unfocused");
    m.focused = true;
    m.overlay = Overlay::Help;
    assert!(
        update(&mut m, Msg::Tick(now)).is_empty(),
        "not under an overlay"
    );
    m.overlay = Overlay::None;
    m.objects.loading[1] = true;
    assert!(
        update(&mut m, Msg::Tick(now)).is_empty(),
        "not while that kind loads"
    );
    m.objects.loading[1] = false;
    assert_eq!(
        update(&mut m, Msg::Tick(now)),
        vec![Cmd::LoadObjects {
            kind: ObjectKind::LabelGroups,
            mode: pce::CacheMode::Bypass
        }],
        "current kind only, bypassing the cache (opted in)"
    );
    m.screen = Screen::Traffic;
    m.objects.loading[1] = false;
    m.objects.auto.last_run = Some(Instant::now());
    assert!(
        update(&mut m, Msg::Tick(now)).is_empty(),
        "only on its own screen"
    );
}

#[test]
fn explicit_r_on_policy_bypasses_the_deny_fallback_but_auto_refresh_does_not() {
    use crate::tui::test_support::model_with_policy;
    use pce::models::PolicyScope;
    use std::time::Duration;
    let href = "/orgs/5/sec_policy/draft/rule_sets/100";
    let reload = |m: &mut Model| {
        let mut rs: Vec<pce::models::RuleSet> =
            serde_json::from_str(include_str!("../../../../../../fixtures/rule_sets.json"))
                .unwrap();
        rs[0].deny_rules.clear();
        update(
            m,
            Msg::RuleSetsLoaded {
                scope: PolicyScope::Draft,
                result: Ok((rs, 0, false)),
            },
        )
    };
    let mut m = model_with_policy();
    update(&mut m, key('2'));
    update(&mut m, key('r'));
    assert_eq!(
        reload(&mut m),
        vec![Cmd::LoadDenyRules {
            href: href.into(),
            mode: pce::CacheMode::Bypass
        }]
    );
    // the bypass applies to that one reload only
    assert_eq!(
        reload(&mut m),
        vec![Cmd::LoadDenyRules {
            href: href.into(),
            mode: pce::CacheMode::Use
        }]
    );
    update(&mut m, key('a'));
    let now = Instant::now() + Duration::from_secs(120);
    m.policy.auto.last_run = Some(Instant::now());
    assert!(matches!(
        update(&mut m, Msg::Tick(now)).as_slice(),
        [Cmd::LoadRuleSets {
            mode: pce::CacheMode::Bypass,
            ..
        }]
    ));
    assert_eq!(
        reload(&mut m),
        vec![Cmd::LoadDenyRules {
            href: href.into(),
            mode: pce::CacheMode::Use
        }]
    );
}

#[test]
fn policy_reload_keeps_fetched_deny_rows_until_the_new_ones_arrive() {
    use pce::models::PolicyScope;
    let href = "/orgs/5/sec_policy/draft/rule_sets/100";
    let load = |m: &mut Model| {
        let mut rs: Vec<pce::models::RuleSet> =
            serde_json::from_str(include_str!("../../../../../../fixtures/rule_sets.json"))
                .unwrap();
        rs[0].deny_rules.clear();
        update(
            m,
            Msg::RuleSetsLoaded {
                scope: PolicyScope::Draft,
                result: Ok((rs, 0, false)),
            },
        )
    };
    let mut m = model_fixture();
    update(&mut m, key('2'));
    load(&mut m);
    let deny: Vec<pce::models::DenyRule> =
        serde_json::from_str(include_str!("../../../../../../fixtures/deny_rules.json")).unwrap();
    update(
        &mut m,
        Msg::DenyRulesLoaded {
            href: href.into(),
            result: Ok(deny),
        },
    );
    assert_eq!(m.policy.rules.len(), 4);
    let cmds = load(&mut m);
    assert_eq!(cmds.len(), 1, "the fallback is asked again after a reload");
    assert_eq!(m.policy.rules.len(), 4, "no flicker while it is in flight");
    // a scope switch still forgets everything
    update(&mut m, key('d'));
    assert!(m.policy.rules.is_empty());
}

#[test]
fn objects_recover_from_a_failed_index_with_r() {
    use crate::tui::model::ObjectKind;
    let mut m = model_fixture();
    let Connection::Connected { index, .. } = &mut m.connection else {
        unreachable!()
    };
    let idx = index.take().unwrap();
    update(&mut m, Msg::IndexFailed("boom".into()));
    update(&mut m, key('1'));
    assert!(update(&mut m, Msg::SwitchScreen(Screen::Objects)).is_empty());
    assert_eq!(
        update(&mut m, key('r')),
        vec![Cmd::LoadIndex(pce::CacheMode::Use)]
    );
    assert_eq!(m.status.text, "loading name index…");
    assert!(
        update(&mut m, key('r')).is_empty(),
        "no second index request while it is in flight"
    );
    let cmds = update(
        &mut m,
        Msg::IndexLoaded {
            index: idx,
            age_secs: 0,
        },
    );
    assert_eq!(cmds.len(), 3);
    assert!(cmds.contains(&Cmd::LoadObjects {
        kind: ObjectKind::Labels,
        mode: pce::CacheMode::Use
    }));
}

#[test]
fn a_failed_explicit_policy_refresh_does_not_leak_its_bypass() {
    use crate::tui::test_support::model_with_policy;
    use pce::models::PolicyScope;
    use std::time::Duration;
    let mut m = model_with_policy();
    update(&mut m, key('2'));
    update(&mut m, key('r'));
    update(
        &mut m,
        Msg::RuleSetsLoaded {
            scope: PolicyScope::Draft,
            result: Err("boom".into()),
        },
    );
    update(&mut m, key('a'));
    m.policy.auto.last_run = Some(Instant::now());
    let now = Instant::now() + Duration::from_secs(120);
    assert!(matches!(
        update(&mut m, Msg::Tick(now)).as_slice(),
        [Cmd::LoadRuleSets { .. }]
    ));
    let mut rs: Vec<pce::models::RuleSet> =
        serde_json::from_str(include_str!("../../../../../../fixtures/rule_sets.json")).unwrap();
    rs[0].deny_rules.clear();
    let cmds = update(
        &mut m,
        Msg::RuleSetsLoaded {
            scope: PolicyScope::Draft,
            result: Ok((rs, 0, false)),
        },
    );
    assert_eq!(
        cmds,
        vec![Cmd::LoadDenyRules {
            href: "/orgs/5/sec_policy/draft/rule_sets/100".into(),
            mode: pce::CacheMode::Use
        }]
    );
}

#[test]
fn opening_workloads_loads_once_and_t_jumps_to_traffic() {
    use crate::tui::test_support::model_with_workloads;
    let mut m = model_fixture();
    let cmds = update(&mut m, key('4'));
    assert_eq!(cmds, vec![Cmd::LoadWorkloads(pce::CacheMode::Use)]);
    let wl: Vec<pce::models::Workload> =
        serde_json::from_str(include_str!("../../../../../../fixtures/workloads.json")).unwrap();
    update(&mut m, Msg::WorkloadsLoaded(Ok((wl, 0, false))));
    assert_eq!(m.workloads.table.len(), 4);
    assert!(update(&mut m, key('4')).is_empty());

    let mut m = model_with_workloads();
    update(&mut m, key('4'));
    update(
        &mut m,
        Msg::Key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE)),
    );
    let cmds = update(&mut m, key('t'));
    assert_eq!(m.screen, Screen::Traffic);
    assert_eq!(m.traffic.filter.src, "wl:host-2.example");
    assert!(
        matches!(
            cmds.as_slice(),
            [Cmd::RunQuery {
                mode: pce::CacheMode::Use,
                ..
            }]
        ),
        "{cmds:?}"
    );
    let seq = m.traffic.seq;
    update(
        &mut m,
        Msg::QueryDone {
            seq,
            result: Err("x".into()),
        },
    );
    update(&mut m, key('4'));
    let cmds = update(&mut m, key('T'));
    assert_eq!(m.traffic.filter.dst, "wl:host-2.example");
    assert_eq!(
        m.traffic.filter.src, "wl:host-2.example",
        "src kept from before"
    );
    assert!(matches!(cmds.as_slice(), [Cmd::RunQuery { .. }]));
}

#[test]
fn workloads_wait_for_the_index_and_recover_from_a_failed_one() {
    let mut m = model_fixture();
    let Connection::Connected { index, .. } = &mut m.connection else {
        unreachable!()
    };
    let idx = index.take().unwrap();
    assert!(
        update(&mut m, key('4')).is_empty(),
        "the startup load is still running: no duplicate requests"
    );
    assert!(!m.workloads.loading);
    assert_eq!(
        update(
            &mut m,
            Msg::IndexLoaded {
                index: idx,
                age_secs: 0
            }
        ),
        vec![Cmd::LoadWorkloads(pce::CacheMode::Use)]
    );
    assert!(m.workloads.loading);

    let mut m = model_fixture();
    let Connection::Connected { index, .. } = &mut m.connection else {
        unreachable!()
    };
    index.take();
    update(&mut m, key('4'));
    update(&mut m, Msg::IndexFailed("boom".into()));
    assert!(m.workloads.index_failed);
    assert_eq!(
        update(&mut m, key('r')),
        vec![Cmd::LoadIndex(pce::CacheMode::Use)]
    );
    assert!(update(&mut m, key('r')).is_empty(), "retry is issued once");
}

#[test]
fn workloads_error_keeps_rows_and_r_bypasses_unless_loading() {
    use crate::tui::test_support::model_with_workloads;
    let mut m = model_with_workloads();
    update(&mut m, key('4'));
    assert_eq!(
        update(&mut m, key('r')),
        vec![Cmd::LoadWorkloads(pce::CacheMode::Bypass)]
    );
    assert!(m.workloads.loading);
    assert!(update(&mut m, key('r')).is_empty(), "already loading");
    update(&mut m, Msg::WorkloadsLoaded(Err("boom".into())));
    assert!(!m.workloads.loading);
    assert_eq!(m.status.kind, StatusKind::Error);
    assert_eq!(m.workloads.table.len(), 4, "rows kept on error");
}

#[test]
fn workloads_enter_toggles_detail_only_with_a_selection() {
    use crate::tui::test_support::model_with_workloads;
    let mut m = model_fixture();
    m.screen = Screen::Workloads;
    update(&mut m, kc(KeyCode::Enter));
    assert!(!m.workloads.show_detail, "nothing selected");
    let mut m = model_with_workloads();
    update(&mut m, key('4'));
    update(&mut m, kc(KeyCode::Enter));
    assert!(m.workloads.show_detail);
    update(&mut m, kc(KeyCode::Enter));
    assert!(!m.workloads.show_detail);
}

#[test]
fn t_needs_a_selection_and_does_not_fire_while_searching() {
    use crate::tui::test_support::model_with_workloads;
    let mut m = model_fixture();
    m.screen = Screen::Workloads;
    assert!(update(&mut m, key('t')).is_empty());
    assert_eq!(m.screen, Screen::Workloads, "nothing selected: stay");

    let mut m = model_with_workloads();
    update(&mut m, key('4'));
    update(&mut m, key('/'));
    assert!(update(&mut m, key('t')).is_empty());
    assert_eq!(m.screen, Screen::Workloads);
    assert_eq!(
        m.workloads.table.search_text(),
        "t",
        "typed into the search"
    );
    assert!(m.traffic.filter.src.is_empty());
}

#[test]
fn t_goes_through_the_cost_guard() {
    use crate::tui::test_support::model_with_workloads;
    let mut m = model_with_workloads();
    update(&mut m, key('4'));
    m.traffic.filter.window = "30d".into();
    let cmds = update(&mut m, key('t'));
    assert!(cmds.is_empty(), "{cmds:?}");
    assert_eq!(m.screen, Screen::Workloads, "the query has not started");
    assert!(matches!(m.overlay, Overlay::Confirm { .. }));
    assert!(m.traffic.filter.src.is_empty(), "not applied before y");
    let cmds = update(&mut m, key('y'));
    assert!(matches!(cmds.as_slice(), [Cmd::RunQuery { .. }]));
    assert_eq!(m.screen, Screen::Traffic);
    assert_eq!(m.traffic.filter.src, "wl:host-1.example");
}

#[test]
fn t_while_a_query_runs_replaces_it_with_one_new_query() {
    use crate::tui::test_support::model_with_workloads;
    let mut m = model_with_workloads();
    update(&mut m, key('4'));
    let first = update(&mut m, key('t'));
    assert!(matches!(first.as_slice(), [Cmd::RunQuery { .. }]));
    let seq = m.traffic.seq;
    update(&mut m, key('4'));
    let second = update(&mut m, key('T'));
    assert!(matches!(second.as_slice(), [Cmd::RunQuery { .. }]));
    assert_eq!(m.traffic.seq, seq + 1, "the older query is superseded");
}

#[test]
fn jump_uses_the_href_id_when_the_name_would_break_the_filter_syntax() {
    use crate::tui::test_support::model_with_workloads;
    let mut m = model_with_workloads();
    m.workloads.raw[0].hostname = Some("db, primary|1".into());
    m.workloads.raw[1].hostname = Some("my host".into());
    m.workloads.rebuild(&m.limits.key_order());
    update(&mut m, key('4'));
    update(&mut m, key('t'));
    assert_eq!(m.traffic.filter.src, "wl:aaaa-1");
    assert!(m.traffic.filter.validate().is_ok());
    update(&mut m, key('4'));
    update(&mut m, key('j'));
    update(&mut m, key('T'));
    assert_eq!(m.traffic.filter.dst, "wl:my host", "spaces are fine");
    assert!(m.traffic.filter.validate().is_ok());
}

#[test]
fn h_toggles_unhealthy_only_and_capital_h_sorts_by_health() {
    use crate::tui::test_support::model_with_workloads;
    let mut m = model_with_workloads();
    assert!(update(&mut m, key('4')).is_empty());
    assert_eq!(m.workloads.table.len(), 4);
    assert!(update(&mut m, key('h')).is_empty());
    assert!(m.workloads.unhealthy_only);
    assert_eq!(
        m.workloads.table.len(),
        2,
        "host-2 (offline) and host-4 (suspended) are unhealthy at the fixed test clock"
    );
    assert!(update(&mut m, key('h')).is_empty());
    assert_eq!(m.workloads.table.len(), 4);
    assert!(update(&mut m, key('H')).is_empty());
    let t = &m.workloads.table;
    assert_eq!(
        t.rows()[t.visible()[0]].workload.hostname.as_deref(),
        Some("host-2.example"),
        "worst health first"
    );
}

#[test]
fn workload_filter_keys_are_local_combine_and_c_clears_them() {
    use crate::tui::test_support::model_with_workloads;
    let mut m = model_with_workloads();
    update(&mut m, key('4'));
    let calls = m.stats.calls;
    let hosts = |m: &Model| -> Vec<String> {
        let t = &m.workloads.table;
        t.visible()
            .iter()
            .map(|&i| t.rows()[i].workload.display_name())
            .collect()
    };
    // E: full, then selective.
    assert!(update(&mut m, key('E')).is_empty());
    assert_eq!(hosts(&m), ["host-1.example"]);
    assert!(update(&mut m, key('E')).is_empty());
    assert_eq!(hosts(&m), ["host-4.example"]);
    // u combines: host-4 has no labels.
    assert!(update(&mut m, key('u')).is_empty());
    assert_eq!(hosts(&m), ["host-4.example"]);
    assert!(update(&mut m, key('c')).is_empty());
    assert!(m.workloads.filters.is_empty());
    assert_eq!(m.workloads.table.len(), 4);
    // m: managed, then unmanaged.
    update(&mut m, key('m'));
    assert_eq!(m.workloads.table.len(), 3);
    update(&mut m, key('m'));
    assert_eq!(hosts(&m), ["unmanaged-nas"]);
    update(&mut m, key('m'));
    // o: offline only; h replaces it (health is one filter, #72).
    update(&mut m, key('o'));
    assert_eq!(hosts(&m), ["host-2.example"]);
    update(&mut m, key('h'));
    assert!(!m.workloads.filters.offline);
    assert_eq!(hosts(&m), ["host-2.example", "host-4.example"]);
    // `/` combines with them.
    update(&mut m, key('/'));
    update(&mut m, key('z'));
    update(
        &mut m,
        Msg::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
    );
    assert!(hosts(&m).is_empty(), "no offline host matches z");
    // c clears h and the filters, not the search.
    update(&mut m, key('c'));
    assert!(!m.workloads.unhealthy_only);
    assert!(m.workloads.filters.is_empty());
    assert_eq!(m.workloads.table.search_text(), "z");
    assert_eq!(m.stats.calls, calls, "no request");
    assert!(!m.quit);
}

/// 200 workloads, `host-NNN`: every tenth (from 5) without labels.
fn many_workloads() -> Model {
    use crate::tui::test_support::model_with_workloads;
    let mut m = model_with_workloads();
    let base = m.workloads.raw[0].clone();
    m.workloads.raw = (0..200)
        .map(|i| {
            let mut w = base.clone();
            w.href = format!("/orgs/5/workloads/w{i:03}");
            w.hostname = Some(format!("host-{i:03}"));
            if i % 10 == 5 {
                w.labels.clear();
            }
            w
        })
        .collect();
    m.workloads.rebuild(&m.limits.key_order());
    update(&mut m, key('4'));
    m
}

fn selected_host(m: &Model) -> String {
    m.workloads
        .table
        .selected_item()
        .unwrap()
        .workload
        .display_name()
}

#[test]
fn a_filter_that_shrinks_the_list_keeps_the_selected_row_in_view() {
    use crate::tui::test_support::render_to_string;
    let mut m = many_workloads();
    for _ in 0..150 {
        update(&mut m, key('j'));
    }
    assert_eq!(selected_host(&m), "host-150");
    assert!(m.workloads.table.offset() > 100);
    update(&mut m, key('u'));
    let t = &m.workloads.table;
    assert_eq!(t.len(), 20);
    assert!(
        t.offset() <= t.selected_pos(),
        "{} > {}",
        t.offset(),
        t.selected_pos()
    );
    let host = selected_host(&m);
    assert!(render_to_string(&m, 80, 24).contains(&host), "{host} drawn");
}

#[test]
fn a_filter_change_keeps_the_selected_workload_when_it_stays() {
    use crate::tui::test_support::render_to_string;
    let mut m = many_workloads();
    for _ in 0..145 {
        update(&mut m, key('j'));
    }
    assert_eq!(selected_host(&m), "host-145");
    update(&mut m, key('u'));
    assert_eq!(selected_host(&m), "host-145", "still shown: still selected");
    assert!(render_to_string(&m, 80, 24).contains("host-145"));
    update(&mut m, key('c'));
    assert_eq!(selected_host(&m), "host-145");
    assert!(render_to_string(&m, 80, 24).contains("host-145"));
}

#[test]
fn workload_search_skips_the_missing_column() {
    let mut m = many_workloads();
    update(&mut m, key('u'));
    update(&mut m, key('/'));
    for c in "role".chars() {
        update(&mut m, key(c));
    }
    update(
        &mut m,
        Msg::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
    );
    assert_eq!(
        m.workloads.table.len(),
        0,
        "`role` is only in the missing column"
    );
    m.workloads.table.search_clear();
    update(&mut m, key('/'));
    for c in "host-01".chars() {
        update(&mut m, key(c));
    }
    update(
        &mut m,
        Msg::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
    );
    assert_eq!(selected_host(&m), "host-015", "other columns still match");
}

#[test]
fn u_without_label_model_keys_says_so() {
    let mut m = many_workloads();
    m.set_label_model(Vec::new(), String::new(), Vec::new());
    assert!(update(&mut m, key('u')).is_empty());
    assert!(!m.workloads.filters.unlabeled);
    assert_eq!(m.status.text, "no label model keys");
}

#[test]
fn workload_filter_keys_type_into_an_open_search() {
    use crate::tui::test_support::model_with_workloads;
    let mut m = model_with_workloads();
    update(&mut m, key('4'));
    update(&mut m, key('/'));
    for c in "Eumoc".chars() {
        assert!(update(&mut m, key(c)).is_empty());
    }
    assert!(m.workloads.filters.is_empty());
    assert!(m.workloads.table.is_searching());
}

#[test]
fn h_with_no_unhealthy_rows_is_empty_not_a_panic_and_capital_h_flips() {
    use crate::tui::test_support::model_with_workloads;
    let mut m = model_with_workloads();
    update(&mut m, key('4'));
    m.workloads.raw.remove(3);
    m.workloads.raw.remove(1);
    m.workloads.rebuild(&m.limits.key_order());
    update(&mut m, key('h'));
    assert_eq!(m.workloads.table.len(), 0);
    assert!(m.workloads.table.selected_item().is_none());
    let counts = pce::models::VenHealth::ALL.map(|h| m.workloads.ven.count(h));
    assert_eq!(counts, [1, 0, 0, 0, 1], "summary ignores the filter");
    update(&mut m, key('h'));
    update(&mut m, key('H'));
    assert_eq!(m.workloads.table.sort(), Some((8, true)));
    update(&mut m, key('H'));
    assert_eq!(m.workloads.table.sort(), Some((8, false)), "second H flips");
    assert_eq!(
        m.workloads
            .table
            .selected_item()
            .unwrap()
            .workload
            .hostname
            .as_deref(),
        Some("host-1.example"),
        "ascending: healthy before unmanaged"
    );
}

#[test]
fn workloads_auto_refresh_obeys_the_guards_and_bypasses() {
    use crate::tui::test_support::model_with_workloads;
    use std::time::Duration;
    let mut m = model_with_workloads();
    update(&mut m, key('4'));
    let now = Instant::now() + Duration::from_secs(120);
    m.workloads.auto.last_run = Some(Instant::now());
    assert!(update(&mut m, Msg::Tick(now)).is_empty(), "opt-in only");
    update(&mut m, key('a'));
    assert!(m.workloads.auto.enabled);
    let early = Instant::now() + Duration::from_secs(10);
    assert!(
        update(&mut m, Msg::Tick(early)).is_empty(),
        "minimum interval"
    );
    m.focused = false;
    assert!(update(&mut m, Msg::Tick(now)).is_empty(), "not unfocused");
    m.focused = true;
    m.overlay = Overlay::Help;
    assert!(
        update(&mut m, Msg::Tick(now)).is_empty(),
        "not under an overlay"
    );
    m.overlay = Overlay::None;
    m.workloads.loading = true;
    assert!(
        update(&mut m, Msg::Tick(now)).is_empty(),
        "not while loading"
    );
    m.workloads.loading = false;
    assert_eq!(
        update(&mut m, Msg::Tick(now)),
        vec![Cmd::LoadWorkloads(pce::CacheMode::Bypass)]
    );
    m.screen = Screen::Traffic;
    m.workloads.loading = false;
    m.workloads.auto.last_run = Some(Instant::now());
    assert!(
        update(&mut m, Msg::Tick(now)).is_empty(),
        "only on its own screen"
    );
}

#[test]
fn a_second_profile_switch_waits_for_the_pending_one() {
    use crate::tui::commands::Command;
    use crate::tui::test_support::model_with_flows;
    let mut m = model_with_flows();
    assert_eq!(
        update(&mut m, Msg::Command(Command::Profile("a".into()))),
        vec![Cmd::SwitchProfile("a".into())]
    );
    assert!(m.switching);
    assert!(update(&mut m, Msg::Command(Command::Profile("b".into()))).is_empty());
    assert_eq!(m.status.text, "profile switch in progress");
    update(&mut m, Msg::ProfileSwitched(Err("boom".into())));
    assert!(!m.switching);
    assert_eq!(
        update(&mut m, Msg::Command(Command::Profile("b".into()))),
        vec![Cmd::SwitchProfile("b".into())]
    );
}

#[test]
fn profile_command_resets_state_and_reconnects() {
    use crate::tui::commands::Command;
    use crate::tui::model::{Limits, ProfileInfo};
    use crate::tui::test_support::model_with_flows;
    let mut m = model_with_flows();
    let cmds = update(&mut m, Msg::Command(Command::Profile("prod".into())));
    assert_eq!(cmds, vec![Cmd::SwitchProfile("prod".into())]);
    assert!(
        m.traffic.data.is_some(),
        "nothing changes until the switch succeeds"
    );
    let cmds = update(
        &mut m,
        Msg::ProfileSwitched(Ok((
            ProfileInfo {
                name: "prod".into(),
                host: "prod.example".into(),
                org_id: 9,
            },
            Limits {
                auto_refresh_min_seconds: 300,
                traffic_max_results: 500,
                traffic_default_window: "1h".into(),
                traffic_auto_backfill: false,
                scope_keys: vec!["site".into()],
                tier_key: "tier".into(),
                ignore_keys: vec!["os".into()],
                ruleset_name: "RF-{site}".into(),
                consolidate_min_peers: 5,
                event_exclude_types: Vec::new(),
                exposure_services: Ok(pce::traffic::default_services()),
            },
        ))),
    );
    assert_eq!(cmds, vec![Cmd::Connect]);
    assert_eq!(m.limits.traffic_max_results, 500);
    assert_eq!(m.limits.auto_refresh_min_seconds, 300);
    assert_eq!(
        m.limits.scope_keys,
        vec!["site"],
        "the label model follows the profile"
    );
    assert_eq!(m.limits.tier_key, "tier");
    assert_eq!(m.limits.ignore_keys, vec!["os"]);
    assert_eq!(
        m.traffic.key_order.ignored(),
        vec!["os"],
        "the traffic views hide the new profile's ignored keys"
    );
    assert_eq!(m.limits.traffic_default_window, "1h");
    assert_eq!(m.traffic.filter.max_results, 500);
    assert_eq!(m.traffic.filter.window, "1h");
    assert_eq!(m.traffic.auto.interval.as_secs(), 300);
    assert!(matches!(m.connection, Connection::Connecting));
    assert!(m.traffic.data.is_none());
    assert_eq!(m.traffic.flows.len(), 0);
    assert_eq!(m.profile.host, "prod.example");
    assert_eq!(m.stats.calls, 0);
    let mut m2 = model_with_flows();
    update(
        &mut m2,
        Msg::ProfileSwitched(Err("profile \"nope\" not found".into())),
    );
    assert!(m2.traffic.data.is_some());
    assert_eq!(m2.status.kind, StatusKind::Error);
    assert_eq!(m2.profile.name, "demo");
}

#[test]
fn disconnected_model_still_takes_profile_and_quit() {
    use crate::tui::commands::Command;
    let mut m = model_fixture();
    m.connection = Connection::Disconnected {
        error: "authentication failed (HTTP 401)".into(),
    };
    assert_eq!(
        update(&mut m, Msg::Command(Command::Profile("other".into()))),
        vec![Cmd::SwitchProfile("other".into())]
    );
    assert_eq!(update(&mut m, key('q')), vec![Cmd::Quit]);
    assert!(
        update(&mut m, key('2')).is_empty(),
        "no loads while disconnected"
    );
}

#[test]
fn draft_and_active_commands_and_error_overlay() {
    use crate::tui::commands::Command;
    use crate::tui::model::Overlay;
    use crate::tui::test_support::model_with_policy;
    use pce::models::PolicyScope;
    let mut m = model_with_policy();
    let cmds = update(&mut m, Msg::Command(Command::Active));
    assert_eq!(
        cmds,
        vec![Cmd::LoadRuleSets {
            scope: PolicyScope::Active,
            mode: pce::CacheMode::Use
        }]
    );
    assert_eq!(m.screen, Screen::Policy);
    update(
        &mut m,
        Msg::RuleSetsLoaded {
            scope: PolicyScope::Active,
            result: Err(
                "authentication failed (HTTP 401); check API key, secret and org id".into(),
            ),
        },
    );
    assert!(matches!(m.overlay, Overlay::Error(_)));
    update(
        &mut m,
        Msg::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)),
    );
    assert!(matches!(m.overlay, Overlay::None));
    let cmds = update(&mut m, Msg::Command(Command::Draft));
    assert_eq!(
        cmds,
        vec![Cmd::LoadRuleSets {
            scope: PolicyScope::Draft,
            mode: pce::CacheMode::Use
        }]
    );
}

#[test]
fn draft_and_active_do_nothing_while_disconnected() {
    use crate::tui::commands::Command;
    let mut m = model_fixture();
    m.connection = Connection::Disconnected { error: "x".into() };
    assert!(update(&mut m, Msg::Command(Command::Active)).is_empty());
    assert_eq!(m.status.text, "not connected");
    assert_eq!(m.status.kind, StatusKind::Error);
}

#[test]
fn long_errors_open_the_overlay_short_ones_stay_in_the_status_line() {
    use crate::tui::test_support::model_with_workloads;
    let mut m = model_with_workloads();
    update(&mut m, Msg::WorkloadsLoaded(Err("timeout".into())));
    assert!(matches!(m.overlay, Overlay::None));
    assert_eq!(m.status.text, "could not load workloads: timeout");
    let long = "x".repeat(200);
    update(&mut m, Msg::WorkloadsLoaded(Err(long.clone())));
    match &m.overlay {
        Overlay::Error(t) => assert!(t.contains(&long)),
        o => panic!("{o:?}"),
    }
    for code in [KeyCode::Enter, KeyCode::Esc] {
        m.overlay = Overlay::Error("e".into());
        update(&mut m, Msg::Key(KeyEvent::new(code, KeyModifiers::NONE)));
        assert!(matches!(m.overlay, Overlay::None));
    }
    m.overlay = Overlay::Error("e".into());
    assert!(
        update(&mut m, key('2')).is_empty(),
        "keys do not leak through"
    );
    assert!(matches!(m.overlay, Overlay::Error(_)));
}

#[test]
fn an_error_never_replaces_an_open_editor() {
    use crate::tui::test_support::model_with_workloads;
    let mut m = model_with_workloads();
    m.overlay = Overlay::Prompt {
        input: "fil".into(),
    };
    update(&mut m, Msg::WorkloadsLoaded(Err("x".repeat(100))));
    assert!(matches!(m.overlay, Overlay::Prompt { .. }));
    assert_eq!(m.status.kind, StatusKind::Error);
}

#[test]
fn auth_failure_at_startup_opens_the_overlay_and_disconnects() {
    let mut m = model_fixture();
    m.connection = Connection::Connecting;
    update(
        &mut m,
        Msg::ConnectFailed("authentication failed (HTTP 401)".into()),
    );
    assert!(matches!(m.connection, Connection::Disconnected { .. }));
    assert!(matches!(m.overlay, Overlay::Error(_)));
}

#[test]
fn reset_data_cancels_the_query_and_disarms_every_auto_refresh() {
    use crate::tui::test_support::model_with_flows;
    let mut m = model_with_flows();
    let f = m.traffic.filter.clone();
    let cmds = update(&mut m, Msg::ApplyFilter(f));
    let Cmd::RunQuery { cancel, .. } = &cmds[0] else {
        panic!("{cmds:?}")
    };
    let cancel = cancel.clone();
    m.policy.auto.enabled = true;
    m.objects.auto.enabled = true;
    m.workloads.auto.enabled = true;
    m.traffic.auto.enabled = true;
    m.traffic.auto.from_tick = true;
    m.objects.index_failed = true;
    m.workloads.index_failed = true;
    m.workloads.loading = true;
    m.policy.loading = true;
    m.policy.bypass_deny = true;
    m.busy = 3;
    m.overlay = Overlay::Help;
    m.reset_data();
    assert!(cancel.is_cancelled(), "a running query is cancelled");
    assert!(!m.any_auto_enabled());
    assert!(!m.traffic.auto.from_tick);
    assert!(!m.objects.index_failed && !m.workloads.index_failed);
    assert!(!m.workloads.loading && !m.policy.loading && !m.policy.bypass_deny);
    assert!(m.workloads.loaded_at.is_none() && m.policy.loaded_at.is_none());
    assert!(!m.traffic.is_running());
    assert_eq!((m.busy, m.overlay.clone()), (0, Overlay::None));
    assert_eq!(m.workloads.table.len(), 0);
}

#[test]
fn auto_refresh_failure_disables_it_on_traffic() {
    use crate::tui::test_support::model_with_flows;
    use std::time::Duration;
    let mut m = model_with_flows();
    update(&mut m, key('a'));
    let now = Instant::now() + Duration::from_secs(120);
    let cmds = update(&mut m, Msg::Tick(now));
    assert!(matches!(cmds.as_slice(), [Cmd::RunQuery { .. }]));
    let seq = m.traffic.seq;
    update(
        &mut m,
        Msg::QueryDone {
            seq,
            result: Err("HTTP 429".into()),
        },
    );
    assert!(!m.traffic.auto.enabled, "never retried unattended");
    assert_eq!(m.status.text, "auto-refresh off: HTTP 429");
    assert!(update(&mut m, Msg::Tick(now + Duration::from_secs(600))).is_empty());
    // A manual run that fails is reported normally.
    update(&mut m, key('a'));
    assert!(m.traffic.auto.enabled, "`a` re-arms");
    update(&mut m, key('r'));
    let seq = m.traffic.seq;
    update(
        &mut m,
        Msg::QueryDone {
            seq,
            result: Err("HTTP 500".into()),
        },
    );
    assert!(m.traffic.auto.enabled, "a manual failure does not trip it");
    assert_eq!(m.status.text, "query: HTTP 500");
}

#[test]
fn auto_refresh_failure_disables_it_on_load_based_screens() {
    use crate::tui::test_support::{model_with_objects, model_with_policy, model_with_workloads};
    use std::time::Duration;
    let now = Instant::now() + Duration::from_secs(120);

    let mut m = model_with_workloads();
    update(&mut m, key('4'));
    update(&mut m, key('a'));
    m.workloads.auto.last_run = Some(Instant::now());
    assert_eq!(
        update(&mut m, Msg::Tick(now)),
        vec![Cmd::LoadWorkloads(pce::CacheMode::Bypass)]
    );
    update(&mut m, Msg::WorkloadsLoaded(Err("HTTP 429".into())));
    assert!(!m.workloads.auto.enabled);
    assert_eq!(m.status.text, "auto-refresh off: HTTP 429");
    assert!(update(&mut m, Msg::Tick(now + Duration::from_secs(600))).is_empty());
    // `r` is a user-caused retry and re-arms what the failure switched off.
    assert_eq!(
        update(&mut m, key('r')),
        vec![Cmd::LoadWorkloads(pce::CacheMode::Bypass)]
    );
    assert!(m.workloads.auto.enabled);

    let mut m = model_with_policy();
    update(&mut m, key('2'));
    update(&mut m, key('a'));
    m.policy.auto.last_run = Some(Instant::now());
    assert!(matches!(
        update(&mut m, Msg::Tick(now)).as_slice(),
        [Cmd::LoadRuleSets { .. }]
    ));
    let scope = m.policy.scope;
    update(
        &mut m,
        Msg::RuleSetsLoaded {
            scope,
            result: Err("HTTP 429".into()),
        },
    );
    assert!(!m.policy.auto.enabled);
    assert_eq!(m.status.text, "auto-refresh off: HTTP 429");

    let mut m = model_with_objects();
    update(&mut m, key('3'));
    update(&mut m, key('a'));
    m.objects.auto.last_run = Some(Instant::now());
    assert!(matches!(
        update(&mut m, Msg::Tick(now)).as_slice(),
        [Cmd::LoadObjects { .. }]
    ));
    let kind = m.objects.kind;
    update(
        &mut m,
        Msg::ObjectsLoaded {
            kind,
            result: Err("HTTP 429".into()),
        },
    );
    assert!(!m.objects.auto.enabled);
    assert_eq!(m.status.text, "auto-refresh off: HTTP 429");
}

#[test]
fn r_while_disconnected_reconnects_once_and_never_says_still_loading() {
    for screen in ['3', '4'] {
        let mut m = model_fixture();
        m.objects.index_failed = true;
        m.workloads.index_failed = true;
        m.connection = Connection::Disconnected {
            error: "authentication failed (HTTP 401)".into(),
        };
        update(&mut m, key(screen));
        assert_eq!(update(&mut m, key('r')), vec![Cmd::Connect]);
        assert!(matches!(m.connection, Connection::Connecting));
        assert!(!m.objects.index_failed && !m.workloads.index_failed);
        assert!(
            !m.status.text.contains("still loading"),
            "{}",
            m.status.text
        );
        assert!(
            update(&mut m, key('r')).is_empty(),
            "one attempt while a connect is in flight"
        );
    }
}

#[test]
fn disconnected_bodies_show_the_error_not_loading() {
    use crate::tui::test_support::render_to_string;
    for screen in [Screen::Objects, Screen::Workloads] {
        let mut m = model_fixture();
        m.connection = Connection::Disconnected {
            error: "connection refused".into(),
        };
        m.screen = screen;
        let out = render_to_string(&m, 80, 24);
        assert!(out.contains("connection refused"), "{out}");
        assert!(!out.contains("loading"), "{out}");
    }
}

#[test]
fn policy_r_while_loading_is_a_noop_that_keeps_the_deny_bypass_unspent() {
    use crate::tui::test_support::model_with_policy;
    let mut m = model_with_policy();
    update(&mut m, key('2'));
    assert_eq!(update(&mut m, key('r')).len(), 1);
    assert!(m.policy.loading && m.policy.bypass_deny);
    assert!(update(&mut m, key('r')).is_empty());
    assert_eq!(m.status.text, "already loading");
    assert!(m.policy.bypass_deny);
}

#[test]
fn declining_the_cost_guard_after_t_stays_on_workloads() {
    use crate::tui::test_support::model_with_workloads;
    let mut m = model_with_workloads();
    m.traffic.filter.window = "30d".into();
    update(&mut m, key('4'));
    let before = m.traffic.filter.clone();
    let status = m.status.clone();
    assert!(update(&mut m, key('t')).is_empty());
    assert!(matches!(m.overlay, Overlay::Confirm { .. }));
    assert_eq!(
        m.screen,
        Screen::Workloads,
        "no switch before the query starts"
    );
    assert_eq!(m.status, status);
    update(&mut m, key('n'));
    assert_eq!(m.screen, Screen::Workloads);
    assert_eq!(m.traffic.filter, before);
    assert!(!m.traffic.is_running());
    // Confirming switches and starts the query.
    update(&mut m, key('t'));
    let cmds = update(&mut m, key('y'));
    assert!(matches!(cmds.as_slice(), [Cmd::RunQuery { .. }]));
    assert_eq!(m.screen, Screen::Traffic);
    assert_eq!(m.traffic.filter.src, "wl:host-1.example");
}

#[test]
fn policy_reloads_after_a_profile_switch_once_the_index_is_back() {
    use crate::tui::commands::Command;
    use crate::tui::model::{Limits, ProfileInfo};
    use crate::tui::test_support::{fixture_index, model_with_policy, render_to_string};
    let mut m = model_with_policy();
    update(&mut m, key('2'));
    assert_eq!(
        update(&mut m, Msg::Command(Command::Profile("other".into()))),
        vec![Cmd::SwitchProfile("other".into())]
    );
    let cmds = update(
        &mut m,
        Msg::ProfileSwitched(Ok((
            ProfileInfo {
                name: "other".into(),
                host: "o.example".into(),
                org_id: 7,
            },
            Limits {
                auto_refresh_min_seconds: 60,
                traffic_max_results: 1000,
                traffic_default_window: "24h".into(),
                traffic_auto_backfill: false,
                scope_keys: vec!["app".into(), "env".into()],
                tier_key: "role".into(),
                ignore_keys: Vec::new(),
                ruleset_name: "RF-{app}-{env}".into(),
                consolidate_min_peers: 5,
                event_exclude_types: Vec::new(),
                exposure_services: Ok(pce::traffic::default_services()),
            },
        ))),
    );
    assert_eq!(cmds, vec![Cmd::Connect]);
    assert_eq!(m.screen, Screen::Policy);
    assert!(!m.policy.loading && m.policy.loaded_at.is_none());
    let out = render_to_string(&m, 80, 24);
    assert!(out.contains("connecting"), "{out}");
    assert!(!out.contains("ruleset-"), "no stale rows: {out}");
    assert_eq!(
        update(&mut m, Msg::Connected),
        vec![Cmd::LoadIndex(CacheMode::Use)]
    );
    let cmds = update(
        &mut m,
        Msg::IndexLoaded {
            index: fixture_index(),
            age_secs: 0,
        },
    );
    assert_eq!(
        cmds,
        vec![Cmd::LoadRuleSets {
            scope: PolicyScope::Draft,
            mode: CacheMode::Use
        }]
    );
    assert!(m.policy.loading);
    let out = render_to_string(&m, 80, 24);
    assert!(out.contains("loading…"), "{out}");
    // A second index event does not double-load.
    let again = update(
        &mut m,
        Msg::IndexLoaded {
            index: fixture_index(),
            age_secs: 0,
        },
    );
    assert!(again.is_empty(), "{again:?}");
}

#[test]
fn mouse_wheel_scrolls_and_tab_click_switches() {
    use crate::tui::test_support::model_with_flows;
    use crossterm::event::{MouseButton, MouseEventKind};
    let mut m = model_with_flows();
    assert!(update(&mut m, Msg::Resize(80, 24)).is_empty());
    assert!(update(&mut m, mouse(MouseEventKind::ScrollDown, 10, 10)).is_empty());
    assert!(update(&mut m, mouse(MouseEventKind::ScrollDown, 10, 10)).is_empty());
    assert_eq!(m.traffic.flows.selected_pos(), 2);
    assert!(update(&mut m, mouse(MouseEventKind::ScrollUp, 10, 10)).is_empty());
    assert_eq!(m.traffic.flows.selected_pos(), 1);
    let x = crate::tui::ui::tab_x(Screen::Policy, 80);
    update(&mut m, mouse(MouseEventKind::Down(MouseButton::Left), x, 1));
    assert_eq!(m.screen, Screen::Policy);
}

#[test]
fn wheel_selected_ruleset_notes_missing_deny_rules_until_a_key_loads_them() {
    use crate::tui::test_support::{model_with_policy, render_to_string};
    use crossterm::event::MouseEventKind;
    let href = "/orgs/5/sec_policy/draft/rule_sets/101".to_string();
    let mut m = model_with_policy();
    m.screen = Screen::Policy;
    update(&mut m, Msg::Resize(80, 24));
    assert!(!render_to_string(&m, 80, 24).contains("deny rules not loaded"));
    assert!(update(&mut m, mouse(MouseEventKind::ScrollDown, 10, 10)).is_empty());
    assert_eq!(m.policy.selected_href().as_deref(), Some(href.as_str()));
    assert!(render_to_string(&m, 80, 24).contains("deny rules not loaded"));
    // A key move away (Up) and back (Down) loads them through the existing path, once.
    let up = update(&mut m, kc(KeyCode::Up));
    let down = update(&mut m, kc(KeyCode::Down));
    let loads: Vec<_> = up.iter().chain(down.iter()).collect();
    assert!(
        loads.contains(&&Cmd::LoadDenyRules {
            href: href.clone(),
            mode: pce::CacheMode::Use
        }),
        "{loads:?}"
    );
    assert!(!render_to_string(&m, 80, 24).contains("deny rules not loaded"));
    update(
        &mut m,
        Msg::DenyRulesLoaded {
            href,
            result: Ok(Vec::new()),
        },
    );
    assert!(!render_to_string(&m, 80, 24).contains("deny rules not loaded"));
    // Enter on the ruleset row is the other way in.
    let mut m = model_with_policy();
    m.screen = Screen::Policy;
    update(&mut m, mouse(MouseEventKind::ScrollDown, 10, 10));
    assert_eq!(update(&mut m, kc(KeyCode::Enter)).len(), 1);
    assert!(update(&mut m, kc(KeyCode::Enter)).is_empty());
}

#[test]
fn mouse_ignores_everything_else_and_overlays() {
    use crate::tui::test_support::model_with_flows;
    use crossterm::event::{MouseButton, MouseEventKind};
    let mut m = model_with_flows();
    update(&mut m, Msg::Resize(80, 24));
    let x = crate::tui::ui::tab_x(Screen::Policy, 80);
    for kind in [
        MouseEventKind::Down(MouseButton::Right),
        MouseEventKind::Up(MouseButton::Left),
        MouseEventKind::Moved,
        MouseEventKind::Drag(MouseButton::Left),
    ] {
        assert!(update(&mut m, mouse(kind, x, 1)).is_empty());
    }
    // Clicks off the tabs row, past the last tab, and on a divider do nothing.
    let left = MouseEventKind::Down(MouseButton::Left);
    assert!(update(&mut m, mouse(left, x, 5)).is_empty());
    assert!(update(&mut m, mouse(left, 79, 1)).is_empty());
    let divider = crate::tui::ui::tab_x(Screen::Policy, 80) - 2;
    assert!(update(&mut m, mouse(left, divider, 1)).is_empty());
    assert_eq!(m.screen, Screen::Traffic);
    assert_eq!(m.traffic.flows.selected_pos(), 0);
    // Overlays swallow wheel and clicks.
    m.overlay = Overlay::Help;
    update(&mut m, mouse(MouseEventKind::ScrollDown, 10, 10));
    update(&mut m, mouse(left, x, 1));
    assert_eq!(m.traffic.flows.selected_pos(), 0);
    assert_eq!(m.screen, Screen::Traffic);
}

#[test]
fn mouse_wheel_follows_policy_pane_aggregate_view_and_search() {
    use crate::tui::test_support::{model_with_flows, model_with_policy, model_with_workloads};
    use crossterm::event::MouseEventKind;
    let down = || mouse(MouseEventKind::ScrollDown, 10, 10);
    let mut m = model_with_policy();
    m.screen = Screen::Policy;
    update(&mut m, Msg::Resize(160, 48));
    assert!(
        update(&mut m, down()).is_empty(),
        "the wheel never requests"
    );
    assert_eq!(m.policy.rulesets.selected_pos(), 1);
    update(
        &mut m,
        Msg::Key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE)),
    );
    assert_eq!(m.policy.pane, PolicyPane::Rules);
    let before = m.policy.rulesets.selected_pos();
    update(&mut m, down());
    assert_eq!(m.policy.rulesets.selected_pos(), before);

    let mut m = model_with_flows();
    update(&mut m, Msg::Resize(80, 24));
    update(
        &mut m,
        Msg::Key(KeyEvent::new(KeyCode::Char('v'), KeyModifiers::NONE)),
    );
    assert_eq!(m.traffic.view, TrafficView::Aggregate);
    update(&mut m, down());
    assert_eq!(m.traffic.groups.selected_pos(), 1);
    assert_eq!(m.traffic.flows.selected_pos(), 0);
    // An active search keeps routing the wheel to the table.
    update(
        &mut m,
        Msg::Key(KeyEvent::new(KeyCode::Char('/'), KeyModifiers::NONE)),
    );
    update(&mut m, down());
    assert_eq!(
        m.traffic.groups.selected_pos(),
        2.min(m.traffic.groups.len() - 1)
    );

    // Empty table: no panic, no request.
    let mut m = model_with_workloads();
    m.screen = Screen::Workloads;
    m.workloads.table.set_rows(Vec::new());
    assert!(update(&mut m, down()).is_empty());
}

#[test]
fn opening_events_loads_once_and_editor_reloads() {
    use crate::tui::model::Overlay;
    use crate::tui::msg::Cmd;
    let mut m = model_fixture();
    let cmds = update(&mut m, key('5'));
    assert_eq!(cmds.len(), 1);
    assert!(
        matches!(&cmds[0], Cmd::LoadEvents { mode: pce::CacheMode::Use, filter } if filter.severity.is_none() && filter.since.is_some())
    );
    assert_eq!(m.screen, Screen::Events);
    let events: Vec<pce::models::Event> =
        serde_json::from_str(include_str!("../../../../../../fixtures/events.json")).unwrap();
    update(
        &mut m,
        Msg::EventsLoaded(Ok(crate::tui::msg::EventsPayload {
            events,
            age_secs: 0,
            truncated: false,
            window: ("2026-10-09T08:00:00Z".into(), "2026-10-10T08:00:00Z".into()),
        })),
    );
    assert_eq!(m.events.table.len(), 4);
    assert!(update(&mut m, key('1')).is_empty());
    assert!(update(&mut m, key('5')).is_empty(), "already loaded");
    update(&mut m, key('f'));
    assert!(matches!(m.overlay, Overlay::EventsEditor(_)));
    for c in "err".chars() {
        update(&mut m, key(c));
    }
    let cmds = update(&mut m, enter());
    assert!(
        matches!(&cmds[0], Cmd::LoadEvents { filter, .. } if filter.severity.as_deref() == Some("err"))
    );
    assert!(matches!(m.overlay, Overlay::None));
    update(&mut m, key('f'));
    // Severity, type, actor, then the window.
    for _ in 0..3 {
        update(
            &mut m,
            Msg::Key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE)),
        );
    }
    for _ in 0..3 {
        update(
            &mut m,
            Msg::Key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE)),
        );
    }
    for c in "soon".chars() {
        update(&mut m, key(c));
    }
    update(&mut m, enter());
    assert!(
        matches!(&m.overlay, Overlay::EventsEditor(e) if e.error.as_ref().is_some_and(|e| e.contains("window")))
    );
}

#[test]
fn events_type_filter_detail_and_severity_sort_are_local() {
    use crate::tui::model::Overlay;
    use crate::tui::test_support::model_with_events;
    let mut m = model_with_events();
    assert!(update(&mut m, key('5')).is_empty());
    assert_eq!(m.events.table.len(), 4);
    m.events.filter.event_type = "agent.".into();
    m.events.rebuild();
    assert_eq!(m.events.table.len(), 1);
    m.events.filter.event_type.clear();
    m.events.rebuild();
    assert!(update(&mut m, key('S')).is_empty());
    assert_eq!(
        m.events
            .table
            .selected_item()
            .unwrap()
            .event_type
            .as_deref(),
        Some("agent.tampering"),
        "worst severity first"
    );
    assert!(update(&mut m, enter()).is_empty());
    assert!(matches!(m.overlay, Overlay::EventDetail { scroll: 0, .. }));
    // The detail scrolls line by line (^e / ^y) and stops at the top and the last line.
    let total = event_detail_lines(&m, m.events.table.selected_item().unwrap(), false);
    assert!(total > 2, "{total}");
    assert!(
        update(
            &mut m,
            Msg::Key(KeyEvent::new(KeyCode::Char('e'), KeyModifiers::CONTROL))
        )
        .is_empty()
    );
    assert!(matches!(m.overlay, Overlay::EventDetail { scroll: 1, .. }));
    assert!(update(&mut m, kc(KeyCode::End)).is_empty());
    assert!(matches!(&m.overlay, Overlay::EventDetail { scroll, .. } if *scroll == total - 1));
    assert!(
        update(
            &mut m,
            Msg::Key(KeyEvent::new(KeyCode::Char('e'), KeyModifiers::CONTROL))
        )
        .is_empty()
    );
    assert!(matches!(&m.overlay, Overlay::EventDetail { scroll, .. } if *scroll == total - 1));
    assert!(update(&mut m, key('g')).is_empty());
    assert!(
        update(
            &mut m,
            Msg::Key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::CONTROL))
        )
        .is_empty()
    );
    assert!(matches!(m.overlay, Overlay::EventDetail { scroll: 0, .. }));
    // j / k (and ↓ / ↑) step to the next and previous event, at the top,
    // keeping the view; the table cursor follows.
    let first = m.events.table.selected_pos();
    assert!(
        update(
            &mut m,
            Msg::Key(KeyEvent::new(KeyCode::Char('e'), KeyModifiers::CONTROL))
        )
        .is_empty()
    );
    assert!(update(&mut m, key('J')).is_empty());
    assert!(update(&mut m, key('j')).is_empty());
    assert_eq!(m.events.table.selected_pos(), first + 1);
    let Overlay::EventDetail { event, scroll, raw } = &m.overlay else {
        panic!("{:?}", m.overlay)
    };
    assert_eq!(
        (*scroll, *raw),
        (0, true),
        "next event, from the top, still raw"
    );
    assert_eq!(Some(event), m.events.table.selected_item());
    assert!(update(&mut m, kc(KeyCode::Up)).is_empty());
    assert_eq!(m.events.table.selected_pos(), first);
    assert!(update(&mut m, key('k')).is_empty());
    assert_eq!(
        m.events.table.selected_pos(),
        first.saturating_sub(1),
        "stops at the top"
    );
    assert!(update(&mut m, key('J')).is_empty());
    assert!(
        update(
            &mut m,
            Msg::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)),
        )
        .is_empty()
    );
    assert_eq!(
        crate::tui::commands::parse("events").unwrap(),
        crate::tui::commands::Command::Screen(Screen::Events)
    );
}

#[test]
fn events_invalid_window_sends_nothing_and_enter_needs_a_selection() {
    use crate::tui::model::Overlay;
    use crate::tui::test_support::model_with_events;
    let mut m = model_with_events();
    m.screen = Screen::Events;
    update(&mut m, key('f'));
    update(
        &mut m,
        Msg::Key(KeyEvent::new(KeyCode::BackTab, KeyModifiers::NONE)),
    );
    for _ in 0..3 {
        update(
            &mut m,
            Msg::Key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE)),
        );
    }
    assert!(
        update(&mut m, enter()).is_empty(),
        "invalid window: no request"
    );
    assert!(!m.events.loading);
    assert!(matches!(
        &m.overlay,
        Overlay::EventsEditor(e) if e.error.is_some()
    ));
    update(
        &mut m,
        Msg::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)),
    );
    assert_eq!(
        m.events.filter.window, "24h",
        "a rejected edit is not stored"
    );
    m.events.filter.event_type = "no.such.type".into();
    m.events.rebuild();
    assert!(update(&mut m, enter()).is_empty());
    assert!(
        matches!(m.overlay, Overlay::None),
        "nothing selected, no detail"
    );
}

#[test]
fn events_r_bypasses_once_and_not_while_loading() {
    use crate::tui::test_support::model_with_events;
    let mut m = model_with_events();
    m.screen = Screen::Events;
    let cmds = update(&mut m, key('r'));
    assert!(matches!(
        &cmds[..],
        [Cmd::LoadEvents {
            mode: pce::CacheMode::Bypass,
            ..
        }]
    ));
    assert!(m.events.loading);
    assert!(update(&mut m, key('r')).is_empty(), "already loading");
}

#[test]
fn events_auto_refresh_obeys_the_guards_and_trips_on_failure() {
    use crate::tui::model::Overlay;
    use crate::tui::test_support::model_with_events;
    use std::time::Duration;
    let mut m = model_with_events();
    update(&mut m, key('5'));
    let now = Instant::now() + Duration::from_secs(120);
    m.events.auto.last_run = Some(Instant::now());
    assert!(update(&mut m, Msg::Tick(now)).is_empty(), "opt-in only");
    update(&mut m, key('a'));
    assert!(m.events.auto.enabled);
    assert!(m.any_auto_enabled());
    assert!(update(&mut m, Msg::Tick(Instant::now() + Duration::from_secs(10))).is_empty());
    m.focused = false;
    assert!(update(&mut m, Msg::Tick(now)).is_empty(), "not unfocused");
    m.focused = true;
    m.overlay = Overlay::Help;
    assert!(update(&mut m, Msg::Tick(now)).is_empty(), "no overlay");
    m.overlay = Overlay::None;
    m.events.loading = true;
    assert!(
        update(&mut m, Msg::Tick(now)).is_empty(),
        "not while loading"
    );
    m.events.loading = false;
    let cmds = update(&mut m, Msg::Tick(now));
    assert!(matches!(
        &cmds[..],
        [Cmd::LoadEvents {
            mode: pce::CacheMode::Bypass,
            ..
        }]
    ));
    assert!(m.events.auto.from_tick);
    update(&mut m, Msg::EventsLoaded(Err("boom".into())));
    assert!(!m.events.auto.enabled && m.events.auto.tripped);
    assert!(
        m.status.text.contains("auto-refresh off"),
        "{}",
        m.status.text
    );
    assert!(!m.events.loading);
    m.events.auto.enabled = true;
    m.events.loading = false;
    m.events.auto.last_run = Some(Instant::now());
    m.screen = Screen::Traffic;
    assert!(
        update(&mut m, Msg::Tick(now)).is_empty(),
        "only its own screen"
    );
}

#[test]
fn events_mouse_wheel_scrolls_without_requests_and_reset_clears_state() {
    use crate::tui::test_support::model_with_events;
    use crossterm::event::MouseEventKind;
    let mut m = model_with_events();
    m.screen = Screen::Events;
    assert!(update(&mut m, mouse(MouseEventKind::ScrollDown, 10, 10)).is_empty());
    assert_eq!(m.events.table.selected_pos(), 1);
    m.events.auto.enabled = true;
    m.events.filter.window = "7d".into();
    m.reset_data();
    assert!(m.events.table.is_empty() && m.events.raw.is_empty());
    assert!(m.events.loaded_at.is_none() && !m.events.auto.enabled);
    assert_eq!(m.events.filter.window, "24h");
}

#[test]
fn events_open_while_connecting_loads_when_connected() {
    let mut m = model_fixture();
    m.connection = Connection::Connecting;
    assert!(update(&mut m, key('5')).is_empty(), "not connected yet");
    let cmds = update(&mut m, Msg::Connected);
    assert!(cmds.iter().any(|c| matches!(
        c,
        Cmd::LoadEvents {
            mode: pce::CacheMode::Use,
            ..
        }
    )));
    assert!(cmds.iter().any(|c| matches!(c, Cmd::LoadIndex(_))));
}

fn query_done_with(m: &mut Model, filter: TrafficFilter) {
    let seq = m.traffic.seq;
    update(
        m,
        Msg::QueryDone {
            seq,
            result: Ok(QueryOutcome {
                flows: crate::tui::test_support::fixture_flows(),
                truncated: false,
                incomplete: false,
                elapsed: std::time::Duration::from_secs(1),
                hit: false,
                filter,
                bounds: None,
                hrefs: Vec::new(),
            }),
        },
    );
}

fn is_bypass_run(cmds: &[Cmd]) -> bool {
    matches!(
        cmds,
        [Cmd::RunQuery {
            mode: CacheMode::Bypass,
            ..
        }]
    )
}

#[test]
fn r_honours_the_cost_guard_unless_the_filter_produced_the_data() {
    use crate::tui::model::Limits;
    let mut m = model_fixture();
    // Profile defaults above the confirmation cap: nothing confirmed them yet.
    m.limits = Limits {
        traffic_max_results: 20_000,
        ..m.limits.clone()
    };
    m.reset_data();
    assert_eq!(m.traffic.filter.max_results, 20_000);
    let cmds = update(&mut m, key('r'));
    assert!(cmds.is_empty(), "{cmds:?}");
    assert!(
        matches!(&m.overlay, Overlay::Confirm { reason, action: PendingAction::RunNow(_) } if reason.contains("20000"))
    );
    // Declining sends nothing.
    assert!(update(&mut m, key('n')).is_empty());
    assert_eq!(m.overlay, Overlay::None);
    update(&mut m, key('r'));
    let cmds = update(&mut m, key('y'));
    assert!(is_bypass_run(&cmds), "{cmds:?}");
    assert!(m.traffic.is_running());
    let f = m.traffic.filter.clone();
    query_done_with(&mut m, f);
    // The same filter produced the data on screen: `r` reruns it directly.
    let cmds = update(&mut m, key('r'));
    assert!(is_bypass_run(&cmds), "{cmds:?}");
    assert_eq!(m.overlay, Overlay::None);
}

#[test]
fn r_with_a_long_default_window_asks_first() {
    let mut m = model_fixture();
    m.traffic.filter = TrafficFilter::defaults("30d", 1000);
    assert!(update(&mut m, key('r')).is_empty());
    assert!(matches!(m.overlay, Overlay::Confirm { .. }));
    assert!(is_bypass_run(&update(&mut m, key('y'))));
}

#[test]
fn an_identical_filter_does_not_restart_the_running_query() {
    use crate::tui::test_support::model_with_workloads;
    let mut m = model_fixture();
    let f = TrafficFilter::defaults("1h", 50);
    assert_eq!(update(&mut m, Msg::ApplyFilter(f.clone())).len(), 1);
    let seq = m.traffic.seq;
    let again = update(&mut m, Msg::ApplyFilter(f));
    assert!(again.is_empty(), "{again:?}");
    assert_eq!(m.status.text, "query already running");
    assert_eq!(m.traffic.seq, seq);
    assert!(m.traffic.is_running());
    // Key auto-repeat on `t` from Workloads: one query, not one per repeat.
    let mut m = model_with_workloads();
    update(&mut m, key('4'));
    assert_eq!(update(&mut m, key('t')).len(), 1);
    update(&mut m, key('4'));
    assert!(update(&mut m, key('t')).is_empty());
    assert_eq!(m.screen, Screen::Traffic);
}

#[test]
fn jump_uses_the_href_id_when_the_name_is_not_unique() {
    use crate::tui::test_support::model_with_workloads;
    let mut m = model_with_workloads();
    m.workloads.raw[1].hostname = m.workloads.raw[0].hostname.clone();
    m.workloads.rebuild(&m.limits.key_order());
    update(&mut m, key('4'));
    update(&mut m, key('t'));
    assert_eq!(m.traffic.filter.src, "wl:aaaa-1");
    update(&mut m, key('4'));
    update(&mut m, key('j'));
    update(&mut m, key('t'));
    assert_eq!(m.traffic.filter.src, "wl:aaaa-2");
    // A unique name stays readable.
    update(&mut m, key('4'));
    update(&mut m, key('j'));
    update(&mut m, key('T'));
    assert_eq!(m.traffic.filter.dst, "wl:unmanaged-nas");
}

#[test]
fn policy_d_and_p_do_not_double_a_load_in_flight() {
    use crate::tui::test_support::model_with_policy;
    let mut m = model_with_policy();
    update(&mut m, key('2'));
    assert_eq!(update(&mut m, key('r')).len(), 1);
    assert!(m.policy.loading);
    assert!(update(&mut m, key('d')).is_empty());
    assert_eq!(m.status.text, "already loading");
    assert_eq!(m.policy.scope, PolicyScope::Draft);

    let mut m = model_with_policy();
    update(&mut m, key('2'));
    assert_eq!(
        update(&mut m, key('p')),
        vec![Cmd::LoadPending(CacheMode::Use)]
    );
    assert!(m.policy.pending_loading);
    assert!(update(&mut m, key('p')).is_empty());
    update(&mut m, Msg::PendingLoaded(Err("boom".into())));
    assert!(!m.policy.pending_loading, "cleared on Err");
    assert_eq!(update(&mut m, key('p')).len(), 1);
    update(&mut m, Msg::PendingLoaded(Ok(Vec::new())));
    assert!(!m.policy.pending_loading, "cleared on Ok");
    m.policy.pending_loading = true;
    m.policy.clear();
    assert!(!m.policy.pending_loading, "reset by clear()");
}

#[test]
fn auto_refresh_ticks_only_while_connected() {
    use crate::tui::test_support::model_with_flows;
    use std::time::{Duration, Instant};
    let mut m = model_with_flows();
    update(&mut m, key('a'));
    let later = Instant::now() + Duration::from_secs(61);
    m.connection = Connection::Disconnected {
        error: "gone".into(),
    };
    assert!(update(&mut m, Msg::Tick(later)).is_empty());
    m.connection = Connection::Connecting;
    assert!(update(&mut m, Msg::Tick(later)).is_empty());
    m.connection = Connection::Connected {
        index: Some(crate::tui::test_support::fixture_index()),
        index_age_secs: 0,
    };
    assert!(is_bypass_run(&update(&mut m, Msg::Tick(later))));
}

#[test]
fn draft_and_active_commands_reuse_the_loaded_scope() {
    use crate::tui::commands::Command;
    use crate::tui::test_support::model_with_policy;
    let mut m = model_with_policy();
    m.screen = Screen::Traffic;
    let cmds = update(&mut m, Msg::Command(Command::Draft));
    assert!(cmds.is_empty(), "draft already loaded: {cmds:?}");
    assert_eq!(m.screen, Screen::Policy);
    assert!(!m.policy.rulesets.is_empty());
    assert_eq!(
        update(&mut m, Msg::Command(Command::Active)),
        vec![Cmd::LoadRuleSets {
            scope: PolicyScope::Active,
            mode: CacheMode::Use
        }]
    );
    // In flight: neither the same nor the other scope requests again.
    assert!(update(&mut m, Msg::Command(Command::Active)).is_empty());
    assert!(update(&mut m, Msg::Command(Command::Draft)).is_empty());
    assert_eq!(m.status.text, "already loading");
    assert_eq!(m.policy.scope, PolicyScope::Active);
}

#[test]
fn index_retry_from_either_screen_clears_both_flags() {
    let mut m = model_fixture();
    m.connection = Connection::Connected {
        index: None,
        index_age_secs: 0,
    };
    update(&mut m, Msg::IndexFailed("boom".into()));
    update(&mut m, key('3'));
    assert_eq!(
        update(&mut m, key('r')),
        vec![Cmd::LoadIndex(CacheMode::Use)]
    );
    assert!(!m.objects.index_failed && !m.workloads.index_failed);
    update(&mut m, Msg::IndexFailed("boom".into()));
    update(&mut m, key('4'));
    assert_eq!(
        update(&mut m, key('r')),
        vec![Cmd::LoadIndex(CacheMode::Use)]
    );
    assert!(!m.objects.index_failed && !m.workloads.index_failed);
}

#[test]
fn a_form_overlay_takes_every_key_and_esc_sends_nothing() {
    use crate::tui::edit::EditIntent;
    use crate::tui::form::{Field, Form};
    let mut m = model_fixture();
    m.overlay = Overlay::Form(Form::new(
        "New label",
        vec![
            Field::new("Key", "", "app", true),
            Field::new("Value", "", "web", true),
        ],
        EditIntent::CreateLabel,
    ));
    for c in "q2?".chars() {
        assert!(update(&mut m, key(c)).is_empty());
    }
    assert!(!m.quit, "q is typed, not a quit");
    assert_eq!(m.screen, Screen::Traffic);
    let Overlay::Form(f) = &m.overlay else {
        panic!("{:?}", m.overlay)
    };
    assert_eq!(f.value("Key"), "q2?");
    assert!(update(&mut m, kc(KeyCode::Esc)).is_empty());
    assert_eq!(m.overlay, Overlay::None);
    assert_eq!(m.status.text, "cancelled, nothing sent");
}
