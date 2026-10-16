//! The generator's transitions (`G` / `:gen`): opening it, its keys, its
//! rebuilds when traffic or rulesets land, and `Enter`'s plan. Pure; the
//! only requests are the cached draft rulesets on open, the focused
//! re-query on `r` (through the cost guard) and the plan on `Enter`.
use super::{
    Cmd, Model, Overlay, PendingAction, StatusLine, load_form_drafts, start_focused, writes_refused,
};
use crate::tui::gen_view::{Drafts, GenView};
use crate::tui::model::Gap;
use crossterm::event::{KeyCode, KeyEvent};
use pce::CacheMode;
use pce::generate::{GenInput, Side, keys_refusal, propose, rows};
use pce::traffic::{Focus, FocusDirection, queries_text};
use pce::write::BatchInput;

/// Opens the generator for `app` under the keys in force. Reopening the
/// same app keeps its rows, checkboxes and slider. The only request: the
/// draft rulesets when no screen or form holds them (cached, counted).
pub(super) fn open_gen(model: &mut Model, app: pce::traffic::GroupKey, from_app: bool) -> Vec<Cmd> {
    let keys = model.traffic.keys.clone();
    if let Some(e) = keys_refusal(&keys, &model.limits.scope_keys) {
        model.status = StatusLine::warn(e);
        return Vec::new();
    }
    let focus = Focus::new(app, keys).with_direction(FocusDirection::In);
    if !focus.is_label_group() {
        model.status = StatusLine::warn(format!(
            "{} is not an app of labels: the generator scopes rules by them",
            focus.app_text()
        ));
        return Vec::new();
    }
    let Some(idx) = model.index().cloned() else {
        model.status = StatusLine::info("the name index is not loaded yet");
        return Vec::new();
    };
    let window = model.traffic.filter.window.clone();
    let mut v = match model.gen_view.take() {
        Some(mut v) if v.focus.same_app(&focus) => {
            v.window = window;
            v
        }
        _ => {
            model.gen_seq += 1;
            GenView::new(focus, window, model.gen_seq)
        }
    };
    v.list = None;
    v.error = None;
    v.planning = false;
    v.all_services = idx.all_services_href().is_some();
    model.gen_from_app = from_app;
    model.gen_view = Some(v);
    let cmds = ensure_drafts(model);
    model.overlay = Overlay::Generator;
    rebuild_gen(model);
    if let Some(v) = &model.gen_view {
        model.status = StatusLine::info(format!(
            "{} · generator · {} req",
            v.focus.app_text(),
            cmds.len()
        ));
    }
    cmds
}

/// The draft rulesets for the open generator: held (nothing sent), on
/// their way, or one load now, counted in its header.
pub(super) fn ensure_drafts(model: &mut Model) -> Vec<Cmd> {
    if model.draft_rulesets().is_some() {
        if let Some(v) = model.gen_view.as_mut() {
            v.drafts = Drafts::Loaded;
        }
        return Vec::new();
    }
    let cmds = load_form_drafts(model);
    if let Some(v) = model.gen_view.as_mut() {
        v.drafts = Drafts::Loading;
        v.requests += u32::try_from(cmds.len()).unwrap_or(u32::MAX);
        v.drafts_sent = !cmds.is_empty();
    }
    cmds
}

/// What the header says when the data the generator reads may miss flows
/// (`None`: complete), and whether `r` can help.
fn gap_text(model: &Model, v: &GenView) -> (Option<String>, bool) {
    let mut sides = vec![FocusDirection::In];
    if v.outbound {
        sides.push(FocusDirection::Out);
    }
    let n = v.focus.direction.query_count();
    let app = v.focus.app_text();
    for d in sides {
        let side = v.focus.clone().with_direction(d);
        match model.traffic.complete_for(&side) {
            Ok(()) => {}
            Err(Gap::Capped { cap, focused: true }) => {
                return (
                    Some(format!(
                        "still capped at {cap}: B raises the cap, or narrow the window"
                    )),
                    false,
                );
            }
            Err(Gap::NoData) => {
                return (
                    Some(format!(
                        "no traffic loaded: r queries {app} ({})",
                        queries_text(n)
                    )),
                    true,
                );
            }
            Err(_) => {
                return (
                    Some(format!(
                        "incomplete: r re-queries {app} ({})",
                        queries_text(n)
                    )),
                    true,
                );
            }
        }
    }
    (None, false)
}

/// The rows again from the loaded flows (checkboxes kept), the gap, and
/// the proposal. After a traffic answer, `o`, and on open. No request.
pub(super) fn rebuild_gen(model: &mut Model) {
    let Some(idx) = model.index().cloned() else {
        return;
    };
    let Some(v) = model.gen_view.as_ref() else {
        return;
    };
    if v.focus.keys != model.limits.scope_keys || v.focus.keys != model.traffic.keys {
        // The label model changed under it (a profile switch).
        model.gen_view = None;
        if model.overlay == Overlay::Generator {
            model.overlay = Overlay::None;
        }
        model.status = StatusLine::warn("g changed: G reopens the generator");
        return;
    }
    let fresh = rows(
        model.traffic.flows_for(&v.focus),
        &v.focus,
        &model.limits.tier_key,
        &idx,
    );
    let (gap, requery) = gap_text(model, v);
    if let Some(v) = model.gen_view.as_mut() {
        v.set_rows(fresh);
        v.gap = gap;
        v.requery = requery;
    }
    regen(model);
}

/// The proposal again from the rows (every key press). Pure.
pub(super) fn regen(model: &mut Model) {
    let Some(idx) = model.index().cloned() else {
        return;
    };
    let p = {
        let (Some(v), Some(drafts)) = (model.gen_view.as_ref(), model.draft_rulesets()) else {
            return;
        };
        let input = GenInput {
            flows: model.traffic.flows_for(&v.focus),
            focus: &v.focus,
            tier_key: &model.limits.tier_key,
            drafts,
            idx: &idx,
            ruleset_name: &model.limits.ruleset_name,
            window: &v.window,
            outbound: v.outbound,
            picked: v.picked.as_deref(),
        };
        propose(&v.rows, v.global, &input)
    };
    if let Some(v) = model.gen_view.as_mut() {
        v.proposal = Some(p);
        v.drafts = Drafts::Loaded;
    }
}

/// Draft rulesets arrived (`Ok(hit)`) or failed to. Fills the view only.
pub(super) fn gen_drafts_answer(model: &mut Model, outcome: Result<bool, &str>) {
    let Some(v) = model.gen_view.as_mut() else {
        return;
    };
    match outcome {
        // Any draft load (its own, or the reload after a write) refreshes it.
        Ok(hit) => {
            if std::mem::take(&mut v.drafts_sent) && hit {
                v.cached += 1;
            }
            regen(model);
        }
        Err(e) if v.drafts == Drafts::Loading => v.drafts = Drafts::Failed(e.to_string()),
        Err(_) => {}
    }
}

/// `Esc` / `q`: back to the app overlay it came from (its view is kept),
/// else to the screen.
fn close_gen(model: &mut Model) {
    let back = std::mem::take(&mut model.gen_from_app) && model.app_view.is_some();
    model.overlay = if back { Overlay::App } else { Overlay::None };
}

/// The generator's keys. Only `r` (a load or the focused query, through
/// the cost guard) and `Enter` (the plan) can cause a request.
pub(super) fn gen_key(model: &mut Model, k: KeyEvent) -> Vec<Cmd> {
    let scope_keys = model.limits.scope_keys.clone();
    let Some(v) = model.gen_view.as_mut() else {
        model.overlay = Overlay::None;
        return Vec::new();
    };
    if let Some(scroll) = v.list {
        match k.code {
            KeyCode::Esc | KeyCode::Char('p') | KeyCode::Char('q') => v.list = None,
            KeyCode::Down | KeyCode::Char('j') => v.list = Some(scroll.saturating_add(1)),
            KeyCode::Up | KeyCode::Char('k') => v.list = Some(scroll.saturating_sub(1)),
            _ => {}
        }
        return Vec::new();
    }
    match k.code {
        KeyCode::Esc | KeyCode::Char('q') => close_gen(model),
        KeyCode::Down | KeyCode::Char('j') => v.move_cursor(1),
        KeyCode::Up | KeyCode::Char('k') => v.move_cursor(-1),
        KeyCode::Tab => v.switch_section(),
        KeyCode::Char(' ') => {
            if let Err(e) = v.toggle(&scope_keys) {
                model.status = StatusLine::warn(e);
            }
            regen(model);
        }
        KeyCode::Left | KeyCode::Char('h') => {
            v.slide(-1);
            regen(model);
        }
        KeyCode::Right | KeyCode::Char('l') => {
            v.slide(1);
            regen(model);
        }
        KeyCode::Char('<') => {
            v.slide_row(-1);
            regen(model);
        }
        KeyCode::Char('>') => {
            v.slide_row(1);
            regen(model);
        }
        KeyCode::Char('=') => {
            v.clear_row_stop();
            regen(model);
        }
        KeyCode::Char('o') => {
            v.toggle_outbound();
            rebuild_gen(model);
        }
        KeyCode::Char('R') => {
            match v.pick_next() {
                Ok(name) => model.status = StatusLine::info(format!("rules go into {name}")),
                Err(e) => model.status = StatusLine::info(e),
            }
            regen(model);
        }
        KeyCode::Char('p') => v.list = Some(0),
        KeyCode::Char('r') => return gen_reload(model),
        KeyCode::Enter => return gen_enter(model),
        _ => {}
    }
    Vec::new()
}

/// `r`: the draft rulesets again after a failed load, else the focused
/// query for the app when the data is incomplete (asking past the cost
/// threshold). The traffic screen's focus becomes the generator's.
fn gen_reload(model: &mut Model) -> Vec<Cmd> {
    let Some(v) = model.gen_view.as_ref() else {
        return Vec::new();
    };
    if matches!(v.drafts, Drafts::Failed(_)) {
        return ensure_drafts(model);
    }
    if !v.requery {
        model.status = StatusLine::info(match &v.gap {
            Some(g) => g.clone(),
            None => "the data is complete: nothing to re-query".into(),
        });
        return Vec::new();
    }
    let focus = v.focus.clone();
    let filter = model.traffic.filter.clone();
    model.traffic.set_focus(focus.clone());
    if let Some(reason) = filter.needs_confirmation() {
        model.overlay = Overlay::Confirm {
            reason: format!(
                "{reason}. Run {} for {} anyway? (y/n)",
                queries_text(focus.direction.query_count()),
                focus.app_text()
            ),
            action: PendingAction::GenRequery(filter, focus),
        };
        return Vec::new();
    }
    start_focused(model, filter, focus, CacheMode::Use)
}

/// `y` on `r`'s prompt: the focused query, back to the generator.
pub(super) fn gen_requery(
    model: &mut Model,
    filter: crate::tui::filter::TrafficFilter,
    focus: Focus,
) -> Vec<Cmd> {
    let cmds = start_focused(model, filter, focus, CacheMode::Use);
    model.overlay = Overlay::Generator;
    cmds
}

/// `Enter`: refused with the reason, or one `Cmd::PlanBatch` (its reads
/// are counted in the confirm). The generator stays open while it plans.
fn gen_enter(model: &mut Model) -> Vec<Cmd> {
    if writes_refused(model) {
        return Vec::new();
    }
    if model.switching {
        model.status = StatusLine::warn("a profile switch is in progress: nothing planned");
        return Vec::new();
    }
    let Some(v) = model.gen_view.as_ref() else {
        return Vec::new();
    };
    let refusal = if let Some(g) = &v.gap {
        Some(format!("{g}: no rules from incomplete data"))
    } else if !v
        .rows
        .iter()
        .any(|r| r.proposed() && (r.side == Side::Inbound || v.outbound))
    {
        Some("no row is checked: Space checks one".to_string())
    } else if let Drafts::Failed(e) = &v.drafts {
        Some(format!("the draft rulesets did not load ({e}): r retries"))
    } else if let Some(p) = &v.proposal {
        p.blocker().or_else(|| {
            (!p.writes())
                .then(|| "every proposed rule is in the draft already: nothing to write".into())
        })
    } else {
        Some("the draft rulesets are not loaded yet".to_string())
    };
    if let Some(why) = refusal {
        model.status = StatusLine::warn(why);
        return Vec::new();
    }
    let (Some(p), Some(index)) = (v.proposal.clone(), model.index().cloned()) else {
        return Vec::new();
    };
    let input = BatchInput {
        proposal: p,
        ruleset_name: model.limits.ruleset_name.clone(),
        picked: v.picked.clone(),
    };
    if let Some(v) = model.gen_view.as_mut() {
        v.planning = true;
        v.error = None;
    }
    model.writes.in_flight = true;
    model.status = StatusLine::info("planning: the draft rulesets, read fresh…");
    vec![Cmd::PlanBatch {
        input: Box::new(input),
        index,
    }]
}

#[cfg(test)]
mod tests {
    use super::super::update;
    use crate::tui::commands::Command;
    use crate::tui::gen_view::{Drafts, GenView};
    use crate::tui::model::{Model, Overlay, PendingAction, WritesState};
    use crate::tui::msg::{Cmd, FocusedOutcome, Msg, QueryOutcome};
    use crate::tui::test_support::{model_with_window, with_policy};
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use pce::CacheMode;
    use pce::generate::{Side, Stop};
    use pce::models::PolicyScope;

    fn key(c: char) -> Msg {
        Msg::Key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE))
    }

    fn kc(code: KeyCode) -> Msg {
        Msg::Key(KeyEvent::new(code, KeyModifiers::NONE))
    }

    fn view(m: &Model) -> &GenView {
        m.gen_view.as_ref().expect("the generator's state")
    }

    /// The rich flows of a known 24h window, the rulesets loaded.
    fn loaded() -> Model {
        with_policy(model_with_window("24h"))
    }

    fn open_shop(m: &mut Model) -> Vec<Cmd> {
        update(m, Msg::Command(Command::Gen(Some("shop/prod".into()))))
    }

    #[test]
    fn g_in_the_app_overlay_opens_it_for_that_app_with_nothing_sent() {
        let mut m = loaded();
        update(&mut m, Msg::Command(Command::App(Some("shop/prod".into()))));
        assert_eq!(update(&mut m, key('G')), Vec::new());
        assert_eq!(m.overlay, Overlay::Generator);
        assert!(m.gen_from_app);
        let v = view(&m);
        assert_eq!(v.focus.app_text(), "shop/prod");
        assert_eq!(v.rows.len(), 6);
        assert_eq!(v.header_meta(), "0 req · cached");
        assert_eq!(v.drafts, Drafts::Loaded);
        assert_eq!(v.proposal.as_ref().unwrap().rules.len(), 2);
        assert_eq!(update(&mut m, kc(KeyCode::Esc)), Vec::new());
        assert_eq!(m.overlay, Overlay::App, "back where it came from");
    }

    #[test]
    fn opening_without_drafts_costs_one_counted_request() {
        let mut m = model_with_window("24h");
        let cmds = open_shop(&mut m);
        assert_eq!(
            cmds,
            vec![Cmd::LoadRuleSets {
                scope: PolicyScope::Draft,
                mode: CacheMode::Use
            }]
        );
        assert_eq!(view(&m).header_meta(), "1 req");
        assert!(view(&m).proposal.is_none());
        assert!(!m.gen_from_app);
        let rs: Vec<pce::models::RuleSet> =
            serde_json::from_str(include_str!("../../../../../fixtures/rule_sets.json")).unwrap();
        update(
            &mut m,
            Msg::RuleSetsLoaded {
                scope: PolicyScope::Draft,
                result: Ok((rs, 3, true)),
            },
        );
        assert_eq!(view(&m).header_meta(), "1 req · cached");
        assert!(view(&m).proposal.is_some());
        assert_eq!(update(&mut m, kc(KeyCode::Esc)), Vec::new());
        assert_eq!(m.overlay, Overlay::None);
        assert_eq!(open_shop(&mut m), Vec::new(), "reopening sends nothing");
    }

    #[test]
    fn keys_other_than_the_scope_keys_are_refused() {
        let mut m = loaded();
        m.traffic.keys = vec!["app".into(), "env".into(), "role".into()];
        assert_eq!(
            update(
                &mut m,
                Msg::Command(Command::Gen(Some("shop/prod/web".into())))
            ),
            Vec::new()
        );
        assert_eq!(
            m.status.text,
            "the generator needs g = app,env (the scope keys)"
        );
        assert_eq!(m.overlay, Overlay::None);
        assert!(m.gen_view.is_none());
    }

    #[test]
    fn the_slider_rows_and_sections_change_the_proposal_without_requests() {
        let mut m = loaded();
        open_shop(&mut m);
        for msg in [
            kc(KeyCode::Left),
            key('j'),
            key('j'),
            key(' '),
            key('>'),
            key('o'),
            key('p'),
        ] {
            assert_eq!(update(&mut m, msg), Vec::new());
        }
        let v = view(&m);
        assert_eq!(v.global, Stop::AppAll);
        assert!(v.rows[2].checked, "shop/dev checked");
        assert_eq!(v.rows[2].stop, Some(Stop::AppPorts));
        assert!(v.outbound);
        assert_eq!(v.list, Some(0));
        let p = v.proposal.as_ref().unwrap();
        assert_eq!(p.counts().rules, 4, "intra, hr, shop/dev, crm (outbound)");
        update(&mut m, kc(KeyCode::Esc));
        assert_eq!(view(&m).list, None, "Esc leaves the list first");
        assert_eq!(m.overlay, Overlay::Generator);
        update(&mut m, kc(KeyCode::Tab));
        assert_eq!(view(&m).section, Side::Outbound);
        update(&mut m, key('='));
        update(&mut m, key('R'));
        assert_eq!(
            m.status.text,
            "one ruleset or none covers the app: nothing to pick"
        );
    }

    #[test]
    fn incomplete_data_refuses_enter_and_r_runs_the_focused_query() {
        let mut m = loaded();
        m.writes = WritesState::on();
        m.traffic.data.as_mut().unwrap().truncated = true;
        open_shop(&mut m);
        assert_eq!(
            view(&m).gap.as_deref(),
            Some("incomplete: r re-queries shop/prod (1 query)")
        );
        assert_eq!(update(&mut m, kc(KeyCode::Enter)), Vec::new());
        assert_eq!(
            m.status.text,
            "incomplete: r re-queries shop/prod (1 query): no rules from incomplete data"
        );
        let cmds = update(&mut m, key('r'));
        assert!(
            matches!(&cmds[..], [Cmd::RunFocused { focus, .. }] if focus.app_text() == "shop/prod"),
            "{cmds:?}"
        );
        assert_eq!(m.overlay, Overlay::Generator);
        // The focused answer lands: the generator rebuilds, complete.
        let Cmd::RunFocused {
            seq, filter, focus, ..
        } = cmds[0].clone()
        else {
            unreachable!()
        };
        let flows = m.traffic.flows_for(&focus).to_vec();
        update(
            &mut m,
            Msg::FocusedDone {
                seq,
                result: Ok(FocusedOutcome {
                    outcome: QueryOutcome {
                        flows,
                        truncated: false,
                        incomplete: false,
                        elapsed: std::time::Duration::ZERO,
                        hit: false,
                        filter,
                        bounds: None,
                        hrefs: Vec::new(),
                    },
                    focus,
                    queries: 1,
                }),
            },
        );
        assert_eq!(view(&m).gap, None, "complete now");
    }

    #[test]
    fn a_capped_focused_result_says_so_and_r_does_not_repeat_it() {
        let mut m = loaded();
        m.traffic.data.as_mut().unwrap().truncated = true;
        open_shop(&mut m);
        let focus = view(&m).focus.clone();
        let data = crate::tui::model::TrafficData {
            flows: m.traffic.flows_for(&focus).to_vec(),
            truncated: true,
            incomplete: false,
            elapsed: std::time::Duration::ZERO,
            cache_hit: false,
            fetched_at: std::time::Instant::now(),
            filter: m.traffic.filter.clone(),
            range: None,
            hrefs: Vec::new(),
            draft: None,
        };
        m.traffic.apply_focused(focus, 1, data);
        assert!(
            !m.traffic.partial_for(&view(&m).focus),
            "partial_for misses it"
        );
        super::rebuild_gen(&mut m);
        assert_eq!(
            view(&m).gap.as_deref(),
            Some("still capped at 1000: B raises the cap, or narrow the window")
        );
        assert_eq!(update(&mut m, key('r')), Vec::new());
    }

    #[test]
    fn r_past_the_cost_threshold_asks_and_comes_back() {
        let mut m = loaded();
        m.traffic.data.as_mut().unwrap().truncated = true;
        m.traffic.filter.window = "30d".into();
        open_shop(&mut m);
        assert_eq!(update(&mut m, key('r')), Vec::new());
        assert!(matches!(
            &m.overlay,
            Overlay::Confirm {
                action: PendingAction::GenRequery(..),
                ..
            }
        ));
        assert_eq!(update(&mut m, key('n')), Vec::new());
        assert_eq!(m.overlay, Overlay::Generator);
        update(&mut m, key('r'));
        let cmds = update(&mut m, key('y'));
        assert!(matches!(&cmds[..], [Cmd::RunFocused { .. }]), "{cmds:?}");
        assert_eq!(m.overlay, Overlay::Generator);
    }

    #[test]
    fn enter_with_writes_off_is_refused() {
        let mut m = loaded();
        open_shop(&mut m);
        assert_eq!(update(&mut m, kc(KeyCode::Enter)), Vec::new());
        assert_eq!(m.status.text, crate::tui::model::WRITES_OFF);
    }

    #[test]
    fn enter_plans_once_and_a_y_typed_meanwhile_confirms_nothing() {
        let mut m = loaded();
        m.writes = WritesState::on();
        open_shop(&mut m);
        let cmds = update(&mut m, kc(KeyCode::Enter));
        let [Cmd::PlanBatch { input, .. }] = &cmds[..] else {
            panic!("{cmds:?}")
        };
        assert_eq!(input.ruleset_name, "RF-{app}-{env}");
        assert_eq!(input.proposal.rules.len(), 2);
        assert!(view(&m).planning && m.writes.in_flight);
        assert_eq!(update(&mut m, key('y')), Vec::new());
        assert_eq!(update(&mut m, kc(KeyCode::Enter)), Vec::new(), "in flight");
        assert_eq!(m.status.text, "a write is already in progress");
        update(
            &mut m,
            Msg::BatchPlanned(Ok(crate::tui::test_support::batch_plan())),
        );
        assert!(matches!(m.overlay, Overlay::ConfirmBatch { .. }));
        update(&mut m, key('n'));
        assert_eq!(m.overlay, Overlay::Generator);
    }

    #[test]
    fn a_failed_plan_keeps_the_generator_and_shows_why() {
        let mut m = loaded();
        m.writes = WritesState::on();
        open_shop(&mut m);
        update(&mut m, kc(KeyCode::Enter));
        update(
            &mut m,
            Msg::BatchPlanned(Err("too many draft rulesets".into())),
        );
        assert_eq!(m.overlay, Overlay::Generator);
        assert_eq!(
            view(&m).footer(),
            "plan failed, nothing sent: too many draft rulesets"
        );
        assert!(!view(&m).planning);
    }

    #[test]
    fn a_plan_landing_after_esc_is_discarded() {
        let mut m = loaded();
        m.writes = WritesState::on();
        open_shop(&mut m);
        update(&mut m, kc(KeyCode::Enter));
        update(&mut m, kc(KeyCode::Esc));
        update(
            &mut m,
            Msg::BatchPlanned(Ok(crate::tui::test_support::batch_plan())),
        );
        assert_eq!(m.overlay, Overlay::None);
        assert_eq!(m.status.text, "requests discarded, nothing sent");
    }

    #[test]
    fn enter_during_a_profile_switch_is_refused() {
        let mut m = loaded();
        m.writes = WritesState::on();
        open_shop(&mut m);
        m.switching = true;
        assert_eq!(update(&mut m, kc(KeyCode::Enter)), Vec::new());
        assert_eq!(
            m.status.text,
            "a profile switch is in progress: nothing planned"
        );
        assert!(!view(&m).planning && !m.writes.in_flight);
    }

    #[test]
    fn other_group_keys_close_it_on_the_next_rebuild() {
        let mut m = loaded();
        open_shop(&mut m);
        m.traffic.keys = vec!["app".into(), "env".into(), "role".into()];
        super::rebuild_gen(&mut m);
        assert_eq!(m.overlay, Overlay::None);
        assert!(m.gen_view.is_none());
        assert_eq!(m.status.text, "g changed: G reopens the generator");
    }

    #[test]
    fn nothing_checked_is_refused() {
        let mut m = loaded();
        m.writes = WritesState::on();
        open_shop(&mut m);
        update(&mut m, key(' ')); // intra off
        update(&mut m, key('j'));
        update(&mut m, key(' ')); // hr off
        assert_eq!(update(&mut m, kc(KeyCode::Enter)), Vec::new());
        assert_eq!(m.status.text, "no row is checked: Space checks one");
    }

    #[test]
    fn gen_alone_opens_the_picker_for_the_generator() {
        let mut m = loaded();
        update(&mut m, Msg::Command(Command::Gen(None)));
        let Overlay::AppPicker(p) = &m.overlay else {
            panic!("{:?}", m.overlay)
        };
        assert_eq!(
            p.purpose,
            crate::tui::app_picker::AppPickerPurpose::Generator
        );
        for c in "shop/prod".chars() {
            update(&mut m, key(c));
        }
        update(&mut m, kc(KeyCode::Enter));
        assert_eq!(m.overlay, Overlay::Generator);
    }
}
