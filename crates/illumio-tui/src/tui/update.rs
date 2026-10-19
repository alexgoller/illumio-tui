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

mod bulk;
mod generator;
mod marks;
#[cfg(test)]
mod marks_tests;
mod traffic;
use traffic::*;
mod app_overlay;
use app_overlay::*;
mod policy;
pub use policy::REVERT_WORD;
use policy::*;
mod objects;
use objects::*;
mod workloads;
pub(crate) use workloads::apply_workload_labels;
use workloads::*;
mod events;
pub use events::event_detail_lines;
use events::*;
mod writes;
use writes::*;
mod keys;
use keys::*;
mod cmdline;
pub use cmdline::on_command;

/// The status and header note for a result a supercluster region did not
/// answer.
pub const INCOMPLETE: &str = " · incomplete: a PCE region did not respond";

pub fn update(model: &mut Model, msg: Msg) -> Vec<Cmd> {
    let traffic_answer = matches!(
        msg,
        Msg::QueryDone { .. } | Msg::FocusedDone { .. } | Msg::DraftDone { .. }
    );
    let cmds = on_msg(model, msg);
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
            model.status = StatusLine::info("ready");
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
                    model.policy.fallback_deny.insert(href, d);
                    if let Some(idx) = model.index().cloned() {
                        model
                            .policy
                            .rebuild_rules(&idx, model.workloads.impact_source());
                    }
                    model.objects.ip_lists.recount(&model.policy);
                    model.recount_labels();
                }
                Err(e) if e.contains("not found") => {
                    model.policy.fallback_deny.insert(href, Vec::new());
                }
                Err(e) => {
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
                Ok((rows, age, window)) => {
                    let e = &mut model.events;
                    e.raw = rows;
                    e.rebuild();
                    e.window = Some(window);
                    e.loaded_at = Some(Instant::now());
                    e.auto.from_tick = false;
                    e.age_secs = age;
                    model.status = StatusLine::info(format!("{} events", model.events.table.len()));
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
                    model.status = StatusLine::info(review);
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
mod tests {
    use super::*;
    use crate::tui::model::{Connection, Model, Screen, StatusKind};
    use crate::tui::msg::{Cmd, Msg};
    use crate::tui::test_support::model_fixture;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    fn key(c: char) -> Msg {
        Msg::Key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE))
    }

    /// Sends `msg` and asserts it asked for nothing: a key, a tab or a screen
    /// change must not reach the PCE.
    #[track_caller]
    fn quiet(m: &mut Model, msg: Msg) {
        assert_eq!(update(m, msg), Vec::<Cmd>::new());
    }

    #[test]
    fn v_cycles_views_and_g_sets_group_keys_without_requests() {
        use crate::tui::model::{Overlay, TrafficView};
        use crate::tui::test_support::model_with_flows;
        let mut m = model_with_flows();
        assert_eq!(m.traffic.view, TrafficView::Flows);
        assert!(update(&mut m, key('v')).is_empty());
        assert_eq!(m.traffic.view, TrafficView::Aggregate);
        assert!(
            m.traffic.groups.len() >= 3,
            "groups built from data: {}",
            m.traffic.groups.len()
        );
        assert_eq!(m.traffic.group_keys, vec!["app", "env"]);
        assert!(update(&mut m, key('g')).is_empty());
        assert!(matches!(m.overlay, Overlay::GroupKeys { .. }));
        for _ in 0..7 {
            assert!(
                update(
                    &mut m,
                    Msg::Key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE)),
                )
                .is_empty()
            );
        }
        for c in "role".chars() {
            assert!(update(&mut m, key(c)).is_empty());
        }
        assert!(
            update(
                &mut m,
                Msg::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
            )
            .is_empty()
        );
        assert_eq!(m.traffic.group_keys, vec!["role"]);
        assert!(
            m.traffic.groups.rows().iter().any(|g| g.src == vec!["web"]),
            "{:?}",
            m.traffic.groups.rows()
        );
        assert!(update(&mut m, key('v')).is_empty());
        assert_eq!(m.traffic.view, TrafficView::Mesh);
        assert!(update(&mut m, key('v')).is_empty());
        assert_eq!(m.traffic.view, TrafficView::Blocked);
        let headers = |m: &Model| {
            m.traffic
                .blocked
                .rows()
                .iter()
                .filter(|r| r.is_header())
                .count()
        };
        assert_eq!(
            (headers(&m), m.traffic.blocked.len()),
            (2, 4),
            "two destination roles, one blocked source each"
        );
        assert!(update(&mut m, key('v')).is_empty());
        assert_eq!(m.traffic.view, TrafficView::Treemap);
        assert!(update(&mut m, key('v')).is_empty());
        assert_eq!(m.traffic.view, TrafficView::Flows);
    }

    #[test]
    fn g_opens_the_picker_and_enter_regroups_every_view_without_requests() {
        use crate::tui::test_support::model_with_rich_flows;
        let mut m = model_with_rich_flows();
        assert_eq!(
            m.traffic.group_keys,
            vec!["app", "env"],
            "the profile's scope keys"
        );
        assert_eq!(m.traffic.flows.rows()[0].src_group.as_ref(), "shop/prod");
        quiet(&mut m, key('g'));
        let Overlay::GroupKeys(p) = &m.overlay else {
            panic!("{:?}", m.overlay)
        };
        let listed: Vec<&str> = p.entries.iter().map(|e| e.key.as_str()).collect();
        assert_eq!(listed, vec!["app", "env", "role", "datacenter", "loc"]);
        assert_eq!(p.entries[3].seen, Some(11));
        for msg in [key('j'), key('j'), key(' '), enter()] {
            assert!(update(&mut m, msg).is_empty(), "the picker never requests");
        }
        assert_eq!(m.overlay, Overlay::None);
        assert_eq!(m.traffic.group_keys, vec!["app", "env", "role"]);
        assert_eq!(m.status.text, "grouped by app,env,role");
        assert_eq!(
            m.traffic.flows.rows()[0].src_group.as_ref(),
            "shop/prod/web"
        );
        assert_eq!(m.traffic.flows.rows()[0].dst_group.as_ref(), "shop/prod/db");
        assert!(m.traffic.groups.rows().iter().all(|g| g.src.len() == 3));
        assert!(m.traffic.blocked.rows().iter().all(|b| b.app.len() == 3));
        // A non-workload endpoint shows `-` in every slot.
        let corp = m
            .traffic
            .flows
            .rows()
            .iter()
            .find(|r| r.src.ip_lists == vec!["Corp"])
            .unwrap();
        assert_eq!(corp.src_group.as_ref(), "-/-/-");
    }

    #[test]
    fn all_labels_groups_by_every_seen_key_until_a_profile_switch() {
        use crate::tui::model::{Limits, ProfileInfo};
        use crate::tui::test_support::model_with_rich_flows;
        let mut m = model_with_rich_flows();
        quiet(&mut m, key('g'));
        for _ in 0..5 {
            quiet(&mut m, key('j'));
        }
        quiet(&mut m, key(' '));
        quiet(&mut m, enter());
        assert!(m.traffic.all_keys);
        assert_eq!(
            m.traffic.effective_keys(),
            vec!["app", "env", "role", "datacenter", "loc"],
            "scope keys, tier key, then alphabetical"
        );
        assert!(m.traffic.groups.rows().iter().all(|g| g.src.len() == 5));
        assert_eq!(
            m.traffic.flows.rows()[4].src_group.as_ref(),
            "crm/prod/web/fra1/us"
        );
        // Reopening shows `all labels` checked and no key.
        quiet(&mut m, key('g'));
        let Overlay::GroupKeys(p) = &m.overlay else {
            panic!("{:?}", m.overlay)
        };
        assert!(p.all && !p.is_checked("app"));
        quiet(&mut m, kc(KeyCode::Esc));
        assert_eq!(
            update(
                &mut m,
                Msg::ProfileSwitched(Ok((
                    ProfileInfo {
                        name: "prod".into(),
                        host: "prod.example".into(),
                        org_id: 9,
                    },
                    Limits {
                        auto_refresh_min_seconds: 60,
                        traffic_max_results: 500,
                        traffic_default_window: "1h".into(),
                        traffic_auto_backfill: false,
                        scope_keys: vec!["site".into()],
                        tier_key: "tier".into(),
                        ignore_keys: Vec::new(),
                        ruleset_name: "RF-{site}".into(),
                        consolidate_min_peers: 5,
                    },
                ))),
            ),
            vec![Cmd::Connect]
        );
        assert_eq!(
            m.traffic.group_keys,
            vec!["site"],
            "the new profile's label model"
        );
        assert!(!m.traffic.all_keys);
    }

    #[test]
    fn all_labels_lists_the_profile_scope_and_tier_keys_first_then_alphabetical() {
        use crate::tui::model::Overlay;
        use crate::tui::test_support::model_with_rich_flows;
        let mut m = model_with_rich_flows();
        m.set_label_model(vec!["bu".into(), "app".into()], "tier".into(), Vec::new());
        // The flows carry `bu` and `tier` in place of `loc` and `role`.
        if let Some(d) = m.traffic.data.as_mut() {
            for f in &mut d.flows {
                for l in f.src.labels.iter_mut().chain(f.dst.labels.iter_mut()) {
                    match l.key.as_str() {
                        "loc" => l.key = "bu".into(),
                        "role" => l.key = "tier".into(),
                        _ => {}
                    }
                }
            }
        }
        quiet(&mut m, key('g'));
        let Overlay::GroupKeys(p) = &m.overlay else {
            panic!("{:?}", m.overlay)
        };
        for _ in 0..p.all_row() {
            quiet(&mut m, key('j'));
        }
        quiet(&mut m, key(' '));
        quiet(&mut m, enter());
        assert!(m.traffic.all_keys);
        assert_eq!(
            m.traffic.effective_keys(),
            vec!["bu", "app", "tier", "datacenter", "env"]
        );
    }

    #[test]
    fn ignored_keys_stay_out_of_the_picker_and_all_labels_but_can_be_typed() {
        use crate::tui::model::Overlay;
        use crate::tui::test_support::model_with_rich_flows;
        let mut m = model_with_rich_flows();
        m.set_label_model(
            vec!["app".into(), "env".into()],
            "role".into(),
            vec!["datacenter".into()],
        );
        quiet(&mut m, key('g'));
        let Overlay::GroupKeys(p) = &m.overlay else {
            panic!("{:?}", m.overlay)
        };
        let listed: Vec<&str> = p.entries.iter().map(|e| e.key.as_str()).collect();
        assert_eq!(listed, vec!["app", "env", "role", "loc"]);
        for _ in 0..p.all_row() {
            quiet(&mut m, key('j'));
        }
        quiet(&mut m, key(' '));
        quiet(&mut m, enter());
        assert!(m.traffic.all_keys);
        assert_eq!(
            m.traffic.effective_keys(),
            vec!["app", "env", "role", "loc"]
        );
        // Typed by hand, an ignored key still groups.
        quiet(&mut m, key('g'));
        let Overlay::GroupKeys(p) = &m.overlay else {
            panic!("{:?}", m.overlay)
        };
        let to_text = p.text_row() - p.cursor;
        for _ in 0..to_text {
            quiet(&mut m, key('j'));
        }
        for c in "datacenter".chars() {
            quiet(&mut m, key(c));
        }
        quiet(&mut m, enter());
        assert!(!m.traffic.all_keys);
        assert_eq!(m.traffic.effective_keys(), vec!["datacenter"]);
        assert!(
            m.traffic
                .flows
                .rows()
                .iter()
                .any(|r| &*r.src_group == "fra1" || &*r.dst_group == "fra1"),
            "grouping by an ignored key still reads its values"
        );
    }

    #[test]
    fn the_flow_detail_hides_ignored_keys() {
        use crate::tui::test_support::{model_with_flows, render_to_string};
        let mut m = model_with_flows();
        m.set_label_model(vec!["app".into()], "role".into(), vec!["env".into()]);
        m.traffic.show_detail = true;
        // Stacked (80 columns): the labels line.
        let screen = render_to_string(&m, 80, 30);
        let line = screen
            .lines()
            .find(|l| l.contains("src labels"))
            .unwrap_or_else(|| panic!("{screen}"));
        assert!(line.contains("app=shop role=web"), "{line}");
        assert!(!line.contains("env="), "{line}");
        // In columns (160 columns): one label per line, no `env`.
        let screen = render_to_string(&m, 160, 50);
        assert!(screen.contains("│role  web"), "{screen}");
        assert!(screen.contains("│app   shop"), "{screen}");
        assert!(!screen.contains("env   prod"), "{screen}");
    }

    #[test]
    fn esc_keeps_the_keys_and_the_drill_and_applying_clears_the_drill() {
        use crate::tui::test_support::model_with_rich_flows;
        let mut m = model_with_rich_flows();
        quiet(&mut m, key('v'));
        update(&mut m, enter()); // crm/prod -> crm/prod
        assert_eq!(m.traffic.view, TrafficView::Flows);
        assert_eq!(m.traffic.flows.len(), 2);
        quiet(&mut m, key('g'));
        update(&mut m, key(' ')); // app off
        quiet(&mut m, kc(KeyCode::Esc));
        assert_eq!(m.traffic.group_keys, vec!["app", "env"]);
        assert!(m.traffic.drill.is_some());
        quiet(&mut m, key('g'));
        quiet(&mut m, enter());
        assert!(m.traffic.drill.is_none(), "applying drops the drill-down");
        assert_eq!(m.traffic.flows.len(), 30);
        // An empty text falls back to the profile's scope keys.
        quiet(&mut m, key('g'));
        for _ in 0..7 {
            quiet(&mut m, kc(KeyCode::Backspace));
        }
        quiet(&mut m, enter());
        assert_eq!(m.traffic.group_keys, vec!["app", "env"]);
        // Unknown keys are allowed and show `-`.
        quiet(&mut m, key('g'));
        for c in ",nope".chars() {
            quiet(&mut m, key(c));
        }
        quiet(&mut m, enter());
        assert_eq!(m.traffic.flows.rows()[0].src_group.as_ref(), "shop/prod/-");
    }

    #[test]
    fn w_on_a_blocked_row_drafts_from_the_first_flow_behind_it() {
        use crate::tui::test_support::model_with_rich_flows;
        let mut m = writes_on(model_with_rich_flows());
        m.traffic.view = TrafficView::Blocked;
        let row = m.traffic.blocked.selected_item().unwrap().clone();
        assert!(row.is_header(), "{row:?}");
        assert!(update(&mut m, key('w')).is_empty(), "no request");
        assert_eq!(m.overlay, Overlay::None, "a header holds a whole app");
        assert_eq!(
            m.status.text,
            "select a peer row (a header holds a whole app)"
        );
        quiet(&mut m, key('j'));
        let (_, peer) = m.traffic.blocked.selected_item().unwrap().peer().unwrap();
        let service = peer.service.clone();
        assert_eq!(update(&mut m, key('w')).len(), 1, "the draft rulesets");
        let Overlay::Form(f) = &m.overlay else {
            panic!("{:?}", m.overlay)
        };
        assert_eq!(f.title, "Allow rule from flow");
        assert_eq!(f.value("Services"), service);
        quiet(&mut m, kc(KeyCode::Esc));
        m.traffic.blocked.set_rows(Vec::new());
        quiet(&mut m, key('w'));
        assert_eq!(m.status.text, "no row selected");
    }

    #[test]
    fn mesh_sits_between_app_to_app_and_blocked_and_u_p_regroup_without_requests() {
        use crate::tui::test_support::model_with_rich_flows;
        let mut m = model_with_rich_flows();
        let mut seen = Vec::new();
        for _ in 0..5 {
            assert!(update(&mut m, key('v')).is_empty());
            seen.push(m.traffic.view);
        }
        assert_eq!(
            seen,
            vec![
                TrafficView::Aggregate,
                TrafficView::Mesh,
                TrafficView::Blocked,
                TrafficView::Treemap,
                TrafficView::Flows
            ]
        );
        quiet(&mut m, key('v'));
        quiet(&mut m, key('v'));
        assert_eq!(m.traffic.view, TrafficView::Mesh);
        assert_eq!(m.traffic.mesh.len(), 30);
        quiet(&mut m, key('j'));
        quiet(&mut m, key('s'));
        assert!(update(&mut m, key('u')).is_empty(), "no request");
        assert!(!m.traffic.mesh_cols.user);
        assert_eq!(m.traffic.mesh.len(), 29);
        assert_eq!(m.traffic.mesh.selected_pos(), 0, "back on the first row");
        assert_eq!(m.traffic.mesh.sort(), None, "a new table");
        assert!(update(&mut m, key('p')).is_empty());
        assert_eq!(m.traffic.mesh.len(), 28);
        assert_eq!(m.traffic.mesh.columns[0].title, "src");
        quiet(&mut m, key('u'));
        quiet(&mut m, key('p'));
        assert_eq!(m.traffic.mesh.len(), 30);
        // u and p mean nothing on the other views.
        quiet(&mut m, key('v'));
        assert!(update(&mut m, key('u')).is_empty());
        assert!(m.traffic.mesh_cols.user);
    }

    #[test]
    fn enter_on_a_mesh_row_drills_to_its_flows_and_esc_clears() {
        use crate::tui::test_support::model_with_rich_flows;
        let mut m = model_with_rich_flows();
        m.traffic.view = TrafficView::Mesh;
        quiet(&mut m, key('u'));
        quiet(&mut m, key('p'));
        let row = m
            .traffic
            .mesh
            .rows()
            .iter()
            .position(|r| r.flows == 3)
            .unwrap();
        for _ in 0..row {
            quiet(&mut m, key('j'));
        }
        assert!(update(&mut m, enter()).is_empty());
        assert_eq!(m.traffic.view, TrafficView::Flows);
        assert_eq!(
            m.traffic.flows.len(),
            3,
            "java, batch and python3 to shop-db-p"
        );
        assert!(m.traffic.flows.rows().iter().all(|r| r.port == Some(5432)));
        assert!(matches!(m.traffic.drill, Some(Drill::Mesh { .. })));
        assert_eq!(
            m.traffic.drill.as_ref().unwrap().title(),
            "shop/prod → shop/prod 5432/tcp"
        );
        assert!(update(&mut m, kc(KeyCode::Esc)).is_empty());
        assert!(m.traffic.drill.is_none());
        assert_eq!(m.traffic.flows.len(), 30);
    }

    #[test]
    fn w_on_a_mesh_row_drafts_from_its_first_blocked_flow() {
        use crate::tui::test_support::model_with_rich_flows;
        let mut m = writes_on(model_with_rich_flows());
        m.traffic.view = TrafficView::Mesh;
        assert!(update(&mut m, key('w')).is_empty());
        assert_eq!(m.status.text, "this flow is allowed; nothing to add");
        let pos = m
            .traffic
            .mesh
            .rows()
            .iter()
            .position(|r| r.blocked > 0)
            .unwrap();
        for _ in 0..pos {
            quiet(&mut m, key('j'));
        }
        assert_eq!(update(&mut m, key('w')).len(), 1, "the draft rulesets");
        let Overlay::Form(f) = &m.overlay else {
            panic!("{:?}", m.overlay)
        };
        assert_eq!(f.title, "Allow rule from flow");
    }

    /// The rich fixture on App-to-app `pairs` with `o` on.
    fn on_pairs_by_service(m: Model) -> Model {
        let mut m = m;
        m.traffic.view = TrafficView::Aggregate;
        assert!(update(&mut m, key('o')).is_empty(), "no request");
        m
    }

    #[test]
    fn o_groups_the_pairs_by_service_without_a_request_and_holds() {
        use crate::tui::test_support::model_with_rich_flows;
        let mut m = on_pairs_by_service(model_with_rich_flows());
        let t = &m.traffic;
        assert!(t.apps_state.by_service);
        let titles: Vec<&str> = t.pairs_table().columns.iter().map(|c| c.title).collect();
        assert_eq!(
            titles,
            [
                "src", "dst", "service", "flows", "conns", "bytes", "decision", "allow.c", "pot.c",
                "block.c"
            ]
        );
        let tuples = t.services.rows();
        assert!(tuples.len() > t.groups.len(), "some pair has two services");
        assert!(tuples.iter().all(|g| g.service.is_some()));
        let conns =
            |rows: &[pce::traffic::FlowGroup]| rows.iter().map(|g| g.connections).sum::<u64>();
        assert_eq!(conns(tuples), conns(t.groups.rows()));
        assert!(m.status.text.starts_with("pairs · by service"));
        assert!(
            t.groups.rows().iter().all(|g| g.service.is_none()),
            "the treemap's pairs stay"
        );

        // s and / act on the table on screen.
        quiet(&mut m, key('s'));
        assert!(m.traffic.services.sort().is_some());
        assert!(m.traffic.groups.sort().is_none());
        quiet(&mut m, key('/'));
        assert!(m.traffic.is_searching());
        for c in "5432".chars() {
            quiet(&mut m, key(c));
        }
        quiet(&mut m, enter());
        let shown = m.traffic.services.visible().len();
        assert!(
            shown > 0 && shown < m.traffic.services.rows().len(),
            "{shown}"
        );

        // Held across x and a regroup; in, out and both are already per service.
        quiet(&mut m, key('x'));
        assert!(update(&mut m, key('o')).is_empty());
        assert!(
            m.traffic.apps_state.by_service,
            "o does nothing outside pairs"
        );
        assert_eq!(m.status.text, "o: pairs only (in/out/both are per service)");
        for _ in 0..3 {
            quiet(&mut m, key('x'));
        }
        assert_eq!(m.traffic.apps_state.by, By::Pairs);
        m.traffic.rebuild_views();
        assert!(!m.traffic.services.is_empty());

        assert!(update(&mut m, key('o')).is_empty());
        assert!(!m.traffic.apps_state.by_service);
        assert!(m.traffic.services.is_empty());
        assert_eq!(
            m.traffic.pairs_table().columns[2].title,
            "flows",
            "today's pairs"
        );
    }

    #[test]
    fn enter_on_a_pair_by_service_drills_to_that_service_only() {
        use crate::tui::test_support::model_with_rich_flows;
        let mut m = on_pairs_by_service(model_with_rich_flows());
        let pos = m
            .traffic
            .services
            .rows()
            .iter()
            .position(|g| g.flows > 1)
            .unwrap();
        for _ in 0..pos {
            quiet(&mut m, key('j'));
        }
        let row = m.traffic.services.selected_item().unwrap().clone();
        let service = row.service.clone().unwrap();
        assert!(update(&mut m, enter()).is_empty(), "no request");
        assert_eq!(m.traffic.view, TrafficView::Flows);
        assert_eq!(m.traffic.flows.len() as u64, row.flows);
        assert!(
            m.traffic
                .flows
                .rows()
                .iter()
                .all(|r| r.flow.service_text() == service)
        );
        assert_eq!(
            m.traffic.drill.as_ref().unwrap().title(),
            format!("{} → {} {service}", row.src.join("/"), row.dst.join("/"))
        );
    }

    fn flows_of(rows: &[pce::traffic::FlowGroup]) -> u64 {
        rows.iter().map(|g| g.flows).sum()
    }

    #[test]
    fn d_filters_the_pairs_and_by_service_without_a_request_and_spares_the_treemap() {
        use crate::tui::test_support::{model_with_rich_flows, render_to_string};
        use pce::traffic::Decisions;
        let mut m = model_with_rich_flows();
        m.traffic.view = TrafficView::Aggregate;
        let tree = m.traffic.treemap.groups.clone();
        let tiles = m.traffic.treemap.tiles.clone();
        assert_eq!(flows_of(m.traffic.groups.rows()), 30);
        let allowed = m
            .traffic
            .groups
            .rows()
            .iter()
            .find(|g| g.blocked + g.potentially_blocked == 0)
            .expect("a pair with only allowed flows")
            .clone();

        assert!(update(&mut m, key('d')).is_empty(), "no request");
        assert_eq!(m.status.text, "decision blocked");
        let t = &m.traffic;
        assert_eq!(t.apps_state.decisions, Decisions::Blocked);
        let rows = t.groups.rows();
        assert_eq!(flows_of(rows), 14);
        assert!(rows.iter().all(|g| g.allowed == 0 && g.flows > 0));
        assert!(
            rows.iter().any(|g| g.potentially_blocked > 0),
            "pot.blocked kept"
        );
        assert!(
            !rows
                .iter()
                .any(|g| g.src == allowed.src && g.dst == allowed.dst),
            "an allowed-only pair disappears"
        );
        let out = render_to_string(&m, 100, 24);
        assert!(out.contains("pairs · blocked · keys"), "{out}");

        // By service: the same filter, held across o.
        quiet(&mut m, key('o'));
        let rows = m.traffic.services.rows();
        assert_eq!(flows_of(rows), 14);
        assert!(rows.iter().all(|g| g.allowed == 0));
        let out = render_to_string(&m, 100, 24);
        assert!(out.contains("pairs · by service · blocked · keys"), "{out}");

        assert!(update(&mut m, key('d')).is_empty(), "no request");
        assert_eq!(m.status.text, "decision blocked only");
        for rows in [m.traffic.services.rows(), m.traffic.groups.rows()] {
            assert_eq!(flows_of(rows), 7);
            assert!(
                rows.iter()
                    .all(|g| g.allowed == 0 && g.potentially_blocked == 0)
            );
        }
        let out = render_to_string(&m, 100, 24);
        assert!(out.contains("by service · blocked only"), "{out}");

        quiet(&mut m, key('d'));
        assert_eq!(flows_of(m.traffic.services.rows()), 30);
        assert_eq!(flows_of(m.traffic.groups.rows()), 30);
        let out = render_to_string(&m, 100, 24);
        assert!(out.contains("pairs · by service · keys"), "{out}");

        // The treemap kept every flow throughout, and a regroup keeps it so.
        quiet(&mut m, key('d'));
        m.traffic.rebuild_views();
        assert_eq!(flows_of(m.traffic.groups.rows()), 14);
        assert_eq!(m.traffic.treemap.groups, tree);
        assert_eq!(m.traffic.treemap.tiles, tiles);
    }

    #[test]
    fn enter_on_a_filtered_pair_drills_to_the_flows_it_counts() {
        use crate::tui::test_support::model_with_rich_flows;
        let mut m = model_with_rich_flows();
        m.traffic.view = TrafficView::Aggregate;
        let mixed: Vec<(Vec<String>, Vec<String>)> = m
            .traffic
            .groups
            .rows()
            .iter()
            .filter(|g| g.allowed > 0 && g.blocked + g.potentially_blocked > 0)
            .map(|g| (g.src.clone(), g.dst.clone()))
            .collect();
        assert!(!mixed.is_empty(), "a pair with allowed and blocked flows");
        quiet(&mut m, key('d'));
        let pos = m
            .traffic
            .groups
            .visible()
            .iter()
            .position(|&i| {
                let g = &m.traffic.groups.rows()[i];
                mixed.contains(&(g.src.clone(), g.dst.clone()))
            })
            .unwrap();
        for _ in 0..pos {
            quiet(&mut m, key('j'));
        }
        let row = m.traffic.groups.selected_item().unwrap().clone();
        assert!(update(&mut m, enter()).is_empty(), "no request");
        assert_eq!(m.traffic.flows.len() as u64, row.flows);
        assert!(
            m.traffic
                .flows
                .rows()
                .iter()
                .all(|r| r.flow.policy_decision != PolicyDecision::Allowed)
        );
        assert_eq!(
            m.traffic.drill.as_ref().unwrap().title(),
            format!("{} → {} · blocked", row.src.join("/"), row.dst.join("/"))
        );

        // By service under blocked only.
        quiet(&mut m, kc(KeyCode::Esc));
        m.traffic.view = TrafficView::Aggregate;
        quiet(&mut m, key('o'));
        quiet(&mut m, key('d'));
        let row = m.traffic.services.selected_item().unwrap().clone();
        assert!(update(&mut m, enter()).is_empty(), "no request");
        assert_eq!(m.traffic.flows.len() as u64, row.flows);
        assert!(
            m.traffic
                .flows
                .rows()
                .iter()
                .all(|r| r.flow.policy_decision == PolicyDecision::Blocked)
        );
        assert!(
            m.traffic
                .drill
                .as_ref()
                .unwrap()
                .title()
                .ends_with(" · blocked only")
        );
    }

    #[test]
    fn w_on_a_pair_by_service_drafts_under_every_decision_filter() {
        use crate::tui::test_support::model_with_rich_flows;
        let mut m = on_pairs_by_service(writes_on(model_with_rich_flows()));
        for _ in 0..3 {
            let pred = labelled_blocked(&m);
            let target = m
                .traffic
                .services
                .rows()
                .iter()
                .find(|g| pred(g))
                .unwrap()
                .clone();
            drop(pred);
            select_service_pair(&mut m, |g| *g == target);
            let (_, _, services, row) = w_form_of_row(&mut m);
            assert_eq!(Some(services), row.service);
            quiet(&mut m, key('d'));
        }
    }

    /// Moves the cursor to the first by-service row `pred` accepts.
    fn select_service_pair(m: &mut Model, pred: impl Fn(&pce::traffic::FlowGroup) -> bool) {
        m.traffic.services.home();
        let pos = (0..m.traffic.services.len())
            .find(|&i| pred(&m.traffic.services.rows()[m.traffic.services.visible()[i]]))
            .expect("a matching row");
        for _ in 0..pos {
            quiet(m, key('j'));
        }
    }

    /// A blocked row both of whose groups are labelled workloads under
    /// the keys in force (no `-` slot, no host-name fallback).
    fn labelled_blocked(m: &Model) -> impl Fn(&pce::traffic::FlowGroup) -> bool + '_ {
        let keys = m.traffic.effective_keys();
        move |g| {
            g.blocked + g.potentially_blocked > 0
                && m.traffic.loaded_flows().iter().any(|f| {
                    g.contains(f, &keys)
                        && keys.iter().all(|k| {
                            f.src.label_value(k).is_some() && f.dst.label_value(k).is_some()
                        })
                })
        }
    }

    /// `app=shop, env=prod`: a group under `keys` as actor tokens.
    fn group_tokens(keys: &[String], group: &[String]) -> String {
        keys.iter()
            .zip(group)
            .filter(|(_, v)| *v != "-")
            .map(|(k, v)| format!("{k}={v}"))
            .collect::<Vec<_>>()
            .join(", ")
    }

    /// `w` on the selected by-service row: the form's Providers,
    /// Consumers and Services, and the row.
    fn w_form_of_row(m: &mut Model) -> (String, String, String, pce::traffic::FlowGroup) {
        let row = m.traffic.services.selected_item().unwrap().clone();
        // The draft rulesets, once: none while that load is on its way.
        let loading = m.policy.loading;
        let want = if loading {
            Vec::new()
        } else {
            vec![Cmd::LoadRuleSets {
                scope: PolicyScope::Draft,
                mode: CacheMode::Use,
            }]
        };
        assert_eq!(update(m, key('w')), want);
        let Overlay::Form(f) = &m.overlay else {
            panic!("{:?} {}", m.overlay, m.status.text)
        };
        assert_eq!(f.title, "Allow rule from flow");
        let out = (
            f.value("Providers").to_string(),
            f.value("Consumers").to_string(),
            f.value("Services").to_string(),
            row,
        );
        quiet(m, kc(KeyCode::Esc));
        out
    }

    #[test]
    fn w_on_a_pair_by_service_drafts_the_rows_groups_and_service() {
        use crate::tui::test_support::model_with_rich_flows;
        let mut m = writes_on(model_with_rich_flows());
        m.traffic.view = TrafficView::Aggregate;
        quiet(&mut m, key('w'));
        assert_eq!(
            m.status.text, "select a flow in the Flows view (Enter on a pair)",
            "plain pairs hold several services"
        );
        let mut m = on_pairs_by_service(m);

        // The default keys: the row's app/env groups, not one flow's tier.
        let keys = m.traffic.effective_keys();
        let pred = labelled_blocked(&m);
        let target = m
            .traffic
            .services
            .rows()
            .iter()
            .find(|g| pred(g))
            .unwrap()
            .clone();
        drop(pred);
        select_service_pair(&mut m, |g| *g == target);
        let (providers, consumers, services, row) = w_form_of_row(&mut m);
        assert_eq!(providers, group_tokens(&keys, &row.dst));
        assert_eq!(consumers, group_tokens(&keys, &row.src));
        assert_eq!(Some(services), row.service);

        // A row whose blocked flows differ in tier still gets its groups.
        let keep = |f: &pce::traffic::ResolvedFlow| {
            row.contains(f, &keys)
                && matches!(
                    f.policy_decision,
                    PolicyDecision::Blocked | PolicyDecision::PotentiallyBlocked
                )
        };
        let mut odd = m
            .traffic
            .loaded_flows()
            .iter()
            .find(|f| keep(f))
            .unwrap()
            .clone();
        for side in [&mut odd.src, &mut odd.dst] {
            for l in side.labels.iter_mut().filter(|l| l.key == "role") {
                l.value = "batch-tier".into();
                l.href = "/orgs/5/labels/9999".into();
            }
        }
        m.traffic.data.as_mut().unwrap().flows.insert(0, odd);
        m.traffic.rebuild_views();
        select_service_pair(&mut m, |g| {
            g.src == row.src && g.dst == row.dst && g.service == row.service
        });
        let (p2, c2, _, _) = w_form_of_row(&mut m);
        assert_eq!((p2, c2), (providers, consumers), "group level, not tier");

        // A non-scope `g` key: the row's loc groups.
        m.traffic.group_keys = vec!["loc".to_string()];
        m.traffic.rebuild_views();
        let loc = vec!["loc".to_string()];
        assert_eq!(m.traffic.effective_keys(), loc);
        let pred = labelled_blocked(&m);
        let target = m
            .traffic
            .services
            .rows()
            .iter()
            .find(|g| pred(g))
            .unwrap()
            .clone();
        drop(pred);
        select_service_pair(&mut m, |g| *g == target);
        let (providers, consumers, services, row) = w_form_of_row(&mut m);
        assert!(providers.starts_with("loc="), "{providers}");
        assert_eq!(providers, group_tokens(&loc, &row.dst));
        assert_eq!(consumers, group_tokens(&loc, &row.src));
        assert_eq!(Some(services), row.service);

        // Nothing blocked behind the row: nothing to add.
        select_service_pair(&mut m, |g| g.blocked + g.potentially_blocked == 0);
        assert!(update(&mut m, key('w')).is_empty());
        assert_eq!(m.overlay, Overlay::None);
        assert_eq!(
            m.status.text,
            "every flow of this row is allowed; nothing to add"
        );
    }

    #[test]
    fn export_of_pairs_by_service_is_traffic_groups_services_csv() {
        use crate::cli::output::TableRows;
        use crate::cli::traffic::ServiceGroups;
        use crate::tui::test_support::model_with_rich_flows;
        let mut m = on_pairs_by_service(model_with_rich_flows());
        let (_, _, headers, rows) = export_cmd(&mut m, "export p.csv");
        let cli = ServiceGroups(m.traffic.services.rows().to_vec());
        assert_eq!(
            headers,
            cli.full_headers(),
            "the CLI's --services --csv header"
        );
        assert_eq!(
            headers,
            [
                "src",
                "dst",
                "service",
                "flows",
                "conns",
                "bytes",
                "allowed",
                "pot.blocked",
                "blocked",
                "allowed_conns",
                "pot_blocked_conns",
                "blocked_conns",
                "unknown_conns"
            ]
        );
        assert_eq!(rows.len(), m.traffic.services.len());
        let first = m.traffic.services.selected_item().unwrap();
        assert_eq!(rows[0][2], first.service.clone().unwrap());
        assert_eq!(rows[0][5], first.bytes.to_string(), "bytes whole");
        let mixed = m
            .traffic
            .services
            .rows()
            .iter()
            .find(|g| g.blocked > 0 && g.allowed + g.potentially_blocked > 0)
            .or_else(|| m.traffic.services.rows().iter().find(|g| g.blocked > 0))
            .unwrap();
        let line = rows
            .iter()
            .find(|r| {
                r[0] == mixed.src.join("/")
                    && r[1] == mixed.dst.join("/")
                    && Some(&r[2]) == mixed.service.as_ref()
            })
            .unwrap();
        assert_eq!(line[8], mixed.blocked.to_string(), "blocked counted");
        assert_eq!(line[6], mixed.allowed.to_string());

        assert_eq!(line[11], mixed.conns.blocked_conns.to_string());

        // On screen: the compact decision column.
        let titles: Vec<&str> = m.traffic.services.columns.iter().map(|c| c.title).collect();
        assert!(titles.contains(&"decision"));
    }

    #[test]
    fn export_of_pairs_is_traffic_groups_csv() {
        use crate::cli::output::TableRows;
        use crate::tui::test_support::model_with_rich_flows;
        let mut m = model_with_rich_flows();
        m.traffic.view = TrafficView::Aggregate;
        m.traffic.rebuild_views();
        let (_, _, headers, rows) = export_cmd(&mut m, "export p.csv");
        let shown: Vec<pce::traffic::FlowGroup> = m
            .traffic
            .groups
            .visible()
            .iter()
            .map(|&i| m.traffic.groups.rows()[i].clone())
            .collect();
        assert_eq!(headers, shown.full_headers(), "the CLI's --csv header");
        assert_eq!(rows, shown.full_rows(), "the CLI's --csv rows");
        assert_eq!(
            headers,
            [
                "src",
                "dst",
                "flows",
                "conns",
                "bytes",
                "allowed",
                "pot.blocked",
                "blocked",
                "allowed_conns",
                "pot_blocked_conns",
                "blocked_conns",
                "unknown_conns"
            ]
        );
        assert!(!rows.is_empty());
    }

    fn on_treemap(w: u16, h: u16) -> Model {
        let mut m = crate::tui::test_support::model_with_rich_flows();
        quiet(&mut m, Msg::Resize(w, h));
        for _ in 0..4 {
            quiet(&mut m, key('v'));
        }
        assert_eq!(m.traffic.view, TrafficView::Treemap);
        m
    }

    #[test]
    fn the_treemap_follows_the_terminal_size_and_v_comes_back_to_flows() {
        use pce::traffic::treemap::Rect;
        let mut m = on_treemap(80, 24);
        let tm = &m.traffic.treemap;
        assert_eq!(
            tm.area,
            Rect {
                x: 1,
                y: 9,
                w: 78,
                h: 13
            }
        );
        assert_eq!(tm.tiles.len(), 9);
        assert_eq!(tm.tiles[0].label, "ipl:Corp/-");
        assert_eq!(tm.cursor, 0);
        quiet(&mut m, Msg::Resize(160, 48));
        assert_eq!(
            m.traffic.treemap.area,
            Rect {
                x: 1,
                y: 9,
                w: 158,
                h: 37
            }
        );
        let covered: u32 = m.traffic.treemap.tiles.iter().map(|t| t.rect.area()).sum();
        assert_eq!(covered, 158 * 37);
        quiet(&mut m, key('v'));
        assert_eq!(m.traffic.view, TrafficView::Flows);
        // No flows: no tiles, and the keys still cost nothing.
        let mut empty = model_fixture();
        empty.traffic.view = TrafficView::Treemap;
        for msg in [key('l'), enter(), kc(KeyCode::Esc), key('b')] {
            quiet(&mut empty, msg);
        }
        assert!(empty.traffic.treemap.tiles.is_empty());
    }

    #[test]
    fn the_treemap_cursor_moves_towards_the_key_and_stays_at_an_edge() {
        let mut m = on_treemap(80, 24);
        let centre_x = |m: &Model| {
            let r = m.traffic.treemap.current().unwrap().rect;
            2 * u32::from(r.x) + u32::from(r.w)
        };
        let start = centre_x(&m);
        quiet(&mut m, key('l'));
        let right = centre_x(&m);
        assert!(right > start, "{start} -> {right}");
        quiet(&mut m, kc(KeyCode::Left));
        assert!(centre_x(&m) < right);
        for _ in 0..40 {
            quiet(&mut m, key('h'));
        }
        let leftmost = m.traffic.treemap.cursor;
        quiet(&mut m, key('h'));
        assert_eq!(m.traffic.treemap.cursor, leftmost, "nothing further left");
        for _ in 0..40 {
            quiet(&mut m, key('j'));
        }
        let r = m.traffic.treemap.current().unwrap().rect;
        assert_eq!(r.y + r.h, 22, "the bottom row of the area");
    }

    #[test]
    fn enter_zooms_into_a_source_then_drills_and_esc_zooms_out() {
        let mut m = on_treemap(80, 24);
        let shop = m
            .traffic
            .treemap
            .targets()
            .iter()
            .position(|t| {
                t.inner.is_none() && m.traffic.treemap.tiles[t.outer].label == "shop/prod"
            })
            .unwrap();
        m.traffic.treemap.cursor = shop;
        quiet(&mut m, enter());
        assert_eq!(
            m.traffic.treemap.zoom,
            Some(vec!["shop".to_string(), "prod".to_string()])
        );
        let labels: Vec<&str> = m
            .traffic
            .treemap
            .tiles
            .iter()
            .map(|t| t.label.as_str())
            .collect();
        assert_eq!(
            labels,
            vec!["shop/prod", "crm/prod", "api.partner.example/-"]
        );
        quiet(&mut m, kc(KeyCode::Esc));
        assert_eq!(m.traffic.treemap.zoom, None);
        assert_eq!(m.traffic.treemap.cursor, shop, "back on the source");
        quiet(&mut m, enter());
        quiet(&mut m, enter()); // the largest destination
        assert_eq!(m.traffic.view, TrafficView::Flows);
        assert_eq!(
            m.traffic.drill,
            Some(Drill::Pair(
                vec!["shop".into(), "prod".into()],
                vec!["shop".into(), "prod".into()],
                Decisions::All
            ))
        );
        assert_eq!(m.traffic.flows.len(), 4);
        // An inner tile drills straight away; an `other` tile only explains.
        let mut m = on_treemap(80, 24);
        let tm = &m.traffic.treemap;
        let crm = tm
            .targets()
            .iter()
            .position(|t| {
                t.inner.is_some()
                    && tm.tiles[t.outer].label == "ipl:Corp/-"
                    && tm.tile(*t).label == "crm/prod"
            })
            .unwrap();
        let other = tm
            .targets()
            .iter()
            .position(|t| t.inner.is_some() && tm.tile(*t).is_other())
            .unwrap();
        m.traffic.treemap.cursor = other;
        quiet(&mut m, enter());
        assert!(
            m.status.text.starts_with("other (1): too small"),
            "{}",
            m.status.text
        );
        assert_eq!(m.traffic.view, TrafficView::Treemap);
        m.traffic.treemap.cursor = crm;
        quiet(&mut m, enter());
        assert_eq!(m.traffic.view, TrafficView::Flows);
        assert_eq!(m.traffic.flows.len(), 1, "Corp to crm/prod");
    }

    #[test]
    fn b_weighs_by_bytes_or_says_there_are_none() {
        use pce::traffic::treemap::Weight;
        let mut m = on_treemap(80, 24);
        quiet(&mut m, key('b'));
        assert_eq!(m.traffic.treemap.weight, Weight::Bytes);
        assert_eq!(m.traffic.treemap.tiles[0].weight, 8_350_000);
        quiet(&mut m, key('b'));
        assert_eq!(m.traffic.treemap.weight, Weight::Connections);
        if let Some(d) = m.traffic.data.as_mut() {
            for f in &mut d.flows {
                f.bytes_in = None;
                f.bytes_out = None;
            }
        }
        assert!(m.traffic.treemap.has_bytes);
        m.traffic.rebuild_views();
        assert!(!m.traffic.treemap.has_bytes, "the meta greys `b bytes`");
        quiet(&mut m, key('b'));
        assert_eq!(m.status.text, "no byte counts in this result");
        assert_eq!(m.traffic.treemap.weight, Weight::Connections);
    }

    #[test]
    fn the_picked_keys_reach_the_mesh_and_the_treemap() {
        let mut m = on_treemap(80, 24);
        for msg in [key('g'), key('j'), key('j'), key(' '), enter()] {
            quiet(&mut m, msg);
        }
        assert_eq!(m.traffic.group_keys, vec!["app", "env", "role"]);
        assert!(m.traffic.mesh.rows().iter().all(|r| r.src.len() == 3));
        assert_eq!(m.traffic.treemap.tiles[0].label, "ipl:Corp/-/-");
        assert!(
            m.traffic
                .treemap
                .tiles
                .iter()
                .any(|t| t.label == "shop/prod/web"),
            "sources by app, env and role"
        );
    }

    #[test]
    fn the_treemap_cursor_on_the_last_tile_is_clamped_when_a_relayout_has_fewer() {
        let mut m = on_treemap(160, 48);
        let before = m.traffic.treemap.targets().len();
        m.traffic.treemap.cursor = before - 1;
        quiet(&mut m, Msg::Resize(80, 24));
        let tm = &m.traffic.treemap;
        let after = tm.targets().len();
        assert!(after < before, "{before} -> {after}");
        assert_eq!(tm.cursor, after - 1, "clamped onto the last tile");
        let cur = tm.current().expect("a valid tile");
        assert!(!tm.tile(cur).label.is_empty());
        // The same through `b`: bytes leave fewer tiles at 80x24.
        let mut m = on_treemap(80, 24);
        let conns = m.traffic.treemap.targets().len();
        m.traffic.treemap.cursor = conns - 1;
        quiet(&mut m, key('b'));
        let tm = &m.traffic.treemap;
        assert!(
            tm.targets().len() < conns,
            "{conns} -> {}",
            tm.targets().len()
        );
        assert_eq!(tm.cursor, tm.targets().len() - 1);
        assert!(tm.current().is_some_and(|c| !tm.tile(c).label.is_empty()));
    }

    #[test]
    fn a_terminal_too_small_for_the_treemap_takes_every_key_quietly() {
        use pce::traffic::treemap::Rect;
        let mut m = on_treemap(80, 24);
        quiet(&mut m, Msg::Resize(40, 10));
        assert_eq!(m.traffic.treemap.area, Rect::default());
        assert!(m.traffic.treemap.tiles.is_empty());
        assert_eq!(m.traffic.treemap.current(), None);
        for msg in [
            key('h'),
            key('j'),
            key('k'),
            key('l'),
            kc(KeyCode::Left),
            enter(),
            key('b'),
            key('b'),
            kc(KeyCode::Esc),
        ] {
            quiet(&mut m, msg);
        }
        assert_eq!(m.traffic.view, TrafficView::Treemap);
        assert!(m.traffic.treemap.tiles.is_empty());
        quiet(&mut m, Msg::Resize(80, 24));
        assert_eq!(
            m.traffic.treemap.tiles.len(),
            9,
            "back when the terminal grows"
        );
    }

    #[test]
    fn new_keys_and_a_new_result_drop_the_zoom() {
        let mut m = on_treemap(80, 24);
        quiet(&mut m, enter());
        assert!(m.traffic.treemap.zoom.is_some());
        quiet(&mut m, key('g'));
        quiet(&mut m, enter());
        assert_eq!(m.traffic.treemap.zoom, None, "applying keys zooms out");
        quiet(&mut m, enter());
        assert!(m.traffic.treemap.zoom.is_some());
        let mut keys_role = m.traffic.group_keys.clone();
        keys_role.push("role".into());
        m.traffic.group_keys = keys_role;
        m.traffic.rebuild_views();
        assert_eq!(
            m.traffic.treemap.zoom, None,
            "the zoomed source no longer exists"
        );
        // A new result drops the zoom and the drill under the user.
        quiet(&mut m, enter());
        quiet(&mut m, enter());
        assert_eq!(m.traffic.view, TrafficView::Flows);
        assert!(m.traffic.drill.is_some());
        assert!(m.traffic.treemap.zoom.is_some());
        let outcome = crate::tui::msg::QueryOutcome {
            flows: crate::tui::test_support::fixture_flows_rich(),
            truncated: false,
            incomplete: false,
            elapsed: std::time::Duration::from_secs(1),
            hit: false,
            filter: m.traffic.filter.clone(),
            bounds: None,
            hrefs: Vec::new(),
        };
        let seq = m.traffic.seq;
        quiet(
            &mut m,
            Msg::QueryDone {
                seq,
                result: Ok(outcome),
            },
        );
        assert_eq!(m.traffic.treemap.zoom, None, "a new result zooms out");
        assert_eq!(m.traffic.drill, None, "and drops the drill");
    }

    #[test]
    fn enter_in_aggregate_drills_into_the_pair_and_esc_clears() {
        use crate::tui::model::TrafficView;
        use crate::tui::test_support::model_with_flows;
        let mut m = model_with_flows();
        update(&mut m, key('v'));
        // first group is the 310-connection shop/prod -> shop/prod pair
        update(
            &mut m,
            Msg::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
        );
        assert_eq!(m.traffic.view, TrafficView::Flows);
        assert!(m.traffic.drill.is_some());
        assert_eq!(m.traffic.flows.len(), 1);
        update(
            &mut m,
            Msg::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)),
        );
        assert!(m.traffic.drill.is_none());
        assert_eq!(m.traffic.flows.len(), 4);
    }

    #[test]
    fn query_done_rebuilds_all_views() {
        use crate::tui::filter::TrafficFilter;
        use crate::tui::test_support::fixture_flows;
        let mut m = model_fixture();
        let f = TrafficFilter::defaults("1h", 50);
        update(&mut m, Msg::ApplyFilter(f.clone()));
        let outcome = crate::tui::msg::QueryOutcome {
            flows: fixture_flows(),
            truncated: false,
            incomplete: false,
            elapsed: std::time::Duration::from_secs(1),
            hit: false,
            filter: f,
            bounds: None,
            hrefs: Vec::new(),
        };
        update(
            &mut m,
            Msg::QueryDone {
                seq: 1,
                result: Ok(outcome),
            },
        );
        assert!(!m.traffic.groups.is_empty());
        assert_eq!(
            m.traffic.blocked.len(),
            3,
            "the shop/prod header and its two blocked sources"
        );
    }

    /// A cap past 10000 needs a `y` each run: `a` will not turn auto-refresh
    /// on for it, and a tick turns it off instead of running unasked.
    #[test]
    fn auto_refresh_never_runs_a_cap_that_needs_a_confirm() {
        use crate::tui::model::QueryState;
        use crate::tui::test_support::model_with_flows;
        use std::time::{Duration, Instant};
        let mut m = model_with_flows();
        m.traffic.filter.max_results = 10_000;
        assert!(update(&mut m, key(']')).is_empty());
        let cmds = update(&mut m, key('y'));
        assert!(
            matches!(cmds.as_slice(), [Cmd::RunQuery { filter, .. }] if filter.max_results == 25_000),
            "{cmds:?}"
        );
        m.traffic.query = QueryState::Idle;
        let off = "auto off: cap 25000 needs a confirm each run";
        assert!(update(&mut m, key('a')).is_empty());
        assert!(!m.traffic.auto.enabled);
        assert_eq!(m.status.text, off);
        m.traffic.auto.enabled = true; // on from before the raise
        m.status = StatusLine::info("");
        let cmds = update(
            &mut m,
            Msg::Tick(Instant::now() + Duration::from_secs(3600)),
        );
        assert_eq!(cmds, Vec::<Cmd>::new());
        assert!(!m.traffic.auto.enabled);
        assert_eq!(m.status.text, off);
    }

    #[test]
    fn auto_refresh_respects_interval_running_and_focus() {
        use crate::tui::test_support::model_with_flows;
        use std::time::{Duration, Instant};
        let mut m = model_with_flows();
        let t0 = Instant::now();
        assert!(update(&mut m, Msg::Tick(t0)).is_empty(), "off by default");
        update(&mut m, key('a'));
        assert!(m.traffic.auto.enabled);
        assert!(m.status.text.contains("60s"), "{}", m.status.text);
        // first tick after enabling: not due, last_run is the data's fetch time, 3 s ago.
        assert!(update(&mut m, Msg::Tick(t0)).is_empty());
        let later = t0 + Duration::from_secs(61);
        let cmds = update(&mut m, Msg::Tick(later));
        assert!(
            matches!(
                cmds.as_slice(),
                [Cmd::RunQuery {
                    mode: pce::CacheMode::Bypass,
                    ..
                }]
            ),
            "{cmds:?}"
        );
        // while running, no second run
        assert!(update(&mut m, Msg::Tick(later + Duration::from_secs(120))).is_empty());
        let seq = m.traffic.seq;
        update(
            &mut m,
            Msg::QueryDone {
                seq,
                result: Err("boom".into()),
            },
        );
        assert!(!m.traffic.auto.enabled, "a failed tick query disarms it");
        update(&mut m, key('a'));
        assert!(m.traffic.auto.enabled, "`a` re-arms");
        // unfocused: no run even when due
        update(&mut m, Msg::Focus(false));
        assert!(update(&mut m, Msg::Tick(later + Duration::from_secs(200))).is_empty());
        update(&mut m, Msg::Focus(true));
        assert!(!update(&mut m, Msg::Tick(later + Duration::from_secs(200))).is_empty());
        let seq = m.traffic.seq;
        update(
            &mut m,
            Msg::QueryDone {
                seq,
                result: Err("boom".into()),
            },
        );
        assert!(!m.traffic.auto.enabled);
        assert_eq!(m.status.text, "auto-refresh off: boom");
        assert!(update(&mut m, Msg::Tick(later + Duration::from_secs(1000))).is_empty());
    }

    #[test]
    fn auto_refresh_interval_never_below_profile_minimum() {
        use crate::tui::model::AutoRefresh;
        let a = AutoRefresh::new(10);
        assert_eq!(a.interval, std::time::Duration::from_secs(60));
        let b = AutoRefresh::new(300);
        assert_eq!(b.interval, std::time::Duration::from_secs(300));
    }

    #[test]
    fn auto_refresh_without_data_does_nothing() {
        use std::time::{Duration, Instant};
        let mut m = model_fixture();
        update(&mut m, key('a'));
        assert!(
            update(&mut m, Msg::Tick(Instant::now() + Duration::from_secs(600))).is_empty(),
            "nothing to refresh until a query ran"
        );
    }

    #[test]
    fn result_panel_shows_countdown_only_while_auto_is_enabled() {
        use crate::tui::test_support::{model_with_flows, render_to_string};
        let mut m = model_with_flows();
        assert!(!render_to_string(&m, 120, 30).contains("auto in"));
        update(&mut m, key('a'));
        assert!(render_to_string(&m, 120, 30).contains("auto in 5"));
    }

    #[test]
    fn auto_refresh_is_held_back_by_overlay_and_other_screens() {
        use crate::tui::test_support::model_with_flows;
        use std::time::{Duration, Instant};
        let mut m = model_with_flows();
        update(&mut m, key('a'));
        let due = Instant::now() + Duration::from_secs(600);
        m.overlay = Overlay::Help;
        assert!(update(&mut m, Msg::Tick(due)).is_empty());
        m.overlay = Overlay::None;
        m.screen = Screen::Policy;
        assert!(update(&mut m, Msg::Tick(due)).is_empty());
    }

    #[test]
    fn search_captures_keys_on_aggregate_mesh_and_blocked_views() {
        use crate::tui::test_support::model_with_flows;
        for presses in [1, 2, 3] {
            let mut m = model_with_flows();
            for _ in 0..presses {
                quiet(&mut m, key('v'));
            }
            quiet(&mut m, key('/'));
            for c in ['q', ':', '?'] {
                quiet(&mut m, key(c));
            }
            assert_eq!(m.screen, Screen::Traffic);
            assert!(!m.quit);
            assert_eq!(m.overlay, Overlay::None);
            let table_search = |m: &Model| match m.traffic.view {
                TrafficView::Aggregate => m.traffic.groups.search_text().to_string(),
                TrafficView::Mesh => m.traffic.mesh.search_text().to_string(),
                TrafficView::Blocked => m.traffic.blocked.search_text().to_string(),
                TrafficView::Flows | TrafficView::Treemap => unreachable!("v moved off Flows"),
            };
            assert_eq!(table_search(&m), "q:?");
            quiet(
                &mut m,
                Msg::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)),
            );
            assert!(!m.traffic.is_searching());
            assert_eq!(table_search(&m), "");
        }
    }

    #[test]
    fn search_captures_global_keys() {
        use crate::tui::test_support::model_with_flows;
        let mut m = model_with_flows();
        update(&mut m, key('/'));
        for c in ['1', '0', '.', 'q', ':', '?'] {
            update(&mut m, key(c));
        }
        assert_eq!(m.screen, Screen::Traffic);
        assert!(!m.quit);
        assert_eq!(m.overlay, Overlay::None);
        assert_eq!(m.traffic.flows.search_text(), "10.q:?");
        update(
            &mut m,
            Msg::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)),
        );
        assert!(!m.traffic.flows.is_searching());
        assert_eq!(m.traffic.flows.search_text(), "");
        update(&mut m, key('q'));
        assert!(m.quit);
    }

    #[test]
    fn flow_table_navigation_sort_search_and_detail_never_emit_commands() {
        use crate::tui::test_support::model_with_flows;
        let mut m = model_with_flows();
        assert_eq!(m.traffic.flows.len(), 4);
        assert!(
            update(
                &mut m,
                Msg::Key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE))
            )
            .is_empty()
        );
        assert_eq!(m.traffic.flows.selected_pos(), 1);
        assert!(update(&mut m, key('j')).is_empty());
        assert_eq!(m.traffic.flows.selected_pos(), 2);
        assert!(update(&mut m, key('k')).is_empty());
        assert!(
            update(
                &mut m,
                Msg::Key(KeyEvent::new(KeyCode::End, KeyModifiers::NONE)),
            )
            .is_empty()
        );
        assert_eq!(m.traffic.flows.selected_pos(), 3);
        assert!(update(&mut m, key('s')).is_empty());
        assert_eq!(m.traffic.flows.sort(), Some((0, false)));
        assert!(update(&mut m, key('/')).is_empty());
        assert!(m.traffic.flows.is_searching());
        for c in "nas".chars() {
            assert!(update(&mut m, key(c)).is_empty());
        }
        assert_eq!(m.traffic.flows.len(), 1);
        assert!(
            update(
                &mut m,
                Msg::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
            )
            .is_empty()
        );
        assert!(!m.traffic.flows.is_searching());
        assert!(update(&mut m, key('/')).is_empty());
        assert!(
            update(
                &mut m,
                Msg::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)),
            )
            .is_empty()
        );
        assert_eq!(m.traffic.flows.len(), 4, "Esc clears the search");
        assert!(!m.traffic.show_detail);
        assert!(
            update(
                &mut m,
                Msg::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
            )
            .is_empty()
        );
        assert!(m.traffic.show_detail);
    }

    #[test]
    fn query_done_fills_the_flow_table() {
        use crate::tui::filter::TrafficFilter;
        use crate::tui::test_support::fixture_flows;
        let mut m = model_fixture();
        let f = TrafficFilter::defaults("1h", 50);
        update(&mut m, Msg::ApplyFilter(f.clone()));
        let outcome = crate::tui::msg::QueryOutcome {
            flows: fixture_flows(),
            truncated: false,
            incomplete: false,
            elapsed: std::time::Duration::from_secs(1),
            hit: false,
            filter: f,
            bounds: None,
            hrefs: Vec::new(),
        };
        update(
            &mut m,
            Msg::QueryDone {
                seq: 1,
                result: Ok(outcome),
            },
        );
        assert_eq!(m.traffic.flows.len(), 4);
        assert_eq!(m.traffic.flows.selected_pos(), 0);
    }

    #[test]
    fn q_and_ctrl_c_quit() {
        let mut m = model_fixture();
        assert_eq!(update(&mut m, key('q')), vec![Cmd::Quit]);
        assert!(m.quit);
        let mut m = model_fixture();
        let cmds = update(
            &mut m,
            Msg::Key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL)),
        );
        assert_eq!(cmds, vec![Cmd::Quit]);
    }

    #[test]
    fn number_keys_switch_screens_policy_objects_and_workloads_load() {
        let mut m = model_fixture();
        assert_eq!(m.screen, Screen::Traffic);
        // entering Policy for the first time loads its rulesets (see the policy tests)
        assert_eq!(update(&mut m, key('2')).len(), 1);
        assert_eq!(m.screen, Screen::Policy);
        // entering Objects for the first time loads all three kinds (cache hits)
        assert_eq!(update(&mut m, key('3')).len(), 3);
        assert_eq!(m.screen, Screen::Objects);
        // entering Workloads for the first time loads the list once (a cache hit)
        assert_eq!(
            update(&mut m, key('4')),
            vec![Cmd::LoadWorkloads(pce::CacheMode::Use)]
        );
        assert_eq!(m.screen, Screen::Workloads);
        // there is no sixth screen: 6 does nothing
        assert!(update(&mut m, key('6')).is_empty());
        assert_eq!(m.screen, Screen::Workloads);
        assert!(update(&mut m, key('1')).is_empty());
        assert_eq!(m.screen, Screen::Traffic);
    }

    #[test]
    fn startup_connect_flow() {
        let mut m = Model::new_connecting(
            model_fixture().profile,
            model_fixture().theme_name.clone(),
            60,
            1000,
            "24h".into(),
            crate::tui::model::WritesState::default(),
        );
        assert!(matches!(m.connection, Connection::Connecting));
        let cmds = update(&mut m, Msg::Connected);
        assert_eq!(cmds, vec![Cmd::LoadIndex(pce::CacheMode::Use)]);
        assert_eq!(
            m.status.text,
            "connected; loading labels, label groups, IP lists, services"
        );
        let cmds = update(
            &mut m,
            Msg::ConnectFailed("authentication failed (HTTP 401)".into()),
        );
        assert!(cmds.is_empty());
        match &m.connection {
            Connection::Disconnected { error } => assert!(error.contains("401")),
            other => panic!("{other:?}"),
        }
        assert_eq!(m.status.kind, StatusKind::Error);
    }

    #[test]
    fn resize_and_focus_update_model() {
        let mut m = model_fixture();
        update(&mut m, Msg::Resize(120, 40));
        assert_eq!(m.size, (120, 40));
        update(&mut m, Msg::Focus(false));
        assert!(!m.focused);
    }

    /// Types `:text` and Enter; what the command asked for.
    fn colon(m: &mut Model, text: &str) -> Vec<Cmd> {
        let mut cmds = update(m, key(':'));
        for c in text.chars() {
            cmds.extend(update(m, key(c)));
        }
        cmds.extend(update(
            m,
            Msg::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
        ));
        cmds
    }

    /// The one `Cmd::Export` `:text` asked for: `(path, force, headers, rows)`.
    #[track_caller]
    fn export_cmd(
        m: &mut Model,
        text: &str,
    ) -> (String, bool, Vec<&'static str>, Vec<Vec<String>>) {
        let mut cmds = colon(m, text);
        assert_eq!(cmds.len(), 1, "one Cmd, nothing for the PCE: {cmds:?}");
        match cmds.remove(0) {
            Cmd::Export {
                path,
                force,
                headers,
                rows,
            } => (path, force, headers, rows),
            other => panic!("not an export: {other:?}"),
        }
    }

    #[test]
    fn export_asks_the_runtime_to_write_the_table_on_screen() {
        use crate::tui::test_support::model_with_flows;
        let mut m = model_with_flows();
        let (path, force, headers, rows) = export_cmd(&mut m, "export ~/flows.csv");
        assert_eq!((path.as_str(), force), ("~/flows.csv", false));
        assert_eq!(
            headers,
            [
                "last seen",
                "src",
                "src group",
                "dst",
                "dst group",
                "service",
                "conns",
                "decision"
            ]
        );
        assert_eq!(rows.len(), 4);
        // Every column, the hidden-below-140 `last seen` whole.
        assert_eq!(rows[0][..2], ["2026-10-10T07:31:02Z", "host-1.example"]);

        // The search applies; `!` forces.
        m.traffic.flows.start_search();
        for c in "icmp".chars() {
            m.traffic.flows.search_push(c);
        }
        m.traffic.flows.search_commit();
        let (_, force, _, rows) = export_cmd(&mut m, "export! out.csv");
        assert!(force);
        assert_eq!(rows.len(), 1);
        assert!(rows[0].contains(&"icmp".to_string()), "{rows:?}");

        // The runtime's answer lands in the status line.
        assert!(update(&mut m, Msg::Exported(Ok("wrote 1 row to out.csv".into()))).is_empty());
        assert_eq!(m.status.kind, StatusKind::Info);
        assert_eq!(m.status.text, "wrote 1 row to out.csv");
        assert!(update(&mut m, Msg::Exported(Err("out.csv exists".into()))).is_empty());
        assert_eq!(m.status.kind, StatusKind::Error);
    }

    #[test]
    fn export_of_sections_is_every_peer_as_the_cli_prints_them_folded_or_not() {
        use crate::cli::traffic::SECTION_CSV_HEADERS;
        use crate::tui::model::TrafficView;
        use crate::tui::test_support::model_with_flows;
        let mut m = model_with_flows();
        while m.traffic.view != TrafficView::Blocked {
            update(&mut m, key('v'));
        }
        let (_, _, headers, open) = export_cmd(&mut m, "export b.csv");
        assert_eq!(headers, SECTION_CSV_HEADERS);
        assert_eq!(
            open[0],
            [
                "shop/prod",
                "in",
                "ipl:Corp/-",
                "1",
                "10.1.4.9",
                "1",
                "host-1.example",
                "8443/tcp",
                "1",
                "12",
                "0",
                "2026-10-10T07:30:58Z",
                "potentially_blocked",
                "0",
                "12",
                "0",
                "0"
            ]
        );
        update(&mut m, key('z'));
        let shown = m.traffic.section_table().unwrap();
        assert!(
            (0..shown.len()).all(|i| shown.rows()[shown.visible()[i]].is_header()),
            "every section folded"
        );
        let (_, _, _, folded) = export_cmd(&mut m, "export b.csv");
        assert_eq!(folded, open, "folding hides nothing from the export");

        update(&mut m, key('v'));
        assert_eq!(m.traffic.view, TrafficView::Treemap);
        assert!(colon(&mut m, "export t.csv").is_empty());
        assert_eq!(m.status.kind, StatusKind::Error);
        assert_eq!(m.status.text, "the treemap has no table to export");
    }

    #[test]
    fn export_of_workloads_matches_workloads_csv() {
        use crate::cli::output::TableRows;
        use crate::cli::workloads::WorkloadRow;
        use crate::tui::test_support::model_with_workloads;
        let mut m = model_with_workloads();
        m.screen = Screen::Workloads;
        let (_, _, headers, rows) = export_cmd(&mut m, "export w.csv");
        let now = m.workloads.now.unwrap();
        let order = m.limits.key_order();
        let cli: Vec<WorkloadRow> = m
            .workloads
            .raw
            .iter()
            .cloned()
            .map(|w| WorkloadRow::new(w, now, &order))
            .collect();
        let (cli_headers, cli_rows) = (cli.full_headers(), cli.full_rows());
        // Both in the fetched order (the table is unsorted).
        assert_eq!(rows.len(), cli_rows.len());
        let mut shared = 0;
        for (i, h) in headers.iter().enumerate() {
            let Some(j) = cli_headers.iter().position(|c| c == h) else {
                continue;
            };
            shared += 1;
            for (row, cli_row) in rows.iter().zip(&cli_rows) {
                assert_eq!(row[i], cli_row[j], "{h} of {}", cli_row[j]);
            }
        }
        assert!(headers.contains(&"heartbeat_age_secs"), "{headers:?}");
        assert!(shared >= 10, "{headers:?}");
        let labels = headers.iter().position(|h| *h == "labels").unwrap();
        assert_eq!(rows[0][labels], "app=shop; env=prod; role=web");
    }

    #[test]
    fn colon_opens_prompt_enter_runs_command_and_errors_show_in_status() {
        use crate::tui::model::Overlay;
        let mut m = model_fixture();
        update(&mut m, key(':'));
        assert!(matches!(m.overlay, Overlay::Prompt { .. }));
        for c in "policy".chars() {
            update(&mut m, key(c));
        }
        update(
            &mut m,
            Msg::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
        );
        assert!(matches!(m.overlay, Overlay::None));
        assert_eq!(m.screen, Screen::Policy);

        update(&mut m, key(':'));
        for c in "bogus".chars() {
            update(&mut m, key(c));
        }
        update(
            &mut m,
            Msg::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
        );
        assert_eq!(m.status.kind, StatusKind::Error);
        assert!(
            m.status.text.contains("unknown command"),
            "{}",
            m.status.text
        );
        assert_eq!(m.screen, Screen::Policy, "a bad command changes nothing");

        update(&mut m, key(':'));
        update(&mut m, key('x'));
        update(
            &mut m,
            Msg::Key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE)),
        );
        update(
            &mut m,
            Msg::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)),
        );
        assert!(matches!(m.overlay, Overlay::None));
    }

    #[test]
    fn help_overlay_toggles_and_swallows_keys() {
        use crate::tui::model::Overlay;
        let mut m = model_fixture();
        update(&mut m, key('?'));
        assert!(matches!(m.overlay, Overlay::Help));
        update(&mut m, key('2'));
        assert_eq!(
            m.screen,
            Screen::Traffic,
            "keys go to the overlay, not the screen"
        );
        update(
            &mut m,
            Msg::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)),
        );
        assert!(matches!(m.overlay, Overlay::None));
        update(&mut m, key('?'));
        update(&mut m, key('?'));
        assert!(matches!(m.overlay, Overlay::None), "? toggles");
        assert!(
            update(&mut m, Msg::Command(crate::tui::commands::Command::Quit)).contains(&Cmd::Quit)
        );
    }

    #[test]
    fn f_opens_editor_enter_applies_or_confirms() {
        use crate::tui::model::{Overlay, PendingAction};
        let mut m = model_fixture();
        update(&mut m, key('f'));
        assert!(matches!(m.overlay, Overlay::FilterEditor(_)));
        // Type a source filter into field 1.
        update(
            &mut m,
            Msg::Key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE)),
        );
        for c in "app=shop".chars() {
            update(&mut m, key(c));
        }
        let msgs = update(
            &mut m,
            Msg::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
        );
        assert!(matches!(m.overlay, Overlay::None));
        assert_eq!(m.traffic.filter.src, "app=shop");
        assert!(
            matches!(msgs.as_slice(), [Cmd::RunQuery { .. }]),
            "an applied filter runs the query"
        );

        // A big window asks for confirmation before anything happens.
        update(&mut m, key('f'));
        // field 0 = window: clear and type 30d
        for _ in 0..3 {
            update(
                &mut m,
                Msg::Key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE)),
            );
        }
        for c in "30d".chars() {
            update(&mut m, key(c));
        }
        update(
            &mut m,
            Msg::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
        );
        match &m.overlay {
            Overlay::Confirm {
                reason,
                action: PendingAction::RunQuery(f),
            } => {
                assert!(reason.contains("30d"), "{reason}");
                assert_eq!(f.window, "30d");
            }
            other => panic!("{other:?}"),
        }
        update(&mut m, key('n'));
        assert!(matches!(m.overlay, Overlay::None));
        assert_eq!(m.traffic.filter.window, "24h", "declined: filter unchanged");
        update(&mut m, key('f'));
        update(
            &mut m,
            Msg::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)),
        );
        assert!(matches!(m.overlay, Overlay::None));
    }

    #[test]
    fn editor_shows_validation_error_and_stays_open() {
        use crate::tui::model::Overlay;
        let mut m = model_fixture();
        update(&mut m, key('f'));
        for _ in 0..3 {
            update(
                &mut m,
                Msg::Key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE)),
            );
        }
        for c in "soon".chars() {
            update(&mut m, key(c));
        }
        update(
            &mut m,
            Msg::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
        );
        match &m.overlay {
            Overlay::FilterEditor(e) => {
                assert!(e.error.as_deref().unwrap_or("").contains("window"))
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn filter_and_window_commands_apply_without_the_editor() {
        use crate::tui::commands::Command;
        let mut m = model_fixture();
        update(&mut m, Msg::Command(Command::Filter("env=prod".into())));
        assert_eq!(m.traffic.filter.src, "env=prod");
        update(&mut m, Msg::Command(Command::Window("2h".into())));
        assert_eq!(m.traffic.filter.window, "2h");
        update(&mut m, Msg::Command(Command::Window("never".into())));
        assert_eq!(m.status.kind, StatusKind::Error);
        assert_eq!(m.traffic.filter.window, "2h");
    }

    #[test]
    fn unicast_toggles_the_transmission_and_runs_one_query() {
        use crate::tui::commands::Command;
        use pce::models::Transmission;
        let mut m = model_fixture();
        let cmds = update(&mut m, Msg::Command(Command::Unicast));
        assert_eq!(m.traffic.filter.transmission, vec![Transmission::Unicast]);
        let runs = cmds
            .iter()
            .filter(|c| matches!(c, Cmd::RunQuery { .. }))
            .count();
        assert_eq!(runs, 1, "one query, user-caused: {cmds:?}");
        assert!(m.traffic.filter.summary().contains("transmission unicast"));
        update(&mut m, Msg::Command(Command::Unicast));
        assert!(
            m.traffic.filter.transmission.is_empty(),
            "again: every kind"
        );
    }

    fn running(m: &Model) -> bool {
        m.traffic.is_running()
    }

    #[test]
    fn apply_filter_runs_query_and_r_reruns_with_bypass() {
        use crate::tui::filter::TrafficFilter;
        let mut m = model_fixture();
        let f = TrafficFilter::defaults("1h", 50);
        let cmds = update(&mut m, Msg::ApplyFilter(f.clone()));
        assert_eq!(cmds.len(), 1);
        match &cmds[0] {
            Cmd::RunQuery {
                seq, filter, mode, ..
            } => {
                assert_eq!(*seq, 1);
                assert_eq!(filter, &f);
                assert_eq!(*mode, pce::CacheMode::Use);
            }
            other => panic!("{other:?}"),
        }
        assert!(running(&m));
        // r while running: no-op
        let cmds = update(&mut m, key('r'));
        assert!(cmds.is_empty());
        assert!(
            m.status.text.contains("already running"),
            "{}",
            m.status.text
        );
        // finish it
        let outcome = crate::tui::msg::QueryOutcome {
            flows: vec![],
            truncated: false,
            incomplete: false,
            elapsed: std::time::Duration::from_millis(5),
            hit: false,
            filter: f.clone(),
            bounds: None,
            hrefs: Vec::new(),
        };
        update(
            &mut m,
            Msg::QueryDone {
                seq: 1,
                result: Ok(outcome),
            },
        );
        assert!(!running(&m));
        assert!(m.traffic.data.is_some());
        // r now reruns with Bypass and a new seq
        let cmds = update(&mut m, key('r'));
        match &cmds[0] {
            Cmd::RunQuery { seq, mode, .. } => {
                assert_eq!(*seq, 2);
                assert_eq!(*mode, pce::CacheMode::Bypass);
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn new_filter_cancels_running_query_and_ignores_its_late_result() {
        use crate::tui::filter::TrafficFilter;
        use crate::tui::model::QueryState;
        let mut m = model_fixture();
        update(&mut m, Msg::ApplyFilter(TrafficFilter::defaults("1h", 50)));
        let first_token = match &m.traffic.query {
            QueryState::Running { cancel, .. } => cancel.clone(),
            other => panic!("{other:?}"),
        };
        let mut f2 = TrafficFilter::defaults("1h", 50);
        f2.src = "app=shop".into();
        let cmds = update(&mut m, Msg::ApplyFilter(f2));
        assert!(first_token.is_cancelled(), "old query cancelled");
        assert!(matches!(cmds[0], Cmd::RunQuery { seq: 2, .. }));
        // The old query's late error must not disturb the new one.
        update(
            &mut m,
            Msg::QueryDone {
                seq: 1,
                result: Err("cancelled".into()),
            },
        );
        assert!(running(&m));
        assert!(!matches!(m.traffic.query, QueryState::Failed(_)));
        update(
            &mut m,
            Msg::QueryDone {
                seq: 2,
                result: Err("traffic query failed: boom".into()),
            },
        );
        assert!(matches!(&m.traffic.query, QueryState::Failed(e) if e.contains("boom")));
        assert_eq!(m.status.kind, StatusKind::Error);
    }

    #[test]
    fn progress_updates_status_while_running() {
        use crate::tui::filter::TrafficFilter;
        use pce::traffic::QueryProgress;
        let mut m = model_fixture();
        update(&mut m, Msg::ApplyFilter(TrafficFilter::defaults("1h", 50)));
        update(
            &mut m,
            Msg::QueryProgress {
                seq: 1,
                progress: QueryProgress::Polling {
                    status: "working".into(),
                    elapsed: std::time::Duration::from_secs(2),
                },
            },
        );
        assert!(m.status.text.contains("working"), "{}", m.status.text);
        update(
            &mut m,
            Msg::QueryProgress {
                seq: 7,
                progress: QueryProgress::Downloading,
            },
        );
        assert!(m.status.text.contains("working"), "stale seq ignored");
    }

    #[test]
    fn truncated_result_warns() {
        use crate::tui::filter::TrafficFilter;
        let mut m = model_fixture();
        let f = TrafficFilter::defaults("1h", 50);
        update(&mut m, Msg::ApplyFilter(f.clone()));
        let outcome = crate::tui::msg::QueryOutcome {
            flows: vec![],
            truncated: true,
            incomplete: false,
            elapsed: std::time::Duration::from_secs(1),
            hit: false,
            filter: f,
            bounds: None,
            hrefs: Vec::new(),
        };
        update(
            &mut m,
            Msg::QueryDone {
                seq: 1,
                result: Ok(outcome),
            },
        );
        assert_eq!(m.status.kind, StatusKind::Warn);
        assert_eq!(m.status.text, "0 flows, capped at 50; B raises the cap");
    }

    #[test]
    fn confirm_requires_y_and_enter_is_ignored() {
        use crate::tui::model::Overlay;
        let enter = || Msg::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        let mut m = model_fixture();
        update(&mut m, key('f'));
        for _ in 0..3 {
            update(
                &mut m,
                Msg::Key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE)),
            );
        }
        for c in "30d".chars() {
            update(&mut m, key(c));
        }
        assert!(update(&mut m, enter()).is_empty());
        assert!(matches!(m.overlay, Overlay::Confirm { .. }));
        let cmds = update(&mut m, enter());
        assert!(cmds.is_empty(), "Enter must not confirm");
        assert!(matches!(m.overlay, Overlay::Confirm { .. }));
        let cmds = update(&mut m, key('y'));
        assert!(matches!(m.overlay, Overlay::None));
        assert_eq!(cmds.len(), 1);
        assert!(matches!(
            &cmds[0],
            Cmd::RunQuery {
                mode: pce::CacheMode::Use,
                ..
            }
        ));
        assert_eq!(m.traffic.filter.window, "30d");
    }

    // ---- Policy screen (Task 10) ----

    fn kc(code: KeyCode) -> Msg {
        Msg::Key(KeyEvent::new(code, KeyModifiers::NONE))
    }

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
            serde_json::from_str(include_str!("../../../../fixtures/rule_sets.json")).unwrap();
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
            serde_json::from_str(include_str!("../../../../fixtures/pending_list.json")).unwrap(),
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
            serde_json::from_str(include_str!("../../../../fixtures/rule_sets.json")).unwrap();
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
            serde_json::from_str(include_str!("../../../../fixtures/deny_rules.json")).unwrap();
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
            serde_json::from_str(include_str!("../../../../fixtures/rule_sets.json")).unwrap();
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
            serde_json::from_str(include_str!("../../../../fixtures/labels.json")).unwrap();
        let groups =
            serde_json::from_str(include_str!("../../../../fixtures/label_groups.json")).unwrap();
        let services =
            serde_json::from_str(include_str!("../../../../fixtures/services.json")).unwrap();
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
            serde_json::from_str(include_str!("../../../../fixtures/labels.json")).unwrap();
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
                serde_json::from_str(include_str!("../../../../fixtures/rule_sets.json")).unwrap();
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
                serde_json::from_str(include_str!("../../../../fixtures/rule_sets.json")).unwrap();
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
            serde_json::from_str(include_str!("../../../../fixtures/deny_rules.json")).unwrap();
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
            serde_json::from_str(include_str!("../../../../fixtures/rule_sets.json")).unwrap();
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
            serde_json::from_str(include_str!("../../../../fixtures/workloads.json")).unwrap();
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
        // o: offline only; with h too.
        update(&mut m, key('o'));
        assert_eq!(hosts(&m), ["host-2.example"]);
        update(&mut m, key('h'));
        assert_eq!(hosts(&m), ["host-2.example"]);
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
        assert_eq!(
            m.workloads.summary(),
            [1, 1, 0, 0, 0],
            "summary ignores the filter"
        );
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
        use crate::tui::test_support::{
            model_with_objects, model_with_policy, model_with_workloads,
        };
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

    fn mouse(kind: crossterm::event::MouseEventKind, column: u16, row: u16) -> Msg {
        Msg::Mouse(crossterm::event::MouseEvent {
            kind,
            column,
            row,
            modifiers: KeyModifiers::NONE,
        })
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

    fn enter() -> Msg {
        Msg::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
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
            serde_json::from_str(include_str!("../../../../fixtures/events.json")).unwrap();
        update(
            &mut m,
            Msg::EventsLoaded(Ok((
                events,
                0,
                ("2026-10-09T08:00:00Z".into(), "2026-10-10T08:00:00Z".into()),
            ))),
        );
        assert_eq!(m.events.table.len(), 4);
        assert!(update(&mut m, key('1')).is_empty());
        assert!(update(&mut m, key('5')).is_empty(), "already loaded");
        update(&mut m, key('f'));
        assert!(matches!(m.overlay, Overlay::EventsEditor { .. }));
        for c in "err".chars() {
            update(&mut m, key(c));
        }
        let cmds = update(&mut m, enter());
        assert!(
            matches!(&cmds[0], Cmd::LoadEvents { filter, .. } if filter.severity.as_deref() == Some("err"))
        );
        assert!(matches!(m.overlay, Overlay::None));
        update(&mut m, key('f'));
        update(
            &mut m,
            Msg::Key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE)),
        );
        update(
            &mut m,
            Msg::Key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE)),
        );
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
            matches!(&m.overlay, Overlay::EventsEditor { error: Some(e), .. } if e.contains("window"))
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
            Overlay::EventsEditor { error: Some(_), .. }
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

    // ---- Editing keys (Plan 4) ----

    fn writes_on(mut m: Model) -> Model {
        m.writes = crate::tui::model::WritesState::on();
        m
    }

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
            serde_json::from_str(include_str!("../../../../fixtures/ip_lists.json")).unwrap();
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
            serde_json::from_str(include_str!("../../../../fixtures/rule_sets.json")).unwrap();
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
                t.starts_with("write failed: ")
                    && t.contains("object_in_use: referenced by a rule"),
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
            serde_json::from_str(include_str!("../../../../fixtures/pending_list.json")).unwrap(),
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
            serde_json::from_str(include_str!("../../../../fixtures/labels.json")).unwrap();
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
            serde_json::from_str(include_str!("../../../../fixtures/pending_list.json")).unwrap(),
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
            serde_json::from_str(include_str!("../../../../fixtures/rule_sets.json")).unwrap();
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
                    Some(
                        r#"no ruleset named "New-App": Ctrl-R creates it scoped app=shop, env=prod"#
                    )
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
            serde_json::from_str(include_str!("../../../../fixtures/ip_lists.json")).unwrap();
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
        let wl = serde_json::from_str(include_str!("../../../../fixtures/workloads.json")).unwrap();
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
        let wl = serde_json::from_str(include_str!("../../../../fixtures/workloads.json")).unwrap();
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
        let wl = serde_json::from_str(include_str!("../../../../fixtures/workloads.json")).unwrap();
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
            serde_json::from_str(include_str!("../../../../fixtures/rule_sets.json")).unwrap();
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
}

/// The Labels tab's `f` key filter, `=` searches, `i` and `u` (issue #9).
#[cfg(test)]
mod label_filter_tests;

#[cfg(test)]
mod window_step_tests;

#[cfg(test)]
mod app_sections_tests;

#[cfg(test)]
mod backfill_tests;

#[cfg(test)]
mod filter_completion_tests;

#[cfg(test)]
mod app_tests;
#[cfg(test)]
mod focus_tests;

#[cfg(test)]
mod draft_tests;

#[cfg(test)]
mod rule_form_ruleset_tests;

#[cfg(test)]
mod batch_tests;

#[cfg(test)]
mod section_order_tests;

#[cfg(test)]
mod workload_group_tests;

#[cfg(test)]
mod ctrl_u_tests;
