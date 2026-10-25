//! Pure state transitions. Never performs I/O; returns `Cmd`s instead.
use crate::tui::app_picker::{AppPicker, AppPickerOutcome, AppPickerPurpose};
use crate::tui::app_view::{AppPanel, AppView, Loadable};
use crate::tui::commands::{self, Command};
use crate::tui::edit::{self, EditIntent, RulesetTarget, Start, WriteIntent};
use crate::tui::filter::{FilterEditor, TrafficFilter};
use crate::tui::form::{Completing, FieldKind, Form, FormEvent};
use crate::tui::help::{HelpContext, HelpPage};
use crate::tui::label_editor::LabelFilterEditor;
use crate::tui::model::{
    By, Connection, Dir, Drill, EventsFilterEdit, Model, ObjectKind, ObjectsState, Overlay,
    PceUsage, PendingAction, PolicyPane, PolicyState, QueryState, Range, Screen, SectionRow,
    StatusKind, StatusLine, TrafficData, TrafficState, TrafficView, WritesState, queries_text,
    rule_line,
};
use crate::tui::msg::{
    Cmd, DraftFailure, DraftOutcome, FocusedOutcome, Msg, ObjectsPayload, QueryOutcome,
};
use crate::tui::picker::{KeyPicker, PickerOutcome};
use crate::tui::ui::widgets::overlay::{
    batch_report_last_scroll, confirm_batch_last_scroll, confirm_write_last_scroll, help_body_rows,
    help_last_scroll,
};
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use pce::models::{EventFilter, PolicyDecision, PolicyScope, Workload};
use pce::traffic::treemap::Weight;
use pce::traffic::{Decisions, Direction, Focus, FocusDirection, GroupKey, QueryProgress};
use pce::write::{
    Outcome, Plan, ProvisionRequest, ProvisionSubset, RevertRequest, WriteError, provision_counts,
};
use pce::{CacheMode, RuleKind, WriteMethod};
use std::time::Instant;

mod app_overlay;
mod bulk;
mod cmdline;
#[cfg(test)]
mod default_filter_tests;
mod enforce;
mod events;
mod explain;
#[cfg(test)]
mod explain_tests;
mod generator;
mod keys;
mod marks;
#[cfg(test)]
mod marks_tests;
mod objects;
mod policy;
mod saved;
#[cfg(test)]
mod saved_tests;
mod traffic;
mod where_used;
#[cfg(test)]
mod where_used_tests;
mod workloads;
mod writes;

pub use cmdline::on_command;
pub use events::event_detail_lines;
pub use policy::{REVERT_WORD, deny_unloaded_text};
pub use saved::apply_defaults;
pub(crate) use workloads::apply_workload_labels;

use app_overlay::*;
use events::*;
use explain::*;
use keys::*;
use objects::*;
use policy::*;
use traffic::*;
use where_used::*;
use workloads::*;
use writes::*;

/// The status and header note for a result a supercluster region did not
/// answer.
pub const INCOMPLETE: &str = " · incomplete: a PCE region did not respond";

pub fn update(model: &mut Model, msg: Msg) -> Vec<Cmd> {
    let traffic_answer = matches!(
        msg,
        Msg::QueryDone { .. } | Msg::FocusedDone { .. } | Msg::DraftDone { .. }
    );
    let ours_before = enforce::is_ours(&model.overlay);
    let data = enforce::is_data(&msg);
    let write = enforce::is_write(&msg);
    // What the where-used overlay reads: its rows follow when it lands.
    let used_data = matches!(
        msg,
        Msg::RuleSetsLoaded { .. }
            | Msg::DenyRulesLoaded { .. }
            | Msg::WorkloadsLoaded(_)
            | Msg::ObjectsLoaded { .. }
            | Msg::IndexLoaded { .. }
    );
    let mut cmds = on_msg(model, msg);
    if used_data {
        where_used_answer(model);
    }
    // A traffic answer that lands under the open app overlay (or one of its
    // prompts): its peers follow (no request).
    let app_open = matches!(
        model.overlay,
        Overlay::App
            | Overlay::Confirm {
                action: PendingAction::ReloadApp,
                ..
            }
    );
    if traffic_answer && app_open {
        rebuild_app_traffic(model);
    }
    // The same for the generator: its rows and its gap follow.
    let gen_open = matches!(
        model.overlay,
        Overlay::Generator
            | Overlay::Confirm {
                action: PendingAction::GenRequery(..),
                ..
            }
    );
    if traffic_answer && gen_open {
        generator::rebuild_gen(model);
    }
    // `:readiness` and the enforcement overlay: back after a step's form or
    // confirm, and re-evaluated when data lands.
    cmds.extend(enforce::after_msg(model, ours_before, data, write));
    // The marks file changed into (or out of) an invalid state, or marks
    // expire soon: shown once, beside an info status; a warning or an
    // error on the line keeps it waiting.
    if model.status.kind == StatusKind::Info
        && let Some(n) = model.traffic.take_marks_notice()
    {
        let text = &model.status.text;
        model.status = if text.is_empty() || text == "ready" {
            n
        } else {
            StatusLine {
                text: format!("{text} · {}", n.text),
                kind: n.kind,
            }
        };
    }
    cmds
}

fn on_msg(model: &mut Model, msg: Msg) -> Vec<Cmd> {
    match msg {
        Msg::Key(k) => {
            // `Esc again clears N marks` holds for the very next key only,
            // whatever it does (a screen switch, `?`, `:` too).
            if k.code != KeyCode::Esc {
                model.workloads.esc_armed = false;
            }
            on_key(model, k)
        }
        Msg::Mouse(me) => on_mouse(model, me),
        Msg::Resize(w, h) => {
            model.size = (w, h);
            let area = crate::tui::ui::screens::traffic::treemap_area(model.size);
            if area != model.traffic.treemap.area {
                model.traffic.treemap.area = area;
                model.traffic.relayout_treemap();
            }
            Vec::new()
        }
        Msg::Tick(now) => {
            if model.traffic.is_running() {
                refresh_running_status_at(model, now);
            }
            // Auto-refresh only on a live connection (never while connecting or
            // after a failed connect or profile switch).
            if !model.is_connected() {
                return Vec::new();
            }
            if !model.traffic.is_running()
                && model.screen == Screen::Traffic
                && model.focused
                && model.overlay == Overlay::None
                && model.traffic.data.is_some()
                && model.traffic.auto.due(now)
            {
                let f = model.traffic.filter.clone();
                if let Some(status) = auto_off_status(&f) {
                    model.traffic.auto.enabled = false;
                    model.status = status;
                    return Vec::new();
                }
                let cmds = start_query(model, f, CacheMode::Bypass);
                model.traffic.auto.from_tick = true;
                return cmds;
            }
            if model.screen == Screen::Policy
                && model.focused
                && model.overlay == Overlay::None
                && model.policy.loaded_at.is_some()
                && !model.policy.loading
                && model.policy.auto.due(now)
            {
                let cmds = load_rule_sets(model, CacheMode::Bypass);
                model.policy.auto.from_tick = true;
                return cmds;
            }
            let o = &model.objects;
            let kind = o.kind;
            if model.screen == Screen::Objects
                && kind != ObjectKind::IpLists
                && model.focused
                && model.overlay == Overlay::None
                && o.ages[kind.index()].is_some()
                && !o.loading[kind.index()]
                && o.auto.due(now)
            {
                // Opted in with `a`: a real refresh of the current kind, like Traffic and Policy.
                let cmds = load_objects(model, kind, CacheMode::Bypass);
                model.objects.auto.from_tick = true;
                return cmds;
            }
            let ipl = &model.objects.ip_lists;
            if model.screen == Screen::Objects
                && kind == ObjectKind::IpLists
                && model.focused
                && model.overlay == Overlay::None
                && ipl.loaded_at.is_some()
                && !ipl.loading
                && ipl.auto.due(now)
            {
                let cmds = load_ip_lists(model, CacheMode::Bypass);
                model.objects.ip_lists.auto.from_tick = true;
                return cmds;
            }
            if model.screen == Screen::Workloads
                && model.focused
                && model.overlay == Overlay::None
                && model.workloads.loaded_at.is_some()
                && !model.workloads.loading
                && model.workloads.auto.due(now)
            {
                let cmds = load_workloads(model, CacheMode::Bypass);
                model.workloads.auto.from_tick = true;
                return cmds;
            }
            if model.screen == Screen::Events
                && model.focused
                && model.overlay == Overlay::None
                && model.events.loaded_at.is_some()
                && !model.events.loading
                && model.events.auto.due(now)
            {
                let cmds = load_events(model, CacheMode::Bypass);
                model.events.auto.from_tick = !cmds.is_empty();
                return cmds;
            }
            Vec::new()
        }
        Msg::Focus(f) => {
            model.focused = f;
            Vec::new()
        }
        Msg::Connected => {
            model.connection = Connection::Connected {
                index: None,
                index_age_secs: 0,
            };
            model.status =
                StatusLine::info("connected; loading labels, label groups, IP lists, services");
            let mut cmds = vec![Cmd::LoadIndex(CacheMode::Use)];
            // Events do not need the name index, only a connection: the screen was
            // opened while connecting.
            if model.screen == Screen::Events {
                cmds.extend(load_missing_events(model));
            }
            cmds
        }
        Msg::ConnectFailed(e) => {
            fail(model, "not connected", &e, false);
            model.connection = Connection::Disconnected { error: e };
            Vec::new()
        }
        Msg::IndexLoaded { index, age_secs } => {
            model.connection = Connection::Connected {
                index: Some(index),
                index_age_secs: age_secs,
            };
            let unresolved = saved::resolve_search(model);
            model.status = match model.saved.warning.take() {
                Some(w) => StatusLine::warn(w),
                None if !unresolved.is_empty() => {
                    StatusLine::warn(format!("ready · rule search: {}", unresolved.join(", ")))
                }
                None => StatusLine::info("ready"),
            };
            model.objects.index_failed = false;
            model.workloads.index_failed = false;
            model.refresh_label_warnings();
            let mut cmds = Vec::new();
            // Rulesets that arrived before the names could be resolved.
            if !model.policy.raw.is_empty()
                && model.policy.rulesets.is_empty()
                && let Some(idx) = model.index().cloned()
            {
                let raw = std::mem::take(&mut model.policy.raw);
                model
                    .policy
                    .set_rule_sets(raw, &idx, model.workloads.impact_source());
                if model.screen == Screen::Policy {
                    cmds.extend(deny_fallback(model, CacheMode::Use));
                }
            }
            // The Objects screen (IP lists tab included) was opened while the
            // startup load was still running.
            if model.screen == Screen::Objects {
                cmds.extend(load_missing_objects(model));
            }
            if model.screen == Screen::Workloads {
                cmds.extend(load_missing_workloads(model));
            }
            // After a reconnect or profile switch nothing else would load Policy.
            if model.screen == Screen::Policy
                && model.policy.loaded_at.is_none()
                && !model.policy.loading
            {
                cmds.extend(load_rule_sets(model, CacheMode::Use));
            }
            cmds
        }
        Msg::IndexFailed(e) => {
            fail(model, "could not load objects", &e, false);
            model.objects.index_failed = true;
            model.workloads.index_failed = true;
            Vec::new()
        }
        Msg::Stats(s) => {
            model.stats = s;
            Vec::new()
        }
        Msg::SwitchScreen(s) => switch(model, s),
        Msg::Command(c) => on_command(model, c),
        Msg::ApplyFilter(f) => {
            model.screen = Screen::Traffic;
            // A new filter replaces the result: a draft run for it is stale.
            model.traffic.cancel_draft_run();
            // Key auto-repeat on `t` would otherwise restart (and re-POST) the same query.
            if model.traffic.is_running() && model.traffic.filter == f {
                model.status = StatusLine::info("query already running");
                return Vec::new();
            }
            model.traffic.filter = f.clone();
            start_query(model, f, CacheMode::Use)
        }
        Msg::RunNow => {
            model.traffic.cancel_draft_run();
            if model.traffic.is_running() {
                model.status = StatusLine::info("query already running");
                return Vec::new();
            }
            if let Some(focus) = model.traffic.focus_requery() {
                return requery_focus(model, focus);
            }
            let f = model.traffic.filter.clone();
            // The cost guard applies unless this exact filter produced the data on
            // screen (e.g. profile defaults above the thresholds, never confirmed).
            let fetched_with = model.traffic.data.as_ref().is_some_and(|d| d.filter == f);
            if !fetched_with && let Some(reason) = f.needs_confirmation() {
                model.overlay = Overlay::Confirm {
                    reason: format!("{reason}. Run anyway? (y/n)"),
                    action: PendingAction::RunNow(f),
                };
                return Vec::new();
            }
            run_now(model, f)
        }
        Msg::QueryProgress { seq, progress } => {
            if let QueryState::Running {
                seq: cur, status, ..
            } = &mut model.traffic.query
                && *cur == seq
            {
                *status = match progress {
                    QueryProgress::Submitted { .. } => "submitted".into(),
                    QueryProgress::Polling { status, .. } => format!("polling ({status})"),
                    QueryProgress::Evaluating { rules, .. } => format!("draft ({rules})"),
                    QueryProgress::Downloading => "downloading".into(),
                    QueryProgress::Done { rows } => format!("done, {rows} rows"),
                };
                refresh_running_status(model);
            }
            Vec::new()
        }
        Msg::QueryDone { seq, result } => {
            if model.traffic.seq != seq {
                return Vec::new(); // a cancelled or superseded query
            }
            match result {
                Ok(o) => {
                    // A new unfocused result: a focused one kept aside is stale.
                    model.traffic.unfocused = None;
                    model.traffic.focus_result = None;
                    let data = outcome_data(o);
                    // The opt-in raises the cap only for a new filter or
                    // window, never over a cap the user stepped to.
                    let new_filter = model.traffic.data.as_ref().is_none_or(|p| {
                        TrafficFilter {
                            max_results: data.filter.max_results,
                            ..p.filter.clone()
                        } != data.filter
                    });
                    let (n, truncated, cap) =
                        (data.flows.len(), data.truncated, data.filter.max_results);
                    model.traffic.data = Some(data);
                    model.traffic.drill = None;
                    model.traffic.treemap.zoom = None;
                    model.traffic.rebuild_views();
                    model.traffic.show_detail = false;
                    model.traffic.query = QueryState::Idle;
                    model.traffic.auto.from_tick = false;
                    if truncated
                        && new_filter
                        && model.limits.traffic_auto_backfill
                        && cap < pce::traffic::CONFIRM_MAX_RESULTS
                    {
                        return auto_backfill(model, n, cap);
                    }
                    let focus_hint = model
                        .traffic
                        .focus
                        .as_ref()
                        .map(|f| format!(" · focus is partial: r re-queries {}", f.app_text()))
                        .unwrap_or_default();
                    let z = model.traffic.squelched_hidden();
                    let count = crate::tui::traffic_marks::flows_text(n - z, n, z);
                    if let Some(d) = model.traffic.data.as_ref() {
                        model.status = result_status(d, &count, "", &focus_hint);
                    }
                }
                Err(e) => {
                    model.traffic.query = QueryState::Failed(e.clone());
                    let tripped = model.traffic.auto.take_tick_failure();
                    fail(model, "query", &e, tripped);
                }
            }
            Vec::new()
        }
        Msg::FocusedDone { seq, result } => {
            if model.traffic.seq != seq {
                return Vec::new(); // a cancelled or superseded query
            }
            match result {
                Ok(FocusedOutcome {
                    outcome,
                    focus,
                    queries,
                }) => {
                    model.traffic.query = QueryState::Idle;
                    // Only for the focus still set: clearing or changing it
                    // cancels the run, and a late answer never revives it.
                    if model.traffic.focus.as_ref() != Some(&focus) {
                        return Vec::new();
                    }
                    let data = outcome_data(outcome);
                    let app = focus.app_text();
                    model.traffic.show_detail = false;
                    model.traffic.apply_focused(focus, queries, data);
                    let t = &model.traffic;
                    let (n, z) = (t.focus_flows().len(), t.squelched_hidden());
                    let count = crate::tui::traffic_marks::flows_text(n - z, n, z);
                    let q = format!(" · {}", queries_text(queries));
                    if let Some(d) = model.traffic.data.as_ref() {
                        model.status = result_status(d, &format!("{count} for {app}"), &q, "");
                    }
                }
                Err(e) => {
                    model.traffic.query = QueryState::Failed(e.clone());
                    fail(model, "focused query", &e, false);
                }
            }
            Vec::new()
        }
        Msg::DraftDone { seq, result } => draft_done(model, seq, result),
        // Draft rulesets while the Policy screen shows the active ones (a
        // `w` form's load, or the screen's own from before a `d`): held for
        // forms, the screen's list untouched.
        Msg::RuleSetsLoaded {
            scope: PolicyScope::Draft,
            result,
        } if model.policy.scope != PolicyScope::Draft => {
            // Not the form's: the screen's own answer from before a `d`,
            // which takes the write's quiet flag as an ignored answer did.
            if !std::mem::take(&mut model.policy.form_drafts_loading) {
                model.policy.quiet_reload = false;
            }
            // Read before a policy write that changed them: dropped. A `w`
            // form waiting for them asks again (once); else the next form.
            if std::mem::take(&mut model.policy.form_drafts_stale) {
                let waiting = matches!(
                    &model.overlay,
                    Overlay::Form(f) if matches!(f.intent, EditIntent::RuleFromFlow(_))
                );
                return if model.overlay == Overlay::Generator {
                    // Counted in its header.
                    generator::ensure_drafts(model)
                } else if waiting {
                    load_form_drafts(model)
                } else {
                    Vec::new()
                };
            }
            match result {
                Ok((raw, _, hit)) => {
                    model.policy.form_drafts = Some(raw);
                    refill_rule_form(model);
                    generator::gen_drafts_answer(model, Ok(hit));
                }
                Err(e) => {
                    model.status = StatusLine::warn(format!("draft rulesets: {e}"));
                    generator::gen_drafts_answer(model, Err(&e));
                }
            }
            Vec::new()
        }
        Msg::RuleSetsLoaded { scope, result } => {
            // A reload a write asked for keeps the write's report. Taken before
            // the scope check: an ignored answer must not leave the flag set.
            let quiet = std::mem::take(&mut model.policy.quiet_reload);
            // A form's draft load answered here (a `d` came in between):
            // the next form may ask again.
            if scope == PolicyScope::Draft {
                model.policy.form_drafts_loading = false;
                model.policy.form_drafts_stale = false;
            }
            if scope != model.policy.scope {
                return Vec::new(); // the user toggled draft/active meanwhile
            }
            model.policy.loading = false;
            match result {
                Ok((raw, age, hit)) => {
                    model.policy.loaded_at = Some(Instant::now());
                    model.policy.auto.from_tick = false;
                    model.policy.age_secs = age;
                    // A reload may change deny rules: ask again, but keep the rows
                    // already fetched until the new answer replaces them.
                    model
                        .policy
                        .fallback_deny
                        .retain(|h, _| raw.iter().any(|r| &r.href == h));
                    model.policy.deny_requested.clear();
                    match model.index().cloned() {
                        Some(idx) => {
                            model
                                .policy
                                .set_rule_sets(raw, &idx, model.workloads.impact_source())
                        }
                        None => model.policy.raw = raw, // built when the index arrives
                    }
                    model.objects.ip_lists.recount(&model.policy);
                    model.recount_labels();
                    model.refresh_confirm_impact();
                    refill_rule_form(model);
                    if !quiet {
                        model.status = StatusLine::info(format!(
                            "{} rulesets ({})",
                            model.policy.rulesets.len(),
                            model.policy.scope.segment()
                        ));
                    }
                    // Explain's `Enter` waiting for them: its rule selected.
                    apply_policy_jump(model, None);
                    // Only the selected ruleset's deny rules are fetched on demand;
                    // an explicit `r` bypasses the cache for them as well.
                    let mode = if std::mem::take(&mut model.policy.bypass_deny) {
                        CacheMode::Bypass
                    } else {
                        CacheMode::Use
                    };
                    refresh_app_policy(model, scope, Ok(hit));
                    if scope == PolicyScope::Draft {
                        generator::gen_drafts_answer(model, Ok(hit));
                    }
                    // Off the Policy screen (the app overlay, `u` on Labels)
                    // nothing shows the deny rules: `switch` asks on entry.
                    if model.screen == Screen::Policy {
                        deny_fallback(model, mode)
                    } else {
                        Vec::new()
                    }
                }
                Err(e) => {
                    // The explicit refresh failed: its bypass must not leak into a later reload.
                    model.policy.bypass_deny = false;
                    model.policy.jump = None;
                    let tripped = model.policy.auto.take_tick_failure();
                    fail(model, "rulesets", &e, tripped);
                    refresh_app_policy(model, scope, Err(&e));
                    if scope == PolicyScope::Draft {
                        generator::gen_drafts_answer(model, Err(&e));
                    }
                    Vec::new()
                }
            }
        }
        Msg::DenyRulesLoaded { href, result } => {
            match result {
                Ok(d) => {
                    model.policy.fallback_deny.insert(href.clone(), d);
                    if let Some(idx) = model.index().cloned() {
                        model
                            .policy
                            .rebuild_rules(&idx, model.workloads.impact_source());
                    }
                    model.objects.ip_lists.recount(&model.policy);
                    model.recount_labels();
                    apply_policy_jump(model, Some(&href));
                }
                Err(e) if e.contains("not found") => {
                    model.policy.fallback_deny.insert(href.clone(), Vec::new());
                    model.policy.deny_endpoint_missing = true;
                    // The search's "may hold deny rules" count drops.
                    model.rebuild_rules();
                    apply_policy_jump(model, Some(&href));
                }
                Err(e) => {
                    drop_policy_jump(model, &href);
                    model.policy.deny_requested.remove(&href); // allow a retry on reselect
                    model.status = StatusLine::warn(format!("deny rules: {e}"));
                }
            }
            Vec::new()
        }
        Msg::AppWorkloadsLoaded { seq, result } => {
            let now = model.workloads.now.unwrap_or_else(jiff::Timestamp::now);
            let tier = model.limits.tier_key.clone();
            let idx = model.index().cloned();
            let Some(v) = model.app_view.as_mut().filter(|v| v.seq == seq) else {
                return Vec::new(); // an earlier overlay's answer
            };
            match (result, idx, v.label_hrefs.clone()) {
                (Ok(p), Some(idx), Some(hrefs)) => {
                    if p.hit {
                        v.cached += 1;
                    }
                    let mine: Vec<Workload> = p
                        .workloads
                        .into_iter()
                        .filter(|w| pce::app::workload_in_app(w, &hrefs))
                        .collect();
                    v.workloads =
                        Loadable::Loaded(pce::app::app_workloads(&mine, &idx, &tier, now));
                    v.workloads_truncated = p.truncated;
                }
                (Err(e), _, _) => v.workloads = Loadable::Failed(e),
                _ => v.workloads = Loadable::Unavailable("the name index is not loaded".into()),
            }
            Vec::new()
        }
        Msg::WorkloadsLoaded(result) => {
            model.workloads.loading = false;
            match result {
                Ok((rows, age, truncated)) => {
                    let order = model.limits.key_order();
                    let w = &mut model.workloads;
                    w.raw = rows;
                    w.truncated = truncated;
                    w.error = None;
                    // Marks of workloads gone (deleted elsewhere) go with them.
                    let loaded: std::collections::HashSet<&str> =
                        w.raw.iter().map(|x| x.href.as_str()).collect();
                    w.marked.retain(|h| loaded.contains(h.as_str()));
                    w.rebuild(&order);
                    w.loaded_at = Some(Instant::now());
                    w.auto.from_tick = false;
                    w.age_secs = age;
                    w.show_detail &= w.selected_row().is_some();
                    // The reload after a bulk change keeps its report on the line.
                    if !std::mem::take(&mut model.workloads.keep_status) {
                        model.status =
                            StatusLine::info(format!("{} workloads", model.workloads.table.len()));
                    }
                    // `Enter` on a workload in the app overlay before the list was here.
                    if let Some(href) = model.workloads.open_on_load.take() {
                        let w = &mut model.workloads;
                        w.show_detail = w.select_href(&href);
                    }
                    // The rules pane's P/C and impact follow; an open rule
                    // detail reads them from the rows, so no overlay is
                    // touched here.
                    model.rebuild_rules();
                    model.recount_labels();
                    model.refresh_confirm_impact();
                    model.refresh_label_warnings();
                    // The list counts a label's workloads from now on.
                    model.objects.workload_probes.clear();
                }
                Err(e) => {
                    model.workloads.error = Some(e.clone());
                    model.workloads.open_on_load = None;
                    model.workloads.keep_status = false;
                    let tripped = model.workloads.auto.take_tick_failure();
                    fail(model, "could not load workloads", &e, tripped);
                }
            }
            Vec::new()
        }
        Msg::EventsLoaded(result) => {
            model.events.loading = false;
            match result {
                Ok(p) => {
                    let e = &mut model.events;
                    e.raw = p.events;
                    e.rebuild();
                    e.window = Some(p.window);
                    e.loaded_at = Some(Instant::now());
                    e.auto.from_tick = false;
                    e.age_secs = p.age_secs;
                    e.truncated = p.truncated;
                    e.answered = e.requested.clone();
                    model.status = StatusLine::info(events_loaded_text(&model.events));
                }
                Err(e) => {
                    let tripped = model.events.auto.take_tick_failure();
                    fail(model, "could not load events", &e, tripped);
                }
            }
            Vec::new()
        }
        Msg::ObjectsLoaded { kind, result } => {
            let i = kind.index();
            model.objects.loading[i] = false;
            let mut relabelled = false;
            match result {
                Ok(payload) => {
                    // New labels reach the name index (and so the forms and
                    // filters) at no extra request; the other lists are kept.
                    if let (
                        ObjectsPayload::Labels(v, _),
                        Connection::Connected {
                            index: Some(idx), ..
                        },
                    ) = (&payload, &mut model.connection)
                    {
                        *idx = std::sync::Arc::new(idx.with_labels(v));
                        relabelled = true;
                    }
                    let o = &mut model.objects;
                    let age = match payload {
                        ObjectsPayload::Labels(v, a) => {
                            o.all_labels = v;
                            a
                        }
                        ObjectsPayload::LabelGroups(v, a) => {
                            o.groups.set_rows(v);
                            a
                        }
                        ObjectsPayload::Services(v, a) => {
                            o.services.set_rows(v);
                            a
                        }
                    };
                    o.ages[i] = Some(age);
                    o.loaded_at[i] = Some(Instant::now());
                    o.auto.from_tick = false;
                    if kind != ObjectKind::Services {
                        // The Labels tab's rows (and their usage) follow.
                        model.recount_labels();
                    }
                    if relabelled {
                        // The rules pane's names and stored impacts follow.
                        model.rebuild_rules();
                        model.refresh_confirm_impact();
                    }
                }
                Err(e) => {
                    let tripped = model.objects.auto.take_tick_failure();
                    fail(model, kind.title(), &e, tripped);
                }
            }
            Vec::new()
        }
        Msg::LabelUsageLoaded(result) => label_usage_loaded(model, result),
        Msg::LabelWorkloadsCounted { href, result } => {
            probe_answered(model, href, result);
            Vec::new()
        }
        Msg::IpListsLoaded(result) => {
            model.objects.ip_lists.loading = false;
            match result {
                // No status text: a write's report survives the reload it caused.
                Ok((lists, age)) => {
                    // The index (and so the rule impact's list sizes) follows.
                    if let Connection::Connected {
                        index: Some(idx), ..
                    } = &mut model.connection
                    {
                        *idx = std::sync::Arc::new(idx.with_ip_lists(&lists));
                    }
                    model.objects.ip_lists.set_lists(lists, &model.policy);
                    model.rebuild_rules();
                    model.refresh_confirm_impact();
                    let s = &mut model.objects.ip_lists;
                    s.loaded_at = Some(Instant::now());
                    s.auto.from_tick = false;
                    s.age_secs = age;
                    s.show_detail &= s.table.selected_item().is_some();
                }
                Err(e) => {
                    let tripped = model.objects.ip_lists.auto.take_tick_failure();
                    fail(model, "IP lists", &e, tripped);
                }
            }
            Vec::new()
        }
        Msg::Exported(result) => {
            model.status = match result {
                Ok(done) => StatusLine::info(done),
                Err(e) => StatusLine::error(e),
            };
            Vec::new()
        }
        Msg::WritesSet(state) => {
            model.writes = WritesState {
                in_flight: false,
                ..state
            };
            model.status = if model.writes.enabled {
                StatusLine::info(writes_status(&model.writes))
            } else {
                StatusLine::warn(writes_status(&model.writes))
            };
            Vec::new()
        }
        Msg::ProfileSwitched(result) => match result {
            Ok((info, limits)) => {
                model.switching = false;
                model.profile = info;
                model.limits = limits;
                model.reset_data();
                model.connection = Connection::Connecting;
                model.status =
                    StatusLine::info(format!("switched to {}; connecting…", model.profile.name));
                // The new profile's defaults, before anything loads.
                apply_defaults(model);
                vec![Cmd::Connect]
            }
            Err(e) => {
                model.switching = false;
                model.status = StatusLine::error(format!("profile: {e}"));
                Vec::new()
            }
        },
        Msg::PendingLoaded(result) => {
            model.policy.pending_loading = false;
            match result {
                Ok(p) => {
                    let n = p.len();
                    // After a provision the reload keeps its report in front.
                    model.status = StatusLine::info(match model.policy.provisioned.take() {
                        Some(done) => format!("{done}; {n} pending changes left"),
                        None => format!("{n} pending changes"),
                    });
                    model.policy.pending_selected = 0;
                    model.policy.pending_marked.clear();
                    model.policy.pending = Some(p);
                    // Never over another overlay (an error a provision left open).
                    if matches!(model.overlay, Overlay::None | Overlay::Pending) {
                        model.overlay = Overlay::Pending;
                    }
                }
                Err(e) => {
                    model.policy.provisioned = None;
                    fail(model, "pending", &e, false);
                }
            }
            Vec::new()
        }
        Msg::DiffLoaded(result) => {
            model.policy.diff_loading = false;
            match result {
                // Only over the list it came from (or none): a late diff
                // never replaces another overlay, such as a provision plan.
                Ok(diff) if matches!(model.overlay, Overlay::Pending | Overlay::None) => {
                    model.status = StatusLine::info(format!("diff of {}", diff.name));
                    model.overlay = Overlay::Diff { diff, scroll: 0 };
                }
                Ok(_) => {
                    model.status = StatusLine::info("diff ready; reopen the pending list");
                }
                Err(e) => fail(model, "diff", &e, false),
            }
            Vec::new()
        }
        Msg::WriteDone {
            kind,
            method,
            result,
        } => {
            model.writes.in_flight = false;
            // A write may change what carries a label: probes are asked again.
            model.objects.workload_probes.clear();
            // The runtime dropped `policy:pending`; the next `p` fetches again.
            model.policy.pending = None;
            model.policy.pending_marked.clear();
            let counts = model.policy.provision_counts.take();
            // Only a written ruleset sends the rule waiting for it.
            let rule_after = model.policy.rule_after_ruleset.take();
            let from_flow = std::mem::take(&mut model.policy.write_from_flow);
            match result {
                Ok(Outcome::Written(w)) => {
                    let mut text = if kind == pce::write::ObjectKind::Provision {
                        format!("provisioned {}: {}", w.href, counts.unwrap_or_default())
                    } else if kind == pce::write::ObjectKind::Revert {
                        format!("reverted {}", counts.unwrap_or_default())
                    } else {
                        let verb = match method {
                            Some(WriteMethod::Post) => "created",
                            Some(WriteMethod::Put) => "updated",
                            Some(WriteMethod::Delete) => "deleted",
                            None => "wrote",
                        };
                        let mut t = format!("{verb} {}", w.href);
                        if w.pending {
                            t.push_str(" (pending)");
                        }
                        t
                    };
                    if !w.dropped_fields.is_empty() {
                        text.push_str(&format!(
                            "; the PCE dropped {}",
                            w.dropped_fields.join(", ")
                        ));
                    }
                    for warning in &w.warnings {
                        text.push_str("; ");
                        text.push_str(warning);
                    }
                    model.status = if w.warnings.is_empty() && w.dropped_fields.is_empty() {
                        StatusLine::info(text)
                    } else {
                        StatusLine::warn(text)
                    };
                    // The pending reload shows the write's own report.
                    if kind.whole_policy() {
                        model.policy.provisioned = Some(model.status.text.clone());
                    }
                    // The draft policy (or a workload's scope) may have
                    // changed: the draft decisions on screen are evaluated again.
                    let rule_waits =
                        rule_after.is_some() && kind == pce::write::ObjectKind::RuleSet;
                    let redraft = redraft_after_write(model, from_flow, rule_waits);
                    let mut cmds = reload_after_write(model, kind);
                    cmds.extend(redraft);
                    if !rule_waits {
                        finish_form(model);
                    }
                    if let Some((draft, form)) = rule_after
                        && kind == pce::write::ObjectKind::RuleSet
                    {
                        // The second confirmation: the rule in the new ruleset;
                        // a failed plan brings the form back.
                        model.traffic.rule_form = Some(form);
                        model.writes.in_flight = true;
                        cmds.push(Cmd::PlanWrite(WriteIntent::CreateRule {
                            ruleset: RulesetTarget::Href(w.href.clone()),
                            draft,
                        }));
                    }
                    cmds
                }
                Ok(Outcome::Skipped { reason, .. }) => {
                    finish_form(model);
                    model.status = StatusLine::info(format!("skipped: {reason}"));
                    Vec::new()
                }
                // The write may have applied: say so once (the error's own
                // text carries the advice) and reload to show the real state.
                // A failed audit names its log in the error text.
                Err(e) if is_unanswered(&e) => {
                    model.form_from_app = false; // the error overlay takes over
                    let audited = !matches!(e, WriteError::AuditFailed { .. });
                    model.status = StatusLine::error(if audited {
                        "write not answered; reloading"
                    } else {
                        "write not answered and not audited; reloading"
                    });
                    model.overlay =
                        Overlay::Error(format!("the request was sent but not answered: {e}"));
                    reload_after_write(model, kind)
                }
                Err(e) => {
                    model.form_from_app = false; // the error overlay takes over
                    model.status = StatusLine::error("write failed; nothing reloaded");
                    model.overlay = Overlay::Error(format!("write failed: {e}"));
                    Vec::new()
                }
            }
        }
        // A plan that lands while a batch is sent belongs to nothing on
        // screen; `in_flight` stays the batch's.
        Msg::BatchPlanned(_) | Msg::Planned(_) if model.batch_sending => {
            model.status = StatusLine::warn("plan discarded: a batch is being sent, nothing sent");
            Vec::new()
        }
        Msg::BatchPlanned(result) => {
            model.writes.in_flight = false;
            let planned_here = batch_lands(model);
            let changed = planned_here && generator::changed_since_plan(model);
            if let Some(v) = model.gen_view.as_mut() {
                v.planning = false;
                v.planned = None;
            }
            // Set again only when the confirm opens over the generator.
            model.batch_from_gen = false;
            match result {
                // Keys typed while the batch was planned went elsewhere; a
                // `y` there must not confirm this.
                // Planned on the profile before a switch: never sent here.
                Ok(batch) if !batch_for_session(model, &batch) => {
                    model.status = StatusLine::warn(
                        "requests discarded: planned on another profile, nothing sent",
                    );
                }
                Ok(_) if !planned_here => {
                    model.status = StatusLine::warn("requests discarded, nothing sent");
                }
                Ok(_) if changed => {
                    model.status = StatusLine::warn(
                        "the proposal changed while planning: nothing sent, Enter plans again",
                    );
                }
                Ok(mut batch) => {
                    generator::planned_proposal(model, &mut batch);
                    let what = if batch.sends() == 1 {
                        "request"
                    } else {
                        "requests"
                    };
                    // The difference itself heads the confirm.
                    model.status = if batch.note.is_some() {
                        StatusLine::warn(format!(
                            "the draft changed since the preview: review the {what}"
                        ))
                    } else {
                        StatusLine::info(format!("review the {what}: y sends them all, n cancels"))
                    };
                    model.batch_from_gen = true;
                    model.overlay = Overlay::ConfirmBatch { batch, scroll: 0 };
                }
                Err(e) => batch_plan_failed(model, &e, planned_here),
            }
            Vec::new()
        }
        Msg::BatchDone {
            kinds,
            summaries,
            target,
            results,
        } => batch_done(model, &kinds, &summaries, &target, &results),
        Msg::BulkDone(report) => bulk::bulk_done(model, &report),
        Msg::EnforceDone(o) => enforce::enforce_done(model, *o),
        Msg::RollbackDone(o) => enforce::rollback_done(model, *o),
        Msg::RollbackRead { seq, result } => {
            enforce::rollback_read_done(model, seq, result);
            Vec::new()
        }
        Msg::WatchDone { seq, result } => {
            enforce::watch_done(model, seq, result);
            Vec::new()
        }
        Msg::BulkCheckDone { seq, result } => {
            bulk::check_done(model, seq, result);
            Vec::new()
        }
        Msg::BatchProgress { done, of } => {
            if model.batch_sending {
                model.status = StatusLine::info(format!("sending: {done} of {of} answered …"));
            }
            Vec::new()
        }
        Msg::Planned(result) => {
            model.writes.in_flight = false;
            let flow_form = model.traffic.rule_form.take();
            let mut rule_after = model.policy.rule_after_ruleset.take();
            let scope_warnings = std::mem::take(&mut model.policy.scope_warnings);
            // The ruleset `Ctrl-R` asked for exists after all (the fresh read
            // found it): nothing is created; the form comes back naming it,
            // and Enter adds the rule to it through its own confirm.
            if model.overlay == Overlay::None
                && let Ok((Plan::Noop { existing_href, .. }, _)) = &result
                && let Some((_, mut form)) = rule_after.take()
            {
                let rs = model
                    .draft_rulesets()
                    .and_then(|sets| sets.iter().find(|rs| &rs.href == existing_href));
                let name = rs.map_or_else(
                    || form.value("Ruleset").trim().to_string(),
                    |rs| rs.name.clone(),
                );
                let empty = pce::NameIndex::default();
                let idx = model.index().cloned();
                let note =
                    edit::existing_ruleset_note(&form, &name, rs, idx.as_deref().unwrap_or(&empty));
                if let Some(field) = form.fields.iter_mut().find(|x| x.label == "Ruleset") {
                    field.value = name;
                    field.hint = note.clone().into();
                }
                form.offers_ruleset = None;
                form.error = Some(note.clone());
                model.overlay = Overlay::Form(form);
                model.status = StatusLine::info(note);
                return Vec::new();
            }
            match result {
                // Keys typed while the plan was on its way belong to the
                // overlay they went to; a `y` there must not confirm this.
                Ok((Plan::Send { .. }, _)) if model.overlay != Overlay::None => {
                    model.form_from_app = false;
                    model.status = StatusLine::warn("request discarded, nothing sent");
                }
                Ok((mut plan @ Plan::Send { .. }, kind)) => {
                    // A `w` form's write evaluates the draft once it is done.
                    model.policy.write_from_flow = flow_form.is_some() || rule_after.is_some();
                    // A ruleset `Ctrl-R` asked for keeps its rule up to its confirm.
                    model.policy.rule_after_ruleset = rule_after;
                    if kind == pce::write::ObjectKind::Revert {
                        name_rulesets(&mut plan, &model.policy.raw);
                    }
                    model.policy.revert_typed.clear();
                    let review = if kind == pce::write::ObjectKind::Revert {
                        "review the revert: type revert and Enter to send it, Esc cancels"
                    } else {
                        "review the request: y sends it, n cancels"
                    };
                    // The rule form's Scope warnings, checked again at its
                    // `Enter`, stay in sight until `y`.
                    model.status = if scope_warnings.is_empty() {
                        StatusLine::info(review)
                    } else {
                        StatusLine::warn(format!("⚠ {} · {review}", scope_warnings.join(" · ")))
                    };
                    model.overlay = Overlay::ConfirmWrite {
                        plan,
                        kind,
                        scroll: 0,
                    };
                    model.refresh_confirm_impact();
                }
                // Nothing to send is an answer, not a failure: provisioning
                // with nothing pending, an update whose fields already match
                // (a stale form, a retyped entry), a create that exists.
                Ok((Plan::Noop { reason, .. }, _)) => {
                    finish_form(model);
                    model.status = StatusLine::info(reason);
                }
                Err(e) => {
                    // A `w` form comes back with the planner's answer (an
                    // unknown ruleset name lists similar ones, a name several
                    // rulesets share lists them), fields as typed: the one
                    // submitted, or the one `Ctrl-R` carried.
                    let form = flow_form.or(rule_after.map(|(_, f)| f));
                    if let Some(mut f) = form
                        && model.overlay == Overlay::None
                    {
                        // An unknown name: the error says what Ctrl-R creates
                        // (or what to do instead); the hint row shows the offer.
                        let keys = &model.limits.scope_keys;
                        f.offers_ruleset = edit::new_ruleset_offer(&f, &e, keys);
                        let e = edit::explain_unknown_ruleset(&f, &e, keys);
                        f.error = Some(e.clone());
                        model.overlay = Overlay::Form(f);
                        fail(model, "write", &e, false);
                    } else {
                        model.form_from_app = false;
                        fail(model, "write", &e, false);
                    }
                }
            }
            Vec::new()
        }
    }
}

#[cfg(test)]
mod app_sections_tests;
#[cfg(test)]
mod app_tests;
#[cfg(test)]
mod backfill_tests;
#[cfg(test)]
mod batch_tests;
#[cfg(test)]
mod ctrl_u_tests;
#[cfg(test)]
mod draft_tests;
#[cfg(test)]
mod event_actors_tests;
#[cfg(test)]
mod event_types_tests;
#[cfg(test)]
mod filter_completion_tests;
#[cfg(test)]
mod focus_tests;
/// The Labels tab's `f` key filter, `=` searches, `i` and `u` (issue #9).
#[cfg(test)]
mod label_filter_tests;
/// `I` on Policy: the workloads behind a rule's impact (issue #53).
#[cfg(test)]
mod matched_tests;
#[cfg(test)]
mod rule_form_ruleset_tests;
/// `f` on Policy: the rule search across rulesets (issue #51).
#[cfg(test)]
mod rule_search_tests;
#[cfg(test)]
mod section_order_tests;
#[cfg(test)]
mod tests;
/// `t` on Policy: a rule or ruleset enabled ↔ disabled (issue #52).
#[cfg(test)]
mod toggle_tests;
#[cfg(test)]
mod ven_tests;
#[cfg(test)]
mod window_step_tests;
#[cfg(test)]
mod workload_group_tests;
