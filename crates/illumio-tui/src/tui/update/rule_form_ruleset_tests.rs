//! Issue #30: a `w` form opened off the Policy screen's draft list.
use super::*;
use crate::tui::form::Form;
use crate::tui::model::{Model, Screen, WritesState};
use crate::tui::msg::{Cmd, Msg};
use crate::tui::test_support::model_with_flows;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

fn key(c: char) -> Msg {
    Msg::Key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE))
}

fn ctrl(c: char) -> Msg {
    Msg::Key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL))
}

fn kc(code: KeyCode) -> Msg {
    Msg::Key(KeyEvent::new(code, KeyModifiers::NONE))
}

fn drafts() -> Vec<pce::models::RuleSet> {
    serde_json::from_str(include_str!("../../../../../fixtures/rule_sets.json")).unwrap()
}

fn drafts_loaded(raw: Vec<pce::models::RuleSet>) -> Msg {
    Msg::RuleSetsLoaded {
        scope: PolicyScope::Draft,
        result: Ok((raw, 0, false)),
    }
}

fn form(m: &Model) -> &Form {
    match &m.overlay {
        Overlay::Form(f) => f,
        other => panic!("{other:?}"),
    }
}

const DRAFT_LOAD: Cmd = Cmd::LoadRuleSets {
    scope: PolicyScope::Draft,
    mode: CacheMode::Use,
};

/// `w` on the blocked flow (row 1, destination app=shop env=prod).
fn w_form() -> (Model, Vec<Cmd>) {
    let mut m = model_with_flows();
    m.writes = WritesState::on();
    update(&mut m, key('j'));
    let cmds = update(&mut m, key('w'));
    (m, cmds)
}

#[test]
fn w_without_loaded_rulesets_loads_the_drafts_once_and_picks_the_scope() {
    let (mut m, cmds) = w_form();
    assert_eq!(cmds, vec![DRAFT_LOAD], "one cached load");
    assert_eq!(form(&m).value("Ruleset"), "", "the form opens at once");
    // Keys while it loads send nothing more.
    assert!(update(&mut m, kc(KeyCode::Tab)).is_empty());
    assert!(update(&mut m, kc(KeyCode::BackTab)).is_empty());
    assert_eq!(
        m.status.text,
        "loading the draft rulesets for the form · 1 req"
    );
    assert!(update(&mut m, drafts_loaded(drafts())).is_empty());
    assert_eq!(m.status.text, "2 draft rulesets for the form");
    let f = form(&m);
    assert_eq!(
        f.value("Ruleset"),
        "Shop-Prod",
        "its scope covers the destination"
    );
    // Closed and opened again: the list is held, nothing is sent.
    update(&mut m, kc(KeyCode::Esc));
    assert!(update(&mut m, key('w')).is_empty());
    assert_eq!(form(&m).value("Ruleset"), "Shop-Prod");
}

#[test]
fn the_policy_screen_on_active_keeps_its_list_and_the_form_gets_the_drafts() {
    let mut m = model_with_flows();
    m.writes = WritesState::on();
    let mut active = drafts();
    for rs in &mut active {
        rs.href = rs.href.replace("/draft/", "/active/");
        rs.name = format!("Active-{}", rs.name);
    }
    m.policy.scope = PolicyScope::Active;
    m.policy.set_rule_sets(
        active.clone(),
        &crate::tui::test_support::fixture_index(),
        None,
    );
    m.policy.loaded_at = Some(Instant::now());
    update(&mut m, key('j'));
    assert_eq!(update(&mut m, key('w')), vec![DRAFT_LOAD]);
    assert!(!m.policy.loading, "the Policy screen is not reloading");
    assert!(update(&mut m, drafts_loaded(drafts())).is_empty());
    assert_eq!(form(&m).value("Ruleset"), "Shop-Prod");
    assert_eq!(m.policy.scope, PolicyScope::Active);
    assert_eq!(m.policy.raw, active, "the screen's list as chosen");
    // Another form reuses the drafts; nothing is sent.
    update(&mut m, kc(KeyCode::Esc));
    assert!(update(&mut m, key('w')).is_empty());
    // A rule write forgets them: the next form loads them again, once.
    update(&mut m, kc(KeyCode::Esc));
    reload_kind_after_write(&mut m, pce::write::ObjectKind::Rule);
    assert_eq!(update(&mut m, key('w')), vec![DRAFT_LOAD]);
    update(&mut m, kc(KeyCode::Esc));
    assert!(update(&mut m, key('w')).is_empty(), "already on its way");
}

#[test]
fn a_scope_toggle_racing_the_forms_load_never_leaves_it_stuck() {
    let mut m = model_with_flows();
    m.writes = WritesState::on();
    m.policy.scope = PolicyScope::Active;
    update(&mut m, key('j'));
    assert_eq!(update(&mut m, key('w')), vec![DRAFT_LOAD]);
    update(&mut m, kc(KeyCode::Esc));
    // `d` on Policy while the form's load is on its way.
    m.screen = Screen::Policy;
    assert_eq!(update(&mut m, key('d')), vec![DRAFT_LOAD]);
    assert!(!m.policy.form_drafts_loading, "the toggle forgets it");
    // Both answers land as the Policy screen's draft list.
    update(&mut m, drafts_loaded(drafts()));
    update(&mut m, drafts_loaded(drafts()));
    assert!(!m.policy.form_drafts_loading);
    assert_eq!(m.policy.scope, PolicyScope::Draft);
    // Back to active: a new form asks for the drafts again.
    assert_eq!(update(&mut m, key('d')).len(), 1);
    update(
        &mut m,
        Msg::RuleSetsLoaded {
            scope: PolicyScope::Active,
            result: Ok((Vec::new(), 0, false)),
        },
    );
    m.screen = Screen::Traffic;
    assert_eq!(update(&mut m, key('w')), vec![DRAFT_LOAD]);
    // The same answer arriving with the form's flag set by hand (the
    // race the other way round) clears it as well.
    m.policy.scope = PolicyScope::Draft;
    m.policy.form_drafts_loading = true;
    update(&mut m, drafts_loaded(drafts()));
    assert!(!m.policy.form_drafts_loading);
}

#[test]
fn a_late_answer_never_overwrites_what_was_typed_nor_reopens_a_closed_form() {
    let (mut m, _) = w_form();
    for c in "Mine".chars() {
        update(&mut m, key(c));
    }
    update(&mut m, drafts_loaded(drafts()));
    assert_eq!(form(&m).value("Ruleset"), "Mine", "as typed");
    // Closed before the answer: it stays closed.
    let (mut m, _) = w_form();
    update(&mut m, kc(KeyCode::Esc));
    update(&mut m, drafts_loaded(drafts()));
    assert_eq!(m.overlay, Overlay::None);
    // The same with the Policy screen on active.
    let mut m = model_with_flows();
    m.writes = WritesState::on();
    m.policy.scope = PolicyScope::Active;
    update(&mut m, key('j'));
    assert_eq!(
        update(&mut m, key('w')),
        vec![DRAFT_LOAD],
        "the drafts only"
    );
    update(&mut m, kc(KeyCode::Esc));
    update(&mut m, drafts_loaded(drafts()));
    assert_eq!(m.overlay, Overlay::None);
    assert!(m.policy.form_drafts.is_some(), "held for the next form");
}

#[test]
fn ruleset_names_complete_once_the_list_arrives_after_the_form_opened() {
    let (mut m, _) = w_form();
    update(&mut m, ctrl('u'));
    update(&mut m, key('o'));
    assert!(update(&mut m, ctrl('n')).is_empty());
    assert_eq!(form(&m).value("Ruleset"), "o", "nothing loaded yet");
    let mut sets = drafts();
    let mut other = sets[0].clone();
    other.href = "/orgs/5/sec_policy/draft/rule_sets/102".into();
    other.name = "Shop-Dev".into();
    other.scopes.clear();
    sets.push(other);
    update(&mut m, drafts_loaded(sets));
    assert_eq!(form(&m).value("Ruleset"), "o", "typed: kept");
    // Case-insensitive substring, Ctrl-N cycles and wraps.
    update(&mut m, ctrl('u'));
    for c in "SHOP".chars() {
        update(&mut m, key(c));
    }
    assert!(update(&mut m, ctrl('n')).is_empty());
    assert_eq!(form(&m).value("Ruleset"), "Shop-Prod");
    update(&mut m, ctrl('n'));
    assert_eq!(form(&m).value("Ruleset"), "Shop-Dev");
    update(&mut m, ctrl('n'));
    assert_eq!(form(&m).value("Ruleset"), "Shop-Prod");
    update(&mut m, kc(KeyCode::Esc));
    assert_eq!(
        form(&m).value("Ruleset"),
        "SHOP",
        "Esc puts the typed text back"
    );
    update(&mut m, ctrl('u'));
    for c in "infra".chars() {
        update(&mut m, key(c));
    }
    update(&mut m, ctrl('n'));
    assert_eq!(form(&m).value("Ruleset"), "Global-Infra", "inside the name");
}

#[test]
fn the_rulesets_that_cover_the_destination_complete_first() {
    let (mut m, _) = w_form();
    let label = |h: &str| serde_json::json!({"label": {"href": h}});
    let mut sets = drafts();
    // Listed before the ones that cover app=shop env=prod.
    let mut first = sets[0].clone();
    first.href = "/orgs/5/sec_policy/draft/rule_sets/103".into();
    first.name = "A-Prod-EU".into();
    first.scopes = vec![vec![
        serde_json::from_value(label("/orgs/5/labels/4")).unwrap(),
        serde_json::from_value(label("/orgs/5/labels/5")).unwrap(),
    ]];
    sets.insert(0, first);
    let mut shop = sets[1].clone();
    shop.href = "/orgs/5/sec_policy/draft/rule_sets/102".into();
    shop.name = "Shop".into();
    shop.scopes = vec![vec![
        serde_json::from_value(label("/orgs/5/labels/3")).unwrap(),
    ]];
    sets.push(shop);
    update(&mut m, drafts_loaded(sets));
    assert_eq!(form(&m).value("Ruleset"), "Shop-Prod", "scoped exactly");
    assert_eq!(
        form(&m).fields[0].hint,
        "2 rulesets cover app=shop, env=prod: Ctrl-N lists them first"
    );
    update(&mut m, ctrl('u'));
    update(&mut m, ctrl('n'));
    let Some(strip) = form(&m).completion_strip() else {
        panic!("no completion")
    };
    let names: Vec<&str> = strip.iter().map(|(s, _)| s.as_str()).collect();
    assert_eq!(names, ["Shop-Prod", "Shop", "A-Prod-EU", "Global-Infra"]);
}

#[test]
fn a_new_name_is_created_with_ctrl_r_and_the_refusal_names_it() {
    use crate::tui::edit::WriteIntent;
    let (mut m, _) = w_form();
    update(&mut m, drafts_loaded(drafts()));
    // An existing name: Ctrl-R refuses without a request.
    assert!(update(&mut m, ctrl('r')).is_empty());
    assert_eq!(
        form(&m).error.as_deref(),
        Some(r#"ruleset "Shop-Prod" exists: Enter adds the rule to it"#)
    );
    // No name: says what to type.
    update(&mut m, ctrl('u'));
    assert!(update(&mut m, ctrl('r')).is_empty());
    assert_eq!(
        form(&m).error.as_deref(),
        Some("type the new ruleset's name, then Ctrl-R")
    );
    // Enter on a new name: the planner's refusal names Ctrl-R and the scope.
    for c in "New-App".chars() {
        update(&mut m, key(c));
    }
    assert_eq!(update(&mut m, kc(KeyCode::Enter)).len(), 1);
    let e = r#"no ruleset named "New-App"; similar: Shop-Prod"#;
    update(&mut m, Msg::Planned(Err(e.into())));
    let want = r#"no ruleset named "New-App": Ctrl-R creates it scoped app=shop, env=prod; similar: Shop-Prod"#;
    assert_eq!(form(&m).error.as_deref(), Some(want));
    assert!(
        form(&m).ruleset_offer_armed().is_some(),
        "the hint row shows the offer"
    );
    assert_eq!(m.status.text, format!("write: {want}"));
    // Ctrl-R plans the ruleset, the rule waits for it.
    let cmds = update(&mut m, ctrl('r'));
    assert!(
        matches!(cmds.as_slice(), [Cmd::PlanWrite(WriteIntent::CreateRuleSetIfMissing(d))] if d.name == "New-App"),
        "{cmds:?}"
    );
    assert!(m.policy.rule_after_ruleset.is_some());
    // Typed straight away, no Enter first: the same.
    let (mut m, _) = w_form();
    update(&mut m, drafts_loaded(drafts()));
    update(&mut m, ctrl('u'));
    for c in "Other".chars() {
        update(&mut m, key(c));
    }
    let cmds = update(&mut m, ctrl('r'));
    assert!(
        matches!(cmds.as_slice(), [Cmd::PlanWrite(WriteIntent::CreateRuleSetIfMissing(d))] if d.name == "Other"),
        "{cmds:?}"
    );
}

/// Ctrl-R on `name` in the `w` form; returns the commands.
fn ctrl_r_on(m: &mut Model, name: &str) -> Vec<Cmd> {
    update(m, ctrl('u'));
    for c in name.chars() {
        update(m, key(c));
    }
    update(m, ctrl('r'))
}

fn exists(href: &str, name: &str) -> Msg {
    Msg::Planned(Ok((
        pce::write::Plan::Noop {
            existing_href: href.into(),
            reason: format!("rule_set {name} exists"),
        },
        pce::write::ObjectKind::RuleSet,
    )))
}

#[test]
fn ctrl_r_on_a_ruleset_that_exists_brings_the_form_back_naming_it() {
    use crate::tui::edit::{RulesetTarget, WriteIntent};
    // The drafts still loading: the plan's fresh read decides.
    let (mut m, _) = w_form();
    let cmds = ctrl_r_on(&mut m, "shop-prod");
    assert!(
        matches!(
            cmds.as_slice(),
            [Cmd::PlanWrite(WriteIntent::CreateRuleSetIfMissing(_))]
        ),
        "{cmds:?}"
    );
    // The list arrives meanwhile; the planner finds the name: nothing
    // is created or chained, the form is back with its scope.
    update(&mut m, drafts_loaded(drafts()));
    let existing = "/orgs/5/sec_policy/draft/rule_sets/100";
    assert!(update(&mut m, exists(existing, "shop-prod")).is_empty());
    assert!(!m.writes.in_flight);
    assert!(m.policy.rule_after_ruleset.is_none());
    let note =
        r#"ruleset "Shop-Prod" exists (scope app=shop, env=prod): Enter adds the rule to it"#;
    let f = form(&m);
    assert_eq!(f.value("Ruleset"), "Shop-Prod", "its own name");
    assert_eq!(f.error.as_deref(), Some(note));
    assert_eq!(f.fields[0].hint, note);
    assert_eq!(f.value("Consumers"), "iplist:Corp", "the rest as typed");
    assert_eq!(m.status.text, note);
    // Enter plans the rule into it as usual.
    let cmds = update(&mut m, kc(KeyCode::Enter));
    assert!(
        matches!(
            cmds.as_slice(),
            [Cmd::PlanWrite(WriteIntent::CreateRule { ruleset: RulesetTarget::Name(n), .. })]
                if n == "Shop-Prod"
        ),
        "{cmds:?}"
    );
    // One that does not cover the destination says so. The held list
    // lacks it when Ctrl-R is pressed (stale), and has it by the answer.
    let label = |h: &str| -> pce::models::Actor {
        serde_json::from_value(serde_json::json!({"label": {"href": h}})).unwrap()
    };
    let mut eu = drafts()[0].clone();
    eu.href = "/orgs/5/sec_policy/draft/rule_sets/7".into();
    eu.name = "Shop-EU".into();
    eu.scopes = vec![vec![label("/orgs/5/labels/3"), label("/orgs/5/labels/5")]];
    let (mut m, _) = w_form();
    update(&mut m, drafts_loaded(drafts()));
    assert_eq!(ctrl_r_on(&mut m, "Shop-EU").len(), 1);
    m.policy.raw.push(eu);
    update(
        &mut m,
        exists("/orgs/5/sec_policy/draft/rule_sets/7", "Shop-EU"),
    );
    assert_eq!(
        form(&m).error.as_deref(),
        Some(
            r#"ruleset "Shop-EU" exists (scope app=shop, loc=eu): Enter adds the rule to it · it does not cover this flow's destination"#
        )
    );
    // Not in the held list: the name as typed, no scope.
    let (mut m, _) = w_form();
    ctrl_r_on(&mut m, "Made-Elsewhere");
    update(
        &mut m,
        exists("/orgs/5/sec_policy/draft/rule_sets/9", "Made-Elsewhere"),
    );
    assert_eq!(
        form(&m).error.as_deref(),
        Some(r#"ruleset "Made-Elsewhere" exists: Enter adds the rule to it"#)
    );
}

#[test]
fn ctrl_r_refuses_a_name_several_rulesets_share() {
    let (mut m, _) = w_form();
    let mut sets = drafts();
    let mut twin = sets[0].clone();
    twin.href = "/orgs/5/sec_policy/draft/rule_sets/102".into();
    twin.name = "shop-prod".into();
    sets.push(twin);
    update(&mut m, drafts_loaded(sets));
    // Loaded: refused without a request, both hrefs named.
    assert!(ctrl_r_on(&mut m, "SHOP-PROD").is_empty());
    assert_eq!(
        form(&m).error.as_deref(),
        Some(
            r#""SHOP-PROD" names several rulesets: /orgs/5/sec_policy/draft/rule_sets/100, /orgs/5/sec_policy/draft/rule_sets/102; use the href"#
        )
    );
    // One the list has: refused as existing.
    assert!(ctrl_r_on(&mut m, "global-infra").is_empty());
    assert_eq!(
        form(&m).error.as_deref(),
        Some(r#"ruleset "global-infra" exists: Enter adds the rule to it"#)
    );
    // Not loaded: the planner's own "several" refusal brings the form back.
    let (mut m, _) = w_form();
    assert_eq!(ctrl_r_on(&mut m, "Dup").len(), 1);
    let e = r#""Dup" matches several rulesets: /a, /b"#;
    update(&mut m, Msg::Planned(Err(e.into())));
    assert_eq!(form(&m).error.as_deref(), Some(e));
    assert_eq!(form(&m).value("Ruleset"), "Dup", "as typed");
}

#[test]
fn tab_in_the_ruleset_field_completes_a_partial_name_and_then_moves_on() {
    let (mut m, _) = w_form();
    update(&mut m, drafts_loaded(drafts()));
    // An exact name: Tab moves on.
    assert_eq!(form(&m).value("Ruleset"), "Shop-Prod");
    update(&mut m, kc(KeyCode::Tab));
    assert_eq!(form(&m).active, 1);
    update(&mut m, kc(KeyCode::BackTab));
    // A partial name: Tab completes, the strip shows, the field stays.
    update(&mut m, ctrl('u'));
    for c in "prod".chars() {
        update(&mut m, key(c));
    }
    assert!(update(&mut m, kc(KeyCode::Tab)).is_empty());
    let f = form(&m);
    assert_eq!((f.active, f.value("Ruleset")), (0, "Shop-Prod"));
    assert!(f.completion.is_some());
    // Again: keeps the shown name and moves on.
    update(&mut m, kc(KeyCode::Tab));
    let f = form(&m);
    assert_eq!((f.active, f.value("Ruleset")), (1, "Shop-Prod"));
    assert!(f.completion.is_none());
    // No name matches, or nothing typed: Tab moves on.
    for text in ["zzz", ""] {
        update(&mut m, kc(KeyCode::BackTab));
        update(&mut m, ctrl('u'));
        for c in text.chars() {
            update(&mut m, key(c));
        }
        update(&mut m, kc(KeyCode::Tab));
        assert_eq!((form(&m).active, form(&m).value("Ruleset")), (1, text));
    }
    // An exact name in another case: moves on too.
    update(&mut m, kc(KeyCode::BackTab));
    update(&mut m, ctrl('u'));
    for c in "global-infra".chars() {
        update(&mut m, key(c));
    }
    update(&mut m, kc(KeyCode::Tab));
    assert_eq!(form(&m).active, 1);
}

#[test]
fn a_policy_write_drops_the_answer_of_a_form_load_still_on_its_way() {
    let mut m = model_with_flows();
    m.writes = WritesState::on();
    m.policy.scope = PolicyScope::Active;
    update(&mut m, key('j'));
    assert_eq!(update(&mut m, key('w')), vec![DRAFT_LOAD]);
    update(&mut m, kc(KeyCode::Esc));
    // A rule written meanwhile: the answer on its way is from before it.
    reload_kind_after_write(&mut m, pce::write::ObjectKind::Rule);
    assert!(update(&mut m, drafts_loaded(drafts())).is_empty());
    assert!(m.policy.form_drafts.is_none(), "dropped");
    assert!(!m.policy.form_drafts_loading && !m.policy.form_drafts_stale);
    // The next form loads them again, and keeps that answer.
    assert_eq!(update(&mut m, key('w')), vec![DRAFT_LOAD]);
    update(&mut m, drafts_loaded(drafts()));
    assert!(m.policy.form_drafts.is_some());
    assert_eq!(form(&m).value("Ruleset"), "Shop-Prod");
    // A form open while the dropped answer lands asks again, once.
    update(&mut m, kc(KeyCode::Esc));
    m.policy.form_drafts = None;
    assert_eq!(update(&mut m, key('w')), vec![DRAFT_LOAD]);
    reload_kind_after_write(&mut m, pce::write::ObjectKind::Rule);
    assert_eq!(update(&mut m, drafts_loaded(drafts())), vec![DRAFT_LOAD]);
    assert!(m.policy.form_drafts_loading && !m.policy.form_drafts_stale);
    update(&mut m, drafts_loaded(drafts()));
    assert_eq!(form(&m).value("Ruleset"), "Shop-Prod");
    // Without a load on its way, a write drops nothing later.
    update(&mut m, kc(KeyCode::Esc));
    reload_kind_after_write(&mut m, pce::write::ObjectKind::Rule);
    assert!(!m.policy.form_drafts_stale);
}

const HREF: &str = "/orgs/5/traffic_flows/async_queries/q-1";

/// `w_form` on a result of this session's query `HREF`, the drafts loaded.
fn w_form_on_query() -> Model {
    let (mut m, _) = w_form();
    m.traffic.data.as_mut().unwrap().hrefs = vec![HREF.into()];
    update(&mut m, drafts_loaded(drafts()));
    m
}

/// The plan of `kind` answers, `y` sends it and the write succeeds:
/// the commands of that answer.
fn send(
    m: &mut Model,
    plan: pce::write::Plan,
    kind: pce::write::ObjectKind,
    href: &str,
) -> Vec<Cmd> {
    update(m, Msg::Planned(Ok((plan, kind))));
    let cmds = update(m, key('y'));
    assert!(
        matches!(cmds.as_slice(), [Cmd::ExecuteWrite { .. }]),
        "{cmds:?}"
    );
    update(
        m,
        Msg::WriteDone {
            kind,
            method: Some(pce::WriteMethod::Post),
            result: Ok(pce::write::Outcome::Written(pce::write::Written {
                href: href.into(),
                pending: true,
                status: 201,
                object: serde_json::Value::Null,
                dropped_fields: vec![],
                warnings: vec![],
                audit_failed: None,
            })),
        },
    )
}

const RULE: &str = "/orgs/5/sec_policy/draft/rule_sets/100/sec_rules/9";

fn redrafts(cmds: &[Cmd]) -> usize {
    cmds.iter()
        .filter(|c| matches!(c, Cmd::RunDraft { .. }))
        .count()
}

#[test]
fn a_rule_from_the_w_form_evaluates_the_result_against_the_draft() {
    use crate::tui::test_support::rule_post_plan;
    let mut m = w_form_on_query();
    assert!(!m.traffic.draft_loaded(), "no draft on screen");
    assert_eq!(update(&mut m, kc(KeyCode::Enter)).len(), 1);
    let cmds = send(&mut m, rule_post_plan(), pce::write::ObjectKind::Rule, RULE);
    assert_eq!(redrafts(&cmds), 1, "{cmds:?}");
    assert!(m.traffic.is_draft_running());
    assert_eq!(
        m.status.text,
        format!("created {RULE} (pending) · draft re-evaluation running · Esc cancels")
    );
    assert!(!m.policy.write_from_flow, "taken by the answer");
    // The next write, not from the form, once that run is cancelled: none.
    assert!(m.traffic.cancel_draft_run());
    let cmds = send(&mut m, rule_post_plan(), pce::write::ObjectKind::Rule, RULE);
    assert_eq!(redrafts(&cmds), 0, "{cmds:?}");
    // A cancelled confirm sends nothing, the next unrelated write none.
    let mut m = w_form_on_query();
    update(&mut m, kc(KeyCode::Enter));
    update(
        &mut m,
        Msg::Planned(Ok((rule_post_plan(), pce::write::ObjectKind::Rule))),
    );
    assert!(update(&mut m, key('n')).is_empty());
    let label = crate::tui::test_support::label_post_plan();
    let cmds = send(
        &mut m,
        label,
        pce::write::ObjectKind::Label,
        "/orgs/5/labels/9",
    );
    assert_eq!(redrafts(&cmds), 0, "{cmds:?}");
}

#[test]
fn a_rule_from_the_w_form_asks_nothing_without_a_query_of_this_session_or_while_one_runs() {
    use crate::tui::test_support::rule_post_plan;
    // A cached result: `D` would ask first; nothing on screen was cleared.
    let (mut m, _) = w_form();
    update(&mut m, drafts_loaded(drafts()));
    update(&mut m, kc(KeyCode::Enter));
    let cmds = send(&mut m, rule_post_plan(), pce::write::ObjectKind::Rule, RULE);
    assert_eq!(redrafts(&cmds), 0, "{cmds:?}");
    assert_eq!(m.status.text, format!("created {RULE} (pending)"));
    // A query running at the answer.
    let mut m = w_form_on_query();
    update(&mut m, kc(KeyCode::Enter));
    update(
        &mut m,
        Msg::Planned(Ok((rule_post_plan(), pce::write::ObjectKind::Rule))),
    );
    update(&mut m, key('y'));
    m.traffic.start_query(std::time::Instant::now());
    let cmds = update(
        &mut m,
        Msg::WriteDone {
            kind: pce::write::ObjectKind::Rule,
            method: Some(pce::WriteMethod::Post),
            result: Ok(pce::write::Outcome::Written(pce::write::Written {
                href: RULE.into(),
                pending: true,
                status: 201,
                object: serde_json::Value::Null,
                dropped_fields: vec![],
                warnings: vec![],
                audit_failed: None,
            })),
        },
    );
    assert_eq!(redrafts(&cmds), 0, "{cmds:?}");
    assert!(!m.traffic.is_draft_running(), "the query keeps running");
    // A failed write: none.
    let mut m = w_form_on_query();
    update(&mut m, kc(KeyCode::Enter));
    update(
        &mut m,
        Msg::Planned(Ok((rule_post_plan(), pce::write::ObjectKind::Rule))),
    );
    update(&mut m, key('y'));
    let cmds = update(
        &mut m,
        Msg::WriteDone {
            kind: pce::write::ObjectKind::Rule,
            method: Some(pce::WriteMethod::Post),
            result: Err(pce::write::WriteError::invalid("refused")),
        },
    );
    assert_eq!(redrafts(&cmds), 0, "{cmds:?}");
    assert!(!m.traffic.is_running());
}

/// A drafted result, Ctrl-R on a new name, and its ruleset written: the
/// rule's plan is on its way.
fn ctrl_r_ruleset_written() -> Model {
    use crate::tui::edit::WriteIntent;
    use crate::tui::test_support::with_draft_loaded;
    let mut m = with_draft_loaded(w_form_on_query());
    assert_eq!(ctrl_r_on(&mut m, "New-App").len(), 1);
    let ruleset = pce::write::Plan::Send {
        request: pce::write::WriteRequest {
            method: pce::WriteMethod::Post,
            path: "/orgs/5/sec_policy/draft/rule_sets".into(),
            body: Some(serde_json::json!({"name": "New-App"})),
            summary: "create rule_set New-App".into(),
            objects: Vec::new(),
        },
        kind: pce::write::ObjectKind::RuleSet,
    };
    let rs = "/orgs/5/sec_policy/draft/rule_sets/300";
    let cmds = send(&mut m, ruleset, pce::write::ObjectKind::RuleSet, rs);
    assert_eq!(redrafts(&cmds), 0, "{cmds:?}");
    assert!(
        cmds.iter()
            .any(|c| matches!(c, Cmd::PlanWrite(WriteIntent::CreateRule { .. }))),
        "{cmds:?}"
    );
    m
}

#[test]
fn ctrl_r_evaluates_the_draft_after_the_rule_not_the_ruleset() {
    use crate::tui::test_support::rule_post_plan;
    let mut m = ctrl_r_ruleset_written();
    assert!(!m.traffic.is_running());
    assert!(
        m.traffic.draft_loaded(),
        "an empty ruleset changes no decision"
    );
    let cmds = send(&mut m, rule_post_plan(), pce::write::ObjectKind::Rule, RULE);
    assert_eq!(redrafts(&cmds), 1, "{cmds:?}");
    assert!(m.traffic.is_draft_running() && !m.traffic.draft_loaded());
    // A cancelled rule confirm keeps the draft on screen, nothing sent.
    let mut m = ctrl_r_ruleset_written();
    update(
        &mut m,
        Msg::Planned(Ok((rule_post_plan(), pce::write::ObjectKind::Rule))),
    );
    assert!(update(&mut m, key('n')).is_empty());
    assert!(m.traffic.draft_loaded() && !m.traffic.is_running());
}
