//! `:readiness` and the enforcement overlay (#55, #56): opening them, their
//! keys, the step actions (each through the existing path: `D`, the
//! generator, the `w` and `m` forms, provision, the bulk confirm), the
//! answers of a move, a rollback and a watch, and the re-evaluation when
//! data lands. Opening costs nothing when the data is loaded; what is
//! missing loads once, through the cache, counted in the header.
use super::*;
use crate::tui::bulk::{BulkConfirm, loaded_counts, origin as batch_origin};
use crate::tui::enforce::{
    ENFORCE_WORD, EnforceSend, EnforceView, Missing, ReadinessView, RollbackPreview, RollbackSend,
    Step, Watch, WatchResult, compute, origin_of,
};
use crate::tui::mark_form::{MarkForm, MarkOrigin, Prefill, RowsOf, group_side};
use pce::enforcement::{
    RecordReadiness, RollbackPlans, app_workloads, blocked_since, plan_rollback, rollback_plans,
};
use pce::models::WorkloadFilter;
use pce::readiness::{Gap, accepted, draft_allows, is_unicast};
use pce::traffic::{TrafficQueryBuilder, group_key, keys_or_default};
use pce::write::{BulkChange, BulkTarget, VisibilityLevel, plan_bulk};
use tokio_util::sync::CancellationToken;

/// One of the two overlays of this module.
pub(super) fn is_ours(o: &Overlay) -> bool {
    matches!(o, Overlay::Readiness | Overlay::Enforce)
}

/// A message whose answer may change readiness.
pub(super) fn is_data(msg: &Msg) -> bool {
    matches!(
        msg,
        Msg::QueryDone { .. }
            | Msg::FocusedDone { .. }
            | Msg::DraftDone { .. }
            | Msg::WorkloadsLoaded(_)
            | Msg::PendingLoaded(_)
            | Msg::RuleSetsLoaded { .. }
            | Msg::IndexLoaded { .. }
            | Msg::WriteDone { .. }
            | Msg::BulkDone(_)
            | Msg::EnforceDone(_)
            | Msg::RollbackDone(_)
    )
}

/// A write's answer: what it dropped (pending changes after a rule write)
/// is asked for again once the overlay is back.
pub(super) fn is_write(msg: &Msg) -> bool {
    matches!(
        msg,
        Msg::WriteDone { .. }
            | Msg::BulkDone(_)
            | Msg::BatchDone { .. }
            | Msg::EnforceDone(_)
            | Msg::RollbackDone(_)
    )
}

/// After every message: a confirm the overlay planned is forgotten once
/// it closes; the overlay a step left comes back once nothing is open and
/// no write is on its way; an open overlay re-evaluates when data landed
/// or it was just (re)opened, and after a write asks for what the write
/// dropped (pending changes after a rule write), counted. Reopening after
/// the help or a cancelled form asks for nothing.
pub(super) fn after_msg(model: &mut Model, ours_before: bool, data: bool, write: bool) -> Vec<Cmd> {
    if !matches!(model.overlay, Overlay::ConfirmBulk(_)) {
        model.enforce_send = None;
    }
    if model.overlay == Overlay::None
        && !model.writes.in_flight
        && !model.batch_sending
        && let Some(back) = model.enforce_back.take()
    {
        let alive = match back {
            Overlay::Readiness => model.readiness_view.is_some(),
            Overlay::Enforce => model.enforce_view.is_some(),
            _ => false,
        };
        if alive {
            model.overlay = back;
        }
    }
    if !is_ours(&model.overlay) {
        return Vec::new();
    }
    let mut cmds = Vec::new();
    if write {
        cmds = ensure_loads(model);
    }
    if data || !ours_before {
        refresh(model);
    }
    cmds
}

/// The open overlay's rows again from what is loaded. No request.
pub(super) fn refresh(model: &mut Model) {
    match model.overlay {
        Overlay::Readiness => {
            let rows = compute(model, &[], false);
            if let Some(v) = model.readiness_view.as_mut() {
                v.set_rows(rows);
            }
        }
        Overlay::Enforce => {
            let Some(app) = model.enforce_view.as_ref().map(|v| v.app.clone()) else {
                return;
            };
            let r = compute(model, std::slice::from_ref(&app), true).pop();
            let partial = crate::tui::enforce::partial_reasons(model, &app, r.as_ref());
            if let Some(v) = model.enforce_view.as_mut() {
                v.set_readiness(r);
                v.partial = partial;
            }
            refresh_plan(model);
        }
        _ => {}
    }
}

/// Step 4's plan text again (data landed, or the target changed): the
/// app's workloads planned locally, no request.
fn refresh_plan(model: &mut Model) {
    let text = match (model.index(), model.enforce_view.as_ref()) {
        (Some(idx), Some(v)) if model.workloads.loaded_at.is_some() => {
            let keys = keys_or_default(&model.limits.scope_keys);
            let none = pce::NameIndex::default();
            let targets: Vec<BulkTarget> = app_workloads(&model.workloads.raw, &v.app, &keys, idx)
                .into_iter()
                .map(|w| BulkTarget::of(w, &none))
                .collect();
            if targets.is_empty() {
                "no workloads".to_string()
            } else {
                plan_bulk(
                    &targets,
                    &BulkChange::enforce(v.mode, v.visibility_level()),
                    WorkloadFilter::default(),
                    None,
                )
                .map_or_else(|e| e.to_string(), |p| p.summary())
            }
        }
        (Some(_), Some(_)) => "the workloads are loading".to_string(),
        _ => "-".to_string(),
    };
    if let Some(v) = model.enforce_view.as_mut() {
        v.plan_text = text;
    }
}

/// What readiness reads and is neither loaded nor on its way: one load
/// each, through the cache, counted in the open overlay's header.
fn ensure_loads(model: &mut Model) -> Vec<Cmd> {
    if !model.is_connected() {
        return Vec::new();
    }
    let mut cmds = Vec::new();
    if model.workloads.loaded_at.is_none() && !model.workloads.loading {
        cmds.extend(load_workloads(model, CacheMode::Use));
    }
    if model.policy.pending.is_none() && !model.policy.pending_loading {
        model.policy.pending_loading = true;
        cmds.push(Cmd::LoadPending(CacheMode::Use));
    }
    if model.draft_rulesets().is_none() {
        let status = model.status.clone();
        cmds.extend(load_form_drafts(model));
        model.status = status;
    }
    count(model, cmds.len());
    cmds
}

/// `n` requests sent for the open overlay, in its header.
fn count(model: &mut Model, n: usize) {
    let n = u32::try_from(n).unwrap_or(u32::MAX);
    match model.overlay {
        Overlay::Readiness => {
            if let Some(v) = model.readiness_view.as_mut() {
                v.requests += n;
            }
        }
        _ => {
            if let Some(v) = model.enforce_view.as_mut() {
                v.requests += n;
            }
        }
    }
}

/// `:readiness`: every app of the result on screen. No request when the
/// workloads, pending changes and draft rulesets are loaded.
pub(super) fn open_readiness(model: &mut Model) -> Vec<Cmd> {
    if model.traffic.data.is_none() {
        model.status =
            StatusLine::info("no traffic loaded: r on Traffic runs the query; nothing sent");
        return Vec::new();
    }
    if model.index().is_none() {
        model.status = StatusLine::info("the name index is not loaded yet");
        return Vec::new();
    }
    let v = model.readiness_view.take().unwrap_or_default();
    model.readiness_view = Some(ReadinessView {
        searching: false,
        ..v
    });
    model.overlay = Overlay::Readiness;
    let mut cmds = ensure_loads(model);
    refresh(model);
    if let Some(v) = &model.readiness_view {
        model.status = StatusLine::info(format!(
            "readiness · {} · {} req",
            v.counts_text(),
            cmds.len()
        ));
    }
    // The trend: the snapshot files, read again at every open (a local
    // listing, no request, not counted): a snapshot saved since shows.
    if let Some(root) = &model.snapshots_root {
        let dir = crate::snapshot::profile_dir(root, &model.profile.name);
        cmds.push(Cmd::LoadTrend { dir });
    }
    cmds
}

/// The snapshots' readiness read: the trend column (with two snapshots at
/// least). A failure is said once; the list works without it.
pub(super) fn trend_loaded(
    model: &mut Model,
    result: Result<crate::snapshot::baseline::Trend, String>,
) {
    let Some(v) = model.readiness_view.as_mut() else {
        return;
    };
    match result {
        Ok(t) => {
            if !t.skipped.is_empty() {
                model.status = StatusLine::warn(format!(
                    "trend: {} snapshot{} skipped: {}",
                    t.skipped.len(),
                    if t.skipped.len() == 1 { "" } else { "s" },
                    t.skipped[0]
                ));
            }
            v.trend = Some(t);
        }
        Err(e) => {
            v.trend = Some(Default::default());
            model.status = StatusLine::warn(format!("trend: {e}"));
        }
    }
}

/// `:enforce <app>`: an app of the loaded result or one the labels know;
/// else the closest match, nothing sent.
pub(super) fn enforce_by_name(model: &mut Model, name: &str) -> Vec<Cmd> {
    let keys = keys_or_default(&model.limits.scope_keys);
    let Some(app) = super::app_overlay::typed_app(model, &keys, name) else {
        return Vec::new();
    };
    let text = app.join("/");
    let loaded = pce::traffic::apps_in(model.traffic.loaded_flows(), &keys);
    let labelled = model
        .index()
        .map(|idx| pce::traffic::app_labels(&app, &keys, idx));
    if !loaded.contains(&text) && !matches!(labelled, Some(Ok(_))) {
        let near = pce::traffic::closest(name, &loaded, 3);
        model.status = StatusLine::warn(match (near.is_empty(), labelled) {
            (false, _) => format!(
                "unknown app {name:?}; closest: {}; nothing sent",
                near.join(", ")
            ),
            (true, Some(Err(e))) => format!("{e}; nothing sent"),
            (true, _) => format!("unknown app {name:?}; nothing sent"),
        });
        return Vec::new();
    }
    open_enforce(model, app, Overlay::None)
}

/// `:enforce` alone: the picker over the loaded apps.
pub(super) fn enforce_picker(model: &mut Model) {
    open_app_picker(model, AppPickerPurpose::Enforce, "");
}

/// The enforcement overlay for `app`; `back` is where `Esc` returns (the
/// readiness list, the app overlay, or nothing). The same app keeps its
/// step, cursors and watch; its newest record is read again (a local
/// file). No request when the data is loaded.
pub(super) fn open_enforce(model: &mut Model, app: GroupKey, back: Overlay) -> Vec<Cmd> {
    let keys = keys_or_default(&model.limits.scope_keys);
    let focus = Focus::new(app.clone(), keys);
    if !focus.is_label_group() {
        model.status = StatusLine::warn(format!(
            "{} is not an app of labels: nothing to enforce",
            focus.app_text()
        ));
        return Vec::new();
    }
    if model.index().is_none() {
        model.status = StatusLine::info("the name index is not loaded yet");
        return Vec::new();
    }
    let mut v = match model.enforce_view.take() {
        Some(v) if v.app == app => v,
        old => {
            // Another app: its watch query on the way is cancelled.
            if let Some(mut old) = old {
                old.drop_watch();
            }
            model.enforce_seq += 1;
            EnforceView::new(app, model.enforce_seq)
        }
    };
    v.typing = None;
    v.drop_rollback();
    v.load_record(model.records_dir.as_deref(), &origin_of(&model.profile));
    model.enforce_back = None;
    model.enforce_view = Some(v);
    model.overlay = Overlay::Enforce;
    model.enforce_return = back;
    let cmds = ensure_loads(model);
    refresh(model);
    if let Some(v) = &model.enforce_view {
        let verdict = v
            .readiness
            .as_ref()
            .map_or("-", |r| r.verdict.text())
            .to_string();
        model.status =
            StatusLine::info(format!("{} · {verdict} · {} req", v.app_text(), cmds.len()));
    }
    cmds
}

/// `e` in the app overlay: its app's enforcement overlay; `Esc` returns.
pub(super) fn app_enforce_key(model: &mut Model) -> Vec<Cmd> {
    let Some(v) = model.app_view.as_ref() else {
        return Vec::new();
    };
    let keys = keys_or_default(&model.limits.scope_keys);
    if v.focus.keys != keys {
        model.status = StatusLine::warn(format!(
            "the overlay groups by {}: enforcement goes by the scope keys {}",
            v.focus.keys.join(","),
            keys.join(",")
        ));
        return Vec::new();
    }
    let app = v.focus.app.clone();
    open_enforce(model, app, Overlay::App)
}

/// `D` on either overlay: the draft evaluation of the result on screen,
/// through `D`'s own path and confirm; never clears loaded draft
/// decisions.
fn draft_from(model: &mut Model) -> Vec<Cmd> {
    if model.traffic.data.is_none() {
        model.status =
            StatusLine::info("no traffic loaded: r on Traffic runs the query; nothing sent");
        return Vec::new();
    }
    if model.traffic.draft_loaded() {
        model.status = StatusLine::info("draft decisions are loaded (D on Traffic clears them)");
        return Vec::new();
    }
    let here = model.overlay.clone();
    let cmds = draft_key(model);
    if model.overlay != here {
        model.enforce_back = Some(here);
    }
    count(model, cmds.len());
    cmds
}

/// The readiness overlay's keys: `j`/`k` rows, `s` sort, `/` search,
/// `D` draft, `Enter` the app's enforcement overlay, `Esc` back.
pub(super) fn readiness_key(model: &mut Model, k: KeyEvent) -> Vec<Cmd> {
    let Some(v) = model.readiness_view.as_mut() else {
        model.overlay = Overlay::None;
        return Vec::new();
    };
    if v.searching {
        match k.code {
            KeyCode::Esc => {
                v.search.clear();
                v.searching = false;
            }
            KeyCode::Enter => v.searching = false,
            KeyCode::Backspace => {
                v.search.pop();
            }
            KeyCode::Char('u') if k.modifiers.contains(KeyModifiers::CONTROL) => v.search.clear(),
            KeyCode::Char(c) => v.search.push(c),
            _ => {}
        }
        v.cursor = 0;
        return Vec::new();
    }
    match k.code {
        KeyCode::Esc | KeyCode::Char('q') => {
            model.overlay = Overlay::None;
            model.enforce_back = None;
        }
        KeyCode::Down | KeyCode::Char('j') => v.move_cursor(1),
        KeyCode::Up | KeyCode::Char('k') => v.move_cursor(-1),
        KeyCode::PageDown => v.move_cursor(10),
        KeyCode::PageUp => v.move_cursor(-10),
        KeyCode::Char('s') => {
            v.sort = v.sort.next();
            model.status = StatusLine::info(format!("sorted by {}", v.sort.text()));
        }
        KeyCode::Char('/') => {
            v.searching = true;
            v.search.clear();
            v.cursor = 0;
        }
        KeyCode::Char('D') => return draft_from(model),
        KeyCode::Char('?') => open_help(model),
        KeyCode::Enter => {
            let Some(app) = v.selected().map(|r| r.key.clone()) else {
                return Vec::new();
            };
            return open_enforce(model, app, Overlay::Readiness);
        }
        _ => {}
    }
    Vec::new()
}

/// The enforcement overlay's keys. A typing prompt or a rollback preview
/// takes every key first.
pub(super) fn enforce_key(model: &mut Model, k: KeyEvent) -> Vec<Cmd> {
    let Some(v) = model.enforce_view.as_mut() else {
        model.overlay = Overlay::None;
        return Vec::new();
    };
    if v.rollback.is_some() {
        return rollback_preview_key(model, k);
    }
    if v.typing.is_some() {
        return typing_key(model, k);
    }
    let step = v.step;
    match k.code {
        KeyCode::Esc | KeyCode::Char('q') => {
            close_enforce(model);
            return Vec::new();
        }
        KeyCode::Tab | KeyCode::Right | KeyCode::Char('l') => v.step = v.step.next(),
        KeyCode::BackTab | KeyCode::Left | KeyCode::Char('h') => v.step = v.step.prev(),
        KeyCode::Char(c @ '1'..='5') => v.step = Step::ALL[c as usize - '1' as usize],
        KeyCode::Down | KeyCode::Char('j') => {
            v.move_cursor(1);
            refresh_plan(model);
        }
        KeyCode::Up | KeyCode::Char('k') => {
            v.move_cursor(-1);
            refresh_plan(model);
        }
        KeyCode::Char('?') => open_help(model),
        KeyCode::Char('D') => return draft_from(model),
        KeyCode::Char('G') if step == Step::Rules => return gen_key(model),
        KeyCode::Char('w') if step == Step::Rules => return gap_rule_key(model),
        KeyCode::Char('m') if step == Step::Rules => return gap_mark_key(model),
        KeyCode::Char('P') if step == Step::Provision => return provision_app_key(model),
        KeyCode::Char('v') if step == Step::Enforce => {
            v.visibility = (v.visibility + 1) % (VisibilityLevel::ALL.len() + 1);
            refresh_plan(model);
        }
        KeyCode::Enter if step == Step::Enforce => return review_key(model),
        KeyCode::Char('r') if step == Step::Watch => return watch_key(model),
        KeyCode::Char('R') if step == Step::Watch => return rollback_key(model),
        _ => {}
    }
    Vec::new()
}

/// `Esc`: back where the overlay was opened from (the readiness list,
/// the app overlay), else to the screen.
fn close_enforce(model: &mut Model) {
    model.enforce_back = None;
    if let Some(v) = model.enforce_view.as_mut() {
        v.drop_rollback();
        v.typing = None;
    }
    let back = std::mem::replace(&mut model.enforce_return, Overlay::None);
    model.overlay = match back {
        Overlay::Readiness if model.readiness_view.is_some() => Overlay::Readiness,
        Overlay::App if model.app_view.is_some() => Overlay::App,
        _ => Overlay::None,
    };
    if model.overlay == Overlay::Readiness {
        refresh(model);
    }
}

/// The overlay is left for another one: it comes back when that one ends.
fn leave_for(model: &mut Model) {
    if !is_ours(&model.overlay) {
        model.enforce_back = Some(Overlay::Enforce);
    }
}

/// `G` on step 2: the generator for the app (its own request count).
fn gen_key(model: &mut Model) -> Vec<Cmd> {
    let Some(app) = model.enforce_view.as_ref().map(|v| v.app.clone()) else {
        return Vec::new();
    };
    let cmds = generator::open_gen(model, app, false);
    leave_for(model);
    cmds
}

/// The app's would-block flow behind `gap` (not allowed by the draft,
/// not accepted by a mark): what the `w` form and the mark start from.
fn gap_flow(model: &Model, app: &GroupKey, gap: &Gap) -> Option<pce::traffic::ResolvedFlow> {
    let keys = keys_or_default(&model.limits.scope_keys);
    let marks = model.traffic.active_marks();
    model
        .traffic
        .loaded_flows()
        .iter()
        .filter(|f| is_unicast(f) && !draft_allows(f) && !accepted(marks.as_ref(), f))
        .filter(|f| f.service_text() == gap.service)
        .find(|f| {
            let (src, dst) = (group_key(&f.src, &keys), group_key(&f.dst, &keys));
            match gap.direction {
                Direction::In => &dst == app && src == gap.peer,
                Direction::Out => &src == app && dst != *app && dst == gap.peer,
            }
        })
        .cloned()
}

/// The gap under step 2's cursor and its flow, or why not.
fn selected_gap(model: &mut Model) -> Option<(GroupKey, Gap, pce::traffic::ResolvedFlow)> {
    let v = model.enforce_view.as_ref()?;
    let Some(gap) = v.selected_gap().cloned() else {
        model.status = StatusLine::info(match &v.readiness {
            Some(r) if r.verdict == pce::readiness::Verdict::NotEvaluated => {
                "no draft decisions: D evaluates the result first"
            }
            _ => "no gap to act on",
        });
        return None;
    };
    let app = v.app.clone();
    let Some(flow) = gap_flow(model, &app, &gap) else {
        model.status = StatusLine::warn("no loaded flow behind this gap");
        return None;
    };
    Some((app, gap, flow))
}

/// `w` on a gap: the draft allow rule form for its tuple, from one of its
/// flows, as `w` on a traffic row (writes armed).
fn gap_rule_key(model: &mut Model) -> Vec<Cmd> {
    if writes_refused(model) {
        return Vec::new();
    }
    let Some((_, _, flow)) = selected_gap(model) else {
        return Vec::new();
    };
    let cmds = begin_edit(model, EditIntent::RuleFromFlow(Box::new(flow)));
    leave_for(model);
    count(model, cmds.len());
    cmds
}

/// `m` on a gap: the mark form for its tuple as accept-blocked (saved to
/// marks.toml; never sent). An accepted flow is no gap.
fn gap_mark_key(model: &mut Model) -> Vec<Cmd> {
    if model.traffic.marks.error().is_some() {
        return marks::open_marks(model, None);
    }
    let Some((app, gap, flow)) = selected_gap(model) else {
        return Vec::new();
    };
    let keys = keys_or_default(&model.limits.scope_keys);
    let (peer_ep, app_ep) = match gap.direction {
        Direction::In => (&flow.src, &flow.dst),
        Direction::Out => (&flow.dst, &flow.src),
    };
    let peer = group_side(&gap.peer, &keys, Some(peer_ep));
    let mine = group_side(&app, &keys, Some(app_ep));
    let (src, dst) = match gap.direction {
        Direction::In => (peer, mine),
        Direction::Out => (mine, peer),
    };
    let prefill = Prefill {
        what: format!(
            "{} {} · {} {}",
            app.join("/"),
            gap.direction.as_str(),
            gap.peer_text,
            gap.service
        ),
        action: pce::marks::Action::AcceptBlocked,
        src,
        dst,
        service: gap.service.clone(),
        decision: String::new(),
        draft: String::new(),
        rows: RowsOf::Peers(Box::new(Focus::new(app, keys)), gap.direction),
    };
    let host = model.profile.host.clone();
    marks::open_form(model, MarkForm::new(prefill, MarkOrigin::Traffic, &host));
    leave_for(model);
    Vec::new()
}

/// `P` on step 3: the app's pending changes provisioned through the
/// existing path: the plan reads the pending list once more, the confirm
/// shows the exact body.
fn provision_app_key(model: &mut Model) -> Vec<Cmd> {
    if writes_refused(model) {
        return Vec::new();
    }
    let Some(v) = model.enforce_view.as_ref() else {
        return Vec::new();
    };
    let app = v.app_text();
    if Missing::of(model).pending {
        model.status = StatusLine::info("the pending changes are not loaded yet");
        return Vec::new();
    }
    let hrefs: Vec<String> = v
        .readiness
        .as_ref()
        .map(|r| r.pending.changes.iter().map(|c| c.href.clone()).collect())
        .unwrap_or_default();
    if hrefs.is_empty() {
        model.status = StatusLine::info(format!("nothing pending for {app}"));
        return Vec::new();
    }
    model.overlay = Overlay::None;
    model.status = StatusLine::info(format!("planning the provision of {app}'s changes…"));
    model.writes.in_flight = true;
    model.enforce_back = Some(Overlay::Enforce);
    vec![Cmd::PlanWrite(WriteIntent::Provision(ProvisionRequest {
        description: Some(PROVISION_DESCRIPTION.into()),
        subset: ProvisionSubset::Hrefs(hrefs),
    }))]
}

/// The app's workloads filter (its labels), as the CLI re-reads them.
fn app_filter(model: &Model, app: &GroupKey) -> Result<WorkloadFilter, String> {
    let idx = model
        .index()
        .ok_or_else(|| "the name index is not loaded yet".to_string())?;
    let keys = keys_or_default(&model.limits.scope_keys);
    let m = pce::traffic::app_labels(app, &keys, idx)?;
    Ok(pce::app::app_workload_filter(&pce::app::label_hrefs(&m)))
}

/// `Enter` on step 4: the move reviewed in the bulk confirm (typed count),
/// when readiness allows it; else the typed `enforce` first.
fn review_key(model: &mut Model) -> Vec<Cmd> {
    if writes_refused(model) {
        return Vec::new();
    }
    let missing = Missing::of(model);
    if missing.any() {
        model.status = StatusLine::warn(format!(
            "readiness is not complete (loading {}): nothing sent",
            missing.text()
        ));
        return Vec::new();
    }
    let Some(v) = model.enforce_view.as_mut() else {
        return Vec::new();
    };
    let Some(r) = &v.readiness else {
        return Vec::new();
    };
    if v.needs_word() {
        let why = if r.verdict.allows_enforcement() {
            format!(
                "{} rests on a partial result ({})",
                r.verdict.text(),
                v.partial.join("; ")
            )
        } else {
            format!("is not ready ({})", r.verdict.text())
        };
        model.status = StatusLine::warn(format!(
            "{} {why}: type {ENFORCE_WORD} and Enter to move it anyway; Esc cancels",
            r.app
        ));
        v.typing = Some(String::new());
        return Vec::new();
    }
    review(model, false);
    Vec::new()
}

/// `y` on the bulk confirm of a move the overlay planned without the
/// typed word: readiness computed again from what is loaded now; `Some`
/// (why, nothing sent) when the move would now need the word (not ready,
/// a partial result, or something no longer loaded).
pub(super) fn recheck_send(model: &mut Model) -> Option<String> {
    let send = model.enforce_send.as_ref()?;
    if send.readiness.forced {
        return None;
    }
    let app = model.enforce_view.as_ref()?.app.clone();
    let missing = Missing::of(model);
    if missing.any() {
        return Some(format!(
            "readiness is not complete since the review (loading {}): nothing sent",
            missing.text()
        ));
    }
    let r = compute(model, std::slice::from_ref(&app), false).pop();
    let partial = crate::tui::enforce::partial_reasons(model, &app, r.as_ref());
    match r {
        Some(r) if !r.verdict.allows_enforcement() => Some(format!(
            "{} is no longer ready ({}): nothing sent; Enter on step 4 again",
            r.app,
            r.verdict.text()
        )),
        Some(r) if !partial.is_empty() => Some(format!(
            "{} rests on a partial result since the review ({}): nothing sent; Enter on step 4 again",
            r.app,
            partial.join("; ")
        )),
        Some(_) => None,
        None => Some("no readiness for the app: nothing sent".into()),
    }
}

/// The typed `enforce` prompt: letters type, `Enter` reviews only when it
/// reads `enforce`, `Esc` cancels.
fn typing_key(model: &mut Model, k: KeyEvent) -> Vec<Cmd> {
    let Some(v) = model.enforce_view.as_mut() else {
        return Vec::new();
    };
    let Some(typed) = v.typing.as_mut() else {
        return Vec::new();
    };
    match k.code {
        KeyCode::Esc => {
            v.typing = None;
            model.status = StatusLine::info("cancelled, nothing sent");
        }
        KeyCode::Enter if typed == ENFORCE_WORD => {
            v.typing = None;
            review(model, true);
        }
        KeyCode::Enter => {
            model.status = StatusLine::warn(format!(
                "type {ENFORCE_WORD}, then Enter, to move it anyway"
            ));
        }
        KeyCode::Backspace => {
            typed.pop();
        }
        KeyCode::Char('u') if k.modifiers.contains(KeyModifiers::CONTROL) => typed.clear(),
        KeyCode::Char(c) if typed.chars().count() < ENFORCE_WORD.len() + 8 => typed.push(c),
        _ => {}
    }
    Vec::new()
}

/// The move planned locally and opened in the bulk confirm: the app's
/// workloads as the Workloads screen holds them, re-read by the app's
/// labels on `y`. `forced`: readiness did not allow it, or rests on a
/// partial result (typed through). What readiness reads is checked again:
/// it may have gone while the word was typed.
fn review(model: &mut Model, forced: bool) {
    let missing = Missing::of(model);
    if missing.any() {
        model.status = StatusLine::warn(format!(
            "readiness is not complete (loading {}): nothing sent",
            missing.text()
        ));
        return;
    }
    if !forced
        && model
            .enforce_view
            .as_ref()
            .is_none_or(EnforceView::needs_word)
    {
        model.status = StatusLine::warn(format!("type {ENFORCE_WORD} first: nothing sent"));
        return;
    }
    let Some(dir) = model.records_dir.clone() else {
        model.status = StatusLine::error(
            "no $XDG_STATE_HOME or $HOME to place the change records; nothing sent",
        );
        return;
    };
    if model.workloads.truncated {
        model.status = StatusLine::warn(
            "the workloads list is truncated: the app may have more workloads; nothing sent",
        );
        return;
    }
    let Some(idx) = model.index().cloned() else {
        return;
    };
    let Some(v) = model.enforce_view.as_ref() else {
        return;
    };
    let Some(r) = v.readiness.clone() else {
        return;
    };
    let keys = keys_or_default(&model.limits.scope_keys);
    let targets: Vec<BulkTarget> = app_workloads(&model.workloads.raw, &v.app, &keys, &idx)
        .into_iter()
        .map(|w| BulkTarget::of(w, &idx))
        .collect();
    if targets.is_empty() {
        model.status = StatusLine::warn(format!("{} has no workloads: nothing to move", r.app));
        return;
    }
    let filter = match app_filter(model, &v.app) {
        Ok(f) => f,
        Err(e) => {
            model.status = StatusLine::warn(format!("{e}; nothing sent"));
            return;
        }
    };
    let change = BulkChange::enforce(v.mode, v.visibility_level());
    let plan = match plan_bulk(
        &targets,
        &change,
        filter,
        Some(batch_origin(&model.profile)),
    ) {
        Ok(p) => p,
        Err(e) => {
            model.status = StatusLine::warn(e.to_string());
            return;
        }
    };
    if plan.items.is_empty() {
        model.status = StatusLine::info(format!("nothing to change: {}", plan.summary()));
        return;
    }
    let send = EnforceSend {
        app: r.app.clone(),
        target: v.target(),
        readiness: RecordReadiness::of(&r, forced),
        dir,
        origin: origin_of(&model.profile),
    };
    let loaded = model
        .traffic
        .data
        .is_some()
        .then(|| loaded_counts(model.traffic.loaded_flows(), &plan.hrefs()));
    let n = plan.items.len();
    let forced_text = if forced { " (not ready: forced)" } else { "" };
    model.status = if plan.needs_count() {
        StatusLine::warn(format!(
            "review the move{forced_text}: type {n} (the workload count), then y; Esc cancels"
        ))
    } else {
        StatusLine::info(format!(
            "review the move{forced_text}: y sends it, Esc cancels"
        ))
    };
    let org = model.profile.org_id;
    model.overlay = Overlay::ConfirmBulk(Box::new(BulkConfirm::new(plan, loaded, org)));
    model.enforce_send = Some(Box::new(send));
    model.enforce_back = Some(Overlay::Enforce);
}

/// `r` on step 5: the blocked flows since the record's time, one query,
/// through the cost guard.
fn watch_key(model: &mut Model) -> Vec<Cmd> {
    let Some(v) = model.enforce_view.as_ref() else {
        return Vec::new();
    };
    if matches!(v.watch, Watch::Running { .. }) {
        model.status = StatusLine::info("the watch query is running");
        return Vec::new();
    }
    let Some(since) = v.record.as_ref().and_then(|(_, r)| r.at_time()) else {
        model.status = StatusLine::info(format!(
            "no change record for {}: nothing to watch",
            v.app_text()
        ));
        return Vec::new();
    };
    let cap = model.traffic.filter.max_results;
    let probe = TrafficQueryBuilder::between(since, jiff::Timestamp::now())
        .max_results(cap)
        .build();
    match probe.map(|q| q.needs_confirmation()) {
        Err(e) => {
            model.status = StatusLine::warn(format!("watch: {e}"));
            Vec::new()
        }
        Ok(Some(reason)) => {
            model.enforce_back = Some(Overlay::Enforce);
            model.overlay = Overlay::Confirm {
                reason: format!(
                    "{reason}. Query the blocked flows since the change anyway: 1 Explorer query? (y/n)"
                ),
                action: PendingAction::EnforceWatch,
            };
            Vec::new()
        }
        Ok(None) => start_watch(model),
    }
}

/// The watch query, counted.
pub(super) fn start_watch(model: &mut Model) -> Vec<Cmd> {
    model.enforce_seq += 1;
    let seq = model.enforce_seq;
    let keys = keys_or_default(&model.limits.scope_keys);
    let max_results = model.traffic.filter.max_results;
    let Some(v) = model.enforce_view.as_mut() else {
        return Vec::new();
    };
    let Some(since) = v.record.as_ref().and_then(|(_, r)| r.at_time()) else {
        return Vec::new();
    };
    // One at a time: a query still on its way is cancelled.
    v.drop_watch();
    let cancel = CancellationToken::new();
    v.watch = Watch::Running {
        seq,
        cancel: cancel.clone(),
    };
    v.requests += 1;
    model.status = StatusLine::info(format!(
        "watch: 1 Explorer query, blocked flows of {} since {since}",
        v.app_text()
    ));
    vec![Cmd::WatchBlocked {
        seq,
        app: v.app.clone(),
        keys,
        since,
        max_results,
        cancel,
    }]
}

/// The watch query's answer, into the overlay that asked (else dropped).
pub(super) fn watch_done(
    model: &mut Model,
    seq: u64,
    result: Result<crate::tui::enforce::WatchOutcome, String>,
) {
    let keys = keys_or_default(&model.limits.scope_keys);
    let Some(app) = model
        .enforce_view
        .as_ref()
        .filter(|v| matches!(v.watch, Watch::Running { seq: s, .. } if s == seq))
        .map(|v| v.app.clone())
    else {
        return;
    };
    let watch = match result {
        Ok(o) => {
            let marks = model.traffic.active_marks();
            let blocked = blocked_since(&o.flows, &app, &keys, marks.as_ref());
            let partial = if o.truncated {
                " · the query hit its cap: more may exist"
            } else if o.incomplete {
                " · a region did not answer"
            } else {
                ""
            };
            let text = format!(
                "watch: {} flows ({} connections) blocked since {}{partial}",
                blocked.flows, blocked.connections, o.since
            );
            model.status = if blocked.flows > 0 || !partial.is_empty() {
                StatusLine::warn(text)
            } else {
                StatusLine::info(text)
            };
            Watch::Done(Box::new(WatchResult {
                since: o.since.to_string(),
                blocked,
                flows: o.flows.len(),
                truncated: o.truncated,
                incomplete: o.incomplete,
            }))
        }
        Err(e) => {
            model.status = StatusLine::error(format!("watch failed: {e}"));
            Watch::Failed(e)
        }
    };
    if let Some(v) = model.enforce_view.as_mut() {
        v.watch = watch;
    }
}

/// `R` on step 5: the rollback of the app's newest record. Refused when it
/// was rolled back already, or made on another PCE; else one re-read of
/// the app's workloads past the cache, then the preview.
fn rollback_key(model: &mut Model) -> Vec<Cmd> {
    if writes_refused(model) {
        return Vec::new();
    }
    let here = origin_of(&model.profile);
    let Some(v) = model.enforce_view.as_ref() else {
        return Vec::new();
    };
    if v.rollback_reading {
        model.status = StatusLine::info("reading the workloads for the rollback…");
        return Vec::new();
    }
    let Some((path, rec)) = &v.record else {
        model.status = StatusLine::info(format!(
            "no change record for {}: nothing to roll back",
            v.app_text()
        ));
        return Vec::new();
    };
    if let Some(by) = &v.rolled_back_by {
        model.status = StatusLine::warn(format!(
            "{} was already rolled back by {}; nothing sent",
            file_name(path),
            file_name(by)
        ));
        return Vec::new();
    }
    if !rec.made_on(&here) {
        model.status = StatusLine::warn(format!(
            "{} was made on {} (org {}); this profile writes to {} (org {}); nothing sent",
            file_name(path),
            rec.pce,
            rec.org_id,
            here.pce,
            here.org_id
        ));
        return Vec::new();
    }
    let Some(dir) = model.records_dir.clone() else {
        model.status = StatusLine::error(
            "no $XDG_STATE_HOME or $HOME to place the change records; nothing sent",
        );
        return Vec::new();
    };
    let filter = match app_filter(model, &v.app) {
        Ok(f) => f,
        Err(e) => {
            model.status = StatusLine::warn(format!("{e}; nothing sent"));
            return Vec::new();
        }
    };
    let seq = v.seq;
    let Some(v) = model.enforce_view.as_mut() else {
        return Vec::new();
    };
    v.rollback_reading = true;
    v.requests += 1;
    model.status = StatusLine::info("rollback: reading the app's workloads (1 request)…");
    // The records' place is probed before the read, as the CLI does.
    vec![Cmd::RollbackRead { seq, filter, dir }]
}

/// `shop_prod-20261019T100000.000Z.json`.
fn file_name(p: &std::path::Path) -> String {
    p.file_name()
        .map_or_else(|| p.display().to_string(), |n| n.to_string_lossy().into())
}

/// The re-read's answer: the rollback planned (each workload still as the
/// move set it back to its `before`, grouped), shown for `y`.
pub(super) fn rollback_read_done(
    model: &mut Model,
    seq: u64,
    result: Result<(Vec<Workload>, bool), String>,
) {
    let org = batch_origin(&model.profile);
    // Only into the overlay of the app that asked, while it is open.
    let open = model.overlay == Overlay::Enforce;
    let Some(v) = model.enforce_view.as_mut().filter(|v| open && v.seq == seq) else {
        return;
    };
    if !std::mem::take(&mut v.rollback_reading) {
        return;
    }
    let (workloads, truncated) = match result {
        Ok(r) => r,
        Err(e) => {
            model.status = StatusLine::error(format!("rollback re-read failed: {e}; nothing sent"));
            return;
        }
    };
    if truncated {
        model.status = StatusLine::warn(
            "the workloads read was truncated (the app has more workloads than were returned); nothing sent",
        );
        return;
    }
    let Some((path, rec)) = v.record.clone() else {
        return;
    };
    let app = v.app.clone();
    let rp = plan_rollback(&rec, &workloads);
    let filter = match app_filter(model, &app) {
        Ok(f) => f,
        Err(e) => {
            model.status = StatusLine::warn(format!("{e}; nothing sent"));
            return;
        }
    };
    let RollbackPlans {
        plans,
        count,
        needs_count,
    } = match rollback_plans(&rp, &filter, Some(org)) {
        Ok(p) => p,
        Err(e) => {
            model.status = StatusLine::warn(format!("rollback: {e}; nothing sent"));
            return;
        }
    };
    model.status = if count == 0 {
        StatusLine::warn(format!(
            "rollback: nothing to restore ({} skipped); Esc closes",
            rp.skipped.len()
        ))
    } else if needs_count {
        StatusLine::warn(format!(
            "review the rollback: type {count} (the workload count), then y; Esc cancels"
        ))
    } else {
        StatusLine::info("review the rollback: y sends it, Esc cancels")
    };
    if let Some(v) = model.enforce_view.as_mut() {
        v.rollback = Some(RollbackPreview {
            record_path: path,
            record: rec,
            plans,
            skipped: rp.skipped,
            count,
            needs_count,
            typed: String::new(),
        });
    }
}

/// The rollback preview's keys: digits type the count, `y` sends, `Esc` /
/// `n` cancel.
fn rollback_preview_key(model: &mut Model, k: KeyEvent) -> Vec<Cmd> {
    let Some(v) = model.enforce_view.as_mut() else {
        return Vec::new();
    };
    let Some(p) = v.rollback.as_mut() else {
        return Vec::new();
    };
    match k.code {
        KeyCode::Esc | KeyCode::Char('n') | KeyCode::Char('N') => {
            v.rollback = None;
            model.status = StatusLine::info("cancelled, nothing sent");
        }
        KeyCode::Char(d) if d.is_ascii_digit() && p.needs_count => {
            if p.typed.len() < 9 {
                p.typed.push(d);
            }
        }
        KeyCode::Backspace => {
            p.typed.pop();
        }
        KeyCode::Char('u') if k.modifiers.contains(KeyModifiers::CONTROL) => p.typed.clear(),
        KeyCode::Char('y') | KeyCode::Char('Y') => return send_rollback(model),
        _ => {}
    }
    Vec::new()
}

/// `y` on the preview: the rollback sent, with its own record.
fn send_rollback(model: &mut Model) -> Vec<Cmd> {
    if writes_refused(model) {
        return Vec::new();
    }
    let Some(dir) = model.records_dir.clone() else {
        model.status = StatusLine::error(
            "no $XDG_STATE_HOME or $HOME to place the change records; nothing sent",
        );
        return Vec::new();
    };
    let origin = origin_of(&model.profile);
    let Some(v) = model.enforce_view.as_mut() else {
        return Vec::new();
    };
    let Some(p) = v.rollback.as_ref() else {
        return Vec::new();
    };
    if p.count == 0 {
        model.status = StatusLine::info("nothing to restore; Esc closes");
        return Vec::new();
    }
    if !p.count_ok() {
        model.status = StatusLine::warn(format!(
            "type {} (the number of workloads) before y: nothing sent",
            p.count
        ));
        return Vec::new();
    }
    let Some(p) = v.rollback.take() else {
        return Vec::new();
    };
    let plans = p
        .plans
        .into_iter()
        .map(|(to, mut plan)| {
            if p.needs_count {
                plan.confirmed = Some(plan.items.len());
            }
            (to, plan)
        })
        .collect();
    let send = RollbackSend {
        app: p.record.app.clone(),
        record_path: p.record_path,
        plans,
        dir,
        origin,
    };
    model.writes.in_flight = true;
    model.batch_sending = true;
    model.batch_from_gen = false;
    model.status = StatusLine::info(format!(
        "rolling back {} workloads: 1 re-read, then the bulk updates …",
        p.count
    ));
    vec![Cmd::ExecuteRollback(Box::new(send))]
}

/// A move's answer: the bulk answer as on Workloads (status, report,
/// one reload of the workloads), the record named, the overlay on its
/// Watch step; a record that could not be updated opens loudly.
pub(super) fn enforce_done(model: &mut Model, o: crate::tui::enforce::EnforceOutcome) -> Vec<Cmd> {
    let cmds = bulk::bulk_done(model, &o.report);
    if let Some(p) = &o.record {
        model
            .status
            .text
            .push_str(&format!(" · {}: record {}", o.app, file_name(p)));
    }
    let origin = origin_of(&model.profile);
    // Only the overlay of the app that was moved.
    if let Some(v) = model
        .enforce_view
        .as_mut()
        .filter(|v| v.app_text() == o.app)
    {
        v.load_record(model.records_dir.as_deref(), &origin);
        if o.record.is_some() && o.report.touched() {
            v.step = Step::Watch;
        }
    }
    if let Some(e) = o.record_error {
        show_record_error(model, &e);
    }
    cmds
}

/// A rollback's answer: the status, the report (`:batch`), one reload of
/// the workloads, the records read again; loud on a record failure.
pub(super) fn rollback_done(
    model: &mut Model,
    o: crate::tui::enforce::RollbackOutcome,
) -> Vec<Cmd> {
    model.writes.in_flight = false;
    model.batch_sending = false;
    model.batch_from_gen = false;
    model.batch_report = Some(o.reports.iter().flat_map(|r| r.lines()).collect());
    let stopped = o.reports.iter().any(|r| r.stopped());
    let text = o
        .reports
        .iter()
        .map(|r| r.status_text())
        .collect::<Vec<_>>()
        .join(" · ");
    let record = o
        .record
        .as_ref()
        .map(|p| format!(" · record {}", file_name(p)))
        .unwrap_or_default();
    let text = format!("rollback: {text}{record} · :batch shows the report");
    model.status = if stopped {
        StatusLine::error(text)
    } else {
        StatusLine::info(text)
    };
    let mut cmds = Vec::new();
    if o.reports.iter().any(|r| r.touched()) {
        model.workloads.keep_status = true;
        cmds.extend(load_workloads(model, CacheMode::Bypass));
    }
    let origin = origin_of(&model.profile);
    if let Some(v) = model
        .enforce_view
        .as_mut()
        .filter(|v| v.app_text() == o.app)
    {
        v.load_record(model.records_dir.as_deref(), &origin);
    }
    if let Some(e) = o.record_error {
        show_record_error(model, &e);
    }
    cmds
}

/// A record that could not be updated: the whole text (where the record
/// was saved instead, the record itself) in the scrollable report viewer,
/// before the send's own report; the overlay comes back once it is read.
fn show_record_error(model: &mut Model, text: &str) {
    let mut lines: Vec<String> = text.lines().map(String::from).collect();
    lines.push(String::new());
    lines.extend(model.batch_report.take().unwrap_or_default());
    model.batch_report = Some(lines);
    model.status =
        StatusLine::error("CHANGE RECORD NOT UPDATED: the report says where the record was saved");
    if is_ours(&model.overlay) {
        model.enforce_back = Some(model.overlay.clone());
    } else if model.enforce_view.is_some() && model.enforce_back.is_none() {
        model.enforce_back = Some(Overlay::Enforce);
    }
    model.batch_from_gen = false;
    model.overlay = Overlay::BatchReport { scroll: 0 };
}

#[cfg(test)]
#[path = "enforce_tests.rs"]
mod tests;
