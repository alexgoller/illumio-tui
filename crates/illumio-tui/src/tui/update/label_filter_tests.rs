use super::*;
use crate::tui::model::{Model, ObjectKind, Overlay, PendingAction};
use crate::tui::msg::{Cmd, Msg, ObjectsPayload};
use crate::tui::test_support::{model_with_objects, model_with_policy, with_workloads};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

fn kc(code: KeyCode) -> Msg {
    Msg::Key(KeyEvent::new(code, KeyModifiers::NONE))
}

/// Sends the key and asserts it asked for nothing.
#[track_caller]
fn press(m: &mut Model, code: KeyCode) {
    assert_eq!(update(m, kc(code)), Vec::<Cmd>::new());
}

#[track_caller]
fn type_text(m: &mut Model, text: &str) {
    for c in text.chars() {
        press(m, KeyCode::Char(c));
    }
}

/// The Labels tab with the fixture objects, nothing else cached.
fn on_labels() -> Model {
    let mut m = model_with_objects();
    assert!(update(&mut m, Msg::Key(KeyEvent::from(KeyCode::Char('3')))).is_empty());
    assert_eq!(m.objects.kind, ObjectKind::Labels);
    m
}

/// `(key, value)` of the shown rows.
fn shown(m: &Model) -> Vec<(String, String)> {
    let t = &m.objects.labels;
    t.visible()
        .iter()
        .map(|&i| (t.rows()[i].key.clone(), t.rows()[i].value.clone()))
        .collect()
}

/// `/` starts from the current search; it is cleared first.
fn search(m: &mut Model, text: &str) {
    press(m, KeyCode::Char('/'));
    for _ in 0..20 {
        press(m, KeyCode::Backspace);
    }
    type_text(m, text);
    press(m, KeyCode::Enter);
}

fn filter_keys(m: &mut Model, text: &str) {
    press(m, KeyCode::Char('f'));
    assert!(matches!(m.overlay, Overlay::LabelKeys(_)));
    // A printable key jumps to the text line; clear it first.
    for _ in 0..20 {
        press(m, KeyCode::Backspace);
    }
    type_text(m, text);
    press(m, KeyCode::Enter);
    assert_eq!(m.overlay, Overlay::None);
}

#[test]
fn f_opens_the_key_picker_with_label_counts_and_filters_the_tab() {
    let mut m = on_labels();
    press(&mut m, KeyCode::Char('f'));
    let Overlay::LabelKeys(p) = &m.overlay else {
        panic!("{:?}", m.overlay)
    };
    let listed: Vec<(&str, Option<usize>)> =
        p.entries.iter().map(|e| (e.key.as_str(), e.seen)).collect();
    // The profile's scope keys and tier key first, then the other keys
    // the index knows with their label counts.
    assert_eq!(
        &listed[..3],
        &[("app", None), ("env", None), ("role", None)]
    );
    assert!(listed.contains(&("loc", Some(1))), "{listed:?}");
    assert!(p.all, "no filter: all keys checked");
    press(&mut m, KeyCode::Esc);
    assert_eq!(m.objects.labels.len(), 6, "Esc changes nothing");

    filter_keys(&mut m, "role,env");
    assert_eq!(m.objects.label_keys, vec!["role", "env"]);
    assert_eq!(
        shown(&m),
        vec![
            ("role".into(), "web".into()),
            ("role".into(), "db".into()),
            ("env".into(), "prod".into())
        ]
    );
    // `r` keeps the filter: the reload's rows go through it.
    let labels = m.objects.all_labels.clone();
    update(
        &mut m,
        Msg::ObjectsLoaded {
            kind: ObjectKind::Labels,
            result: Ok(ObjectsPayload::Labels(labels, 0)),
        },
    );
    assert_eq!(m.objects.labels.len(), 3, "the filter survives a reload");
    // `all keys` clears it.
    press(&mut m, KeyCode::Char('f'));
    let Overlay::LabelKeys(p) = &m.overlay else {
        unreachable!()
    };
    let all_row = p.all_row();
    for _ in 0..all_row {
        press(&mut m, KeyCode::Down);
    }
    press(&mut m, KeyCode::Char(' '));
    press(&mut m, KeyCode::Enter);
    assert!(m.objects.label_keys.is_empty());
    assert_eq!(m.objects.labels.len(), 6);
}

#[test]
fn search_within_a_key_filter_and_value_only_searches() {
    let mut m = on_labels();
    search(&mut m, "=e");
    assert_eq!(
        shown(&m),
        vec![("role".into(), "web".into()), ("loc".into(), "eu".into())],
        "=text matches values only, not the key env"
    );
    search(&mut m, "=PRO");
    assert_eq!(shown(&m), vec![("env".into(), "prod".into())]);
    filter_keys(&mut m, "role");
    search(&mut m, "=e");
    assert_eq!(shown(&m), vec![("role".into(), "web".into())]);
    search(&mut m, "d");
    assert_eq!(
        shown(&m),
        vec![("role".into(), "db".into())],
        "a plain search stays within the key filter"
    );
    search(&mut m, "app=sh");
    assert_eq!(
        shown(&m),
        vec![("app".into(), "shop".into())],
        "key=text names its key, outside the filter too"
    );
    search(&mut m, "app=");
    assert_eq!(shown(&m).len(), 2, "every app label, the deleted one too");
    press(&mut m, KeyCode::Char('/'));
    press(&mut m, KeyCode::Esc);
    assert_eq!(shown(&m).len(), 2, "back to the key filter alone");
}

#[test]
fn i_toggles_the_ignored_keys_and_counts_them() {
    let mut m = on_labels();
    press(&mut m, KeyCode::Char('i'));
    assert_eq!(m.status.text, "the profile has no ignore_keys");
    assert_eq!(m.objects.labels.len(), 6);
    m.set_label_model(
        vec!["app".into(), "env".into()],
        "role".into(),
        vec!["loc".into()],
    );
    assert!(m.objects.hide_ignored, "hidden by default once set");
    assert_eq!(m.objects.ignored_hidden, 1);
    assert_eq!(m.objects.labels.len(), 5);
    assert!(m.objects.labels_filtered());
    press(&mut m, KeyCode::Char('i'));
    assert_eq!((m.objects.labels.len(), m.objects.ignored_hidden), (6, 0));
    assert!(!m.objects.labels_filtered());
    press(&mut m, KeyCode::Char('i'));
    assert_eq!((m.objects.labels.len(), m.objects.ignored_hidden), (5, 1));
    // A profile switch starts with the new profile's default and no key filter.
    m.objects.label_keys = vec!["role".into()];
    m.objects.hide_ignored = false;
    m.reset_data();
    assert!(m.objects.hide_ignored && m.objects.label_keys.is_empty());
}

#[test]
fn u_without_workloads_asks_first_and_sends_nothing_until_y() {
    // A PCE without usage flags: our counts need workloads and policy.
    let mut m = older_pce(on_labels());
    press(&mut m, KeyCode::Char('u'));
    assert_eq!(
        m.status.text,
        "unused needs workloads and policy loaded: press y to load both \
         (policy may also fetch the selected ruleset's deny rules)"
    );
    assert!(matches!(
        m.overlay,
        Overlay::Confirm {
            action: PendingAction::LoadLabelUsage,
            ..
        }
    ));
    press(&mut m, KeyCode::Char('n'));
    assert_eq!(m.overlay, Overlay::None);
    assert!(!m.objects.unused_only);
    press(&mut m, KeyCode::Char('u'));
    press(&mut m, KeyCode::Esc);
    assert!(!m.objects.unused_only, "Esc cancels too");

    press(&mut m, KeyCode::Char('u'));
    let cmds = update(&mut m, kc(KeyCode::Char('y')));
    assert_eq!(
        cmds,
        vec![
            Cmd::LoadWorkloads(pce::CacheMode::Use),
            Cmd::LoadRuleSets {
                scope: pce::models::PolicyScope::Draft,
                mode: pce::CacheMode::Use
            }
        ],
        "the two screens' own loads, nothing new"
    );
    assert!(m.objects.unused_only);
    assert_eq!(m.objects.labels.len(), 6, "not applied before the data");
    press(&mut m, KeyCode::Char('u'));
    assert!(!m.objects.unused_only, "u while loading turns it off again");
    press(&mut m, KeyCode::Char('u'));
    assert!(
        m.objects.unused_only,
        "both on their way: no second confirm"
    );
    assert_eq!(m.overlay, Overlay::None);

    let workloads: Vec<pce::models::Workload> =
        serde_json::from_str(include_str!("../../../../../fixtures/workloads.json")).unwrap();
    let rule_sets: Vec<pce::models::RuleSet> =
        serde_json::from_str(include_str!("../../../../../fixtures/rule_sets.json")).unwrap();
    update(&mut m, Msg::WorkloadsLoaded(Ok((workloads, 0, false))));
    assert!(
        m.objects.label_usage.is_none(),
        "the policy is still missing"
    );
    update(
        &mut m,
        Msg::RuleSetsLoaded {
            scope: pce::models::PolicyScope::Draft,
            result: Ok((rule_sets, 0, false)),
        },
    );
    assert!(m.objects.label_usage.is_some());
    assert_eq!(
        shown(&m),
        vec![("loc".into(), "eu".into()), ("app".into(), String::new())]
    );
}

#[test]
fn with_everything_cached_u_applies_at_once_and_the_counts_sort() {
    let mut m = with_workloads(model_with_policy());
    let objects = model_with_objects().objects;
    m.objects = objects;
    m.objects.pce_usage = PceUsage::Unsupported;
    m.recount_labels();
    assert!(update(&mut m, Msg::Key(KeyEvent::from(KeyCode::Char('3')))).is_empty());
    let row = |m: &Model, i: usize| {
        let t = &m.objects.labels;
        t.rows()[t.visible()[i]].clone()
    };
    let u = row(&m, 0).usage.unwrap();
    assert_eq!((u.workloads, u.rules), (1, 2), "role=web");
    // `s` cycles to the workloads column, descending on the second press.
    for _ in 0..6 {
        press(&mut m, KeyCode::Char('s'));
    }
    assert_eq!(m.objects.labels.sort(), Some((2, true)));
    assert_eq!(row(&m, 0).usage.unwrap().workloads, 2);
    press(&mut m, KeyCode::Char('u'));
    assert_eq!(m.overlay, Overlay::None, "nothing to load, nothing to ask");
    assert_eq!(
        m.status.text,
        "2 unused labels (usage from workloads, rules and scopes only)"
    );
    assert_eq!(m.objects.labels.len(), 2);
    press(&mut m, KeyCode::Char('u'));
    assert_eq!(m.objects.labels.len(), 6);
    // The other Labels-tab keys stay local too.
    press(&mut m, KeyCode::Char('f'));
    press(&mut m, KeyCode::Esc);
    press(&mut m, KeyCode::Char('i'));
}

/// `m` as from a PCE that answered `usage=true` without usage flags.
fn older_pce(mut m: Model) -> Model {
    m.objects.pce_usage = PceUsage::Unsupported;
    m
}

/// The fixture labels with the PCE's usage flags: loc=eu is held by a
/// pairing profile and a permission, the deleted app label by nothing.
fn usage_answer() -> Msg {
    Msg::LabelUsageLoaded(Ok(serde_json::from_str(include_str!(
        "../../../../../fixtures/labels_usage.json"
    ))
    .unwrap()))
}

/// Workloads, policy and objects cached, usage counted, on the Labels tab.
fn all_cached() -> Model {
    let mut m = with_workloads(model_with_policy());
    m.objects = model_with_objects().objects;
    m.recount_labels();
    assert!(update(&mut m, Msg::Key(KeyEvent::from(KeyCode::Char('3')))).is_empty());
    m
}

#[test]
fn a_truncated_workload_list_is_named_in_the_header_and_the_status() {
    use crate::tui::model::StatusKind;
    use crate::tui::test_support::render_to_string;
    let mut m = older_pce(all_cached());
    m.workloads.truncated = true;
    m.recount_labels();
    press(&mut m, KeyCode::Char('u'));
    assert_eq!(m.status.kind, StatusKind::Warn);
    assert_eq!(
        m.status.text,
        "2 unused labels; the workload list is truncated, some may be in use"
    );
    let out = render_to_string(&m, 100, 24);
    assert!(out.contains("unused (workloads truncated)"), "{out}");
    m.workloads.truncated = false;
    m.recount_labels();
    let out = render_to_string(&m, 100, 24);
    assert!(
        out.contains("unused (workloads, rules and scopes only)"),
        "{out}"
    );
}

#[test]
fn u_asks_the_pce_once_and_its_flags_decide() {
    use crate::tui::test_support::render_to_string;
    let mut m = on_labels();
    assert_eq!(
        update(&mut m, kc(KeyCode::Char('u'))),
        vec![Cmd::LoadLabelUsage(pce::CacheMode::Use)],
        "one request, no confirm, no workloads or policy"
    );
    assert_eq!(m.overlay, Overlay::None);
    assert!(m.objects.unused_only);
    assert_eq!(m.objects.labels.len(), 6, "not applied before the answer");
    assert_eq!(
        m.status.text,
        "unused labels: asking the PCE what holds each label"
    );
    assert!(render_to_string(&m, 100, 24).contains("unused: loading"));
    // Off and on again while asking: no second request.
    press(&mut m, KeyCode::Char('u'));
    assert!(update(&mut m, kc(KeyCode::Char('u'))).is_empty());

    assert!(update(&mut m, usage_answer()).is_empty());
    assert_eq!(
        shown(&m),
        vec![("app".into(), String::new())],
        "loc=eu, unused by our counts, is held by a pairing profile"
    );
    assert_eq!(m.status.text, "1 unused labels");
    let out = render_to_string(&m, 100, 24);
    assert!(out.contains("· unused ·"), "{out}");

    // Off: every label, each with what holds it.
    press(&mut m, KeyCode::Char('u'));
    assert_eq!(m.objects.labels.len(), 6);
    let loc = m
        .objects
        .labels
        .rows()
        .iter()
        .find(|r| r.key == "loc")
        .unwrap()
        .clone();
    assert_eq!(loc.held_by_text(), "pairing_profile,permission");
    m.objects.show_detail = true;
    while m.objects.labels.selected_item().map(|r| r.key.as_str()) != Some("loc") {
        press(&mut m, KeyCode::Down);
    }
    let out = render_to_string(&m, 160, 48);
    assert!(out.contains("held by pairing_profile, permission"), "{out}");
    assert!(
        out.contains("pairing_profile,permission"),
        "the wide table's held by column: {out}"
    );

    // On again: the flags in hand apply at once; the ask is a cache read.
    assert_eq!(
        update(&mut m, kc(KeyCode::Char('u'))),
        vec![Cmd::LoadLabelUsage(pce::CacheMode::Use)]
    );
    assert_eq!(m.objects.labels.len(), 1);
}

#[test]
fn a_pce_without_usage_flags_falls_back_to_our_counts_and_says_so() {
    use crate::tui::test_support::render_to_string;
    // Everything cached: the fallback applies as soon as the answer is in.
    let mut m = all_cached();
    assert_eq!(
        update(&mut m, kc(KeyCode::Char('u'))),
        vec![Cmd::LoadLabelUsage(pce::CacheMode::Use)]
    );
    let plain: Vec<pce::models::Label> =
        serde_json::from_str(include_str!("../../../../../fixtures/labels.json")).unwrap();
    assert!(update(&mut m, Msg::LabelUsageLoaded(Ok(plain.clone()))).is_empty());
    assert_eq!(m.objects.pce_usage, PceUsage::Unsupported);
    assert_eq!(
        shown(&m),
        vec![("loc".into(), "eu".into()), ("app".into(), String::new())]
    );
    assert_eq!(
        m.status.text,
        "2 unused labels (usage from workloads, rules and scopes only)"
    );
    let out = render_to_string(&m, 100, 24);
    assert!(
        out.contains("unused (workloads, rules and scopes only)"),
        "{out}"
    );
    // The PCE has said it has none: no more asking.
    press(&mut m, KeyCode::Char('u'));
    assert!(update(&mut m, kc(KeyCode::Char('u'))).is_empty());
    assert_eq!(m.objects.labels.len(), 2);

    // Nothing cached: the answer leads to the usual confirm.
    let mut m = on_labels();
    update(&mut m, kc(KeyCode::Char('u')));
    assert!(update(&mut m, Msg::LabelUsageLoaded(Ok(plain))).is_empty());
    assert!(!m.objects.unused_only);
    assert!(matches!(
        m.overlay,
        Overlay::Confirm {
            action: PendingAction::LoadLabelUsage,
            ..
        }
    ));
}

/// A successful write of `kind`.
fn wrote(kind: pce::write::ObjectKind) -> Msg {
    Msg::WriteDone {
        kind,
        method: Some(pce::WriteMethod::Put),
        result: Ok(pce::write::Outcome::Written(pce::write::Written {
            href: "/orgs/5/workloads/1".into(),
            pending: false,
            status: 200,
            object: serde_json::json!({"href": "/orgs/5/workloads/1"}),
            dropped_fields: vec![],
            warnings: vec![],
            audit_failed: None,
        })),
    }
}

#[test]
fn a_write_forgets_the_usage_flags_and_u_asks_again_past_the_cache() {
    use crate::tui::test_support::render_to_string;
    let mut m = on_labels();
    update(&mut m, kc(KeyCode::Char('u')));
    update(&mut m, usage_answer());
    assert_eq!(m.objects.labels.len(), 1);

    // A workload write: the flags go, `u` on asks again, counted.
    let cmds = update(&mut m, wrote(pce::write::ObjectKind::Workload));
    assert!(
        cmds.contains(&Cmd::LoadLabelUsage(pce::CacheMode::Bypass)),
        "{cmds:?}"
    );
    assert_eq!(m.objects.pce_usage, PceUsage::Unknown);
    assert!(m.objects.pce_usage_loading && m.objects.unused_only);
    assert_eq!(m.objects.labels.len(), 6, "undecided until the answer");
    assert!(render_to_string(&m, 100, 24).contains("unused: loading"));
    update(&mut m, usage_answer());
    assert_eq!(m.objects.labels.len(), 1);

    // `u` off: a rule write forgets the flags but asks for nothing.
    press(&mut m, KeyCode::Char('u'));
    let cmds = update(&mut m, wrote(pce::write::ObjectKind::Rule));
    assert!(
        !cmds.iter().any(|c| matches!(c, Cmd::LoadLabelUsage(_))),
        "{cmds:?}"
    );
    assert_eq!(m.objects.pce_usage, PceUsage::Unknown);
    assert!(m.objects.labels.rows().iter().all(|r| r.held_by.is_none()));

    // A PCE without flags stays so: nothing to forget, nothing to ask.
    let mut m = older_pce(on_labels());
    let cmds = update(&mut m, wrote(pce::write::ObjectKind::Label));
    assert_eq!(m.objects.pce_usage, PceUsage::Unsupported);
    assert!(!cmds.iter().any(|c| matches!(c, Cmd::LoadLabelUsage(_))));
}

#[test]
fn r_on_the_labels_tab_reads_held_flags_again() {
    let mut m = on_labels();
    let r = |m: &mut Model| update(m, kc(KeyCode::Char('r')));
    assert_eq!(
        r(&mut m),
        vec![Cmd::LoadObjects {
            kind: ObjectKind::Labels,
            mode: pce::CacheMode::Bypass
        }],
        "no flags held: the labels alone"
    );
    let labels = m.objects.all_labels.clone();
    update(
        &mut m,
        Msg::ObjectsLoaded {
            kind: ObjectKind::Labels,
            result: Ok(crate::tui::msg::ObjectsPayload::Labels(labels, 0)),
        },
    );
    update(&mut m, kc(KeyCode::Char('u')));
    update(&mut m, usage_answer());
    press(&mut m, KeyCode::Char('u'));
    assert_eq!(
        r(&mut m),
        vec![
            Cmd::LoadObjects {
                kind: ObjectKind::Labels,
                mode: pce::CacheMode::Bypass
            },
            Cmd::LoadLabelUsage(pce::CacheMode::Bypass)
        ]
    );
    assert!(m.objects.pce_usage_loading);
}

#[test]
fn empty_usage_flags_are_no_flags_never_unused() {
    let mut labels: Vec<pce::models::Label> =
        serde_json::from_str(include_str!("../../../../../fixtures/labels.json")).unwrap();
    for l in &mut labels {
        l.usage = Some(pce::models::LabelUsageFlags::default());
    }
    assert_eq!(
        PceUsage::from_labels(labels.clone()),
        PceUsage::Unsupported,
        "an empty map says nothing: our counts decide"
    );
    let row = crate::tui::model::LabelRow {
        label: labels[0].clone(),
        usage: None,
        held_by: Some(pce::models::LabelUsageFlags::default()),
        in_keys: true,
    };
    assert!(!row.is_unused(true));
}

#[test]
fn a_failed_usage_ask_turns_u_off_and_says_why() {
    let mut m = on_labels();
    update(&mut m, kc(KeyCode::Char('u')));
    update(&mut m, Msg::LabelUsageLoaded(Err("HTTP 500".into())));
    assert!(!m.objects.unused_only);
    assert!(!m.objects.pce_usage_loading);
    assert_eq!(m.objects.pce_usage, PceUsage::Unknown);
    assert_eq!(m.status.text, "label usage: HTTP 500");
    assert_eq!(m.objects.labels.len(), 6);
}

#[test]
fn fallback_deny_rules_count_once_loaded() {
    let mut m = older_pce(all_cached());
    let loc = |m: &Model| {
        m.objects
            .label_usage
            .as_ref()
            .unwrap()
            .get("/orgs/5/labels/5")
            .copied()
    };
    assert_eq!(loc(&m), None, "loc=eu is unused so far");
    let deny: Vec<pce::models::DenyRule> = serde_json::from_value(serde_json::json!([{
        "href": "/orgs/5/sec_policy/draft/rule_sets/101/deny_rules/9",
        "enabled": true,
        "providers": [{"label": {"href": "/orgs/5/labels/5"}}],
        "consumers": [{"actors": "ams"}],
        "ingress_services": [],
        "override": false
    }]))
    .unwrap();
    update(
        &mut m,
        Msg::DenyRulesLoaded {
            href: "/orgs/5/sec_policy/draft/rule_sets/101".into(),
            result: Ok(deny),
        },
    );
    assert_eq!(loc(&m).map(|u| u.rules), Some(1));
    press(&mut m, KeyCode::Char('u'));
    assert_eq!(
        shown(&m),
        vec![("app".into(), String::new())],
        "only the deleted app label is left unused"
    );
}

#[test]
fn u_names_only_what_is_missing_and_leaves_the_filter_off_on_a_gap() {
    // Workloads cached, policy not: the confirm asks for policy alone.
    let mut m = older_pce(with_workloads(model_with_objects()));
    assert!(update(&mut m, Msg::Key(KeyEvent::from(KeyCode::Char('3')))).is_empty());
    press(&mut m, KeyCode::Char('u'));
    assert_eq!(
        m.status.text,
        "unused needs policy loaded: press y to load policy \
         (it may also fetch the selected ruleset's deny rules)"
    );
    assert_eq!(
        update(&mut m, kc(KeyCode::Char('y'))),
        vec![Cmd::LoadRuleSets {
            scope: pce::models::PolicyScope::Draft,
            mode: pce::CacheMode::Use
        }]
    );
    assert!(m.objects.unused_only);

    // Workloads and policy cached, label groups never loaded: nothing
    // to ask, nothing coming, the filter stays off.
    let mut m = older_pce(all_cached());
    m.objects.unused_only = false;
    m.objects.ages[ObjectKind::LabelGroups.index()] = None;
    m.recount_labels();
    press(&mut m, KeyCode::Char('u'));
    assert!(!m.objects.unused_only);
    assert_eq!(m.status.text, "unused labels need label groups loaded");
    assert_eq!(m.overlay, Overlay::None);

    // No name index: `y` loads nothing, so the filter stays off.
    let mut m = older_pce(on_labels());
    if let crate::tui::model::Connection::Connected { index, .. } = &mut m.connection {
        *index = None;
    }
    press(&mut m, KeyCode::Char('u'));
    assert_eq!(update(&mut m, kc(KeyCode::Char('y'))), Vec::<Cmd>::new());
    assert!(!m.objects.unused_only);
    assert!(
        m.status
            .text
            .starts_with("unused labels need workloads and policy")
            && m.status.text.ends_with("(no name index yet)"),
        "{}",
        m.status.text
    );
}

#[test]
fn the_key_picker_leaves_out_hidden_ignored_keys() {
    let mut m = on_labels();
    m.set_label_model(
        vec!["app".into(), "env".into()],
        "role".into(),
        vec!["loc".into()],
    );
    let offered = |m: &mut Model| {
        press(m, KeyCode::Char('f'));
        let Overlay::LabelKeys(p) = &m.overlay else {
            panic!("{:?}", m.overlay)
        };
        let keys: Vec<String> = p.entries.iter().map(|e| e.key.clone()).collect();
        press(m, KeyCode::Esc);
        keys
    };
    assert!(!offered(&mut m).contains(&"loc".to_string()));
    press(&mut m, KeyCode::Char('i'));
    assert!(offered(&mut m).contains(&"loc".to_string()));
}
