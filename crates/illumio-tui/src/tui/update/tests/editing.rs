use super::*;

// ---- Editing keys (Plan 4) ----
#[test]
fn editing_keys_with_writes_off_only_say_why() {
    use crate::tui::test_support::{model_with_objects, model_with_workloads};
    let mut m = model_with_objects();
    m.screen = Screen::Objects;
    for c in ['n', 'e', 'x'] {
        assert!(update(&mut m, key(c)).is_empty(), "{c}");
        assert_eq!(m.status.kind, StatusKind::Warn);
        assert_eq!(m.status.text, crate::tui::model::WRITES_OFF);
        assert_eq!(m.overlay, Overlay::None);
    }
    let mut m = model_with_workloads();
    m.screen = Screen::Workloads;
    for c in ['n', 'e', 'X'] {
        assert!(update(&mut m, key(c)).is_empty(), "{c}");
        assert_eq!(m.status.text, crate::tui::model::WRITES_OFF);
        assert_eq!(m.overlay, Overlay::None);
    }
    let mut m = crate::tui::test_support::model_with_ip_lists();
    crate::tui::test_support::on_ip_lists_tab(&mut m);
    for c in ['n', 'e', 'x'] {
        m.status = StatusLine::info("ready");
        assert!(update(&mut m, key(c)).is_empty(), "{c}");
        assert_eq!(m.status.kind, StatusKind::Warn, "{c}");
        assert_eq!(m.status.text, crate::tui::model::WRITES_OFF, "{c}");
        assert_eq!(m.overlay, Overlay::None);
    }
}

#[test]
fn ip_list_edit_keys_without_a_selected_row_say_so() {
    let mut m = writes_on(model_fixture());
    crate::tui::test_support::on_ip_lists_tab(&mut m);
    assert!(m.objects.ip_lists.table.is_empty());
    for c in ['e', 'x'] {
        m.status = StatusLine::info("ready");
        assert!(update(&mut m, key(c)).is_empty(), "{c}");
        assert_eq!(m.status.kind, StatusKind::Warn, "{c}");
        assert_eq!(m.status.text, "no row selected", "{c}");
        assert_eq!(m.overlay, Overlay::None);
    }
}

#[test]
fn label_groups_and_managed_workloads_are_refused_before_any_form() {
    use crate::tui::edit::WriteIntent;
    use crate::tui::model::ObjectKind;
    use crate::tui::test_support::{model_with_objects, model_with_workloads};
    let mut m = writes_on(model_with_objects());
    m.screen = Screen::Objects;
    m.objects.kind = ObjectKind::LabelGroups;
    for c in ['n', 'e', 'x'] {
        assert!(update(&mut m, key(c)).is_empty(), "{c}");
        assert_eq!(m.status.text, "label groups are read-only in this version");
        assert_eq!(m.overlay, Overlay::None);
    }
    let mut m = writes_on(model_with_workloads());
    m.screen = Screen::Workloads;
    // Row 0 is host-1.example, VEN-managed.
    for c in ['e', 'X'] {
        assert!(update(&mut m, key(c)).is_empty(), "{c}");
        assert_eq!(m.status.text, "only unmanaged workloads can be edited here");
        assert_eq!(m.overlay, Overlay::None);
    }
    update(&mut m, key('j'));
    update(&mut m, key('j')); // unmanaged-nas
    assert_eq!(
        update(&mut m, key('X')),
        vec![Cmd::PlanWrite(WriteIntent::DeleteWorkload {
            href: "/orgs/5/workloads/aaaa-3".into(),
            name: "unmanaged-nas".into(),
        })],
        "a delete plans at once, the confirm overlay comes next"
    );
    // Editing waits for the plan's answer.
    update(&mut m, Msg::Planned(Err("not found".into())));
    assert!(update(&mut m, key('e')).is_empty());
    assert!(
        matches!(&m.overlay, Overlay::Form(f) if f.title == "Edit workload unmanaged-nas"),
        "{:?}",
        m.overlay
    );
}

#[test]
fn ip_list_n_and_x_plan_through_the_plan_4_intents() {
    use crate::tui::edit::WriteIntent;
    use crate::tui::test_support::model_with_ip_lists;
    let mut m = writes_on(model_with_ip_lists());
    crate::tui::test_support::on_ip_lists_tab(&mut m);
    assert!(update(&mut m, key('n')).is_empty());
    let Overlay::Form(f) = &m.overlay else {
        panic!("{:?}", m.overlay)
    };
    assert_eq!(f.title, "New IP list");
    quiet(&mut m, kc(KeyCode::Esc));
    assert_eq!(m.overlay, Overlay::None);
    quiet(&mut m, key('j'));
    assert_eq!(
        update(&mut m, key('x')),
        vec![Cmd::PlanWrite(WriteIntent::DeleteIpList {
            href: "/orgs/5/sec_policy/draft/ip_lists/21".into(),
            name: "Corp".into(),
        })]
    );
    assert!(m.writes.in_flight);
}

#[test]
fn used_by_follows_the_rulesets_as_they_load_and_switch_scope() {
    let mut m = model_fixture();
    crate::tui::test_support::on_ip_lists_tab(&mut m);
    let lists =
        serde_json::from_str(include_str!("../../../../../../fixtures/ip_lists.json")).unwrap();
    update(&mut m, Msg::IpListsLoaded(Ok((lists, 0))));
    let used = |m: &Model| {
        m.objects
            .ip_lists
            .table
            .rows()
            .iter()
            .map(|r| r.used_by)
            .collect::<Vec<_>>()
    };
    assert_eq!(used(&m), vec![None, None]);
    let raw =
        serde_json::from_str(include_str!("../../../../../../fixtures/rule_sets.json")).unwrap();
    update(
        &mut m,
        Msg::RuleSetsLoaded {
            scope: PolicyScope::Draft,
            result: Ok((raw, 0, false)),
        },
    );
    assert_eq!(used(&m), vec![Some(1), Some(1)]);
    // Global-Infra's deny rules come from the endpoint; one names Corp.
    let fetched: pce::models::DenyRule = serde_json::from_value(serde_json::json!({
        "href": "/orgs/5/sec_policy/draft/rule_sets/101/deny_rules/9",
        "providers": [{"actors": "ams"}],
        "consumers": [{"ip_list": {"href": "/orgs/5/sec_policy/draft/ip_lists/21"}}],
        "ingress_services": [{"port": 23, "proto": 6}]
    }))
    .unwrap();
    update(
        &mut m,
        Msg::DenyRulesLoaded {
            href: "/orgs/5/sec_policy/draft/rule_sets/101".into(),
            result: Ok(vec![fetched]),
        },
    );
    assert_eq!(used(&m), vec![Some(1), Some(2)]);
    m.screen = Screen::Policy;
    assert_eq!(
        update(&mut m, key('d')),
        vec![Cmd::LoadRuleSets {
            scope: PolicyScope::Active,
            mode: pce::CacheMode::Use,
        }],
        "the first switch to active loads it"
    );
    assert_eq!(
        used(&m),
        vec![None, None],
        "the active scope is not counted"
    );
}

#[test]
fn e_opens_a_prefilled_form_and_enter_plans_only_the_change() {
    use crate::tui::edit::WriteIntent;
    use crate::tui::test_support::model_with_ip_lists;
    let mut m = writes_on(model_with_ip_lists());
    crate::tui::test_support::on_ip_lists_tab(&mut m);
    quiet(&mut m, key('j')); // Corp
    assert!(update(&mut m, key('e')).is_empty());
    let Overlay::Form(f) = &m.overlay else {
        panic!("{:?}", m.overlay)
    };
    assert_eq!(f.value("Ranges"), "10.1.0.0-10.1.255.255");
    // Unchanged: the form closes, nothing is planned.
    assert!(update(&mut m, enter()).is_empty());
    assert_eq!(m.status.text, "no changes");
    assert_eq!(m.overlay, Overlay::None);
    quiet(&mut m, key('e'));
    quiet(&mut m, kc(KeyCode::Tab));
    quiet(&mut m, kc(KeyCode::Tab));
    for c in ", 10.2.0.0/16".chars() {
        quiet(&mut m, key(c));
    }
    let cmds = update(&mut m, enter());
    assert_eq!(
        cmds,
        vec![Cmd::PlanWrite(WriteIntent::UpdateIpList {
            href: "/orgs/5/sec_policy/draft/ip_lists/21".into(),
            patch: pce::write::IpListPatch {
                add_ranges: vec![pce::write::IpRangeSpec::parse("10.2.0.0/16", false).unwrap()],
                ..Default::default()
            },
        })]
    );
    assert_eq!(m.overlay, Overlay::None);
    assert_eq!(m.status.text, "planning…");
}

#[test]
fn n_opens_an_empty_form_that_validates_and_esc_cancels() {
    use crate::tui::model::ObjectKind;
    use crate::tui::test_support::model_with_objects;
    let mut m = writes_on(model_with_objects());
    m.screen = Screen::Objects;
    m.objects.kind = ObjectKind::Services;
    assert!(update(&mut m, key('n')).is_empty());
    assert!(update(&mut m, enter()).is_empty());
    let Overlay::Form(f) = &m.overlay else {
        panic!("{:?}", m.overlay)
    };
    assert_eq!(f.error.as_deref(), Some("Name is required"));
    assert_eq!(f.active, 0);
    for c in "SSH".chars() {
        update(&mut m, key(c));
    }
    update(&mut m, kc(KeyCode::Tab));
    update(&mut m, kc(KeyCode::Tab));
    for c in "22".chars() {
        update(&mut m, key(c));
    }
    assert!(
        update(&mut m, enter()).is_empty(),
        "a bare port is refused locally"
    );
    let Overlay::Form(f) = &m.overlay else {
        panic!("{:?}", m.overlay)
    };
    assert!(
        f.error.as_deref().unwrap().contains("22/tcp"),
        "{:?}",
        f.error
    );
    assert!(update(&mut m, kc(KeyCode::Esc)).is_empty());
    assert_eq!(m.overlay, Overlay::None);
    assert_eq!(m.status.text, "cancelled, nothing sent");
}

#[test]
fn policy_and_traffic_edit_keys_wait_for_a_write_in_flight() {
    use crate::tui::help::HELP_GROUPS;
    use crate::tui::test_support::{model_with_flows, model_with_policy};
    let busy = "a write is already in progress";
    let mut m = writes_on(model_with_policy());
    m.screen = Screen::Policy;
    m.writes.in_flight = true;
    for c in ['n', 'e', 'x'] {
        assert!(update(&mut m, key(c)).is_empty(), "{c}");
        assert_eq!(m.status.text, busy);
        assert_eq!(m.overlay, Overlay::None);
    }
    update(&mut m, kc(KeyCode::Tab));
    for c in ['n', 'N', 'e', 'x'] {
        assert!(update(&mut m, key(c)).is_empty(), "{c}");
        assert_eq!(m.status.text, busy);
        assert_eq!(m.overlay, Overlay::None);
    }
    m.policy.pending = Some(pending_fixture());
    m.overlay = Overlay::Pending;
    assert!(update(&mut m, key('P')).is_empty());
    assert_eq!(m.status.text, busy);
    assert_eq!(m.overlay, Overlay::Pending);
    let mut t = writes_on(model_with_flows());
    t.writes.in_flight = true;
    update(&mut t, key('j'));
    assert!(update(&mut t, key('w')).is_empty());
    assert_eq!(t.status.text, busy);
    assert_eq!(t.overlay, Overlay::None);
    // Every new key is in the help, in its group.
    for (group, k) in [
        ("Traffic", "w"),
        ("Forms", "Ctrl-A Ctrl-R"),
        ("Forms", "Ruleset"),
        ("Forms", "Ctrl-N Ctrl-P"),
        ("Policy", "N"),
        ("Pending", "j/k Space Enter"),
        ("Pending", "P"),
        ("Pending", "R"),
    ] {
        assert!(
            HELP_GROUPS
                .iter()
                .any(|g| g.title == group && g.lines.iter().any(|l| l.key == k)),
            "{group} {k}"
        );
    }
}

#[test]
fn help_scrolls_to_its_last_line_and_reopens_at_the_top() {
    use crate::tui::help::{HelpTopic, page_rows};
    use crate::tui::ui::widgets::overlay::help_last_scroll;
    let mut m = model_fixture(); // 80x24
    update(&mut m, key('?'));
    assert_eq!(HelpContext::of(&m), HelpContext::Flows);
    // The Flows page: Flows, Sections, Traffic, Marks, Everywhere.
    let rows = page_rows(HelpPage::Context, HelpContext::Flows).len();
    let last = help_last_scroll(&m, m.size);
    assert_eq!(last, rows - 21, "21 rows inside the box above its footer");
    for _ in 0..rows + 5 {
        update(&mut m, key('j'));
    }
    assert_eq!(m.help_scroll, last, "clamped to the page");
    update(&mut m, key('k'));
    assert_eq!(m.help_scroll, last - 1);
    update(&mut m, key('G'));
    assert_eq!(m.help_scroll, last);
    update(&mut m, key('g'));
    assert_eq!(m.help_scroll, 0);
    update(&mut m, kc(KeyCode::PageDown));
    assert_eq!(m.help_scroll, 21.min(last));
    update(&mut m, kc(KeyCode::PageUp));
    assert_eq!(m.help_scroll, 0);
    // Tab: one group at a time from the one after Flows; scroll resets.
    update(&mut m, key('G'));
    update(&mut m, kc(KeyCode::Tab));
    assert_eq!(m.help_page, HelpPage::Group(HelpTopic::AppToApp));
    assert_eq!(m.help_scroll, 0);
    assert_eq!(help_last_scroll(&m, m.size), 0, "one short group fits");
    update(&mut m, key('j'));
    assert_eq!(m.help_scroll, 0, "nothing to scroll");
    update(&mut m, kc(KeyCode::BackTab));
    update(&mut m, kc(KeyCode::BackTab));
    assert_eq!(m.help_page, HelpPage::Group(HelpTopic::Traffic));
    // a: every group; again: the context page.
    update(&mut m, key('a'));
    assert_eq!(m.help_page, HelpPage::All);
    update(&mut m, key('G'));
    assert!(m.help_scroll > last, "all groups are longer");
    update(&mut m, key('a'));
    assert_eq!(m.help_page, HelpPage::Context);
    assert_eq!(m.help_scroll, 0);
    // Other keys are swallowed.
    let screen = m.screen;
    update(&mut m, key('2'));
    assert_eq!(m.screen, screen);
    assert_eq!(m.overlay, Overlay::Help);
    update(&mut m, key('a'));
    update(&mut m, key('G'));
    update(&mut m, kc(KeyCode::Esc));
    assert_eq!(m.overlay, Overlay::None);
    update(&mut m, key('?'));
    assert_eq!(m.help_scroll, 0, "the help reopens at the top");
    assert_eq!(m.help_page, HelpPage::Context, "on the context page");
}

#[test]
fn help_opens_on_the_keys_of_its_context() {
    use crate::tui::test_support::{model_with_events, model_with_flows, model_with_policy};
    let mut m = model_with_flows();
    for (view, ctx) in [
        (TrafficView::Flows, HelpContext::Flows),
        (TrafficView::Aggregate, HelpContext::AppToApp),
        (TrafficView::Mesh, HelpContext::Mesh),
        (TrafficView::Blocked, HelpContext::Blocked),
        (TrafficView::Treemap, HelpContext::Treemap),
    ] {
        m.traffic.view = view;
        update(&mut m, key('?'));
        assert_eq!(HelpContext::of(&m), ctx, "{view:?}");
        update(&mut m, kc(KeyCode::Esc));
        assert_eq!(m.overlay, Overlay::None);
    }
    let mut p = model_with_policy();
    p.screen = Screen::Policy;
    update(&mut p, key('?'));
    assert_eq!(HelpContext::of(&p), HelpContext::Policy);
    update(&mut p, kc(KeyCode::Esc));
    p.policy.pending = Some(pending_fixture());
    p.overlay = Overlay::Pending;
    update(&mut p, key('?'));
    assert_eq!(p.overlay, Overlay::Help);
    assert_eq!(HelpContext::of(&p), HelpContext::Pending);
    update(&mut p, kc(KeyCode::Esc));
    assert_eq!(p.overlay, Overlay::Pending, "back to the pending changes");
    for (screen, ctx) in [
        (Screen::Objects, HelpContext::Objects),
        (Screen::Workloads, HelpContext::Workloads),
        (Screen::Events, HelpContext::Events),
    ] {
        let mut m = model_fixture();
        m.screen = screen;
        update(&mut m, key('?'));
        assert_eq!(HelpContext::of(&m), ctx, "{screen:?}");
    }
    // An event's detail: the Events keys, and back to the detail.
    let mut e = model_with_events();
    e.screen = Screen::Events;
    update(&mut e, kc(KeyCode::Enter));
    assert!(matches!(e.overlay, Overlay::EventDetail { .. }));
    update(&mut e, key('?'));
    assert_eq!(e.overlay, Overlay::Help);
    assert_eq!(HelpContext::of(&e), HelpContext::EventDetail);
    update(&mut e, key('q'));
    assert!(matches!(e.overlay, Overlay::EventDetail { .. }));
    // The marks list: its keys, and back to it.
    let mut k = model_with_flows();
    update(&mut k, Msg::Command(Command::Marks(None)));
    assert_eq!(k.overlay, Overlay::Marks);
    update(&mut k, key('?'));
    assert_eq!(HelpContext::of(&k), HelpContext::Marks);
    update(&mut k, kc(KeyCode::Esc));
    assert_eq!(k.overlay, Overlay::Marks);
    // `:help` from the prompt: the screen's keys.
    let mut c = model_with_flows();
    update(&mut c, key(':'));
    for ch in "help".chars() {
        update(&mut c, key(ch));
    }
    update(&mut c, kc(KeyCode::Enter));
    assert_eq!(c.overlay, Overlay::Help);
    assert_eq!(HelpContext::of(&c), HelpContext::Flows);
}

#[test]
fn planned_opens_the_confirm_overlay_and_y_sends_once() {
    use crate::tui::test_support::label_post_plan;
    let mut m = writes_on(model_fixture());
    let planned = Msg::Planned(Ok((label_post_plan(), pce::write::ObjectKind::Label)));
    assert!(update(&mut m, planned).is_empty());
    assert!(matches!(
        &m.overlay,
        Overlay::ConfirmWrite { scroll: 0, .. }
    ));
    // The body is four lines: `{`, key, value, `}`; at 80x24 all show.
    update(&mut m, key('j'));
    assert!(
        matches!(&m.overlay, Overlay::ConfirmWrite { scroll: 0, .. }),
        "nothing to scroll: {:?}",
        m.overlay
    );
    // Eight rows leave three for the body: the last page starts at line 1.
    m.size = (80, 8);
    for _ in 0..10 {
        update(&mut m, key('j'));
    }
    assert!(
        matches!(&m.overlay, Overlay::ConfirmWrite { scroll: 1, .. }),
        "stops where the last line is drawn: {:?}",
        m.overlay
    );
    update(&mut m, key('k'));
    assert!(matches!(
        &m.overlay,
        Overlay::ConfirmWrite { scroll: 0, .. }
    ));
    assert!(update(&mut m, key('q')).is_empty());
    assert!(!m.quit, "q does not quit from the confirm overlay");
    let cmds = update(&mut m, key('y'));
    assert_eq!(
        cmds,
        vec![Cmd::ExecuteWrite {
            plan: label_post_plan(),
            kind: pce::write::ObjectKind::Label
        }]
    );
    assert_eq!(m.overlay, Overlay::None);
    assert_eq!(m.status.text, "sending …");
    assert!(
        update(&mut m, key('y')).is_empty(),
        "a second y has no overlay to answer"
    );
}

#[test]
fn n_or_esc_cancels_and_a_failed_plan_reports() {
    use crate::tui::test_support::ip_list_delete_plan;
    for code in [KeyCode::Char('n'), KeyCode::Esc] {
        let mut m = writes_on(model_fixture());
        update(
            &mut m,
            Msg::Planned(Ok((ip_list_delete_plan(), pce::write::ObjectKind::IpList))),
        );
        assert!(update(&mut m, kc(code)).is_empty());
        assert_eq!(m.overlay, Overlay::None);
        assert_eq!(m.status.text, "cancelled, nothing sent");
    }
    let mut m = writes_on(model_fixture());
    update(
        &mut m,
        Msg::Planned(Err("not found: /orgs/5/sec_policy/draft/ip_lists/9".into())),
    );
    assert_eq!(m.status.kind, StatusKind::Error);
    assert_eq!(
        m.status.text,
        "write: not found: /orgs/5/sec_policy/draft/ip_lists/9"
    );
    assert_eq!(
        m.overlay,
        Overlay::None,
        "short errors stay in the status line"
    );
    let long = "range 10.9.0.0/16 is not in Corp; an IP list needs at least one range or FQDN";
    update(&mut m, Msg::Planned(Err(long.into())));
    assert_eq!(m.overlay, Overlay::Error(format!("write: {long}")));
    let mut m = writes_on(model_fixture());
    let noop = pce::write::Plan::Noop {
        existing_href: "/orgs/5/labels/1".into(),
        reason: "label role=web exists".into(),
    };
    update(
        &mut m,
        Msg::Planned(Ok((noop, pce::write::ObjectKind::Label))),
    );
    assert_eq!(m.status, StatusLine::info("label role=web exists"));
    assert_eq!(m.overlay, Overlay::None);
}

#[test]
fn an_update_with_nothing_to_change_is_an_answer_not_a_failure() {
    let mut m = writes_on(model_fixture());
    m.writes.in_flight = true;
    let noop = pce::write::Plan::Noop {
        existing_href: "/orgs/5/sec_policy/draft/rule_sets/100".into(),
        reason: "rule_set Shop-Prod already matches".into(),
    };
    assert!(
        update(
            &mut m,
            Msg::Planned(Ok((noop, pce::write::ObjectKind::RuleSet)))
        )
        .is_empty()
    );
    assert!(!m.writes.in_flight);
    assert_eq!(
        m.status,
        StatusLine::info("rule_set Shop-Prod already matches")
    );
    assert_eq!(m.overlay, Overlay::None);
}

#[test]
fn a_late_diff_never_replaces_another_overlay() {
    use crate::tui::diff::{DiffKind, ObjectDiff};
    use crate::tui::test_support::{model_with_policy, provision_plan};
    let diff = ObjectDiff {
        name: "Corp".into(),
        href: "/orgs/5/sec_policy/draft/ip_lists/21".into(),
        change: DiffKind::Created,
        lines: Vec::new(),
    };
    let mut m = model_with_policy();
    m.policy.diff_loading = true;
    let confirm = Overlay::ConfirmWrite {
        plan: provision_plan(),
        kind: pce::write::ObjectKind::Provision,
        scroll: 0,
    };
    m.overlay = confirm.clone();
    assert!(update(&mut m, Msg::DiffLoaded(Ok(diff.clone()))).is_empty());
    assert!(!m.policy.diff_loading);
    assert_eq!(m.overlay, confirm, "the provision plan stays");
    assert_eq!(
        m.status,
        StatusLine::info("diff ready; reopen the pending list")
    );
    // Over no overlay (the list was closed) the diff opens.
    m.overlay = Overlay::None;
    update(&mut m, Msg::DiffLoaded(Ok(diff.clone())));
    assert_eq!(m.overlay, Overlay::Diff { diff, scroll: 0 });
}

fn written(href: &str, pending: bool) -> pce::write::Written {
    pce::write::Written {
        href: href.into(),
        pending,
        status: 201,
        object: serde_json::json!({ "href": href }),
        dropped_fields: vec![],
        warnings: vec![],
        audit_failed: None,
    }
}

#[test]
fn write_done_reports_reloads_the_kind_once_and_forgets_pending() {
    use crate::tui::model::ObjectKind;
    use crate::tui::test_support::model_with_objects;
    use pce::write::Outcome;
    let mut m = model_with_objects();
    m.policy.pending = Some(vec![]);
    let cmds = update(
        &mut m,
        Msg::WriteDone {
            kind: pce::write::ObjectKind::IpList,
            method: Some(pce::WriteMethod::Post),
            result: Ok(Outcome::Written(written(
                "/orgs/5/sec_policy/draft/ip_lists/9",
                true,
            ))),
        },
    );
    assert_eq!(cmds, vec![Cmd::LoadIpLists(pce::CacheMode::Bypass)]);
    assert_eq!(
        m.status.text,
        "created /orgs/5/sec_policy/draft/ip_lists/9 (pending)"
    );
    assert_eq!(m.status.kind, StatusKind::Info);
    assert!(m.policy.pending.is_none(), "the next p fetches again");
    let mut w = written("/orgs/5/labels/9", false);
    w.warnings = vec!["audit entry could not be written to x: denied".into()];
    let cmds = update(
        &mut m,
        Msg::WriteDone {
            kind: pce::write::ObjectKind::Label,
            method: Some(pce::WriteMethod::Put),
            result: Ok(Outcome::Written(w)),
        },
    );
    assert_eq!(
        cmds,
        vec![Cmd::LoadObjects {
            kind: ObjectKind::Labels,
            mode: pce::CacheMode::Bypass
        }]
    );
    assert_eq!(m.status.kind, StatusKind::Warn);
    assert_eq!(
        m.status.text,
        "updated /orgs/5/labels/9; audit entry could not be written to x: denied"
    );
    let cmds = update(
        &mut m,
        Msg::WriteDone {
            kind: pce::write::ObjectKind::Workload,
            method: Some(pce::WriteMethod::Delete),
            result: Ok(Outcome::Written(written("/orgs/5/workloads/aaaa-3", false))),
        },
    );
    assert_eq!(cmds, vec![Cmd::LoadWorkloads(pce::CacheMode::Bypass)]);
    assert_eq!(m.status.text, "deleted /orgs/5/workloads/aaaa-3");
    let cmds = update(
        &mut m,
        Msg::WriteDone {
            kind: pce::write::ObjectKind::Label,
            method: None,
            result: Ok(Outcome::Skipped {
                existing_href: "/orgs/5/labels/1".into(),
                reason: "label role=web exists".into(),
            }),
        },
    );
    assert!(cmds.is_empty());
    assert_eq!(m.status.text, "skipped: label role=web exists");
}

#[test]
fn write_errors_open_the_overlay_and_only_unanswered_reloads() {
    use crate::tui::model::ObjectKind;
    use crate::tui::test_support::model_with_objects;
    use pce::PceError;
    use pce::write::WriteError;
    let mut m = model_with_objects();
    let cmds = update(
        &mut m,
        Msg::WriteDone {
            kind: pce::write::ObjectKind::Service,
            method: Some(pce::WriteMethod::Delete),
            result: Err(WriteError::Pce(PceError::Rejected {
                status: 406,
                body: "object_in_use: referenced by a rule".into(),
            })),
        },
    );
    assert!(cmds.is_empty(), "a rejected write reloads nothing");
    match &m.overlay {
        Overlay::Error(t) => assert!(
            t.starts_with("write failed: ") && t.contains("object_in_use: referenced by a rule"),
            "{t}"
        ),
        other => panic!("{other:?}"),
    }
    assert_eq!(m.status.kind, StatusKind::Error);
    m.overlay = Overlay::None;
    let cmds = update(
        &mut m,
        Msg::WriteDone {
            kind: pce::write::ObjectKind::Service,
            method: Some(pce::WriteMethod::Put),
            result: Err(WriteError::Unanswered(PceError::Timeout(
                std::time::Duration::from_secs(30),
            ))),
        },
    );
    assert_eq!(
        cmds,
        vec![Cmd::LoadObjects {
            kind: ObjectKind::Services,
            mode: pce::CacheMode::Bypass
        }],
        "the user sees the real state"
    );
    match &m.overlay {
        Overlay::Error(t) => {
            assert!(
                t.starts_with("the request was sent but not answered: "),
                "{t}"
            );
            assert_eq!(
                t.matches("check the PCE before retrying").count(),
                1,
                "the advice reads once: {t}"
            );
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn an_unanswered_write_whose_audit_failed_still_reloads_and_names_the_audit_log() {
    use crate::tui::test_support::model_with_objects;
    use pce::PceError;
    use pce::write::WriteError;
    let mut m = model_with_objects();
    let cmds = update(
        &mut m,
        Msg::WriteDone {
            kind: pce::write::ObjectKind::IpList,
            method: Some(pce::WriteMethod::Put),
            result: Err(WriteError::AuditFailed {
                write: Box::new(WriteError::Unanswered(PceError::Timeout(
                    std::time::Duration::from_secs(30),
                ))),
                error: "/home/a/.local/state/illumio-tui/audit.jsonl: permission denied".into(),
            }),
        },
    );
    assert_eq!(
        cmds,
        vec![Cmd::LoadIpLists(pce::CacheMode::Bypass)],
        "the write may have applied: one reload shows the real state"
    );
    assert_eq!(m.status.kind, StatusKind::Error);
    assert_eq!(
        m.status.text,
        "write not answered and not audited; reloading"
    );
    match &m.overlay {
        Overlay::Error(t) => {
            assert!(
                t.starts_with("the request was sent but not answered: "),
                "{t}"
            );
            assert!(
                t.contains("/home/a/.local/state/illumio-tui/audit.jsonl: permission denied"),
                "{t}"
            );
            assert_eq!(t.matches("check the PCE before retrying").count(), 1, "{t}");
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn pending_list_selects_with_j_k_and_enter_asks_for_one_diff() {
    use crate::tui::test_support::model_with_policy;
    let mut m = model_with_policy();
    m.screen = Screen::Policy;
    let mut pending = pce::models::parse_pending(
        serde_json::from_str(include_str!("../../../../../../fixtures/pending_list.json")).unwrap(),
    );
    pending.push(pce::models::PendingChange {
        href: "/orgs/5/sec_policy/draft/label_groups/7".into(),
        change_type: "update".into(),
        name: Some("G".into()),
        object_type: "label_groups".into(),
    });
    pending.push(pce::models::PendingChange {
        href: "/orgs/5/sec_policy/draft/firewall_settings".into(),
        change_type: "update".into(),
        name: None,
        object_type: "firewall_settings".into(),
    });
    update(&mut m, Msg::PendingLoaded(Ok(pending.clone())));
    assert_eq!(m.policy.pending_selected, 0);
    assert_eq!(
        update(&mut m, enter()),
        vec![Cmd::LoadDiff {
            change: pending[0].clone(),
            kind: pce::write::ObjectKind::RuleSet,
            fallback_deny: None,
        }],
        "entry 0 is a ruleset"
    );
    m.policy.diff_loading = false;
    assert_eq!(m.overlay, Overlay::Pending);
    update(&mut m, key('j')); // Corp
    let cmds = update(&mut m, enter());
    assert_eq!(
        cmds,
        vec![Cmd::LoadDiff {
            change: pending[1].clone(),
            kind: pce::write::ObjectKind::IpList,
            fallback_deny: None,
        }]
    );
    assert!(m.policy.diff_loading);
    assert!(
        update(&mut m, enter()).is_empty(),
        "one diff in flight at a time"
    );
    assert_eq!(m.status.text, "diff already loading");
    m.policy.diff_loading = false;
    update(&mut m, key('j')); // the label group
    assert_eq!(
        update(&mut m, enter()),
        vec![Cmd::LoadDiff {
            change: pending[2].clone(),
            kind: pce::write::ObjectKind::LabelGroup,
            fallback_deny: None,
        }],
        "exactly one diff command for a label group"
    );
    m.policy.diff_loading = false;
    update(&mut m, key('j'));
    update(&mut m, key('j'));
    assert_eq!(m.policy.pending_selected, 3, "stops at the last entry");
    assert!(update(&mut m, enter()).is_empty());
    assert_eq!(m.status.text, "no diff for firewall_settings");
    update(&mut m, key('k'));
    assert_eq!(m.policy.pending_selected, 2);
    update(&mut m, kc(KeyCode::Esc));
    assert_eq!(m.overlay, Overlay::None);
}

#[test]
fn diff_loaded_opens_the_overlay_esc_returns_and_q_closes() {
    use crate::tui::diff::{DiffKind, DiffLine, ObjectDiff, Sign};
    use crate::tui::test_support::model_with_policy;
    let mut m = model_with_policy();
    m.policy.pending = Some(vec![]);
    m.overlay = Overlay::Pending;
    m.policy.diff_loading = true;
    m.status = StatusLine::info("loading the active version of Corp");
    let diff = ObjectDiff {
        name: "Corp".into(),
        href: "/orgs/5/sec_policy/draft/ip_lists/21".into(),
        change: DiffKind::Created,
        lines: vec![
            DiffLine {
                sign: Sign::Added,
                text: "name: Corp".into(),
            },
            DiffLine {
                sign: Sign::Added,
                text: "range 10.1.0.0-10.1.255.255".into(),
            },
        ],
    };
    assert!(update(&mut m, Msg::DiffLoaded(Ok(diff.clone()))).is_empty());
    assert!(!m.policy.diff_loading);
    assert_eq!(m.status.kind, StatusKind::Info);
    assert_eq!(
        m.status.text, "diff of Corp",
        "the loading status is replaced"
    );
    assert_eq!(
        m.overlay,
        Overlay::Diff {
            diff: diff.clone(),
            scroll: 0
        }
    );
    update(&mut m, key('j'));
    update(&mut m, key('j'));
    assert_eq!(
        m.overlay,
        Overlay::Diff {
            diff: diff.clone(),
            scroll: 1
        },
        "stops at the last line"
    );
    update(&mut m, kc(KeyCode::Esc));
    assert_eq!(m.overlay, Overlay::Pending, "Esc returns to the list");
    update(&mut m, Msg::DiffLoaded(Ok(diff)));
    update(&mut m, key('q'));
    assert_eq!(m.overlay, Overlay::None, "q closes both");
    m.overlay = Overlay::Pending;
    m.status = StatusLine::info("loading the active version of Corp");
    update(
        &mut m,
        Msg::DiffLoaded(Err("not found: /orgs/5/sec_policy/active/ip_lists".into())),
    );
    assert_eq!(m.status.kind, StatusKind::Error);
    assert_eq!(
        m.status.text,
        "diff: not found: /orgs/5/sec_policy/active/ip_lists"
    );
    assert_eq!(m.overlay, Overlay::Pending);
}

#[test]
fn a_write_in_flight_refuses_edit_keys_and_profile_switches() {
    use crate::tui::model::ObjectKind;
    use crate::tui::test_support::{ip_list_delete_plan, model_with_objects};
    let mut m = writes_on(model_with_objects());
    m.screen = Screen::Objects;
    m.objects.kind = ObjectKind::Services;
    let cmds = update(&mut m, key('x'));
    assert!(matches!(cmds.as_slice(), [Cmd::PlanWrite(_)]), "{cmds:?}");
    assert!(m.writes.in_flight, "a plan is in flight");
    let refused = |m: &mut Model, msg: Msg, what: &str| {
        assert!(update(m, msg).is_empty(), "{what}");
        assert_eq!(m.status.kind, StatusKind::Warn, "{what}");
        assert_eq!(m.status.text, "a write is already in progress", "{what}");
        assert_eq!(m.overlay, Overlay::None, "{what}");
    };
    for c in ['n', 'e', 'x'] {
        refused(&mut m, key(c), "objects");
    }
    m.screen = Screen::Workloads;
    // `x` groups on Workloads: delete is `X`.
    for c in ['n', 'e', 'X'] {
        refused(&mut m, key(c), "workloads");
    }
    m.screen = Screen::Objects;
    assert!(update(&mut m, Msg::Command(Command::Profile("b".into()))).is_empty());
    assert_eq!(m.status.kind, StatusKind::Warn);
    assert_eq!(m.status.text, "finish or cancel the write first");
    assert!(!m.switching);
    // The plan lands: the confirm opens and nothing is in flight.
    let planned = Msg::Planned(Ok((ip_list_delete_plan(), pce::write::ObjectKind::IpList)));
    update(&mut m, planned);
    assert!(matches!(m.overlay, Overlay::ConfirmWrite { .. }));
    assert!(!m.writes.in_flight);
    // `y` sends: in flight again until the write is done.
    let cmds = update(&mut m, key('y'));
    assert!(
        matches!(cmds.as_slice(), [Cmd::ExecuteWrite { .. }]),
        "{cmds:?}"
    );
    assert!(m.writes.in_flight);
    refused(&mut m, key('x'), "while sending");
    update(
        &mut m,
        Msg::WriteDone {
            kind: pce::write::ObjectKind::IpList,
            method: None,
            result: Ok(pce::write::Outcome::Skipped {
                existing_href: "/orgs/5/sec_policy/draft/ip_lists/21".into(),
                reason: "gone".into(),
            }),
        },
    );
    assert!(!m.writes.in_flight);
    assert_eq!(
        update(&mut m, Msg::Command(Command::Profile("b".into()))),
        vec![Cmd::SwitchProfile("b".into())]
    );
    m.writes.in_flight = true;
    m.reset_data();
    assert!(!m.writes.in_flight, "a profile switch forgets the write");
}

#[test]
fn a_plan_landing_on_another_overlay_is_discarded() {
    use crate::tui::test_support::label_post_plan;
    let mut m = writes_on(model_fixture());
    m.writes.in_flight = true;
    // The user opened the prompt and typed `y` while the plan was on its way.
    m.overlay = Overlay::Prompt { input: "y".into() };
    let planned = Msg::Planned(Ok((label_post_plan(), pce::write::ObjectKind::Label)));
    assert!(update(&mut m, planned).is_empty());
    assert_eq!(m.overlay, Overlay::Prompt { input: "y".into() });
    assert_eq!(m.status.kind, StatusKind::Warn);
    assert_eq!(m.status.text, "request discarded, nothing sent");
    assert!(!m.writes.in_flight);
}

#[test]
fn a_created_label_joins_the_name_index_when_the_labels_reload() {
    use crate::tui::model::ObjectKind;
    use crate::tui::test_support::model_with_objects;
    let mut m = model_with_objects();
    assert_eq!(m.index().unwrap().label_href("app", "web2"), None);
    let cmds = update(
        &mut m,
        Msg::WriteDone {
            kind: pce::write::ObjectKind::Label,
            method: Some(pce::WriteMethod::Post),
            result: Ok(pce::write::Outcome::Written(written(
                "/orgs/5/labels/9",
                false,
            ))),
        },
    );
    assert_eq!(
        cmds,
        vec![Cmd::LoadObjects {
            kind: ObjectKind::Labels,
            mode: pce::CacheMode::Bypass
        }],
        "the one reload the write already costs"
    );
    let mut labels: Vec<pce::models::Label> =
        serde_json::from_str(include_str!("../../../../../../fixtures/labels.json")).unwrap();
    labels.push(
        serde_json::from_value(serde_json::json!({
            "href": "/orgs/5/labels/9", "key": "app", "value": "web2"
        }))
        .unwrap(),
    );
    let cmds = update(
        &mut m,
        Msg::ObjectsLoaded {
            kind: ObjectKind::Labels,
            result: Ok(ObjectsPayload::Labels(labels, 0)),
        },
    );
    assert!(cmds.is_empty(), "the index is rebuilt without a request");
    let idx = m.index().expect("still connected with an index");
    assert_eq!(idx.label_href("app", "web2"), Some("/orgs/5/labels/9"));
    assert_eq!(
        idx.ip_list_name("/orgs/5/sec_policy/draft/ip_lists/21"),
        Some("Corp"),
        "IP lists are kept"
    );
    assert_eq!(
        idx.service_name("/orgs/5/sec_policy/draft/services/31"),
        Some("PostgreSQL"),
        "services are kept"
    );
    // A form opened now resolves the new label.
    assert!(
        edit::build_intent(
            &{
                let mut f = edit::form_for(
                    EditIntent::CreateWorkload,
                    Some(idx.as_ref()),
                    m.flow_prefill(),
                )
                .expect("a form");
                f.fields[0].value = "printer-1".into();
                f.fields[4].value = "app=web2".into();
                f
            },
            Some(idx.as_ref()),
            5
        )
        .is_ok_and(|i| i.is_some())
    );
}

#[test]
fn confirm_scroll_counts_wrapped_lines() {
    let long = "w".repeat(150);
    let plan = pce::write::Plan::Send {
        request: pce::write::WriteRequest {
            method: pce::WriteMethod::Post,
            path: "/orgs/5/labels".into(),
            body: Some(serde_json::json!({"key": "app", "value": long})),
            summary: "create label".into(),
            objects: Vec::new(),
        },
        kind: pce::write::ObjectKind::Label,
    };
    let mut m = writes_on(model_fixture());
    update(
        &mut m,
        Msg::Planned(Ok((plan, pce::write::ObjectKind::Label))),
    );
    // 74 columns inside the box: the value line takes three rows, so six
    // body rows in all; nine terminal rows leave four.
    m.size = (80, 9);
    for _ in 0..10 {
        update(&mut m, key('j'));
    }
    assert!(
        matches!(&m.overlay, Overlay::ConfirmWrite { scroll: 2, .. }),
        "{:?}",
        m.overlay
    );
}

#[test]
fn ruleset_keys_open_forms_and_x_plans_the_delete() {
    use crate::tui::edit::WriteIntent;
    use crate::tui::test_support::model_with_policy;
    let mut m = writes_on(model_with_policy());
    m.screen = Screen::Policy;
    assert!(update(&mut m, key('n')).is_empty());
    assert!(
        matches!(&m.overlay, Overlay::Form(f) if f.title == "New ruleset"),
        "{:?}",
        m.overlay
    );
    update(&mut m, kc(KeyCode::Esc));
    assert!(update(&mut m, key('e')).is_empty());
    assert!(
        matches!(&m.overlay, Overlay::Form(f) if f.title == "Edit ruleset Shop-Prod"),
        "{:?}",
        m.overlay
    );
    update(&mut m, kc(KeyCode::Esc));
    assert_eq!(
        update(&mut m, key('x')),
        vec![Cmd::PlanWrite(WriteIntent::DeleteRuleSet {
            href: "/orgs/5/sec_policy/draft/rule_sets/100".into(),
            name: "Shop-Prod".into(),
        })]
    );
    assert!(m.writes.in_flight);
    assert!(update(&mut m, key('e')).is_empty(), "one write at a time");
    assert_eq!(m.status.text, "a write is already in progress");
}

#[test]
fn policy_keys_refuse_in_the_active_scope_and_without_a_selection() {
    use crate::tui::test_support::model_with_policy;
    let mut m = model_with_policy();
    m.screen = Screen::Policy;
    for c in ['n', 'e', 'x'] {
        assert!(update(&mut m, key(c)).is_empty(), "{c}");
        assert_eq!(m.status.kind, StatusKind::Warn);
        assert_eq!(m.status.text, crate::tui::model::WRITES_OFF);
        assert_eq!(m.overlay, Overlay::None);
    }
    let mut m = writes_on(model_with_policy());
    m.screen = Screen::Policy;
    m.policy.scope = PolicyScope::Active;
    for c in ['n', 'e', 'x'] {
        assert!(update(&mut m, key(c)).is_empty(), "{c}");
        assert_eq!(m.status.kind, StatusKind::Warn);
        assert_eq!(m.status.text, "switch to draft to edit policy");
        assert_eq!(m.overlay, Overlay::None);
    }
    let mut m = writes_on(model_fixture());
    m.screen = Screen::Policy;
    for c in ['e', 'x'] {
        assert!(update(&mut m, key(c)).is_empty(), "{c}");
        assert_eq!(m.status.text, "no row selected");
        assert_eq!(m.overlay, Overlay::None);
    }
}

#[test]
fn a_policy_write_reloads_the_draft_rulesets_and_keeps_the_report() {
    use crate::tui::test_support::model_with_policy;
    use pce::write::Outcome;
    let mut m = model_with_policy();
    m.screen = Screen::Policy;
    let cmds = update(
        &mut m,
        Msg::WriteDone {
            kind: pce::write::ObjectKind::RuleSet,
            method: Some(pce::WriteMethod::Post),
            result: Ok(Outcome::Written(written(
                "/orgs/5/sec_policy/draft/rule_sets/102",
                true,
            ))),
        },
    );
    assert_eq!(
        cmds,
        vec![Cmd::LoadRuleSets {
            scope: PolicyScope::Draft,
            mode: pce::CacheMode::Bypass
        }]
    );
    let report = "created /orgs/5/sec_policy/draft/rule_sets/102 (pending)";
    assert_eq!(m.status.text, report);
    let sets = m.policy.raw.clone();
    update(
        &mut m,
        Msg::RuleSetsLoaded {
            scope: PolicyScope::Draft,
            result: Ok((sets.clone(), 0, false)),
        },
    );
    assert_eq!(m.status.text, report, "the reload keeps the write's report");
    update(
        &mut m,
        Msg::RuleSetsLoaded {
            scope: PolicyScope::Draft,
            result: Ok((sets, 0, false)),
        },
    );
    assert_eq!(
        m.status.text, "2 rulesets (draft)",
        "only that one reload is quiet"
    );
    m.policy.scope = PolicyScope::Active;
    let cmds = update(
        &mut m,
        Msg::WriteDone {
            kind: pce::write::ObjectKind::Rule,
            method: Some(pce::WriteMethod::Delete),
            result: Ok(Outcome::Written(written(
                "/orgs/5/sec_policy/draft/rule_sets/100/sec_rules/1",
                true,
            ))),
        },
    );
    assert!(
        cmds.is_empty(),
        "a draft write does not change the active view"
    );
}

#[test]
fn quiet_reload_does_not_outlive_a_scope_switch() {
    use crate::tui::test_support::model_with_policy;
    use pce::write::Outcome;
    let mut m = model_with_policy();
    m.screen = Screen::Policy;
    update(
        &mut m,
        Msg::WriteDone {
            kind: pce::write::ObjectKind::RuleSet,
            method: Some(pce::WriteMethod::Post),
            result: Ok(Outcome::Written(written(
                "/orgs/5/sec_policy/draft/rule_sets/102",
                true,
            ))),
        },
    );
    assert!(m.policy.quiet_reload);
    m.policy.loading = false;
    update(&mut m, key('d')); // to active while the reload is in flight
    let sets = m.policy.raw.clone();
    update(
        &mut m,
        Msg::RuleSetsLoaded {
            scope: PolicyScope::Draft,
            result: Ok((sets.clone(), 0, false)),
        },
    );
    assert!(!m.policy.quiet_reload, "the ignored answer took the flag");
    update(
        &mut m,
        Msg::RuleSetsLoaded {
            scope: PolicyScope::Active,
            result: Ok((sets, 0, false)),
        },
    );
    assert!(
        m.status.text.ends_with("rulesets (active)"),
        "{}",
        m.status.text
    );
}

#[test]
fn rule_keys_open_prefilled_forms_and_x_plans_by_kind() {
    use crate::tui::edit::WriteIntent;
    use crate::tui::test_support::model_with_policy;
    let mut m = writes_on(model_with_policy());
    m.screen = Screen::Policy;
    update(&mut m, kc(KeyCode::Tab));
    assert_eq!(m.policy.pane, PolicyPane::Rules);
    update(&mut m, key('n'));
    assert!(
        matches!(&m.overlay, Overlay::Form(f) if f.title == "New allow rule in ruleset 100"),
        "{:?}",
        m.overlay
    );
    update(&mut m, kc(KeyCode::Esc));
    update(&mut m, key('N'));
    assert!(
        matches!(&m.overlay, Overlay::Form(f) if f.title == "New deny rule in ruleset 100"),
        "{:?}",
        m.overlay
    );
    update(&mut m, kc(KeyCode::Esc));
    // Row 0 is the override deny rule 8 (evaluation order).
    update(&mut m, key('e'));
    match &m.overlay {
        Overlay::Form(f) => {
            assert_eq!(f.title, "Edit deny rule 8 in ruleset 100");
            assert_eq!(f.value("Override"), "y");
        }
        other => panic!("{other:?}"),
    }
    update(&mut m, kc(KeyCode::Esc));
    assert_eq!(
        update(&mut m, key('x')),
        vec![Cmd::PlanWrite(WriteIntent::DeleteDenyRule {
            href: "/orgs/5/sec_policy/draft/rule_sets/100/deny_rules/8".into(),
            name: "override_deny role=db ← All workloads 22/tcp".into(),
        })]
    );
    m.writes.in_flight = false;
    update(&mut m, key('j')); // allow rule 1
    update(&mut m, key('e'));
    match &m.overlay {
        Overlay::Form(f) => {
            assert_eq!(f.title, "Edit allow rule 1 in ruleset 100");
            assert_eq!(f.value("Services"), "PostgreSQL");
        }
        other => panic!("{other:?}"),
    }
    update(&mut m, kc(KeyCode::Esc));
    assert_eq!(
        update(&mut m, key('x')),
        vec![Cmd::PlanWrite(WriteIntent::DeleteRule {
            href: "/orgs/5/sec_policy/draft/rule_sets/100/sec_rules/1".into(),
            name: "allow role=db ← role=web PostgreSQL".into(),
        })]
    );
}

#[test]
fn rules_pane_keys_go_through_the_same_refusals() {
    use crate::tui::test_support::model_with_policy;
    let mut m = model_with_policy();
    m.screen = Screen::Policy;
    update(&mut m, kc(KeyCode::Tab));
    for c in ['n', 'N', 'e', 'x'] {
        assert!(update(&mut m, key(c)).is_empty(), "{c}");
        assert_eq!(m.status.text, crate::tui::model::WRITES_OFF);
        assert_eq!(m.overlay, Overlay::None);
    }
    let mut m = writes_on(model_with_policy());
    m.screen = Screen::Policy;
    m.policy.scope = PolicyScope::Active;
    update(&mut m, kc(KeyCode::Tab));
    for c in ['n', 'N', 'e', 'x'] {
        assert!(update(&mut m, key(c)).is_empty(), "{c}");
        assert_eq!(m.status.text, "switch to draft to edit policy");
        assert_eq!(m.overlay, Overlay::None);
    }
    // `N` means nothing on the rulesets pane.
    let mut m = writes_on(model_with_policy());
    m.screen = Screen::Policy;
    m.status = StatusLine::info("ready");
    assert!(update(&mut m, key('N')).is_empty());
    assert_eq!(m.overlay, Overlay::None);
    assert_eq!(m.status.text, "ready");
    // No rule rows: e and x have nothing to act on.
    update(&mut m, kc(KeyCode::Tab));
    m.policy.rules.set_rows(Vec::new());
    for c in ['e', 'x'] {
        assert!(update(&mut m, key(c)).is_empty(), "{c}");
        assert_eq!(m.status.text, "no row selected");
    }
}

fn pending_fixture() -> Vec<pce::models::PendingChange> {
    pce::models::parse_pending(
        serde_json::from_str(include_str!("../../../../../../fixtures/pending_list.json")).unwrap(),
    )
}

#[test]
fn space_marks_entries_and_p_plans_the_marked_or_all() {
    use crate::tui::edit::WriteIntent;
    use crate::tui::test_support::model_with_policy;
    let provision = |subset| {
        vec![Cmd::PlanWrite(WriteIntent::Provision(ProvisionRequest {
            description: Some("illumio-tui provision".into()),
            subset,
        }))]
    };
    let mut m = writes_on(model_with_policy());
    m.screen = Screen::Policy;
    update(&mut m, Msg::PendingLoaded(Ok(pending_fixture())));
    assert_eq!(m.overlay, Overlay::Pending);
    update(&mut m, key(' '));
    assert!(
        m.policy
            .pending_marked
            .contains("/orgs/5/sec_policy/draft/rule_sets/101")
    );
    update(&mut m, key(' '));
    assert!(m.policy.pending_marked.is_empty(), "Space toggles");
    update(&mut m, key('j'));
    update(&mut m, key(' '));
    assert_eq!(
        update(&mut m, key('P')),
        provision(ProvisionSubset::Hrefs(vec![
            "/orgs/5/sec_policy/draft/ip_lists/21".into()
        ]))
    );
    assert_eq!(
        m.overlay,
        Overlay::None,
        "the confirm overlay opens on the plan's answer"
    );
    assert!(m.writes.in_flight);
    m.writes.in_flight = false;
    update(&mut m, Msg::PendingLoaded(Ok(pending_fixture())));
    assert!(
        m.policy.pending_marked.is_empty(),
        "a reload clears the marks"
    );
    assert_eq!(
        update(&mut m, key('P')),
        provision(ProvisionSubset::All),
        "no marks: everything pending"
    );
    m.writes.in_flight = false;
    update(&mut m, Msg::PendingLoaded(Ok(vec![])));
    assert!(update(&mut m, key('P')).is_empty());
    assert_eq!(m.status.text, "nothing pending");
    let mut off = model_with_policy();
    off.screen = Screen::Policy;
    update(&mut off, Msg::PendingLoaded(Ok(pending_fixture())));
    assert!(update(&mut off, key('P')).is_empty());
    assert_eq!(off.status.text, crate::tui::model::WRITES_OFF);
    assert_eq!(off.overlay, Overlay::Pending);
}

#[test]
fn provision_confirm_reports_the_counts_and_reloads_pending_and_rulesets() {
    use crate::tui::test_support::{model_with_policy, provision_plan};
    use pce::write::Outcome;
    let mut m = writes_on(model_with_policy());
    m.screen = Screen::Policy;
    update(
        &mut m,
        Msg::Planned(Ok((provision_plan(), pce::write::ObjectKind::Provision))),
    );
    assert_eq!(
        update(&mut m, key('y')),
        vec![Cmd::ExecuteWrite {
            plan: provision_plan(),
            kind: pce::write::ObjectKind::Provision
        }]
    );
    assert_eq!(
        m.policy.provision_counts.as_deref(),
        Some("ip_lists=1, rule_sets=1")
    );
    let cmds = update(
        &mut m,
        Msg::WriteDone {
            kind: pce::write::ObjectKind::Provision,
            method: Some(pce::WriteMethod::Post),
            result: Ok(Outcome::Written(written("/orgs/5/sec_policy/7", false))),
        },
    );
    assert_eq!(
        cmds,
        vec![
            Cmd::LoadRuleSets {
                scope: PolicyScope::Draft,
                mode: pce::CacheMode::Bypass
            },
            Cmd::LoadPending(pce::CacheMode::Bypass),
        ]
    );
    let report = "provisioned /orgs/5/sec_policy/7: ip_lists=1, rule_sets=1";
    assert_eq!(m.status.text, report);
    assert!(m.policy.provision_counts.is_none());
    let sets = m.policy.raw.clone();
    update(
        &mut m,
        Msg::RuleSetsLoaded {
            scope: PolicyScope::Draft,
            result: Ok((sets, 0, false)),
        },
    );
    assert_eq!(
        m.status.text, report,
        "the rulesets reload keeps the report"
    );
    update(&mut m, Msg::PendingLoaded(Ok(vec![])));
    assert_eq!(m.status.text, format!("{report}; 0 pending changes left"));
    assert_eq!(m.overlay, Overlay::Pending);
}

#[test]
fn r_reverts_only_marked_entries_and_never_everything() {
    use crate::tui::edit::WriteIntent;
    use crate::tui::test_support::model_with_policy;
    let mut m = writes_on(model_with_policy());
    m.screen = Screen::Policy;
    update(&mut m, Msg::PendingLoaded(Ok(pending_fixture())));
    assert!(update(&mut m, key('R')).is_empty(), "nothing marked");
    assert_eq!(m.status.text, "mark changes with Space first");
    assert_eq!(m.overlay, Overlay::Pending);
    assert!(!m.writes.in_flight);
    update(&mut m, key(' '));
    update(&mut m, key('j'));
    update(&mut m, key(' '));
    assert_eq!(
        update(&mut m, key('R')),
        vec![Cmd::PlanWrite(WriteIntent::Revert(RevertRequest {
            all: false,
            targets: vec![
                "/orgs/5/sec_policy/draft/ip_lists/21".into(),
                "/orgs/5/sec_policy/draft/rule_sets/101".into(),
            ],
        }))]
    );
    assert_eq!(m.overlay, Overlay::None);
    assert!(m.writes.in_flight);
    let mut off = model_with_policy();
    off.screen = Screen::Policy;
    update(&mut off, Msg::PendingLoaded(Ok(pending_fixture())));
    update(&mut off, key(' '));
    assert!(update(&mut off, key('R')).is_empty());
    assert_eq!(off.status.text, crate::tui::model::WRITES_OFF);
}

#[test]
fn revert_confirm_names_each_fate_and_sends_only_on_the_typed_word() {
    use crate::tui::test_support::{model_with_policy, render_to_string, revert_plan};
    let revert = pce::write::ObjectKind::Revert;
    let open = || {
        let mut m = writes_on(model_with_policy());
        m.screen = Screen::Policy;
        update(&mut m, Msg::Planned(Ok((revert_plan(), revert))));
        m
    };
    let mut m = open();
    let confirm = Overlay::ConfirmWrite {
        plan: revert_plan(),
        kind: revert,
        scroll: 0,
    };
    assert_eq!(m.overlay, confirm);
    assert!(m.status.text.contains("type revert"), "{}", m.status.text);
    let out = render_to_string(&m, 80, 24);
    for part in [
        "Revert these changes?",
        "ip_list \"Corp\" (removed)",
        "rule_set \"Global-Infra\" (back to active)",
        "PUT /orgs/5/sec_policy/delete",
        "type revert to confirm:",
    ] {
        assert!(out.contains(part), "{part}: {out}");
    }
    // `y` is a letter here, and a wrong word sends nothing.
    assert!(update(&mut m, key('y')).is_empty());
    assert!(update(&mut m, kc(KeyCode::Enter)).is_empty());
    assert_eq!(m.overlay, confirm, "a wrong word keeps the confirm");
    update(&mut m, kc(KeyCode::Backspace));
    for c in "revert".chars() {
        assert!(update(&mut m, key(c)).is_empty(), "{c}");
    }
    assert!(
        render_to_string(&m, 80, 24).contains("type revert to confirm: revert"),
        "the word shows as typed"
    );
    assert_eq!(
        update(&mut m, kc(KeyCode::Enter)),
        vec![Cmd::ExecuteWrite {
            plan: revert_plan(),
            kind: revert
        }]
    );
    assert_eq!(m.overlay, Overlay::None);
    assert!(m.policy.revert_typed.is_empty());
    // Esc, even after the word, sends nothing; a reopened confirm starts empty.
    let mut e = open();
    for c in "revert".chars() {
        update(&mut e, key(c));
    }
    assert!(update(&mut e, kc(KeyCode::Esc)).is_empty());
    assert_eq!(e.overlay, Overlay::None);
    assert_eq!(e.status.text, "cancelled, nothing sent");
    update(&mut e, Msg::Planned(Ok((revert_plan(), revert))));
    assert!(e.policy.revert_typed.is_empty());
    assert!(update(&mut e, kc(KeyCode::Enter)).is_empty());
}

#[test]
fn a_long_revert_scrolls_its_objects_and_keeps_the_prompt_in_view() {
    use crate::tui::test_support::{model_with_policy, render_to_string, revert_plan_of};
    let revert = pce::write::ObjectKind::Revert;
    let mut m = writes_on(model_with_policy());
    m.size = (80, 24);
    m.screen = Screen::Policy;
    update(&mut m, Msg::Planned(Ok((revert_plan_of(30), revert))));
    let out = render_to_string(&m, 80, 24);
    assert!(out.contains("revert 30 changes:"), "{out}");
    assert!(out.contains("List-01"), "{out}");
    assert!(!out.contains("List-30"), "the list overflows: {out}");
    assert!(out.contains("type revert to confirm:"), "{out}");
    assert!(out.contains("↑/↓ scroll"), "{out}");
    let mut seen_last = false;
    for _ in 0..60 {
        update(&mut m, kc(KeyCode::Down));
        let out = render_to_string(&m, 80, 24);
        assert!(out.contains("type revert to confirm:"), "pinned: {out}");
        assert!(out.contains("revert 30 changes:"), "pinned: {out}");
        seen_last |= out.contains("ip_list \"List-30\" (removed)");
    }
    assert!(seen_last, "the last object is reachable by scrolling");
    let Overlay::ConfirmWrite { scroll, .. } = m.overlay else {
        panic!("{:?}", m.overlay)
    };
    assert!(scroll > 0);
    for _ in 0..60 {
        update(&mut m, kc(KeyCode::Up));
    }
    assert!(render_to_string(&m, 80, 24).contains("List-01"));
    for c in "revert".chars() {
        update(&mut m, key(c));
    }
    assert_eq!(update(&mut m, kc(KeyCode::Enter)).len(), 1);
}

#[test]
fn a_revert_names_a_ruleset_pending_through_its_rules_from_the_loaded_rulesets() {
    use crate::tui::test_support::model_with_policy;
    let mut m = writes_on(model_with_policy());
    m.screen = Screen::Policy;
    let known = m.policy.raw[0].clone();
    let mut plan = crate::tui::test_support::revert_plan();
    if let Plan::Send { request, .. } = &mut plan {
        request.objects = vec![
            format!("rule_set {} (back to active)", known.href),
            "rule_set /orgs/5/sec_policy/draft/rule_sets/999 (back to active)".into(),
        ];
    }
    update(
        &mut m,
        Msg::Planned(Ok((plan, pce::write::ObjectKind::Revert))),
    );
    let Overlay::ConfirmWrite { plan, .. } = &m.overlay else {
        panic!("{:?}", m.overlay)
    };
    assert_eq!(
        plan.request().unwrap().objects,
        vec![
            format!("rule_set {:?} (back to active)", known.name),
            "rule_set /orgs/5/sec_policy/draft/rule_sets/999 (back to active)".to_string(),
        ],
        "named when loaded, the href otherwise"
    );
}

#[test]
fn a_revert_reloads_rulesets_and_pending_and_keeps_its_report() {
    use crate::tui::test_support::{model_with_policy, revert_plan};
    use pce::write::Outcome;
    let mut m = writes_on(model_with_policy());
    m.screen = Screen::Policy;
    let revert = pce::write::ObjectKind::Revert;
    update(&mut m, Msg::Planned(Ok((revert_plan(), revert))));
    for c in "revert".chars() {
        update(&mut m, key(c));
    }
    assert_eq!(update(&mut m, kc(KeyCode::Enter)).len(), 1);
    let cmds = update(
        &mut m,
        Msg::WriteDone {
            kind: revert,
            method: Some(pce::WriteMethod::Put),
            result: Ok(Outcome::Written(written(
                "/orgs/5/sec_policy/delete",
                false,
            ))),
        },
    );
    assert_eq!(
        cmds,
        vec![
            Cmd::LoadRuleSets {
                scope: PolicyScope::Draft,
                mode: pce::CacheMode::Bypass
            },
            Cmd::LoadPending(pce::CacheMode::Bypass),
        ]
    );
    let report = "reverted ip_lists=1, rule_sets=1";
    assert_eq!(m.status.text, report);
    update(&mut m, Msg::PendingLoaded(Ok(vec![])));
    assert_eq!(m.status.text, format!("{report}; 0 pending changes left"));
    assert_eq!(m.overlay, Overlay::Pending);
}

#[test]
fn nothing_to_provision_is_an_info_and_a_pending_reload_never_hides_an_error() {
    let mut m = writes_on(model_fixture());
    update(
        &mut m,
        Msg::Planned(Ok((
            pce::write::Plan::Noop {
                existing_href: "/orgs/5/sec_policy/pending".into(),
                reason: "nothing pending".into(),
            },
            pce::write::ObjectKind::Provision,
        ))),
    );
    assert_eq!(m.status.kind, StatusKind::Info);
    assert_eq!(m.status.text, "nothing pending");
    assert_eq!(m.overlay, Overlay::None);
    m.overlay = Overlay::Error("write failed: 406".into());
    update(&mut m, Msg::PendingLoaded(Ok(vec![])));
    assert_eq!(
        m.overlay,
        Overlay::Error("write failed: 406".into()),
        "the reload does not replace the error"
    );
}

#[test]
fn a_pending_rule_entry_points_at_its_ruleset() {
    use crate::tui::test_support::model_with_policy;
    let mut m = model_with_policy();
    m.screen = Screen::Policy;
    update(
        &mut m,
        Msg::PendingLoaded(Ok(vec![pce::models::PendingChange {
            href: "/orgs/5/sec_policy/draft/rule_sets/100/sec_rules/1".into(),
            change_type: "update".into(),
            name: None,
            object_type: "sec_rules".into(),
        }])),
    );
    assert!(update(&mut m, enter()).is_empty());
    assert_eq!(m.status.text, "rule changes show in their ruleset's diff");
    assert!(!m.policy.diff_loading);
}

#[test]
fn a_ruleset_diff_carries_the_deny_rules_the_screen_fetched() {
    use crate::tui::test_support::model_with_policy;
    let mut m = model_with_policy();
    m.screen = Screen::Policy;
    let href = "/orgs/5/sec_policy/draft/rule_sets/101";
    let fetched: pce::models::DenyRule = serde_json::from_value(serde_json::json!({
        "href": format!("{href}/deny_rules/9"),
        "providers": [{"actors": "ams"}], "consumers": [{"actors": "ams"}],
        "ingress_services": [{"port": 23, "proto": 6}]
    }))
    .unwrap();
    m.policy
        .fallback_deny
        .insert(href.into(), vec![fetched.clone()]);
    let change = pce::models::PendingChange {
        href: href.into(),
        change_type: "update".into(),
        name: None,
        object_type: "rule_sets".into(),
    };
    update(&mut m, Msg::PendingLoaded(Ok(vec![change.clone()])));
    assert_eq!(
        update(&mut m, enter()),
        vec![Cmd::LoadDiff {
            change,
            kind: pce::write::ObjectKind::RuleSet,
            fallback_deny: Some(vec![fetched]),
        }]
    );
}

#[test]
fn w_opens_the_flow_form_only_for_a_blocked_flow() {
    use crate::tui::test_support::model_with_flows;
    let mut m = model_with_flows();
    assert!(update(&mut m, key('w')).is_empty());
    assert_eq!(m.status.text, crate::tui::model::WRITES_OFF);
    let mut m = writes_on(model_with_flows());
    assert!(update(&mut m, key('w')).is_empty(), "row 0 is allowed");
    assert_eq!(m.status.kind, StatusKind::Warn);
    assert_eq!(m.status.text, "this flow is allowed; nothing to add");
    assert_eq!(m.overlay, Overlay::None);
    update(&mut m, key('j'));
    assert_eq!(update(&mut m, key('w')).len(), 1, "the draft rulesets");
    assert!(
        matches!(&m.overlay, Overlay::Form(f) if f.title == "Allow rule from flow"),
        "{:?}",
        m.overlay
    );
    update(&mut m, kc(KeyCode::Esc));
    for view in [TrafficView::Aggregate, TrafficView::Treemap] {
        m.traffic.view = view;
        assert!(update(&mut m, key('w')).is_empty());
        assert_eq!(
            m.status.text,
            "select a flow in the Flows view (Enter on a pair)"
        );
        assert_eq!(m.overlay, Overlay::None);
    }
}

#[test]
fn w_prefills_from_the_profile_label_model_and_the_loaded_draft_rulesets() {
    use crate::tui::test_support::{fixture_index, model_with_flows};
    let mut m = writes_on(model_with_flows());
    let raw: Vec<pce::models::RuleSet> =
        serde_json::from_str(include_str!("../../../../../../fixtures/rule_sets.json")).unwrap();
    m.policy.set_rule_sets(raw, &fixture_index(), None);
    m.policy.loaded_at = Some(Instant::now());
    update(&mut m, key('j'));
    assert!(update(&mut m, key('w')).is_empty(), "no request");
    match &m.overlay {
        Overlay::Form(f) => {
            assert_eq!(f.value("Ruleset"), "Shop-Prod", "the loaded draft ruleset");
            assert_eq!(f.value("Providers"), "role=web");
        }
        other => panic!("{other:?}"),
    }
    // Another profile's model: tiers by env, scoped by app.
    m.overlay = Overlay::None;
    m.limits.scope_keys = vec!["app".into()];
    m.limits.tier_key = "env".into();
    update(&mut m, key('w'));
    match &m.overlay {
        Overlay::Form(f) => {
            assert_eq!(
                f.value("Ruleset"),
                "Shop-Prod",
                "scoped by app and env: the destination carries env=prod too"
            );

            assert_eq!(f.value("Providers"), "env=prod");
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn an_unknown_ruleset_name_reopens_the_w_form_with_the_error() {
    use crate::tui::edit::{RulesetTarget, WriteIntent};
    use crate::tui::test_support::{label_post_plan, model_with_flows};
    let mut m = writes_on(model_with_flows());
    update(&mut m, key('j'));
    update(&mut m, key('w'));
    for c in "Shop".chars() {
        update(&mut m, key(c));
    }
    let cmds = update(&mut m, enter());
    assert!(
        matches!(
            cmds.as_slice(),
            [Cmd::PlanWrite(WriteIntent::CreateRule { ruleset: RulesetTarget::Name(n), .. })]
                if n == "Shop"
        ),
        "{cmds:?}"
    );
    assert!(m.traffic.rule_form.is_some());
    let e = r#"no ruleset named "Shop"; similar: Shop-Prod"#;
    assert!(update(&mut m, Msg::Planned(Err(e.into()))).is_empty());
    let e = r#"no ruleset named "Shop": Ctrl-R creates it scoped app=shop, env=prod; similar: Shop-Prod"#;
    match &m.overlay {
        Overlay::Form(f) => {
            assert_eq!(f.error.as_deref(), Some(e), "the planner's text and Ctrl-R");
            let offer = f.offers_ruleset.as_ref().expect("Ctrl-N is offered");
            assert_eq!(
                (offer.name.as_str(), offer.scope.as_str()),
                ("Shop", "app=shop, env=prod")
            );
            assert_eq!(f.value("Ruleset"), "Shop");
            assert_eq!(f.value("Consumers"), "iplist:Corp", "the fields as typed");
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(m.status.text, format!("write: {e}"));
    assert!(m.traffic.rule_form.is_none());
    // Fixed and submitted again: a plan that arrives leaves nothing behind.
    for c in "-Prod".chars() {
        update(&mut m, key(c));
    }
    assert_eq!(update(&mut m, enter()).len(), 1);
    update(
        &mut m,
        Msg::Planned(Ok((label_post_plan(), pce::write::ObjectKind::Rule))),
    );
    assert!(m.traffic.rule_form.is_none());
    assert!(
        matches!(m.overlay, Overlay::ConfirmWrite { .. }),
        "{:?}",
        m.overlay
    );
}

fn ctrl_key(c: char) -> Msg {
    Msg::Key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL))
}

/// `w` on the potentially blocked flow (Corp -> host-1 in app=shop,
/// env=prod), `New-App` typed, submitted, and the planner's answer.
fn unknown_ruleset_form(m: &mut Model) {
    update(m, key('j'));
    update(m, key('w'));
    for c in "New-App".chars() {
        update(m, key(c));
    }
    assert_eq!(update(m, enter()).len(), 1);
    update(m, Msg::Planned(Err(r#"no ruleset named "New-App""#.into())));
}

#[test]
fn ctrl_r_creates_the_scoped_ruleset_then_plans_the_rule_in_it() {
    use crate::tui::edit::{RulesetTarget, WriteIntent};
    use crate::tui::test_support::model_with_flows;
    use pce::write::{ActorKind, ActorRef};
    let mut m = writes_on(model_with_flows());
    unknown_ruleset_form(&mut m);
    match &m.overlay {
        Overlay::Form(f) => {
            assert_eq!(
                f.error.as_deref(),
                Some(r#"no ruleset named "New-App": Ctrl-R creates it scoped app=shop, env=prod"#)
            );
            assert!(f.ruleset_offer_armed().is_some());
        }
        other => panic!("{other:?}"),
    }
    let cmds = update(&mut m, ctrl_key('r'));
    let [Cmd::PlanWrite(WriteIntent::CreateRuleSetIfMissing(d))] = cmds.as_slice() else {
        panic!("{cmds:?}")
    };
    assert_eq!(d.name, "New-App");
    assert_eq!(d.description, None);
    assert!(d.enabled);
    assert_eq!(
        d.scopes,
        vec![vec![
            ActorRef::new(ActorKind::Label, "/orgs/5/labels/3"),
            ActorRef::new(ActorKind::Label, "/orgs/5/labels/4"),
        ]]
    );
    assert_eq!(m.overlay, Overlay::None);
    assert!(m.writes.in_flight);
    let (draft, _) = m.policy.rule_after_ruleset.clone().expect("the rule waits");
    assert_eq!(
        draft.providers,
        vec![ActorRef::new(ActorKind::Label, "/orgs/5/labels/1")]
    );
    // The ruleset is written: its reload, and the rule planned into it.
    let new = "/orgs/5/sec_policy/draft/rule_sets/102";
    let cmds = update(
        &mut m,
        Msg::WriteDone {
            kind: pce::write::ObjectKind::RuleSet,
            method: Some(pce::WriteMethod::Post),
            result: Ok(Outcome::Written(written(new, true))),
        },
    );
    assert_eq!(
        cmds,
        vec![
            Cmd::LoadRuleSets {
                scope: PolicyScope::Draft,
                mode: CacheMode::Bypass
            },
            Cmd::PlanWrite(WriteIntent::CreateRule {
                ruleset: RulesetTarget::Href(new.into()),
                draft,
            }),
        ]
    );
    assert!(m.policy.rule_after_ruleset.is_none());
    assert!(m.writes.in_flight);
    assert_eq!(m.status.text, format!("created {new} (pending)"));
    // The rule's plan fails: the form comes back, the name now checked.
    assert!(m.traffic.rule_form.is_some());
    update(&mut m, Msg::Planned(Err("rule refused".into())));
    match &m.overlay {
        Overlay::Form(f) => {
            assert_eq!(f.error.as_deref(), Some("rule refused"));
            assert_eq!(f.value("Ruleset"), "New-App");
            assert!(f.offers_ruleset.is_none(), "no offer for another error");
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_ruleset_created_with_n_plans_no_rule() {
    let mut m = writes_on(model_fixture());
    let cmds = update(
        &mut m,
        Msg::WriteDone {
            kind: pce::write::ObjectKind::RuleSet,
            method: Some(pce::WriteMethod::Post),
            result: Ok(Outcome::Written(written(
                "/orgs/5/sec_policy/draft/rule_sets/102",
                true,
            ))),
        },
    );
    assert!(
        !cmds.iter().any(|c| matches!(c, Cmd::PlanWrite(_))),
        "{cmds:?}"
    );
    assert!(!m.writes.in_flight);
}

#[test]
fn editing_the_ruleset_name_disarms_the_offer_and_ctrl_r_takes_the_new_name() {
    use crate::tui::edit::WriteIntent;
    use crate::tui::test_support::model_with_flows;
    let mut m = writes_on(model_with_flows());
    unknown_ruleset_form(&mut m);
    update(&mut m, key('X'));
    match &m.overlay {
        Overlay::Form(f) => assert!(f.ruleset_offer_armed().is_none(), "the offer ends"),
        other => panic!("{other:?}"),
    }
    let cmds = update(&mut m, ctrl_key('r'));
    assert!(
        matches!(cmds.as_slice(), [Cmd::PlanWrite(WriteIntent::CreateRuleSetIfMissing(d))] if d.name == "New-AppX"),
        "{cmds:?}"
    );
    assert!(m.policy.rule_after_ruleset.is_some());
}

#[test]
fn the_waiting_rule_is_dropped_by_any_failure_or_cancel() {
    use crate::tui::test_support::{label_post_plan, model_with_flows};
    let pending = |m: &mut Model| {
        unknown_ruleset_form(m);
        assert_eq!(update(m, ctrl_key('r')).len(), 1);
        assert!(m.policy.rule_after_ruleset.is_some());
    };
    // A failed plan.
    let mut m = writes_on(model_with_flows());
    pending(&mut m);
    update(&mut m, Msg::Planned(Err("refused".into())));
    assert!(m.policy.rule_after_ruleset.is_none());
    // `n` on the ruleset's confirm.
    let mut m = writes_on(model_with_flows());
    pending(&mut m);
    update(
        &mut m,
        Msg::Planned(Ok((label_post_plan(), pce::write::ObjectKind::RuleSet))),
    );
    assert!(matches!(m.overlay, Overlay::ConfirmWrite { .. }));
    update(&mut m, key('n'));
    assert!(m.policy.rule_after_ruleset.is_none());
    // A failed ruleset write sends no rule.
    let mut m = writes_on(model_with_flows());
    pending(&mut m);
    let cmds = update(
        &mut m,
        Msg::WriteDone {
            kind: pce::write::ObjectKind::RuleSet,
            method: Some(pce::WriteMethod::Post),
            result: Err(WriteError::invalid("refused")),
        },
    );
    assert!(cmds.is_empty());
    assert!(m.policy.rule_after_ruleset.is_none());
    // A profile switch forgets it.
    let mut m = writes_on(model_with_flows());
    pending(&mut m);
    m.reset_data();
    assert!(m.policy.rule_after_ruleset.is_none());
}

#[test]
fn ctrl_r_says_what_it_needs_a_name_and_a_scoped_destination() {
    use crate::tui::test_support::model_with_flows;
    // Ctrl-R on a fresh form without a name: what to type.
    let mut m = writes_on(model_with_flows());
    update(&mut m, key('j'));
    update(&mut m, key('w'));
    assert!(update(&mut m, ctrl_key('r')).is_empty());
    match &m.overlay {
        Overlay::Form(f) => assert_eq!(
            f.error.as_deref(),
            Some("type the new ruleset's name, then Ctrl-R")
        ),
        other => panic!("{other:?}"),
    }
    // Another planner error is shown as it is, without the offer.
    let mut m = writes_on(model_with_flows());
    update(&mut m, key('j'));
    update(&mut m, key('w'));
    for c in "New-App".chars() {
        update(&mut m, key(c));
    }
    update(&mut m, enter());
    update(&mut m, Msg::Planned(Err("authentication failed".into())));
    match &m.overlay {
        Overlay::Form(f) => {
            assert_eq!(f.error.as_deref(), Some("authentication failed"));
            assert!(f.offers_ruleset.is_none());
        }
        other => panic!("{other:?}"),
    }
    // A destination without scope labels: what to do instead.
    let mut m = writes_on(model_with_flows());
    m.limits.scope_keys = vec!["site".into()];
    unknown_ruleset_form(&mut m);
    let instead = "the destination has no scope labels to scope a new ruleset by: name an existing one (Ctrl-N) or create it on Policy";
    match &m.overlay {
        Overlay::Form(f) => assert_eq!(
            f.error.as_deref(),
            Some(format!(r#"no ruleset named "New-App"; {instead}"#).as_str())
        ),
        other => panic!("{other:?}"),
    }
    assert!(update(&mut m, ctrl_key('r')).is_empty());
    match &m.overlay {
        Overlay::Form(f) => assert_eq!(f.error.as_deref(), Some(instead)),
        other => panic!("{other:?}"),
    }
}

#[test]
fn the_ip_lists_tab_loads_once_and_r_bypasses_the_cache_once() {
    use crate::tui::model::ObjectKind;
    let mut m = model_fixture();
    assert_eq!(update(&mut m, key('3')).len(), 3, "the three other kinds");
    assert_eq!(
        update(&mut m, kc(KeyCode::BackTab)),
        vec![Cmd::LoadIpLists(pce::CacheMode::Use)],
        "Labels back to IP lists: the first visit loads them"
    );
    assert_eq!(m.objects.kind, ObjectKind::IpLists);
    assert_eq!(m.screen, Screen::Objects);
    quiet(&mut m, kc(KeyCode::Tab));
    assert!(
        update(&mut m, kc(KeyCode::BackTab)).is_empty(),
        "already loading"
    );
    let lists =
        serde_json::from_str(include_str!("../../../../../../fixtures/ip_lists.json")).unwrap();
    assert!(update(&mut m, Msg::IpListsLoaded(Ok((lists, 120)))).is_empty());
    assert_eq!(m.objects.ip_lists.table.len(), 2);
    assert_eq!(m.objects.ip_lists.table.rows()[1].size.display(), "65.5K");
    assert_eq!(m.objects.ip_lists.age_secs, 120);
    assert_eq!(m.status.text, "ready", "the status line is left alone");
    quiet(&mut m, key('1'));
    assert!(
        update(&mut m, key('3')).is_empty(),
        "coming back to the tab costs nothing"
    );
    assert_eq!(m.objects.kind, ObjectKind::IpLists, "the tab is kept");
    assert_eq!(
        update(&mut m, key('r')),
        vec![Cmd::LoadIpLists(pce::CacheMode::Bypass)]
    );
    assert!(update(&mut m, key('r')).is_empty());
    assert_eq!(m.status.text, "IP lists already loading");
    update(&mut m, Msg::IpListsLoaded(Err("boom".into())));
    assert_eq!(m.status.text, "IP lists: boom");
    assert!(!m.objects.ip_lists.loading);
    assert_eq!(
        m.objects.ip_lists.table.len(),
        2,
        "a failed reload keeps the rows"
    );
}

#[test]
fn ip_lists_wait_for_the_name_index_and_the_command_opens_their_tab() {
    use crate::tui::model::ObjectKind;
    let mut m = model_fixture();
    let Connection::Connected { index, .. } = &mut m.connection else {
        unreachable!()
    };
    let idx = index.take();
    assert!(
        on_command(&mut m, Command::IpLists).is_empty(),
        "the startup load reads the same list"
    );
    assert_eq!(
        (m.screen, m.objects.kind),
        (Screen::Objects, ObjectKind::IpLists)
    );
    let cmds = update(
        &mut m,
        Msg::IndexLoaded {
            index: idx.unwrap(),
            age_secs: 0,
        },
    );
    assert_eq!(cmds.len(), 4, "the three other kinds and the IP lists");
    assert!(cmds.contains(&Cmd::LoadIpLists(pce::CacheMode::Use)));
    let mut m = model_fixture();
    let cmds = on_command(&mut m, Command::IpLists);
    assert_eq!(cmds.len(), 4);
    assert_eq!(cmds[3], Cmd::LoadIpLists(pce::CacheMode::Use));
    assert_eq!(m.objects.kind, ObjectKind::IpLists);
}

#[test]
fn enter_toggles_the_ip_list_detail_and_search_captures_keys() {
    use crate::tui::test_support::model_with_ip_lists;
    let mut m = model_with_ip_lists();
    crate::tui::test_support::on_ip_lists_tab(&mut m);
    quiet(&mut m, enter());
    assert!(m.objects.ip_lists.show_detail);
    quiet(&mut m, enter());
    assert!(!m.objects.ip_lists.show_detail);
    quiet(&mut m, key('/'));
    for c in "corp6".chars() {
        assert!(update(&mut m, key(c)).is_empty());
    }
    assert_eq!(m.screen, Screen::Objects, "6 is typed into the search");
    assert!(m.objects.ip_lists.table.is_empty());
    quiet(&mut m, kc(KeyCode::Esc));
    assert_eq!(m.objects.ip_lists.table.len(), 2);
    quiet(&mut m, key('j'));
    assert_eq!(
        m.objects
            .ip_lists
            .table
            .selected_item()
            .map(|r| r.list.name.as_str()),
        Some("Corp")
    );
}

#[test]
fn every_key_and_the_wheel_on_the_ip_lists_tab_stay_on_its_own_state() {
    use crate::tui::model::ObjectKind;
    use crate::tui::test_support::{model_with_ip_lists, on_ip_lists_tab};
    use crossterm::event::MouseEventKind;
    let mut m = model_with_ip_lists();
    on_ip_lists_tab(&mut m);
    update(&mut m, Msg::Resize(80, 24));
    let keys = [
        key('j'),
        key('k'),
        key('s'),
        key('s'),
        enter(),
        enter(),
        key('a'),
        key('a'),
        kc(KeyCode::End),
        kc(KeyCode::Home),
        mouse(MouseEventKind::ScrollDown, 10, 10),
        mouse(MouseEventKind::ScrollUp, 10, 10),
        key('/'),
        key('c'),
        kc(KeyCode::Esc),
    ];
    for msg in keys {
        assert!(update(&mut m, msg).is_empty());
    }
    assert_eq!(m.objects.kind, ObjectKind::IpLists);
    assert_eq!(
        m.objects.ages, [None; 3],
        "the other kinds' slots are untouched"
    );
    assert_eq!(m.objects.loading, [false; 3]);
    assert_eq!(
        update(&mut m, key('r')),
        vec![Cmd::LoadIpLists(pce::CacheMode::Bypass)]
    );
    assert_eq!(m.objects.loading, [false; 3], "r reloads only the IP lists");
    let later = std::time::Instant::now() + std::time::Duration::from_secs(3600);
    assert!(
        update(&mut m, Msg::Tick(later)).is_empty(),
        "a reload in flight and auto-refresh off: the tick does nothing"
    );
}

#[test]
fn ip_lists_auto_refresh_is_opt_in_and_bypasses_when_due() {
    use crate::tui::test_support::model_with_ip_lists;
    let mut m = model_with_ip_lists();
    crate::tui::test_support::on_ip_lists_tab(&mut m);
    let later = std::time::Instant::now() + std::time::Duration::from_secs(61);
    assert!(
        update(&mut m, Msg::Tick(later)).is_empty(),
        "off by default"
    );
    quiet(&mut m, key('a'));
    assert!(m.objects.ip_lists.auto.enabled);
    assert!(m.any_auto_enabled());
    assert_eq!(
        update(&mut m, Msg::Tick(later)),
        vec![Cmd::LoadIpLists(pce::CacheMode::Bypass)]
    );
    assert!(m.objects.ip_lists.auto.from_tick);
}

#[test]
fn i_loads_the_workloads_once_for_the_impact_from_policy_and_the_rule_detail() {
    use crate::tui::test_support::model_with_policy;
    let mut m = model_with_policy();
    m.screen = Screen::Policy;
    assert_eq!(
        update(&mut m, key('i')),
        vec![Cmd::LoadWorkloads(pce::CacheMode::Use)]
    );
    assert_eq!(m.status.text, "loading workloads for impact (1 request)");
    assert!(update(&mut m, key('i')).is_empty());
    assert_eq!(m.status.text, "workloads already loading");
    let wl =
        serde_json::from_str(include_str!("../../../../../../fixtures/workloads.json")).unwrap();
    update(&mut m, Msg::WorkloadsLoaded(Ok((wl, 0, false))));
    assert_eq!(m.policy.rules.rows()[1].pc, "1/1", "the rules pane follows");
    assert!(update(&mut m, key('i')).is_empty());
    assert_eq!(
        m.status.text,
        "4 workloads already loaded; r on Workloads (4) refreshes them"
    );
    // In the rule detail: `i` loads, the detail stays, Esc still closes.
    let mut m = model_with_policy();
    m.screen = Screen::Policy;
    update(&mut m, kc(KeyCode::Tab));
    update(&mut m, enter());
    assert!(matches!(m.overlay, Overlay::RuleDetail(_)));
    assert_eq!(
        update(&mut m, key('i')),
        vec![Cmd::LoadWorkloads(pce::CacheMode::Use)]
    );
    assert!(
        matches!(m.overlay, Overlay::RuleDetail(_)),
        "the detail stays open"
    );
    update(&mut m, kc(KeyCode::Esc));
    assert_eq!(m.overlay, Overlay::None);
}

#[test]
fn a_truncated_or_failed_workload_read_shows_in_the_impact() {
    use crate::tui::impact::rule_detail_rows;
    use crate::tui::test_support::model_with_policy;
    let rule = "/orgs/5/sec_policy/draft/rule_sets/100/sec_rules/1";
    let mut m = model_with_policy();
    let wl =
        serde_json::from_str(include_str!("../../../../../../fixtures/workloads.json")).unwrap();
    update(&mut m, Msg::WorkloadsLoaded(Ok((wl, 0, true))));
    assert!(m.workloads.truncated);
    assert_eq!(m.policy.rules.rows()[1].pc, "≥1/≥1");
    assert_eq!(
        rule_detail_rows(&m, rule)[0].1,
        "≥1 provider · ≥1 consumer (workload list truncated)"
    );
    let mut m = model_with_policy();
    update(&mut m, Msg::WorkloadsLoaded(Err("boom".into())));
    assert_eq!(m.workloads.error.as_deref(), Some("boom"));
    assert_eq!(rule_detail_rows(&m, rule)[0].1, "unavailable (boom)");
    m.screen = Screen::Policy;
    assert_eq!(update(&mut m, key('i')).len(), 1, "i tries again");
    assert_eq!(m.workloads.error, None);
}

#[test]
fn i_in_a_rule_confirm_loads_workloads_and_keeps_the_request_on_screen() {
    use crate::tui::test_support::{label_post_plan, model_with_policy, rule_post_plan};
    let mut m = writes_on(model_with_policy());
    update(
        &mut m,
        Msg::Planned(Ok((rule_post_plan(), pce::write::ObjectKind::Rule))),
    );
    let shown = m.overlay.clone();
    assert!(matches!(shown, Overlay::ConfirmWrite { .. }));
    assert_eq!(
        m.policy.confirm_impact.as_deref(),
        Some("impact: press i to load workloads (1 request)"),
        "computed when the confirm opens"
    );
    assert_eq!(
        update(&mut m, key('i')),
        vec![Cmd::LoadWorkloads(pce::CacheMode::Use)]
    );
    assert_eq!(m.overlay, shown, "the confirm stays");
    assert_eq!(
        m.policy.confirm_impact.as_deref(),
        Some("impact: loading workloads…")
    );
    let wl =
        serde_json::from_str(include_str!("../../../../../../fixtures/workloads.json")).unwrap();
    update(&mut m, Msg::WorkloadsLoaded(Ok((wl, 0, false))));
    assert_eq!(m.overlay, shown, "the answer never replaces the confirm");
    assert_eq!(
        m.policy.confirm_impact.as_deref(),
        Some("impact: 1 provider · 0 consumers")
    );
    let cmds = update(&mut m, key('y'));
    assert!(
        matches!(cmds.as_slice(), [Cmd::ExecuteWrite { .. }]),
        "{cmds:?}"
    );
    // A label's confirm has no impact: `i` does nothing there.
    let mut m = writes_on(model_with_policy());
    update(
        &mut m,
        Msg::Planned(Ok((label_post_plan(), pce::write::ObjectKind::Label))),
    );
    assert_eq!(m.policy.confirm_impact, None);
    assert!(update(&mut m, key('i')).is_empty());
    assert!(!m.workloads.loading);
}

#[test]
fn writes_command_shows_or_asks_for_the_toggle() {
    use crate::tui::model::WRITES_OFF;
    let mut m = model_fixture();
    let cmd = |on| Msg::Command(Command::Writes(on));
    assert!(update(&mut m, cmd(None)).is_empty());
    assert_eq!(m.status.kind, StatusKind::Info);
    assert_eq!(m.status.text, WRITES_OFF);
    assert_eq!(update(&mut m, cmd(Some(true))), vec![Cmd::SetWrites(true)]);
    assert_eq!(
        update(&mut m, cmd(Some(false))),
        vec![Cmd::SetWrites(false)]
    );
    assert!(
        !m.writes.enabled,
        "only the runtime's answer changes the state"
    );
    let mut m = writes_on(model_fixture());
    assert!(update(&mut m, cmd(None)).is_empty());
    assert_eq!(m.status.text, "writes on");
    m.writes.reason = None;
    m.writes.enabled = false;
    update(&mut m, cmd(None));
    assert_eq!(m.status.text, "writes off: writes are not enabled");
}

#[test]
fn writes_toggle_waits_for_a_write_or_a_profile_switch() {
    for on in [true, false] {
        let mut m = writes_on(model_fixture());
        m.writes.in_flight = true;
        assert!(update(&mut m, Msg::Command(Command::Writes(Some(on)))).is_empty());
        assert_eq!(m.status.kind, StatusKind::Warn);
        assert_eq!(m.status.text, "finish or cancel the write first");
        let mut m = writes_on(model_fixture());
        m.switching = true;
        assert!(update(&mut m, Msg::Command(Command::Writes(Some(on)))).is_empty());
        assert_eq!(m.status.kind, StatusKind::Info);
        assert_eq!(m.status.text, "profile switch in progress");
    }
}

#[test]
fn writes_set_replaces_the_state_and_says_so() {
    use crate::tui::model::{WRITES_OFF, WritesState};
    let mut m = model_fixture();
    assert!(update(&mut m, Msg::WritesSet(WritesState::on())).is_empty());
    assert_eq!(m.writes, WritesState::on());
    assert_eq!(m.status.kind, StatusKind::Info);
    assert_eq!(m.status.text, "writes on");
    let denied = "writes are not enabled for this profile; set writes = true in the profile or ILLUMIO_WRITES=1";
    let mut off = WritesState::off(denied);
    off.in_flight = true;
    assert!(update(&mut m, Msg::WritesSet(off)).is_empty());
    assert_eq!(m.writes, WritesState::off(denied), "in_flight stays false");
    assert_eq!(m.status.kind, StatusKind::Warn);
    assert_eq!(m.status.text, format!("writes off: {denied}"));
    update(&mut m, Msg::WritesSet(WritesState::off(WRITES_OFF)));
    assert_eq!(m.status.text, WRITES_OFF);
    assert!(m.writes.reason.is_some());
}

fn ctrl(c: char) -> Msg {
    Msg::Key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL))
}

fn open_form(m: &mut Model, intent: EditIntent) {
    let idx = m.index().cloned();
    let form = edit::form_for(intent, idx.as_deref(), m.flow_prefill()).expect("a form");
    m.overlay = Overlay::Form(form);
}

fn form_value(m: &Model, label: &str) -> String {
    match &m.overlay {
        Overlay::Form(f) => f.value(label).to_string(),
        other => panic!("{other:?}"),
    }
}

#[test]
fn ctrl_n_completes_a_new_labels_key_then_its_values_without_a_request() {
    let mut m = writes_on(model_fixture());
    open_form(&mut m, EditIntent::CreateLabel);
    for c in "RO".chars() {
        update(&mut m, key(c));
    }
    assert!(update(&mut m, ctrl('n')).is_empty());
    assert_eq!(form_value(&m, "Key"), "role", "real case");
    update(&mut m, kc(KeyCode::Tab));
    assert!(update(&mut m, ctrl('n')).is_empty());
    assert_eq!(form_value(&m, "Value"), "db", "the Key field's values");
    assert!(update(&mut m, ctrl('n')).is_empty());
    assert_eq!(form_value(&m, "Value"), "web");
    assert!(update(&mut m, ctrl('p')).is_empty());
    assert_eq!(form_value(&m, "Value"), "db");
    match &m.overlay {
        Overlay::Form(f) => {
            assert_eq!(
                f.completion_strip(),
                Some(vec![("db".to_string(), true), ("web".to_string(), false)])
            );
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_completed_key_continues_into_its_values_and_esc_restores_the_typed_token() {
    let mut m = writes_on(model_fixture());
    open_form(
        &mut m,
        EditIntent::CreateRule {
            ruleset_href: "/orgs/5/sec_policy/draft/rule_sets/100".into(),
        },
    );
    for c in "env=prod, ap".chars() {
        update(&mut m, key(c));
    }
    assert!(update(&mut m, ctrl('n')).is_empty());
    assert_eq!(
        form_value(&m, "Providers"),
        "env=prod, app=",
        "the key first"
    );
    assert!(update(&mut m, ctrl('n')).is_empty());
    assert_eq!(
        form_value(&m, "Providers"),
        "env=prod, app=shop",
        "the second Ctrl-N: app's only value"
    );
    assert!(update(&mut m, kc(KeyCode::Esc)).is_empty());
    assert_eq!(form_value(&m, "Providers"), "env=prod, ap", "as typed");
    assert!(
        matches!(m.overlay, Overlay::Form(_)),
        "Esc ended the completion only"
    );
    update(&mut m, kc(KeyCode::Esc));
    assert_eq!(m.overlay, Overlay::None, "a second Esc cancels the form");
    // Ctrl-P opens on the last candidate.
    open_form(
        &mut m,
        EditIntent::CreateRule {
            ruleset_href: "/orgs/5/sec_policy/draft/rule_sets/100".into(),
        },
    );
    update(&mut m, key('e'));
    assert!(update(&mut m, ctrl('p')).is_empty());
    assert_eq!(form_value(&m, "Providers"), "group:All prod envs");
}

#[test]
fn ctrl_n_does_nothing_without_an_index_or_in_a_text_field() {
    let mut m = writes_on(model_fixture());
    open_form(&mut m, EditIntent::CreateIpList);
    update(&mut m, key('C'));
    let before = m.overlay.clone();
    assert!(update(&mut m, ctrl('n')).is_empty());
    assert_eq!(m.overlay, before, "a text field");
    open_form(&mut m, EditIntent::CreateLabel);
    update(&mut m, key('r'));
    m.connection = Connection::Connected {
        index: None,
        index_age_secs: 0,
    };
    let before = m.overlay.clone();
    assert!(update(&mut m, ctrl('n')).is_empty());
    assert!(update(&mut m, ctrl('p')).is_empty());
    assert_eq!(m.overlay, before, "no index: no candidates");
}

#[test]
fn a_w_forms_ruleset_completes_from_the_loaded_draft_rulesets() {
    use crate::tui::test_support::{fixture_index, model_with_flows};
    let mut m = writes_on(model_with_flows());
    update(&mut m, key('j'));
    update(&mut m, key('w'));
    update(&mut m, key('g'));
    assert!(update(&mut m, ctrl('n')).is_empty());
    assert_eq!(
        form_value(&m, "Ruleset"),
        "g",
        "nothing loaded: no candidates"
    );
    let raw: Vec<pce::models::RuleSet> =
        serde_json::from_str(include_str!("../../../../../../fixtures/rule_sets.json")).unwrap();
    m.policy.set_rule_sets(raw, &fixture_index(), None);
    m.policy.loaded_at = Some(Instant::now());
    assert!(update(&mut m, ctrl('n')).is_empty());
    assert_eq!(form_value(&m, "Ruleset"), "Global-Infra");
    // The active scope's rulesets are not draft names.
    update(&mut m, kc(KeyCode::Esc));
    update(&mut m, kc(KeyCode::Esc));
    m.policy.scope = PolicyScope::Active;
    update(&mut m, key('w'));
    update(&mut m, ctrl('u'));
    assert!(update(&mut m, ctrl('n')).is_empty());
    assert_eq!(form_value(&m, "Ruleset"), "");
}

#[test]
fn a_rule_forms_scope_warnings_follow_it_to_the_confirm() {
    use crate::tui::test_support::{model_with_policy, rule_post_plan};
    let mut m = writes_on(model_with_policy());
    m.screen = Screen::Policy;
    let shop = m.policy.raw[0].href.clone();
    open_form(&mut m, EditIntent::CreateRule { ruleset_href: shop });
    let Overlay::Form(f) = &mut m.overlay else {
        panic!("a form");
    };
    for (label, value) in [
        ("Providers", "role=db"),
        ("Consumers", "role=web"),
        ("Services", "443/tcp"),
        ("Scope", "extra"),
    ] {
        f.fields
            .iter_mut()
            .find(|x| x.label == label)
            .unwrap()
            .value = value.into();
    }
    let Overlay::Form(f) = &m.overlay else {
        unreachable!()
    };
    assert_eq!(
        m.rule_scope_notes(f),
        Some((
            "role=web in any app, env".to_string(),
            vec!["matches role=web in every app, env".to_string()]
        )),
        "the form shows them while it is open"
    );
    assert_eq!(update(&mut m, enter()).len(), 1, "a warning never blocks");
    update(
        &mut m,
        Msg::Planned(Ok((rule_post_plan(), pce::write::ObjectKind::Rule))),
    );
    assert!(matches!(m.overlay, Overlay::ConfirmWrite { .. }));
    assert_eq!(m.status.kind, StatusKind::Warn);
    assert_eq!(
        m.status.text,
        "⚠ matches role=web in every app, env · review the request: y sends it, n cancels"
    );
    assert!(m.policy.scope_warnings.is_empty(), "taken by the plan");
}
