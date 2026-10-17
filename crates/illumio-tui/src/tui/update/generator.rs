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
use pce::generate::{Consolidate, GenInput, Side, keys_refusal, propose, rows};
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
    model.traffic.sync_marks(); // the header's squelched count
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
    v.min_peers = model.limits.consolidate_min_peers;
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
    if !model.is_connected() {
        if let Some(v) = model.gen_view.as_mut() {
            v.drafts = Drafts::Failed("not connected".into());
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
/// (`None`: complete), whether `r` can help, and the window of the result
/// it reads (the filter's while none is loaded).
fn gap_text(model: &Model, v: &GenView) -> (Option<String>, bool, String) {
    let t = &model.traffic;
    let n = v.focus.direction.query_count();
    let app = v.focus.app_text();
    let window = t
        .used_for(&v.focus.clone().with_direction(FocusDirection::In))
        .map_or_else(|_| t.filter.window.clone(), |d| d.filter.window.clone());
    if t.is_running() {
        let ours = t.running_focus().is_some_and(|f| f.same_app(&v.focus));
        let g = if ours {
            format!("re-querying {app}…")
        } else {
            "a query is running: the generator follows when it lands".to_string()
        };
        return (Some(g), false, window);
    }
    let mut sides = vec![FocusDirection::In];
    if v.outbound {
        sides.push(FocusDirection::Out);
    }
    for d in sides {
        let side = v.focus.clone().with_direction(d);
        let gap = match t.complete_for(&side) {
            Ok(()) => {
                // Whole, but of a narrowed filter: flows outside it are missing.
                let narrowed = t.used_for(&side).ok().and_then(|d| d.filter.narrowing());
                match narrowed {
                    Some(what) => (
                        Some(format!(
                            "filtered ({what}): the generator needs every flow; :filter clears it"
                        )),
                        false,
                    ),
                    None => continue,
                }
            }
            Err(Gap::Capped { cap, focused: true }) => (
                Some(format!(
                    "still capped at {cap}: B raises the cap, or narrow the window"
                )),
                false,
            ),
            Err(Gap::NoData) => (
                Some(format!(
                    "no traffic loaded: r queries {app} ({})",
                    queries_text(n)
                )),
                true,
            ),
            Err(_) => (
                Some(format!(
                    "incomplete: r re-queries {app} ({})",
                    queries_text(n)
                )),
                true,
            ),
        };
        return (gap.0, gap.1, window);
    }
    (None, false, window)
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
    // The lists the target ruleset's rules name come first (an IP's row
    // may move when the draft rulesets arrive or `R` picks another).
    let drafts = v
        .fresh_drafts
        .as_deref()
        .or(model.draft_rulesets())
        .unwrap_or_default();
    let flows = model.traffic.flows_for(&v.focus);
    let target = pce::generate::app_ruleset(
        flows,
        &v.focus,
        &idx,
        drafts,
        &model.limits.ruleset_name,
        v.picked.as_deref(),
    );
    let fresh = rows(flows, &v.focus, &model.limits.tier_key, &idx, target);
    // Marks are not policy: the rows above saw every flow; only the
    // header counts the app's squelched ones.
    let squelched = model
        .traffic
        .active_marks()
        .map(|a| {
            flows
                .iter()
                .filter(|f| v.focus.keeps(f))
                .filter(|f| {
                    a.classify(f)
                        .is_some_and(|h| h.action == pce::marks::Action::Squelch)
                })
                .count() as u64
        })
        .unwrap_or(0);
    let (gap, requery, window) = gap_text(model, v);
    let any_missing = pce::generate::consolidate::unavailable(Consolidate::Any, &idx);
    if let Some(v) = model.gen_view.as_mut() {
        v.any_missing = any_missing.map(str::to_string);
        v.squelched = squelched;
        v.set_rows(fresh);
        v.gap = gap;
        v.requery = requery;
        v.window = window;
    }
    regen(model);
}

/// The proposal again from the rows (every key press). Pure.
pub(super) fn regen(model: &mut Model) {
    let Some(idx) = model.index().cloned() else {
        return;
    };
    let p = {
        let Some(v) = model.gen_view.as_ref() else {
            return;
        };
        let Some(drafts) = v.fresh_drafts.as_deref().or(model.draft_rulesets()) else {
            return;
        };
        let consolidate = v.consolidations();
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
            consolidate: &consolidate,
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
            // A load newer than the plan's fresh read; the rows again, for
            // the IP lists its rules name.
            v.fresh_drafts = None;
            rebuild_gen(model);
        }
        Err(e) if v.drafts == Drafts::Loading => v.drafts = Drafts::Failed(e.to_string()),
        Err(_) => {}
    }
}

/// The rule list's last scroll position at the terminal's size.
fn list_last_scroll(model: &Model) -> u16 {
    let area = ratatui::layout::Rect::new(0, 0, model.size.0, model.size.1);
    crate::tui::ui::widgets::generator::rule_list_last_scroll(model, area)
}

/// Whether the plan that just landed is for something else than what the
/// generator shows now: its proposal changed (a traffic answer rebuilt it)
/// or its data became incomplete.
pub(super) fn changed_since_plan(model: &Model) -> bool {
    model
        .gen_view
        .as_ref()
        .is_some_and(|v| v.gap.is_some() || v.planned != v.proposal)
}

/// A plan that opens its confirm: say what the fresh read changed against
/// the preview, and show the proposal the steps write.
pub(super) fn planned_proposal(model: &mut Model, batch: &mut pce::write::Batch) {
    let Some(v) = model.gen_view.as_mut() else {
        return;
    };
    let Some(new) = batch.refreshed.as_ref() else {
        return;
    };
    batch.note = v
        .proposal
        .as_ref()
        .and_then(|old| crate::tui::gen_view::refresh_note(old, new));
    v.proposal = Some(new.clone());
    // `n` comes back to what the fresh read showed, not the preview.
    v.fresh_drafts = batch.drafts.clone();
}

/// `Esc` / `q`: back to the app overlay it came from (its view is kept),
/// else to the screen.
fn close_gen(model: &mut Model) {
    if let Some(v) = model.gen_view.as_mut() {
        v.list = None;
    }
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
        let last = list_last_scroll(model);
        let Some(v) = model.gen_view.as_mut() else {
            return Vec::new();
        };
        match k.code {
            KeyCode::Esc | KeyCode::Char('p') | KeyCode::Char('q') => v.list = None,
            KeyCode::Down | KeyCode::Char('j') => v.list = Some((scroll + 1).min(last)),
            KeyCode::Up | KeyCode::Char('k') => v.list = Some(scroll.saturating_sub(1)),
            _ => {}
        }
        return Vec::new();
    }
    // The plan is on its way for the proposal on screen: nothing may
    // change it until the confirm opens (or `Esc` drops the plan).
    if v.planning
        && !matches!(
            k.code,
            KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('p')
        )
    {
        model.status = StatusLine::info("planning: wait for the confirm, or Esc");
        return Vec::new();
    }
    match k.code {
        KeyCode::Esc | KeyCode::Char('q') => close_gen(model),
        KeyCode::Char('?') => super::open_help(model),
        KeyCode::Down | KeyCode::Char('j') => v.move_cursor(1),
        KeyCode::Up | KeyCode::Char('k') => v.move_cursor(-1),
        KeyCode::Tab | KeyCode::BackTab => v.switch_section(),
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
            // Turned on: the cursor goes to the section (its first row,
            // or its title while it has none), so it is on screen.
            if let Some(v) = model.gen_view.as_mut()
                && v.outbound
            {
                v.section = Side::Outbound;
                v.cursor[1] = 0;
            }
        }
        KeyCode::Char('R') => {
            match v.pick_next() {
                Ok(name) => model.status = StatusLine::info(format!("rules go into {name}")),
                Err(e) => model.status = StatusLine::info(e),
            }
            // The rows too: the lists its rules name come first.
            rebuild_gen(model);
        }
        KeyCode::Char('c') => {
            let any = model
                .index()
                .and_then(|i| pce::generate::consolidate::unavailable(Consolidate::Any, i));
            let Some(v) = model.gen_view.as_mut() else {
                return Vec::new();
            };
            match v.cycle_consolidation(any) {
                Ok(what) => model.status = StatusLine::info(what),
                Err(e) => model.status = StatusLine::warn(e),
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
    // One query at a time: never stack Explorer queries, never cancel one.
    if model.traffic.is_running() {
        model.status = StatusLine::info("query already running");
        return Vec::new();
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
    run_requery(model, filter, focus)
}

/// The focused query for the generator's app: the Traffic screen takes
/// its focus (so the answer is applied), the header says it runs.
fn run_requery(
    model: &mut Model,
    filter: crate::tui::filter::TrafficFilter,
    focus: Focus,
) -> Vec<Cmd> {
    model.traffic.set_focus(focus.clone());
    let cmds = start_focused(model, filter, focus, CacheMode::Use);
    rebuild_gen(model);
    cmds
}

/// `y` on `r`'s prompt: the focused query, back to the generator.
pub(super) fn gen_requery(
    model: &mut Model,
    filter: crate::tui::filter::TrafficFilter,
    focus: Focus,
) -> Vec<Cmd> {
    model.overlay = Overlay::Generator;
    if model.traffic.is_running() {
        model.status = StatusLine::info("query already running");
        return Vec::new();
    }
    run_requery(model, filter, focus)
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
    if model.traffic.is_running() {
        model.status = StatusLine::warn("a query is running: Enter plans once it lands");
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
        && !v.has_consolidated_rule()
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
        v.planned = Some(input.proposal.clone());
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
    fn question_mark_opens_the_help_over_the_app_view_and_the_generator_and_esc_returns() {
        let mut m = loaded();
        update(&mut m, Msg::Command(Command::App(Some("shop/prod".into()))));
        assert_eq!(update(&mut m, key('?')), Vec::new());
        assert_eq!(m.overlay, Overlay::Help);
        use crate::tui::help::HelpContext;
        assert_eq!(HelpContext::of(&m), HelpContext::AppOverlay);
        update(&mut m, Msg::Key(KeyEvent::from(KeyCode::Esc)));
        assert_eq!(m.overlay, Overlay::App, "back to the app view");
        update(&mut m, key('G'));
        assert_eq!(update(&mut m, key('?')), Vec::new());
        assert_eq!(m.overlay, Overlay::Help);
        assert_eq!(HelpContext::of(&m), HelpContext::Generator);
        update(&mut m, key('?'));
        assert_eq!(m.overlay, Overlay::Generator, "back to the generator");
        update(&mut m, key('q'));
        update(&mut m, key('q'));
        update(&mut m, key('?'));
        update(&mut m, Msg::Key(KeyEvent::from(KeyCode::Esc)));
        assert_eq!(
            m.overlay,
            Overlay::None,
            "the plain help returns to the screen"
        );
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
        assert_eq!(view(&m).section, Side::Outbound, "o moves to its rows");
        update(&mut m, kc(KeyCode::Tab));
        assert_eq!(view(&m).section, Side::Inbound);
        // j runs on from the last inbound row into the outbound section.
        for _ in 0..10 {
            update(&mut m, key('j'));
        }
        assert_eq!(view(&m).section, Side::Outbound);
        update(&mut m, key('k'));
        update(&mut m, key('k'));
        assert_eq!(view(&m).section, Side::Inbound, "and k back");
        update(&mut m, kc(KeyCode::Tab));
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
        assert_eq!(m.status.text, "planning: wait for the confirm, or Esc");
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

    /// Writes on, the generator open for shop/prod, `Enter` sent the plan.
    fn planning() -> Model {
        let mut m = loaded();
        m.writes = WritesState::on();
        open_shop(&mut m);
        let cmds = update(&mut m, kc(KeyCode::Enter));
        assert!(matches!(&cmds[..], [Cmd::PlanBatch { .. }]), "{cmds:?}");
        m
    }

    #[test]
    fn edits_are_frozen_while_the_plan_is_on_its_way() {
        let mut m = planning();
        let before = view(&m).rows.clone();
        for msg in [key(' '), kc(KeyCode::Right), key('o'), key('R'), key('>')] {
            assert_eq!(update(&mut m, msg), Vec::new());
            assert_eq!(m.status.text, "planning: wait for the confirm, or Esc");
        }
        assert_eq!(view(&m).rows, before);
        assert_eq!(view(&m).global, Stop::AppPorts);
        assert!(!view(&m).outbound);
        update(
            &mut m,
            Msg::BatchPlanned(Ok(crate::tui::test_support::batch_plan())),
        );
        assert!(matches!(m.overlay, Overlay::ConfirmBatch { .. }));
    }

    #[test]
    fn a_plan_for_a_proposal_that_changed_meanwhile_is_discarded() {
        let mut m = planning();
        // A traffic answer lands while planning: truncated now.
        m.traffic.data.as_mut().unwrap().truncated = true;
        super::rebuild_gen(&mut m);
        update(
            &mut m,
            Msg::BatchPlanned(Ok(crate::tui::test_support::batch_plan())),
        );
        assert_eq!(m.overlay, Overlay::Generator);
        assert_eq!(
            m.status.text,
            "the proposal changed while planning: nothing sent, Enter plans again"
        );
        assert!(!view(&m).planning && !m.writes.in_flight);
    }

    #[test]
    fn the_fresh_read_s_changes_are_named_and_shown() {
        let mut m = planning();
        let mut b = crate::tui::test_support::batch_plan();
        let mut p = view(&m).proposal.clone().unwrap();
        p.rules[1].action = pce::generate::RuleAction::Exists {
            rule_href: "/orgs/5/sec_policy/draft/rule_sets/1/sec_rules/9".into(),
        };
        b.refreshed = Some(p.clone());
        // The fresh read found no ruleset at all (whatever: it is what counts now).
        b.drafts = Some(Vec::new());
        update(&mut m, Msg::BatchPlanned(Ok(b)));
        let Overlay::ConfirmBatch { batch, .. } = &m.overlay else {
            panic!("{:?}", m.overlay)
        };
        assert_eq!(
            batch.note.as_deref(),
            Some("1 rule is in the draft since the preview: not written")
        );
        assert_eq!(
            m.status.text,
            "the draft changed since the preview: review the requests"
        );
        assert_eq!(view(&m).proposal.as_ref(), Some(&p), "the refreshed one");
        // At 80x24 the note heads the confirm.
        let out = crate::tui::test_support::render_to_string(&m, 80, 24);
        assert!(
            out.contains("⚠ 1 rule is in the draft since the preview: not"),
            "{out}"
        );
        // `n`: back to the generator, derived from the fresh read, not the
        // cached drafts of the preview.
        update(&mut m, key('n'));
        assert_eq!(m.overlay, Overlay::Generator);
        assert!(
            matches!(
                view(&m).proposal.as_ref().unwrap().targets[0],
                pce::generate::Target::Create(_)
            ),
            "no ruleset in the fresh read: a new one"
        );
        // A newer draft load wins again.
        let rs: Vec<pce::models::RuleSet> =
            serde_json::from_str(include_str!("../../../../../fixtures/rule_sets.json")).unwrap();
        update(
            &mut m,
            Msg::RuleSetsLoaded {
                scope: PolicyScope::Draft,
                result: Ok((rs, 0, false)),
            },
        );
        assert!(view(&m).fresh_drafts.is_none());
        assert!(matches!(
            view(&m).proposal.as_ref().unwrap().targets[0],
            pce::generate::Target::Existing { .. }
        ));
    }

    fn written_all() -> Msg {
        let w = |href: &str| {
            pce::write::StepResult::Written(pce::write::Written {
                href: href.into(),
                pending: true,
                status: 201,
                object: serde_json::json!({"href": href}),
                dropped_fields: Vec::new(),
                warnings: Vec::new(),
                audit_failed: None,
            })
        };
        Msg::BatchDone {
            kinds: vec![
                pce::write::ObjectKind::IpList,
                pce::write::ObjectKind::RuleSet,
                pce::write::ObjectKind::Rule,
            ],
            summaries: vec!["a".into(), "b".into(), "c".into()],
            target: "demo (pce.example, org 5)".into(),
            results: vec![
                w("/orgs/5/sec_policy/draft/ip_lists/77"),
                w("/orgs/5/sec_policy/draft/rule_sets/200"),
                w("/orgs/5/sec_policy/draft/rule_sets/200/sec_rules/1"),
            ],
        }
    }

    /// `planning()`, the plan landed, `y` sent it.
    fn sending() -> Model {
        let mut m = planning();
        update(
            &mut m,
            Msg::BatchPlanned(Ok(crate::tui::test_support::batch_plan())),
        );
        assert!(matches!(
            &update(&mut m, key('y'))[..],
            [Cmd::ExecuteBatch(_)]
        ));
        m
    }

    #[test]
    fn an_older_report_while_a_plan_is_on_its_way_closes_onto_the_screen() {
        let mut m = planning();
        assert!(!m.batch_from_gen, "set only when the confirm opens");
        update(&mut m, kc(KeyCode::Esc));
        m.batch_report = Some(vec!["sent to demo".into()]);
        update(&mut m, Msg::Command(Command::Batch));
        assert!(matches!(m.overlay, Overlay::BatchReport { .. }));
        update(&mut m, kc(KeyCode::Esc));
        assert_eq!(m.overlay, Overlay::None);
    }

    #[test]
    fn after_o_and_a_re_query_the_cursor_stays_on_its_row() {
        let mut m = loaded();
        open_shop(&mut m);
        update(&mut m, key('j'));
        update(&mut m, key('j')); // shop/dev
        let at = |m: &Model| view(m).selected().map(|i| view(m).rows[i].peer_text());
        assert_eq!(at(&m).as_deref(), Some("shop/dev"));
        // The rows come back in another order: the cursor follows its row.
        let mut rows = view(&m).rows.clone();
        rows.reverse();
        m.gen_view.as_mut().unwrap().set_rows(rows);
        assert_eq!(at(&m).as_deref(), Some("shop/dev"));
        // `o`: the cursor goes to the outbound section and stays there
        // through a rebuild.
        update(&mut m, key('o'));
        assert_eq!(view(&m).section, Side::Outbound);
        let before = at(&m);
        super::rebuild_gen(&mut m);
        assert_eq!(view(&m).section, Side::Outbound);
        assert_eq!(at(&m), before);
    }

    #[test]
    fn back_from_a_batch_the_generator_rebuilds_from_the_traffic_now_loaded() {
        let mut m = sending();
        // Traffic lands under the screen while the batch is sent: truncated.
        m.traffic.data.as_mut().unwrap().truncated = true;
        update(&mut m, written_all());
        assert_eq!(m.overlay, Overlay::Generator);
        assert!(view(&m).gap.is_some(), "the gap is judged again");
        update(&mut m, kc(KeyCode::Enter));
        assert!(
            m.status.text.ends_with("no rules from incomplete data"),
            "{}",
            m.status.text
        );
    }

    #[test]
    fn the_batch_outcome_stays_on_the_status_line_when_the_drafts_load_again() {
        let mut m = sending();
        m.policy.scope = PolicyScope::Active;
        m.policy.form_drafts = None;
        let cmds = update(&mut m, written_all());
        assert!(
            cmds.iter().any(|c| matches!(
                c,
                Cmd::LoadRuleSets {
                    scope: PolicyScope::Draft,
                    ..
                }
            )),
            "{cmds:?}"
        );
        assert!(
            m.status.text.starts_with("wrote 3 of 3 (pending)"),
            "{}",
            m.status.text
        );
        assert_eq!(view(&m).requests, 1, "counted in its header");
    }

    #[test]
    fn batch_later_does_not_reopen_a_closed_generator() {
        let mut m = sending();
        update(&mut m, written_all());
        assert_eq!(m.overlay, Overlay::Generator);
        update(&mut m, kc(KeyCode::Esc));
        assert_eq!(m.overlay, Overlay::None);
        update(&mut m, Msg::Command(Command::Batch));
        assert!(matches!(m.overlay, Overlay::BatchReport { .. }));
        assert_eq!(update(&mut m, kc(KeyCode::Esc)), Vec::new());
        assert_eq!(m.overlay, Overlay::None, "the screen, not the generator");
    }

    #[test]
    fn a_narrowing_filter_is_a_gap_r_cannot_close() {
        let mut m = loaded();
        m.writes = WritesState::on();
        m.traffic.data.as_mut().unwrap().filter.service = "443".into();
        open_shop(&mut m);
        assert_eq!(
            view(&m).gap.as_deref(),
            Some("filtered (svc 443): the generator needs every flow; :filter clears it")
        );
        assert!(view(&m).header_title().ends_with("· incomplete"));
        assert_eq!(update(&mut m, kc(KeyCode::Enter)), Vec::new());
        assert!(m.status.text.ends_with("no rules from incomplete data"));
        assert_eq!(update(&mut m, key('r')), Vec::new());
    }

    #[test]
    fn the_window_is_the_loaded_result_s_and_a_running_query_blocks_enter() {
        let mut m = loaded();
        m.writes = WritesState::on();
        m.traffic.filter.window = "7d".into(); // asked, not yet loaded
        open_shop(&mut m);
        assert!(view(&m).header_title().contains("last 24h"), "the data's");
        let filter = m.traffic.filter.clone();
        let cmds = crate::tui::update::start_query_for_tests(&mut m, filter);
        assert!(!cmds.is_empty());
        super::rebuild_gen(&mut m);
        assert_eq!(
            view(&m).gap.as_deref(),
            Some("a query is running: the generator follows when it lands")
        );
        assert_eq!(update(&mut m, kc(KeyCode::Enter)), Vec::new());
        assert_eq!(
            m.status.text,
            "a query is running: Enter plans once it lands"
        );
        assert_eq!(update(&mut m, key('r')), Vec::new());
        assert_eq!(m.status.text, "query already running");
    }

    #[test]
    fn r_twice_sends_one_query() {
        let mut m = loaded();
        m.traffic.data.as_mut().unwrap().truncated = true;
        open_shop(&mut m);
        assert_eq!(update(&mut m, key('r')).len(), 1);
        assert_eq!(view(&m).gap.as_deref(), Some("re-querying shop/prod…"));
        assert_eq!(update(&mut m, key('r')), Vec::new());
        assert_eq!(m.status.text, "query already running");
    }

    #[test]
    fn n_on_r_s_prompt_leaves_the_traffic_focus_alone() {
        let mut m = loaded();
        m.traffic.data.as_mut().unwrap().truncated = true;
        m.traffic.filter.window = "30d".into();
        open_shop(&mut m);
        update(&mut m, key('r'));
        update(&mut m, key('n'));
        assert!(m.traffic.focus.is_none());
    }

    #[test]
    fn outbound_needs_the_out_side_and_r_queries_both() {
        // Only a focused `in` result is loaded (no unfocused one).
        let mut m = with_policy(crate::tui::test_support::model_fixture());
        let rich = model_with_window("24h");
        let shop = pce::traffic::Focus::new(
            pce::traffic::parse_app("shop/prod"),
            vec!["app".into(), "env".into()],
        )
        .with_direction(pce::traffic::FocusDirection::In);
        let data = crate::tui::model::TrafficData {
            flows: rich.traffic.loaded_flows().to_vec(),
            truncated: false,
            incomplete: false,
            elapsed: std::time::Duration::ZERO,
            cache_hit: false,
            fetched_at: std::time::Instant::now(),
            filter: rich.traffic.filter.clone(),
            range: None,
            hrefs: Vec::new(),
            draft: None,
        };
        m.traffic.apply_focused(shop, 1, data);
        open_shop(&mut m);
        assert_eq!(view(&m).gap, None, "inbound is whole");
        update(&mut m, key('o'));
        assert_eq!(
            view(&m).gap.as_deref(),
            Some("incomplete: r re-queries shop/prod (2 queries)")
        );
        let cmds = update(&mut m, key('r'));
        assert!(
            matches!(&cmds[..], [Cmd::RunFocused { focus, .. }]
                if focus.direction == pce::traffic::FocusDirection::Both),
            "{cmds:?}"
        );
    }

    #[test]
    fn the_rule_list_scrolls_to_its_end_and_no_further() {
        let mut m = loaded();
        open_shop(&mut m);
        update(&mut m, key('p'));
        for _ in 0..200 {
            update(&mut m, key('j'));
        }
        let area = ratatui::layout::Rect::new(0, 0, m.size.0, m.size.1);
        let last = crate::tui::ui::widgets::generator::rule_list_last_scroll(&m, area);
        assert_eq!(view(&m).list, Some(last));
        update(&mut m, key('k'));
        assert_eq!(view(&m).list, Some(last.saturating_sub(1)));
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

    #[test]
    fn c_consolidates_a_fan_in_port_without_requests_and_enter_plans_it_alone() {
        let mut m = loaded();
        m.limits.consolidate_min_peers = 3;
        m.writes = WritesState::on();
        assert_eq!(
            update(&mut m, key('c')),
            Vec::new(),
            "no generator: ignored"
        );
        open_shop(&mut m);
        assert_eq!(view(&m).candidates.len(), 1, "5432/tcp from 3 peers");
        // Every row unchecked; the fan-in line is the cursor's first stop.
        for _ in 0..2 {
            update(&mut m, key('j'));
            update(&mut m, key(' '));
        }
        for _ in 0..5 {
            update(&mut m, key('k'));
        }
        assert_eq!(view(&m).selected_candidate(), Some(0));
        assert_eq!(update(&mut m, key('c')), Vec::new());
        assert!(
            m.status
                .text
                .starts_with("destination only: 5432/tcp from any IP")
        );
        let p = view(&m).proposal.as_ref().unwrap();
        assert_eq!(p.rules.len(), 1);
        assert!(p.rules[0].consolidated.is_some());
        // The state is the port's: it survives a rebuild (`o` twice).
        update(&mut m, key('o'));
        update(&mut m, key('o'));
        assert_eq!(view(&m).consolidations().len(), 1);
        let cmds = update(&mut m, kc(KeyCode::Enter));
        let [Cmd::PlanBatch { input, .. }] = &cmds[..] else {
            panic!("{cmds:?} {}", m.status.text)
        };
        assert!(input.proposal.rules[0].consolidated.is_some());
    }
}
