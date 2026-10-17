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

mod generator;
mod marks;
#[cfg(test)]
mod marks_tests;

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
        Msg::Key(k) => on_key(model, k),
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
                    w.rebuild(&order);
                    w.loaded_at = Some(Instant::now());
                    w.auto.from_tick = false;
                    w.age_secs = age;
                    w.show_detail &= w.selected_row().is_some();
                    model.status =
                        StatusLine::info(format!("{} workloads", model.workloads.table.len()));
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

/// `Ctrl-N` (`forward`) or `Ctrl-P` in a form or the filter editor: cycles
/// an open completion, or opens one from the name index, the loaded draft
/// rulesets and the workloads the Workloads screen holds (`Ctrl-P` opens on
/// the last candidate). A shown key (`app=`) or prefix (`group:`, `wl:`)
/// completes what follows it on `Ctrl-N`. Without an index, or for a text
/// field, nothing happens. Never a request.
/// The candidates for the active field's current token, best first;
/// none without an index or for a text field. Never a request.
fn field_candidates(model: &Model, editor: &impl Completing) -> Vec<String> {
    let Some(idx) = model.index() else {
        return Vec::new();
    };
    let Some((kind, token, earlier)) = editor.completion_request() else {
        return Vec::new();
    };
    let rulesets = model.draft_rulesets().unwrap_or_default();
    let workloads = &model.workloads.raw;
    let mut candidates =
        crate::tui::complete::complete(kind, &token, &earlier, idx, rulesets, workloads);
    // A `w` form's rulesets that cover the destination first, best first.
    if kind == FieldKind::Ruleset {
        let preferred = editor.preferred();
        candidates.sort_by_key(|c| preferred.iter().position(|p| p == c).unwrap_or(usize::MAX));
    }
    candidates
}

/// `Tab` in a Ruleset field completes instead of moving on: the field holds
/// text that is no draft ruleset's exact name (any case) and some name
/// matches it. Empty or exact text, or an open completion (`Tab` then keeps
/// the shown name and moves on), leaves `Tab` to the form.
fn tab_completes(model: &Model, form: &Form) -> bool {
    let Some(field) = form.fields.get(form.active) else {
        return false;
    };
    let text = field.value.trim();
    field.kind == FieldKind::Ruleset
        && form.completion.is_none()
        && !text.is_empty()
        && !model
            .draft_rulesets()
            .is_some_and(|sets| !edit::rulesets_named(sets, text).is_empty())
        && !field_candidates(model, form).is_empty()
}

fn complete_field(model: &Model, editor: &mut impl Completing, forward: bool) {
    let dives = forward && editor.completion_dives();
    if editor.completing() && !dives {
        editor.cycle(forward);
        return;
    }
    let candidates = field_candidates(model, editor);
    if editor.start_completion(candidates) {
        if !forward {
            editor.cycle(false);
        }
    } else if dives {
        editor.cycle(true);
    }
}

/// A failed request: the status line says what failed (or that a failing
/// auto-refresh switched itself off); long or authentication errors also open
/// the error overlay unless another overlay is in use.
/// Lines of the pretty-printed event, the unit the detail overlay scrolls in.
pub fn event_detail_lines(model: &Model, event: &pce::models::Event, raw: bool) -> usize {
    if raw {
        serde_json::to_string_pretty(event)
            .map(|s| s.lines().count())
            .unwrap_or(0)
    } else {
        crate::tui::event_diff::detail_lines(event, &model.event_names()).len()
    }
}

/// A query's answer as the loaded result (its range from the bounds).
fn outcome_data(o: QueryOutcome) -> TrafficData {
    TrafficData {
        flows: o.flows,
        truncated: o.truncated,
        incomplete: o.incomplete,
        elapsed: o.elapsed,
        cache_hit: o.hit,
        fetched_at: Instant::now(),
        filter: o.filter,
        range: o.bounds.map(|(start, end)| Range { start, end }),
        hrefs: o.hrefs,
        // Only `D` loads draft decisions (`Msg::DraftDone`).
        draft: None,
    }
}

/// The status line after a result: `head` (`1234 flows`, `12 flows for
/// shop/prod`), then `queries` (` · 2 queries`, or empty) and the cap or
/// the elapsed time; `hint` follows a warning.
fn result_status(d: &TrafficData, head: &str, queries: &str, hint: &str) -> StatusLine {
    let missing = if d.incomplete { INCOMPLETE } else { "" };
    if d.truncated {
        StatusLine::warn(format!(
            "{head}{queries}, capped at {}; B raises the cap{missing}{hint}",
            d.filter.max_results
        ))
    } else if d.incomplete {
        StatusLine::warn(format!("{head}{queries}{missing}{hint}"))
    } else {
        StatusLine::info(format!(
            "{head} in {:.1}s{queries}{}",
            d.elapsed.as_secs_f64(),
            if d.cache_hit { " (cached)" } else { "" }
        ))
    }
}

fn fail(model: &mut Model, context: &str, e: &str, auto_off: bool) {
    model.status = StatusLine::error(if auto_off {
        format!("auto-refresh off: {e}")
    } else {
        format!("{context}: {e}")
    });
    if model.overlay == Overlay::None && (e.len() > 60 || e.contains("authentication failed")) {
        model.overlay = Overlay::Error(format!("{context}: {e}"));
    }
}

/// `r`: reruns the current filter past the cache (after the cost guard).
fn run_now(model: &mut Model, f: TrafficFilter) -> Vec<Cmd> {
    let cmds = start_query(model, f, CacheMode::Bypass);
    model.traffic.auto.rearm();
    cmds
}

/// A query started as the Traffic screen starts it (tests of the
/// generator's running-query guards).
#[cfg(test)]
pub(crate) fn start_query_for_tests(model: &mut Model, filter: TrafficFilter) -> Vec<Cmd> {
    start_query(model, filter, CacheMode::Use)
}

fn start_query(model: &mut Model, filter: TrafficFilter, mode: CacheMode) -> Vec<Cmd> {
    let (seq, cancel) = model.traffic.start_query(Instant::now());
    model.status = StatusLine::info("query running");
    vec![Cmd::RunQuery {
        seq,
        filter,
        mode,
        cancel,
    }]
}

fn refresh_running_status(model: &mut Model) {
    refresh_running_status_at(model, Instant::now());
}

fn refresh_running_status_at(model: &mut Model, now: Instant) {
    if let QueryState::Running {
        started,
        status,
        draft,
        ..
    } = &model.traffic.query
    {
        let secs = now.saturating_duration_since(*started).as_secs_f64();
        model.status = StatusLine::info(if *draft {
            format!("draft re-evaluation {secs:.1}s · {status} · Esc cancels")
        } else {
            format!("query running {secs:.1}s · {status}")
        });
    }
}

/// What `D` costs per query: one PUT on the query (`update_rules`), status
/// polls and one download; `fresh` adds the POST of a new run.
fn draft_cost(queries: u32, fresh: bool) -> String {
    let per = if fresh {
        "1 POST, 1 PUT, polls and 1 download"
    } else {
        "1 status check, 1 PUT, polls and 1 download"
    };
    if queries > 1 {
        format!("{per} per query, {}", queries_text(queries))
    } else {
        per.to_string()
    }
}

/// `D`: the result on screen against the draft policy (the PCE's
/// Explorer draft view; no policy changes), or back to the reported
/// decisions when they are loaded (no request). A result whose async
/// queries are known is re-evaluated without a new POST, through the cost
/// guard; one without asks first, then runs its queries again with the
/// draft as one action.
fn draft_key(model: &mut Model) -> Vec<Cmd> {
    let t = &mut model.traffic;
    if t.is_draft_running() {
        model.status = StatusLine::info("draft re-evaluation running · Esc cancels");
        return Vec::new();
    }
    if t.is_running() {
        model.status = StatusLine::info("query running: D once it is done");
        return Vec::new();
    }
    if t.clear_draft() {
        model.status = StatusLine::info("draft cleared: the reported decisions (no request)");
        return Vec::new();
    }
    let Some(DraftTarget {
        filter,
        focus,
        hrefs,
        queries,
    }) = draft_target(t)
    else {
        model.status = StatusLine::info("no result to evaluate: r runs the query first");
        return Vec::new();
    };
    match draft_ask(&filter, &hrefs, queries) {
        Some(DraftAsk::Rerun) => {
            model.overlay = Overlay::Confirm {
                reason: format!(
                    "No query of this session to re-evaluate (a cached or dropped result). Run it again with the draft: {}? (y/n)",
                    draft_cost(queries, true)
                ),
                action: PendingAction::RunDraft {
                    filter,
                    focus,
                    hrefs: Vec::new(),
                },
            };
            Vec::new()
        }
        Some(DraftAsk::Costly(reason)) => {
            model.overlay = Overlay::Confirm {
                reason: format!(
                    "{reason}. Re-evaluate against the draft policy anyway: {}? (y/n)",
                    draft_cost(queries, false)
                ),
                action: PendingAction::RunDraft {
                    filter,
                    focus,
                    hrefs,
                },
            };
            Vec::new()
        }
        None => start_draft(model, filter, focus, hrefs),
    }
}

/// What `D` evaluates: the result on screen, its focus, the hrefs of its
/// async queries and how many queries it took.
struct DraftTarget {
    filter: TrafficFilter,
    focus: Option<Focus>,
    hrefs: Vec<String>,
    queries: u32,
}

/// `D`'s target, `None` without a result on screen.
fn draft_target(t: &TrafficState) -> Option<DraftTarget> {
    let d = t.data.as_ref()?;
    Some(DraftTarget {
        filter: d.filter.clone(),
        focus: t.focus_result.as_ref().map(|r| r.focus.clone()),
        hrefs: d.hrefs.clone(),
        queries: t.focus_result.as_ref().map_or(1, |r| r.queries),
    })
}

/// Why `D` asks before it runs.
enum DraftAsk {
    /// No query of this session behind each part of the result (a cached
    /// or dropped one): the queries run again with the draft.
    Rerun,
    /// The filter's own reason (a long window, a high cap).
    Costly(String),
}

/// Whether `D` asks first on this target; `None`: it runs straight away.
fn draft_ask(filter: &TrafficFilter, hrefs: &[String], queries: u32) -> Option<DraftAsk> {
    if hrefs.is_empty() || hrefs.len() != queries as usize {
        return Some(DraftAsk::Rerun);
    }
    filter.needs_confirmation().map(DraftAsk::Costly)
}

/// After a successful write the draft decisions on screen are stale (the
/// runtime dropped the cached ones): they are dropped, and evaluated again
/// as `D` would when they were on screen or running, or the rule came from
/// the Traffic `w` form (`from_flow`). One re-evaluation per write, never
/// while a query runs or where `D` would ask first: the status then says
/// `D` re-evaluates. `waits`: a ruleset whose rule is still to come
/// (`Ctrl-R`) changes no decision; the rule's write evaluates.
fn redraft_after_write(model: &mut Model, from_flow: bool, waits: bool) -> Vec<Cmd> {
    if waits {
        return Vec::new();
    }
    // Only the result on screen counts, not the unfocused one kept aside.
    let shown = model.traffic.draft_loaded() || model.traffic.is_draft_running();
    let dropped = model.traffic.drop_draft();
    let t = &model.traffic;
    if (shown || from_flow)
        && !t.is_running()
        && let Some(target) = draft_target(t)
        && draft_ask(&target.filter, &target.hrefs, target.queries).is_none()
    {
        let write = model.status.clone();
        let cmds = start_draft(model, target.filter, target.focus, target.hrefs);
        // `Esc` cancels it on the Traffic screen only (a form opened from
        // the app overlay goes back to it).
        let run = if model.screen != Screen::Traffic || model.form_from_app {
            "draft re-evaluation running · Esc on Traffic cancels".to_string()
        } else {
            model.status.text.clone()
        };
        model.status = StatusLine {
            text: format!("{} · {run}", write.text),
            ..write
        };
        return cmds;
    }
    if dropped {
        model.status = StatusLine {
            text: format!("{} · draft cleared: D re-evaluates", model.status.text),
            ..model.status.clone()
        };
    }
    Vec::new()
}

/// Starts `D`'s run, when the result it was asked for is still the one on
/// screen (a confirm prompt may have stood in between).
fn start_draft(
    model: &mut Model,
    filter: TrafficFilter,
    focus: Option<Focus>,
    hrefs: Vec<String>,
) -> Vec<Cmd> {
    let t = &mut model.traffic;
    let on_screen = t.data.as_ref().is_some_and(|d| d.filter == filter)
        && t.focus_result.as_ref().map(|r| &r.focus) == focus.as_ref();
    if !on_screen || t.is_running() {
        model.status = StatusLine::info("the result changed: D again re-evaluates it");
        return Vec::new();
    }
    let (seq, cancel) = t.start_draft_run(Instant::now());
    model.status = StatusLine::info("draft re-evaluation running · Esc cancels");
    vec![Cmd::RunDraft {
        seq,
        filter,
        focus,
        hrefs,
        cancel,
    }]
}

/// `D`'s answer: the draft decisions into the result on screen, if it is
/// still the one they were asked for (a late answer for a changed result
/// is dropped by `seq`, and by the check here). A query the PCE dropped
/// asks to run it again; any other failure leaves the plain result.
fn draft_done(model: &mut Model, seq: u64, result: Result<DraftOutcome, DraftFailure>) -> Vec<Cmd> {
    let t = &mut model.traffic;
    if t.seq != seq {
        return Vec::new(); // cancelled, or the result changed
    }
    t.query = QueryState::Idle;
    match result {
        Ok(DraftOutcome {
            outcome,
            focus,
            age,
        }) => {
            let current = t.focus_result.as_ref().map(|r| &r.focus);
            let Some(d) = t
                .data
                .as_mut()
                .filter(|d| d.filter == outcome.filter && current == focus.as_ref())
            else {
                model.status = StatusLine::info("draft answer dropped: the result changed");
                return Vec::new();
            };
            let summary = pce::traffic::DraftSummary::of(&outcome.flows);
            if d.hrefs != outcome.hrefs {
                // The queries ran again: a new result, at its own time range.
                d.fetched_at = Instant::now();
                d.range = outcome.bounds.map(|(start, end)| Range { start, end });
            }
            d.flows = outcome.flows;
            d.hrefs = outcome.hrefs;
            d.truncated = outcome.truncated;
            d.incomplete = outcome.incomplete;
            d.cache_hit = outcome.hit;
            d.draft = Some(summary);
            t.rebuild_views();
            model.status = StatusLine::info(format!(
                "draft: {} blocked flows fixed · {} would be blocked · D clears{}",
                summary.fixed,
                summary.newly_blocked,
                if outcome.hit {
                    format!(
                        " (cached {})",
                        crate::tui::ui::widgets::bars::age_text(age).trim_end_matches(" ago")
                    )
                } else {
                    String::new()
                }
            ));
        }
        Err(DraftFailure::Gone(e)) => {
            let filter = t.data.as_ref().map(|d| d.filter.clone());
            let focus = t.focus_result.as_ref().map(|r| r.focus.clone());
            let queries = t.focus_result.as_ref().map_or(1, |r| r.queries);
            match filter {
                Some(filter) if model.overlay == Overlay::None => {
                    model.overlay = Overlay::Confirm {
                        reason: format!(
                            "The PCE no longer holds this query ({e}). Run it again with the draft: {}? (y/n)",
                            draft_cost(queries, true)
                        ),
                        action: PendingAction::RunDraft {
                            filter,
                            focus,
                            hrefs: Vec::new(),
                        },
                    };
                }
                _ => fail(model, "draft", &e, false),
            }
        }
        Err(DraftFailure::Failed(e)) => fail(model, "draft", &e, false),
    }
    Vec::new()
}

fn on_key(model: &mut Model, k: KeyEvent) -> Vec<Cmd> {
    if k.kind == KeyEventKind::Release {
        return Vec::new();
    }
    if k.code == KeyCode::Char('c') && k.modifiers == KeyModifiers::CONTROL {
        return quit(model);
    }
    match model.overlay.clone() {
        Overlay::Help => {
            help_key(model, k);
            Vec::new()
        }
        Overlay::Prompt { mut input } => match k.code {
            KeyCode::Esc => {
                model.overlay = Overlay::None;
                Vec::new()
            }
            KeyCode::Enter => {
                model.overlay = Overlay::None;
                match commands::parse(&input) {
                    Ok(cmd) => update(model, Msg::Command(cmd)),
                    Err(e) => {
                        model.status = StatusLine::error(e);
                        Vec::new()
                    }
                }
            }
            KeyCode::Backspace => {
                input.pop();
                model.overlay = Overlay::Prompt { input };
                Vec::new()
            }
            KeyCode::Char(c) => {
                input.push(c);
                model.overlay = Overlay::Prompt { input };
                Vec::new()
            }
            _ => Vec::new(),
        },
        Overlay::GroupKeys(picker) => {
            match picker.key(k) {
                PickerOutcome::Open(p) => model.overlay = Overlay::GroupKeys(p),
                PickerOutcome::Close => model.overlay = Overlay::None,
                PickerOutcome::Apply { keys, all } => {
                    model.overlay = Overlay::None;
                    apply_group_keys(model, keys, all);
                }
            }
            Vec::new()
        }
        Overlay::LabelKeys(picker) => {
            match picker.key(k) {
                PickerOutcome::Open(p) => model.overlay = Overlay::LabelKeys(p),
                PickerOutcome::Close => model.overlay = Overlay::None,
                PickerOutcome::Apply { keys, all } => {
                    model.overlay = Overlay::None;
                    apply_label_keys(model, if all { Vec::new() } else { keys });
                }
            }
            Vec::new()
        }
        Overlay::FilterEditor(mut e)
            if k.modifiers.contains(KeyModifiers::CONTROL)
                && matches!(k.code, KeyCode::Char('n' | 'p')) =>
        {
            complete_field(model, &mut e, k.code == KeyCode::Char('n'));
            model.overlay = Overlay::FilterEditor(e);
            Vec::new()
        }
        Overlay::FilterEditor(mut e) => {
            // Esc puts the typed token back; any other key keeps the shown
            // candidate and then does what it always does.
            let esc = k.code == KeyCode::Esc;
            if e.end_completion(esc) && esc {
                model.overlay = Overlay::FilterEditor(e);
                return Vec::new();
            }
            match k.code {
                KeyCode::Esc => model.overlay = Overlay::None,
                KeyCode::Tab | KeyCode::Down => {
                    e.next();
                    model.overlay = Overlay::FilterEditor(e);
                }
                KeyCode::BackTab | KeyCode::Up => {
                    e.prev();
                    model.overlay = Overlay::FilterEditor(e);
                }
                KeyCode::Backspace => {
                    e.pop();
                    model.overlay = Overlay::FilterEditor(e);
                }
                KeyCode::Char(c) => {
                    e.push(c);
                    model.overlay = Overlay::FilterEditor(e);
                }
                KeyCode::Enter => match e.to_filter() {
                    Ok(f) => {
                        model.overlay = Overlay::None;
                        return request_query(model, f);
                    }
                    Err(err) => {
                        e.error = Some(err);
                        model.overlay = Overlay::FilterEditor(e);
                    }
                },
                _ => model.overlay = Overlay::FilterEditor(e),
            }
            Vec::new()
        }
        Overlay::LabelFilter(mut e)
            if k.modifiers.contains(KeyModifiers::CONTROL)
                && matches!(k.code, KeyCode::Char('n' | 'p')) =>
        {
            complete_field(model, &mut e, k.code == KeyCode::Char('n'));
            label_editor_unknown(model, &mut e);
            model.overlay = Overlay::LabelFilter(e);
            Vec::new()
        }
        Overlay::LabelFilter(mut e) => {
            // Esc puts the typed token back first, then closes.
            let esc = k.code == KeyCode::Esc;
            if e.end_completion(esc) && esc {
                model.overlay = Overlay::LabelFilter(e);
                return Vec::new();
            }
            match k.code {
                KeyCode::Esc => model.overlay = Overlay::None,
                KeyCode::Backspace => {
                    e.pop();
                    label_editor_unknown(model, &mut e);
                    model.overlay = Overlay::LabelFilter(e);
                }
                KeyCode::Char(c) => {
                    e.push(c);
                    label_editor_unknown(model, &mut e);
                    model.overlay = Overlay::LabelFilter(e);
                }
                // A malformed filter stays in the editor; the old one holds.
                KeyCode::Enter => match apply_workload_labels(model, &e.value) {
                    Ok(()) => model.overlay = Overlay::None,
                    Err(err) => {
                        e.error = Some(err);
                        model.overlay = Overlay::LabelFilter(e);
                    }
                },
                _ => model.overlay = Overlay::LabelFilter(e),
            }
            Vec::new()
        }
        Overlay::Confirm { action, .. } => match k.code {
            KeyCode::Char('y') | KeyCode::Char('Y') => {
                model.overlay = Overlay::None;
                match action {
                    PendingAction::RunQuery(f) => update(model, Msg::ApplyFilter(f)),
                    PendingAction::RunNow(f) => run_now(model, f),
                    PendingAction::LoadLabelUsage => load_label_usage(model),
                    PendingAction::RunFocused(f, focus, mode) => {
                        start_focused(model, f, focus, mode)
                    }
                    PendingAction::RunDraft {
                        filter,
                        focus,
                        hrefs,
                    } => start_draft(model, filter, focus, hrefs),
                    PendingAction::ReloadApp => app_reload(model),
                    PendingAction::GenRequery(f, focus) => generator::gen_requery(model, f, focus),
                    PendingAction::DeleteMark(id) => marks::delete_mark(model, &id),
                }
            }
            KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
                model.overlay = Overlay::None;
                model.status = StatusLine::info("cancelled");
                if matches!(action, PendingAction::ReloadApp) {
                    model.overlay = Overlay::App;
                }
                if matches!(action, PendingAction::GenRequery(..)) {
                    model.overlay = Overlay::Generator;
                }
                if matches!(action, PendingAction::DeleteMark(_)) {
                    model.overlay = Overlay::Marks;
                }
                Vec::new()
            }
            _ => Vec::new(),
        },
        Overlay::EventsEditor {
            mut values,
            mut active,
            ..
        } => {
            let editor =
                |values: [String; 3], active: usize, error: Option<String>| Overlay::EventsEditor {
                    values,
                    active,
                    error,
                };
            match k.code {
                KeyCode::Esc => model.overlay = Overlay::None,
                KeyCode::Tab | KeyCode::Down => {
                    active = (active + 1) % values.len();
                    model.overlay = editor(values, active, None);
                }
                KeyCode::BackTab | KeyCode::Up => {
                    active = (active + values.len() - 1) % values.len();
                    model.overlay = editor(values, active, None);
                }
                KeyCode::Backspace => {
                    values[active].pop();
                    model.overlay = editor(values, active, None);
                }
                KeyCode::Char(c) => {
                    values[active].push(c);
                    model.overlay = editor(values, active, None);
                }
                KeyCode::Enter => {
                    let edit = EventsFilterEdit {
                        severity: values[0].trim().to_string(),
                        event_type: values[1].trim().to_string(),
                        window: values[2].trim().to_string(),
                    };
                    let applied = match edit.to_filter(jiff::Timestamp::now()) {
                        Ok(_) if model.events.loading => Err("events are still loading".into()),
                        other => other,
                    };
                    match applied {
                        Ok(filter) => {
                            model.overlay = Overlay::None;
                            model.events.filter = edit;
                            model.events.rebuild();
                            return request_events(model, filter, CacheMode::Use);
                        }
                        Err(err) => model.overlay = editor(values, active, Some(err)),
                    }
                }
                _ => model.overlay = editor(values, active, None),
            }
            Vec::new()
        }
        Overlay::EventDetail { event, scroll, raw } => {
            // The renderer clamps to the last page; here only the top is a hard stop.
            let last = event_detail_lines(model, &event, raw).saturating_sub(1);
            let next = match k.code {
                KeyCode::Esc | KeyCode::Enter | KeyCode::Char('q') => {
                    model.overlay = Overlay::None;
                    return Vec::new();
                }
                KeyCode::Char('?') => {
                    open_help(model);
                    return Vec::new();
                }
                // Diff ↔ raw JSON, from the top: the line counts differ.
                KeyCode::Char('J') => {
                    model.overlay = Overlay::EventDetail {
                        event,
                        scroll: 0,
                        raw: !raw,
                    };
                    return Vec::new();
                }
                // ↑/↓ and j/k step through the events behind the detail, as
                // in the table; the detail opens at the top in the same view.
                KeyCode::Down | KeyCode::Up | KeyCode::Char('j') | KeyCode::Char('k') => {
                    let t = &mut model.events.table;
                    if matches!(k.code, KeyCode::Down | KeyCode::Char('j')) {
                        t.select_next();
                    } else {
                        t.select_prev();
                    }
                    let event = t.selected_item().cloned().unwrap_or(event);
                    model.overlay = Overlay::EventDetail {
                        event,
                        scroll: 0,
                        raw,
                    };
                    return Vec::new();
                }
                KeyCode::Char('e') if k.modifiers.contains(KeyModifiers::CONTROL) => {
                    (scroll + 1).min(last)
                }
                KeyCode::Char('y') if k.modifiers.contains(KeyModifiers::CONTROL) => {
                    scroll.saturating_sub(1)
                }
                KeyCode::Char('d') if k.modifiers.contains(KeyModifiers::CONTROL) => {
                    (scroll + 10).min(last)
                }
                KeyCode::Char('u') if k.modifiers.contains(KeyModifiers::CONTROL) => {
                    scroll.saturating_sub(10)
                }
                KeyCode::PageDown | KeyCode::Char(' ') => (scroll + 10).min(last),
                KeyCode::PageUp => scroll.saturating_sub(10),
                KeyCode::Home | KeyCode::Char('g') => 0,
                KeyCode::End | KeyCode::Char('G') => last,
                _ => scroll,
            };
            model.overlay = Overlay::EventDetail {
                event,
                scroll: next,
                raw,
            };
            Vec::new()
        }
        Overlay::MarkForm(mf) => marks::form_key(model, *mf, k),
        Overlay::Marks => marks::overlay_key(model, k),
        Overlay::Form(mut form)
            if k.modifiers.contains(KeyModifiers::CONTROL)
                && matches!(k.code, KeyCode::Char('n' | 'p')) =>
        {
            complete_field(model, &mut form, k.code == KeyCode::Char('n'));
            model.overlay = Overlay::Form(form);
            Vec::new()
        }
        Overlay::Form(mut form)
            if k.code == KeyCode::Tab && k.modifiers.is_empty() && tab_completes(model, &form) =>
        {
            complete_field(model, &mut form, true);
            model.overlay = Overlay::Form(form);
            Vec::new()
        }
        Overlay::Form(mut form) => match form.on_key(k) {
            FormEvent::Continue => {
                model.overlay = Overlay::Form(form);
                Vec::new()
            }
            FormEvent::Cancel => {
                close_form(model);
                model.status = StatusLine::info("cancelled, nothing sent");
                Vec::new()
            }
            // The ruleset is planned now; the rule waits for its href. A
            // name a loaded draft ruleset has gets the rule instead.
            FormEvent::NewRuleset => match existing_ruleset(model, form.value("Ruleset"))
                .map_or_else(
                    || {
                        edit::new_ruleset_intent(
                            &form,
                            model.index().map(|i| i.as_ref()),
                            model.profile.org_id,
                            &model.limits.scope_keys,
                        )
                    },
                    Err,
                ) {
                Ok((intent, draft)) => {
                    model.policy.rule_after_ruleset = Some((draft, form));
                    model.overlay = Overlay::None;
                    model.status = StatusLine::info("planning…");
                    model.writes.in_flight = true;
                    vec![Cmd::PlanWrite(intent)]
                }
                Err(e) => {
                    form.error = Some(e);
                    model.overlay = Overlay::Form(form);
                    Vec::new()
                }
            },
            FormEvent::Submit => {
                match edit::build_intent(
                    &form,
                    model.index().map(|i| i.as_ref()),
                    model.profile.org_id,
                ) {
                    Ok(Some(intent)) => {
                        if matches!(form.intent, EditIntent::RuleFromFlow(_)) {
                            model.traffic.rule_form = Some(form);
                        }
                        model.overlay = Overlay::None;
                        model.status = StatusLine::info("planning…");
                        model.writes.in_flight = true;
                        vec![Cmd::PlanWrite(intent)]
                    }
                    Ok(None) => {
                        close_form(model);
                        model.status = StatusLine::info("no changes");
                        Vec::new()
                    }
                    Err(e) => {
                        form.error = Some(e);
                        model.overlay = Overlay::Form(form);
                        Vec::new()
                    }
                }
            }
        },
        Overlay::ConfirmWrite {
            plan,
            kind: pce::write::ObjectKind::Revert,
            scroll,
        } => revert_confirm_key(model, plan, scroll, k.code),
        Overlay::ConfirmWrite { plan, kind, scroll } => {
            let area = ratatui::layout::Rect::new(0, 0, model.size.0, model.size.1);
            let extra = crate::tui::impact::confirm_rows(&plan);
            let last = confirm_write_last_scroll(&plan, kind, area);
            let last = u16::try_from(last).unwrap_or(u16::MAX);
            match k.code {
                KeyCode::Char('y') | KeyCode::Char('Y') => {
                    if kind == pce::write::ObjectKind::Provision {
                        // The answer is the new policy version; the counts
                        // come from what is sent.
                        model.policy.provision_counts =
                            Some(counts_text(plan.request().and_then(|r| r.body.as_ref())));
                    }
                    model.overlay = Overlay::None;
                    model.status = StatusLine::info("sending …");
                    model.writes.in_flight = true;
                    vec![Cmd::ExecuteWrite { plan, kind }]
                }
                KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
                    close_form(model);
                    model.status = StatusLine::info("cancelled, nothing sent");
                    model.policy.rule_after_ruleset = None;
                    Vec::new()
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    model.overlay = Overlay::ConfirmWrite {
                        plan,
                        kind,
                        scroll: (scroll + 1).min(last),
                    };
                    Vec::new()
                }
                KeyCode::Up | KeyCode::Char('k') => {
                    model.overlay = Overlay::ConfirmWrite {
                        plan,
                        kind,
                        scroll: scroll.saturating_sub(1),
                    };
                    Vec::new()
                }
                KeyCode::Char('i') if extra > 0 => {
                    let cmds = impact_key(model);
                    model.refresh_confirm_impact();
                    cmds
                }
                _ => Vec::new(),
            }
        }
        Overlay::ConfirmBatch { batch, scroll } => {
            let area = ratatui::layout::Rect::new(0, 0, model.size.0, model.size.1);
            let last = u16::try_from(confirm_batch_last_scroll(&batch, area)).unwrap_or(u16::MAX);
            match k.code {
                KeyCode::Char('y') | KeyCode::Char('Y') => {
                    model.overlay = Overlay::None;
                    let n = batch.sends();
                    model.status = StatusLine::info(if n == 1 {
                        "sending 1 request …".to_string()
                    } else {
                        format!("sending {n} requests …")
                    });
                    model.writes.in_flight = true;
                    model.batch_sending = true;
                    vec![Cmd::ExecuteBatch(batch)]
                }
                KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
                    let cmds = batch_closed(model);
                    model.status = StatusLine::info("cancelled, nothing sent");
                    cmds
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    model.overlay = Overlay::ConfirmBatch {
                        batch,
                        scroll: (scroll + 1).min(last),
                    };
                    Vec::new()
                }
                KeyCode::Up | KeyCode::Char('k') => {
                    model.overlay = Overlay::ConfirmBatch {
                        batch,
                        scroll: scroll.saturating_sub(1),
                    };
                    Vec::new()
                }
                _ => Vec::new(),
            }
        }
        Overlay::BatchReport { scroll } => {
            let area = ratatui::layout::Rect::new(0, 0, model.size.0, model.size.1);
            let lines = model.batch_report.clone().unwrap_or_default();
            let last = u16::try_from(batch_report_last_scroll(&lines, area)).unwrap_or(u16::MAX);
            match k.code {
                KeyCode::Esc | KeyCode::Enter | KeyCode::Char('q') => batch_closed(model),
                KeyCode::Down | KeyCode::Char('j') => {
                    model.overlay = Overlay::BatchReport {
                        scroll: (scroll + 1).min(last),
                    };
                    Vec::new()
                }
                KeyCode::Up | KeyCode::Char('k') => {
                    model.overlay = Overlay::BatchReport {
                        scroll: scroll.saturating_sub(1),
                    };
                    Vec::new()
                }
                _ => Vec::new(),
            }
        }
        Overlay::Pending => {
            let n = model.policy.pending.as_ref().map_or(0, Vec::len);
            match k.code {
                KeyCode::Esc | KeyCode::Char('q') => model.overlay = Overlay::None,
                KeyCode::Down | KeyCode::Char('j') => {
                    model.policy.pending_selected =
                        (model.policy.pending_selected + 1).min(n.saturating_sub(1));
                }
                KeyCode::Up | KeyCode::Char('k') => {
                    model.policy.pending_selected = model.policy.pending_selected.saturating_sub(1);
                }
                KeyCode::Char(' ') => {
                    let p = &mut model.policy;
                    if let Some(href) = p
                        .pending
                        .as_ref()
                        .and_then(|l| l.get(p.pending_selected))
                        .map(|c| c.href.clone())
                        && !p.pending_marked.remove(&href)
                    {
                        p.pending_marked.insert(href);
                    }
                }
                KeyCode::Char('P') => return provision_key(model),
                KeyCode::Char('R') => return revert_key(model),
                KeyCode::Enter => return pending_diff(model),
                KeyCode::Char('?') => open_help(model),
                _ => {}
            }
            Vec::new()
        }
        Overlay::Diff { diff, scroll } => {
            let last = u16::try_from(diff.lines.len().saturating_sub(1)).unwrap_or(u16::MAX);
            match k.code {
                KeyCode::Esc => model.overlay = Overlay::Pending,
                KeyCode::Char('q') => model.overlay = Overlay::None,
                KeyCode::Down | KeyCode::Char('j') => {
                    model.overlay = Overlay::Diff {
                        diff,
                        scroll: (scroll + 1).min(last),
                    };
                }
                KeyCode::Up | KeyCode::Char('k') => {
                    model.overlay = Overlay::Diff {
                        diff,
                        scroll: scroll.saturating_sub(1),
                    };
                }
                _ => {}
            }
            Vec::new()
        }
        Overlay::RuleDetail(_) => match k.code {
            KeyCode::Esc | KeyCode::Enter | KeyCode::Char('q') => {
                model.overlay = Overlay::None;
                Vec::new()
            }
            KeyCode::Char('i') => impact_key(model),
            _ => Vec::new(),
        },
        Overlay::Error(_) => {
            if matches!(k.code, KeyCode::Esc | KeyCode::Enter | KeyCode::Char('q')) {
                model.overlay = Overlay::None;
            }
            Vec::new()
        }
        Overlay::AppPicker(picker) => match picker.key(k) {
            AppPickerOutcome::Open(p) => {
                model.overlay = Overlay::AppPicker(p);
                Vec::new()
            }
            AppPickerOutcome::Close => {
                model.overlay = Overlay::None;
                Vec::new()
            }
            AppPickerOutcome::Pick { app, purpose } => {
                model.overlay = Overlay::None;
                match purpose {
                    AppPickerPurpose::Focus => {
                        focus_by_name(model, &app);
                        Vec::new()
                    }
                    AppPickerPurpose::Overlay => open_app_by_name(model, &app),
                    AppPickerPurpose::Generator => gen_by_name(model, &app),
                }
            }
        },
        Overlay::App => app_overlay_key(model, k),
        Overlay::Generator => generator::gen_key(model, k),
        Overlay::None => on_screen_key(model, k),
    }
}

fn on_screen_key(model: &mut Model, k: KeyEvent) -> Vec<Cmd> {
    // An active table search captures every key except Ctrl-C.
    let ctrl_c = k.code == KeyCode::Char('c') && k.modifiers.contains(KeyModifiers::CONTROL);
    if !ctrl_c {
        if model.screen == Screen::Traffic && model.traffic.is_searching() {
            return traffic_key(model, k);
        }
        if model.screen == Screen::Policy && model.policy.is_searching() {
            return policy_key(model, k);
        }
        if model.screen == Screen::Objects && model.objects.is_searching() {
            return objects_key(model, k);
        }
        if model.screen == Screen::Workloads && model.workloads.table.is_searching() {
            return workloads_key(model, k);
        }
        if model.screen == Screen::Events && model.events.table.is_searching() {
            return events_key(model, k);
        }
    }
    if k.code == KeyCode::Char('r')
        && k.modifiers == KeyModifiers::NONE
        && matches!(model.connection, Connection::Disconnected { .. })
    {
        return reconnect(model);
    }
    match (k.code, k.modifiers) {
        (KeyCode::Char('q'), KeyModifiers::NONE) => quit(model),
        (KeyCode::Char(':'), _) => {
            model.overlay = Overlay::Prompt {
                input: String::new(),
            };
            Vec::new()
        }
        (KeyCode::Char('?'), _) => {
            open_help(model);
            Vec::new()
        }
        (KeyCode::Char('1'), _) => switch(model, Screen::Traffic),
        (KeyCode::Char('2'), _) => switch(model, Screen::Policy),
        (KeyCode::Char('3'), _) => switch(model, Screen::Objects),
        (KeyCode::Char('4'), _) => switch(model, Screen::Workloads),
        (KeyCode::Char('5'), _) => switch(model, Screen::Events),
        _ if model.screen == Screen::Traffic => traffic_key(model, k),
        _ if model.screen == Screen::Policy => policy_key(model, k),
        _ if model.screen == Screen::Objects => objects_key(model, k),
        _ if model.screen == Screen::Workloads => workloads_key(model, k),
        _ if model.screen == Screen::Events => events_key(model, k),
        _ => Vec::new(),
    }
}

/// `r` while disconnected: one user-caused reconnect attempt. Moving to
/// `Connecting` is what keeps a second `r` from sending another.
fn reconnect(model: &mut Model) -> Vec<Cmd> {
    model.connection = Connection::Connecting;
    model.objects.index_failed = false;
    model.workloads.index_failed = false;
    model.status = StatusLine::info("reconnecting…");
    vec![Cmd::Connect]
}

fn traffic_page(model: &Model) -> usize {
    let area = ratatui::layout::Rect::new(0, 0, model.size.0, model.size.1);
    crate::tui::ui::layout::chrome(area)
        .map(|c| {
            use crate::tui::ui::screens::traffic::{detail_extra, table_rows};
            table_rows(c.body, detail_extra(&model.traffic))
        })
        .unwrap_or(10)
        .max(1)
}

/// Search editing keys, shared by every view's table.
fn search_key<T>(table: &mut crate::tui::table::TableView<T>, k: KeyEvent) {
    match k.code {
        KeyCode::Esc => table.search_clear(),
        KeyCode::Enter => table.search_commit(),
        KeyCode::Backspace => table.search_pop(),
        KeyCode::Char(c) => table.search_push(c),
        _ => {}
    }
}

/// Navigation, sort and search-start keys, shared by every view's table.
fn table_key<T>(table: &mut crate::tui::table::TableView<T>, k: KeyEvent, page: usize) {
    match k.code {
        KeyCode::Down | KeyCode::Char('j') => table.select_next(),
        KeyCode::Up | KeyCode::Char('k') => table.select_prev(),
        KeyCode::PageDown => table.page_down(page),
        KeyCode::PageUp => table.page_up(page),
        KeyCode::Home => table.home(),
        KeyCode::End => table.end(),
        KeyCode::Char('s') => table.sort_next(),
        KeyCode::Char('/') => table.start_search(),
        _ => {}
    }
}

fn traffic_key(model: &mut Model, k: KeyEvent) -> Vec<Cmd> {
    let page = traffic_page(model);
    if model.traffic.is_searching() {
        let t = &mut model.traffic;
        match t.view {
            TrafficView::Flows if t.flows_grouped() => {
                search_key(&mut t.flows_grouped, k);
                t.flows_grouped.ensure_visible(page);
            }
            TrafficView::Flows => {
                search_key(&mut t.flows, k);
                t.flows.ensure_visible(page);
            }
            TrafficView::Aggregate if t.apps_state.by == By::Pairs => {
                let table = t.pairs_table_mut();
                search_key(table, k);
                table.ensure_visible(page);
            }
            TrafficView::Aggregate => {
                search_key(&mut t.apps, k);
                t.apps.ensure_visible(page);
            }
            TrafficView::Mesh => {
                search_key(&mut t.mesh, k);
                t.mesh.ensure_visible(page);
            }
            TrafficView::Blocked => {
                search_key(&mut t.blocked, k);
                t.blocked.ensure_visible(page);
            }
            // `is_searching` is false on the treemap: it has no table.
            TrafficView::Treemap => {}
        }
        return Vec::new();
    }
    // Esc cancels a draft re-evaluation in flight first: the plain result stays.
    if k.code == KeyCode::Esc && model.traffic.cancel_draft_run() {
        model.status = StatusLine::info("draft re-evaluation cancelled");
        return Vec::new();
    }
    // With a focus set, Esc clears it first; today's Esc applies after.
    if k.code == KeyCode::Esc && model.traffic.focus.is_some() {
        model.traffic.clear_focus();
        model.status = StatusLine::info("focus cleared");
        return Vec::new();
    }
    // A key that changes the result goes first: a draft run for the old
    // one is cancelled (Esc above cancels it alone).
    if matches!(k.code, KeyCode::Char('+' | '-' | ']' | '[' | 'B' | 'r')) {
        model.traffic.cancel_draft_run();
    }
    // With a focused result on screen the cap keys re-run the focused
    // query; the window keys (and the cap keys otherwise) work on the
    // unfocused result.
    if let KeyCode::Char(c @ (']' | '[' | 'B')) = k.code
        && let Some(cmds) = focused_cap_key(model, c)
    {
        return cmds;
    }
    if matches!(k.code, KeyCode::Char('+' | '-' | ']' | '[' | 'B')) {
        model.traffic.back_to_unfocused();
    }
    match k.code {
        KeyCode::Char('f') => {
            model.overlay = Overlay::FilterEditor(FilterEditor::from_filter(&model.traffic.filter));
            return Vec::new();
        }
        KeyCode::Char('F') => return focus_key(model),
        KeyCode::Char('A') => return app_open_key(model),
        KeyCode::Char('X') if model.traffic.focus.is_some() => {
            model.traffic.cycle_focus_direction();
            model.status = focus_status(&model.traffic);
        }
        KeyCode::Char('r') => return update(model, Msg::RunNow),
        KeyCode::Char('D') => return draft_key(model),
        KeyCode::Char('+') => return widen_window(model),
        KeyCode::Char('B') => return backfill_key(model),
        KeyCode::Char('-') => return narrow_window(model),
        KeyCode::Char(c @ (']' | '[')) => return step_cap(model, c == ']'),
        KeyCode::Char('v') => {
            model.traffic.view = model.traffic.view.next();
            // The Flows sections are built only while they are on screen.
            model.traffic.sync_flows_grouped();
        }
        KeyCode::Char('a') if !model.traffic.auto.enabled => {
            model.status = auto_off_status(&model.traffic.filter)
                .unwrap_or_else(|| model.traffic.auto.toggle());
        }
        KeyCode::Char('a') => {
            model.status = model.traffic.auto.toggle();
        }
        KeyCode::Char('w') => return flow_rule_key(model),
        // Squelched flows shown or left out: a rebuild, no request.
        KeyCode::Char('M') => model.status = model.traffic.toggle_squelched(),
        // The mark form for the row: marks.toml only, no request.
        KeyCode::Char('m') => return marks::mark_key(model),
        KeyCode::Char('g') => {
            model.overlay = Overlay::GroupKeys(open_key_picker(model));
            return Vec::new();
        }
        KeyCode::Char(c @ ('u' | 'p')) if model.traffic.view == TrafficView::Mesh => {
            model.traffic.toggle_mesh_column(c == 'u');
        }
        KeyCode::Char('x' | 'X' | 'd') if model.traffic.view == TrafficView::Blocked => {
            model.status = StatusLine::info(
                "Blocked is App-to-app with in · blocked; press v until App-to-app",
            );
        }
        KeyCode::Char(c @ ('x' | 'X')) if model.traffic.view == TrafficView::Aggregate => {
            let t = &mut model.traffic;
            t.apps_state.by = if c == 'x' {
                t.apps_state.by.next()
            } else {
                t.apps_state.by.prev()
            };
            t.apps.home();
            t.rebuild_apps();
        }
        KeyCode::Char(c @ ('x' | 'X')) if model.traffic.view == TrafficView::Flows => {
            let t = &mut model.traffic;
            // The cursor stays on its flow (a header: that app's first flow).
            let keep = t.selected_flow().map(|r| r.flow.clone()).or_else(|| {
                let r = t.flows_grouped.selected_item()?;
                let rows = t.flows_grouped.rows();
                let at = rows.iter().position(|x| std::ptr::eq(x, r))?;
                rows[at..]
                    .iter()
                    .find_map(|x| x.flow().map(|f| f.flow.clone()))
            });
            t.flows_state.by = if c == 'x' {
                t.flows_state.by.next()
            } else {
                t.flows_state.by.prev()
            };
            t.rebuild_flows_grouped();
            let same = |f: &crate::tui::model::FlowRow| {
                keep.as_ref()
                    .is_some_and(|k| std::sync::Arc::ptr_eq(k, &f.flow))
            };
            if t.flows_grouped() {
                t.flows_grouped.home();
                t.flows_grouped.select_where(|r| r.flow().is_some_and(same));
            } else {
                t.flows.select_where(|r| same(r));
            }
            model.status = StatusLine::info(t.flows_state.by.label());
        }
        KeyCode::Char('o') if model.traffic.view == TrafficView::Aggregate => {
            let t = &mut model.traffic;
            if t.apps_state.by != By::Pairs {
                model.status = StatusLine::info("o: pairs only (in/out/both are per service)");
            } else {
                t.apps_state.by_service = !t.apps_state.by_service;
                t.rebuild_pairs();
                model.status = StatusLine::info(if t.apps_state.by_service {
                    "pairs · by service: one row per src group, dst group and port/proto"
                } else {
                    "pairs"
                });
            }
        }
        KeyCode::Char('d') if model.traffic.view == TrafficView::Aggregate => {
            let t = &mut model.traffic;
            t.apps_state.decisions = t.apps_state.decisions.next(t.draft_loaded());
            t.rebuild_pairs();
            t.rebuild_apps();
            model.status = StatusLine::info(format!("decision {}", t.apps_state.decisions.label()));
        }
        KeyCode::Char(' ') if model.traffic.section_table().is_some() => {
            model.traffic.toggle_fold();
        }
        KeyCode::Char('z') if model.traffic.section_table().is_some() => {
            model.traffic.toggle_fold_all();
        }
        KeyCode::Char('s') if model.traffic.section_table().is_some() => {
            model.status = match model.traffic.cycle_section_order() {
                // The order alone, so it fits beside the key hints at 80.
                Some(order) => StatusLine::info(order.label()),
                None => StatusLine::info("sections keep their order (by connections)"),
            };
        }
        KeyCode::Enter if model.traffic.section_table().is_some() => {
            let t = &mut model.traffic;
            match t.section_drill() {
                // A Flows header under a drill narrows it, never widens.
                Some(header) if t.view == TrafficView::Flows && t.drill.is_some() => {
                    let old = t.drill.clone().unwrap_or(header.clone());
                    match narrow_drill(&old, &header) {
                        Some(d) if d == old => {
                            model.status =
                                StatusLine::info("the drill holds only this app already");
                        }
                        Some(d) => {
                            t.drill = Some(d);
                            t.rebuild_views();
                            t.flows_grouped.home();
                        }
                        None => {
                            t.toggle_fold();
                            model.status = StatusLine::info("drill active: Esc first");
                        }
                    }
                }
                Some(drill) => {
                    t.drill = Some(drill);
                    t.view = TrafficView::Flows;
                    t.rebuild_views();
                    // By app, the drilled app's section from its header.
                    t.flows_grouped.home();
                }
                // A flow under a Flows section: its detail, as when flat.
                None if t.view == TrafficView::Flows => {
                    t.show_detail = !t.show_detail;
                }
                None => {}
            }
        }
        KeyCode::Enter => match model.traffic.view {
            TrafficView::Aggregate => {
                let t = &mut model.traffic;
                if let Some(g) = t.pairs_table().selected_item() {
                    let (src, dst) = (g.src.clone(), g.dst.clone());
                    let decisions = t.apps_state.decisions;
                    t.drill = Some(match &g.service {
                        Some(service) => Drill::Service(src, dst, service.clone(), decisions),
                        None => Drill::Pair(src, dst, decisions),
                    });
                    t.view = TrafficView::Flows;
                    t.rebuild_views();
                }
            }
            TrafficView::Mesh => {
                let t = &mut model.traffic;
                if let Some(r) = t.mesh.selected_item() {
                    t.drill = Some(Drill::Mesh {
                        key: r.key(),
                        cols: t.mesh_cols,
                    });
                    t.view = TrafficView::Flows;
                    t.rebuild_views();
                }
            }
            TrafficView::Flows => {
                model.traffic.show_detail = !model.traffic.show_detail;
                // The detail's `mark` line takes a table row: keep the
                // cursor on screen.
                let page = traffic_page(model);
                model.traffic.flows.ensure_visible(page);
            }
            TrafficView::Blocked => {}
            TrafficView::Treemap => treemap_enter(model),
        },
        KeyCode::Esc | KeyCode::Backspace
            if model.traffic.view == TrafficView::Flows && model.traffic.drill.is_some() =>
        {
            model.traffic.drill = None;
            model.traffic.rebuild_views();
        }
        KeyCode::Esc if model.traffic.view == TrafficView::Treemap => treemap_out(model),
        KeyCode::Char('b') if model.traffic.view == TrafficView::Treemap => {
            let t = &mut model.traffic;
            if !t.treemap.has_bytes {
                model.status = StatusLine::info("no byte counts in this result");
            } else {
                t.treemap.weight = match t.treemap.weight {
                    Weight::Connections => Weight::Bytes,
                    Weight::Bytes => Weight::Connections,
                };
                t.relayout_treemap();
            }
        }
        _ => {
            let t = &mut model.traffic;
            match t.view {
                TrafficView::Flows if t.flows_grouped() => table_key(&mut t.flows_grouped, k, page),
                TrafficView::Flows => table_key(&mut t.flows, k, page),
                TrafficView::Aggregate if t.apps_state.by == By::Pairs => {
                    table_key(t.pairs_table_mut(), k, page)
                }
                TrafficView::Aggregate => table_key(&mut t.apps, k, page),
                TrafficView::Mesh => table_key(&mut t.mesh, k, page),
                TrafficView::Blocked => table_key(&mut t.blocked, k, page),
                TrafficView::Treemap => {
                    let dir = match k.code {
                        KeyCode::Left | KeyCode::Char('h') => Some(Dir::Left),
                        KeyCode::Right | KeyCode::Char('l') => Some(Dir::Right),
                        KeyCode::Up | KeyCode::Char('k') => Some(Dir::Up),
                        KeyCode::Down | KeyCode::Char('j') => Some(Dir::Down),
                        _ => None,
                    };
                    if let Some(d) = dir {
                        t.treemap.move_cursor(d);
                    }
                }
            }
        }
    }
    // The page again: a key may have opened or closed the detail's mark
    // line, which takes a table row.
    let page = traffic_page(model);
    let t = &mut model.traffic;
    match t.view {
        TrafficView::Flows if t.flows_grouped() => t.flows_grouped.ensure_visible(page),
        TrafficView::Flows => t.flows.ensure_visible(page),
        TrafficView::Aggregate if t.apps_state.by == By::Pairs => {
            t.pairs_table_mut().ensure_visible(page)
        }
        TrafficView::Aggregate => t.apps.ensure_visible(page),
        TrafficView::Mesh => t.mesh.ensure_visible(page),
        TrafficView::Blocked => t.blocked.ensure_visible(page),
        TrafficView::Treemap => {}
    }
    Vec::new()
}

/// A Flows header's drill (`Drill::App` on one side) within the drill in
/// force: the two combined when one drill can say it (an app on the other
/// side makes a pair; a drill holding one app is unchanged), `None` when
/// it cannot (an app on either side).
fn narrow_drill(old: &Drill, header: &Drill) -> Option<Drill> {
    let Drill::App {
        app,
        direction: Some(dir),
        ..
    } = header
    else {
        return None;
    };
    match old {
        Drill::App {
            app: a,
            direction: Some(d),
            decisions,
        } if d != dir => Some(match d {
            // `* → a` by source app `app`: `app → a`.
            pce::traffic::Direction::In => Drill::Pair(app.clone(), a.clone(), *decisions),
            pce::traffic::Direction::Out => Drill::Pair(a.clone(), app.clone(), *decisions),
        }),
        Drill::App {
            direction: None, ..
        } => None,
        // The same side, a pair, a service, a peer or a Mesh row: every
        // flow already has one app on each side.
        _ => Some(old.clone()),
    }
}

/// Enter on the Treemap: an outer tile zooms into its source; a
/// destination (or any tile while zoomed) opens the Flows view drilled to
/// that pair. An `other (N)` tile only explains itself.
fn treemap_enter(model: &mut Model) {
    let t = &mut model.traffic;
    let Some(target) = t.treemap.current() else {
        return;
    };
    let tile = t.treemap.tile(target).clone();
    if tile.is_other() {
        model.status = StatusLine::info(format!(
            "{}: too small to draw one by one; zoom in or widen the terminal",
            tile.label
        ));
        return;
    }
    let groups = &t.treemap.groups;
    let src_text = match (&t.treemap.zoom, target.inner) {
        (None, None) => {
            let src = groups
                .iter()
                .find(|g| g.src.join("/") == tile.label)
                .map(|g| g.src.clone());
            if let Some(src) = src {
                t.treemap.zoom = Some(src);
                t.treemap.cursor = 0;
                t.relayout_treemap();
            }
            return;
        }
        (Some(z), _) => z.join("/"),
        (None, Some(_)) => t.treemap.tiles[target.outer].label.clone(),
    };
    let pair = groups
        .iter()
        .find(|g| g.src.join("/") == src_text && g.dst.join("/") == tile.label)
        .map(|g| (g.src.clone(), g.dst.clone()));
    if let Some((src, dst)) = pair {
        t.drill = Some(Drill::Pair(src, dst, Decisions::All));
        t.view = TrafficView::Flows;
        t.rebuild_views();
    }
}

/// Esc on the Treemap: out of a zoom, the cursor back on that source.
fn treemap_out(model: &mut Model) {
    let tm = &mut model.traffic.treemap;
    let Some(z) = tm.zoom.take() else {
        return;
    };
    model.traffic.relayout_treemap();
    let tm = &mut model.traffic.treemap;
    let label = z.join("/");
    if let Some(i) = tm
        .targets()
        .iter()
        .position(|t| t.inner.is_none() && tm.tiles[t.outer].label == label)
    {
        tm.cursor = i;
    }
}

/// The `g` picker over the profile's label model and the keys the loaded
/// flows carry.
fn open_key_picker(model: &Model) -> KeyPicker {
    let t = &model.traffic;
    let seen = pce::traffic::keys_seen(t.loaded_flows(), &t.key_order);
    KeyPicker::new(
        &model.limits.scope_keys,
        &model.limits.tier_key,
        &seen,
        &t.group_keys,
        t.all_keys,
    )
}

/// Enter in the picker: the keys (the profile's scope keys when none),
/// every view rebuilt, the drill-down dropped, the focus cleared (its app
/// is not defined under other keys). No request.
fn apply_group_keys(model: &mut Model, keys: Vec<String>, all: bool) {
    let t = &mut model.traffic;
    t.all_keys = all;
    if !all {
        t.group_keys = if keys.is_empty() {
            pce::traffic::keys_or_default(&model.limits.scope_keys)
        } else {
            keys
        };
    }
    let cleared = t.drop_focus();
    t.drill = None;
    t.treemap.zoom = None;
    t.rebuild_views();
    let mut text = format!("grouped by {}", t.effective_keys().join(","));
    let keys = model.workload_group_keys();
    model.workloads.set_group_keys(keys);
    if cleared {
        text.push_str(" · focus cleared: the app is not defined under the new keys");
    }
    model.status = StatusLine::info(text);
}

/// `F`: focus on the selected row's app (a second `F` takes its other
/// side); with no app under the cursor, clear the focus, or open the picker
/// when there is none. Never requests.
fn focus_key(model: &mut Model) -> Vec<Cmd> {
    if model.traffic.data.is_none() {
        model.status = StatusLine::info("no result to focus yet: r runs the query");
        return Vec::new();
    }
    match model.traffic.next_focus_app() {
        Some(app) => focus_on(model, app),
        None if model.traffic.focus.is_some() => {
            model.traffic.clear_focus();
            model.status = StatusLine::info("focus cleared");
        }
        None => open_app_picker(model, AppPickerPurpose::Focus, ""),
    }
    Vec::new()
}

/// The picker over the apps of the loaded result.
fn open_app_picker(model: &mut Model, purpose: AppPickerPurpose, input: &str) {
    model.overlay =
        Overlay::AppPicker(AppPicker::new(model.traffic.loaded_apps(), purpose).with_input(input));
}

/// A new focus on `app` under the keys in force, at `both`. No request.
fn focus_on(model: &mut Model, app: GroupKey) {
    let focus = Focus::new(app, model.traffic.keys.clone());
    model.traffic.set_focus(focus);
    model.status = focus_status(&model.traffic);
}

/// `focus shop/prod · in · 7 flows`; on a partial focus also what `r`
/// would send (asked once, here).
fn focus_status(t: &TrafficState) -> StatusLine {
    let Some(f) = &t.focus else {
        return StatusLine::info("focus cleared");
    };
    let (all, z) = (t.focus_flows().len(), t.squelched_hidden());
    let n = crate::tui::traffic_marks::flows_text(all - z, all, z);
    if t.focus_partial() {
        let q = f.direction.query_count();
        StatusLine::warn(format!(
            "{} · {n} · the result is truncated: r re-queries {} ({})",
            f.badge(),
            f.app_text(),
            crate::tui::model::queries_text(q)
        ))
    } else {
        StatusLine::info(format!("{} · {n}", f.badge()))
    }
}

/// `:focus <app>` and the picker: an app of the loaded result by its text
/// (`shop/prod`), else the closest ones in the status line. Shows the
/// Traffic screen. Never requests.
fn focus_by_name(model: &mut Model, name: &str) {
    if model.traffic.data.is_none() {
        model.status = StatusLine::info("no result to focus yet: r runs the query");
        return;
    }
    let apps = model.traffic.loaded_apps();
    if apps.iter().any(|a| a == name) {
        model.screen = Screen::Traffic;
        focus_on(model, pce::traffic::parse_app(name));
        return;
    }
    let near = pce::traffic::closest(name, &apps, 3);
    model.status = StatusLine::warn(if near.is_empty() {
        format!("unknown app {name:?} in the loaded result; nothing sent")
    } else {
        format!(
            "unknown app {name:?}; closest: {}; nothing sent",
            near.join(", ")
        )
    });
}

/// `r` on a partial focus (or on its focused result, which it refreshes past
/// the cache): one query per side, through the cost guard, whose prompt
/// names the count. An app without labels (`ipl:Corp/-`) is refused.
fn requery_focus(model: &mut Model, focus: Focus) -> Vec<Cmd> {
    if !focus.is_label_group() {
        model.status = StatusLine::error(format!(
            "{} is not a label group: it cannot be re-queried by its labels; Esc clears the focus, then r reruns the whole query",
            focus.app_text()
        ));
        return Vec::new();
    }
    let f = model.traffic.filter.clone();
    let refresh = model
        .traffic
        .focus_result
        .as_ref()
        .is_some_and(|r| r.covers(&focus));
    let mode = if refresh {
        CacheMode::Bypass
    } else {
        CacheMode::Use
    };
    if let Some(reason) = f.needs_confirmation() {
        let n = focus.direction.query_count();
        model.overlay = Overlay::Confirm {
            reason: format!(
                "{reason}. Run {} for {} anyway? (y/n)",
                queries_text(n),
                focus.app_text()
            ),
            action: PendingAction::RunFocused(f, focus, mode),
        };
        return Vec::new();
    }
    start_focused(model, f, focus, mode)
}

/// The direction a new app overlay starts at: the focus's, else `both`.
fn app_direction(model: &Model) -> FocusDirection {
    model
        .traffic
        .focus
        .as_ref()
        .map_or(FocusDirection::Both, |f| f.direction)
}

/// `A`: the app overlay for the row's app (the first of `row_apps`, the
/// order `F` takes), at the focus direction (or `both`); on a row without
/// an app, the picker.
fn app_open_key(model: &mut Model) -> Vec<Cmd> {
    let Some(app) = model.traffic.row_apps().into_iter().next() else {
        open_app_picker(model, AppPickerPurpose::Overlay, "");
        return Vec::new();
    };
    let focus = Focus::new(app, model.traffic.keys.clone()).with_direction(app_direction(model));
    open_app(model, focus)
}

/// `:app <app>` and the picker: an app of the loaded result, or one whose
/// labels the name index knows (it may have no flows in the window); else
/// the closest match in the status line, nothing sent.
fn open_app_by_name(model: &mut Model, name: &str) -> Vec<Cmd> {
    let keys = model.traffic.keys.clone();
    let apps = model.traffic.loaded_apps();
    let app = pce::traffic::parse_app(name);
    let labelled = model
        .index()
        .map(|idx| pce::traffic::app_labels(&app, &keys, idx));
    if !apps.iter().any(|a| a == name) && !matches!(labelled, Some(Ok(_))) {
        let near = pce::traffic::closest(name, &apps, 3);
        model.status = StatusLine::warn(match (near.is_empty(), labelled) {
            (false, _) => format!(
                "unknown app {name:?}; closest: {}; nothing sent",
                near.join(", ")
            ),
            (true, Some(Err(e))) => format!("{e}; nothing sent"),
            (true, _) => {
                format!("unknown app {name:?} in the loaded result or the labels; nothing sent")
            }
        });
        return Vec::new();
    }
    let focus = Focus::new(app, keys).with_direction(app_direction(model));
    open_app(model, focus)
}

/// `:gen <app>` and the picker: as `:app` finds an app (the loaded result
/// or the labels), then the generator for it.
fn gen_by_name(model: &mut Model, name: &str) -> Vec<Cmd> {
    let keys = model.traffic.keys.clone();
    let app = pce::traffic::parse_app(name);
    let loaded = model.traffic.loaded_apps();
    let labelled = model
        .index()
        .map(|idx| pce::traffic::app_labels(&app, &keys, idx));
    if !loaded.iter().any(|a| a == name) && !matches!(labelled, Some(Ok(_))) {
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
    generator::open_gen(model, app, false)
}

/// Opens the overlay for `focus`. The traffic panels come from the loaded
/// result (no request). The workloads come from the Workloads screen when
/// it holds every workload; the rulesets from the Policy screen when it is
/// loaded (none is requested while its load is on the way). Reopening the
/// same app keeps its view (`seq`, panels, cursors): nothing is sent again.
/// Otherwise one request each, counted. A new app gets a new `seq`, so an
/// earlier overlay's late answer is dropped.
fn open_app(model: &mut Model, focus: Focus) -> Vec<Cmd> {
    model.traffic.sync_marks();
    let window = model.traffic.filter.window.clone();
    let (mut v, reused) = match model.app_view.take() {
        Some(mut v) if v.focus.same_app(&focus) => {
            v.focus = focus.clone();
            v.window = window;
            (v, true)
        }
        _ => {
            model.app_seq += 1;
            (AppView::new(focus.clone(), window, model.app_seq), false)
        }
    };
    let t = &model.traffic;
    let a = t.active_marks();
    v.rebuild_traffic(
        t.flows_for(&focus),
        t.partial_for(&focus),
        a.as_ref().map(|a| (a, t.show_squelched)),
    );
    let idx = model.index().cloned();
    v.label_hrefs = idx
        .as_ref()
        .and_then(|idx| focus.label_matches(idx).ok())
        .map(|m| pce::app::label_hrefs(&m));
    let why = if idx.is_none() {
        "the name index is not loaded yet".to_string()
    } else {
        format!(
            "{} is not a label group: no workloads or rulesets to match",
            focus.app_text()
        )
    };
    let mut cmds = Vec::new();
    match (v.label_hrefs.clone(), &idx) {
        (Some(hrefs), Some(idx))
            if model.workloads.loaded_at.is_some() && !model.workloads.truncated =>
        {
            let mine: Vec<Workload> = model
                .workloads
                .raw
                .iter()
                .filter(|w| pce::app::workload_in_app(w, &hrefs))
                .cloned()
                .collect();
            let now = model.workloads.now.unwrap_or_else(jiff::Timestamp::now);
            v.workloads = Loadable::Loaded(pce::app::app_workloads(
                &mine,
                idx,
                &model.limits.tier_key,
                now,
            ));
            v.workloads_truncated = false;
        }
        (Some(_), Some(_)) if reused && !matches!(v.workloads, Loadable::Unavailable(_)) => {}
        (Some(hrefs), Some(_)) => {
            v.workloads = Loadable::Loading;
            cmds.push(Cmd::LoadAppWorkloads {
                seq: v.seq,
                filter: pce::app::app_workload_filter(&hrefs),
                mode: CacheMode::Use,
            });
        }
        _ => v.workloads = Loadable::Unavailable(why.clone()),
    }
    // A panel of the other scope (`:draft` / `:active` since) is stale.
    let same_scope = v.policy_scope == model.policy.scope;
    v.policy_scope = model.policy.scope;
    match (&v.label_hrefs, &idx) {
        (Some(hrefs), Some(idx)) if model.policy.loaded_at.is_some() => {
            v.policy = Loadable::Loaded(pce::app::app_policy(&model.policy.raw, hrefs, idx));
        }
        (Some(_), Some(_))
            if reused && same_scope && !matches!(v.policy, Loadable::Unavailable(_)) => {}
        (Some(_), Some(_)) if model.policy.loading => {
            v.policy = Loadable::Loading;
            v.policy_sent = false;
        }
        (Some(_), Some(_)) => {
            v.policy = Loadable::Loading;
            v.policy_sent = true;
            cmds.extend(load_rule_sets(model, CacheMode::Use));
        }
        _ => v.policy = Loadable::Unavailable(why),
    }
    let sent = u32::try_from(cmds.len()).unwrap_or(u32::MAX);
    v.requests += sent;
    // `req` as in the header: the hints beside it leave about 20 columns.
    model.status = StatusLine::info(format!("{} · {sent} req", focus.app_text()));
    model.app_view = Some(v);
    model.overlay = Overlay::App;
    cmds
}

/// The overlay's peers again from the loaded flows: after `X`, and when a
/// traffic answer lands while it is open. No request.
fn rebuild_app_traffic(model: &mut Model) {
    model.traffic.sync_marks();
    let t = &model.traffic;
    if let Some(v) = model.app_view.as_mut() {
        let focus = v.focus.clone();
        let a = t.active_marks();
        v.rebuild_traffic(
            t.flows_for(&focus),
            t.partial_for(&focus),
            a.as_ref().map(|a| (a, t.show_squelched)),
        );
    }
}

/// `?` / `:help`: the help at its top, on the keys of where it was opened
/// (`help::HelpContext::of`). Opened over the app view, the generator, an
/// event's detail, the pending changes or the marks, closing it goes back there.
pub(super) fn open_help(model: &mut Model) {
    model.help_scroll = 0;
    model.help_page = HelpPage::Context;
    model.help_return = match &model.overlay {
        o @ (Overlay::App
        | Overlay::Generator
        | Overlay::EventDetail { .. }
        | Overlay::Pending
        | Overlay::Marks) => o.clone(),
        _ => Overlay::None,
    };
    model.overlay = Overlay::Help;
}

/// The help's keys: `Esc` / `?` / `q` close it (back to where it was
/// opened), `Tab` / `BackTab` one group at a time, `a` all groups (again:
/// the context page), the scroll keys within the page. Others do nothing.
fn help_key(model: &mut Model, k: KeyEvent) {
    let ctx = HelpContext::of(model);
    let page = model.help_page;
    match k.code {
        KeyCode::Esc | KeyCode::Char('?') | KeyCode::Char('q') => {
            model.overlay = std::mem::replace(&mut model.help_return, Overlay::None);
            return;
        }
        KeyCode::Tab => model.help_page = page.step(ctx, true),
        KeyCode::BackTab => model.help_page = page.step(ctx, false),
        KeyCode::Char('a') => model.help_page = page.toggle_all(),
        _ => {}
    }
    if model.help_page != page {
        model.help_scroll = 0;
        return;
    }
    let last = help_last_scroll(model, model.size);
    let rows = help_body_rows(model, model.size).max(1);
    let s = model.help_scroll.min(last);
    model.help_scroll = match k.code {
        KeyCode::Down | KeyCode::Char('j') => (s + 1).min(last),
        KeyCode::Up | KeyCode::Char('k') => s.saturating_sub(1),
        KeyCode::PageDown => (s + rows).min(last),
        KeyCode::PageUp => s.saturating_sub(rows),
        KeyCode::End | KeyCode::Char('G') => last,
        KeyCode::Home | KeyCode::Char('g') => 0,
        _ => s,
    };
}

/// The overlay's keys: `Esc` / `q` close, `Tab` / `BackTab` panels, `j` /
/// `k` rows, `X` direction, `Enter` opens, `w` drafts a rule, `r` asks to
/// reload, `?` help. Only `r` (after `y`) and `Enter` (on another screen) can send.
fn app_overlay_key(model: &mut Model, k: KeyEvent) -> Vec<Cmd> {
    let Some(v) = model.app_view.as_mut() else {
        model.overlay = Overlay::None;
        return Vec::new();
    };
    match k.code {
        KeyCode::Esc | KeyCode::Char('q') => model.overlay = Overlay::None,
        KeyCode::Tab => v.panel = v.panel.next(),
        KeyCode::BackTab => v.panel = v.panel.prev(),
        KeyCode::Down | KeyCode::Char('j') => v.move_cursor(1),
        KeyCode::Up | KeyCode::Char('k') => v.move_cursor(-1),
        KeyCode::Char('X') => {
            v.focus.direction = v.focus.direction.next();
            rebuild_app_traffic(model);
        }
        // Squelched peers shown or left out here and on the Traffic screen.
        KeyCode::Char('M') => {
            model.status = model.traffic.toggle_squelched();
            rebuild_app_traffic(model);
        }
        KeyCode::Enter => return app_enter(model),
        KeyCode::Char('m') => return marks::mark_app_key(model),
        KeyCode::Char('w') => return app_rule_key(model),
        KeyCode::Char('r') => app_reload_key(model),
        KeyCode::Char('G') => {
            let app = v.focus.app.clone();
            return generator::open_gen(model, app, true);
        }
        KeyCode::Char('?') => open_help(model),
        _ => {}
    }
    Vec::new()
}

/// `r` in the overlay: asks before reloading its two panels.
fn app_reload_key(model: &mut Model) {
    let Some(v) = model.app_view.as_ref() else {
        return;
    };
    let app = v.focus.app_text();
    if v.label_hrefs.is_none() {
        model.status = StatusLine::info(format!("nothing to reload: {app} is not a label group"));
        return;
    }
    // The rulesets are not asked again while a load of them is on its way.
    let reason = if model.policy.loading {
        format!("Reload the workloads of {app} (the rulesets are on their way): 1 request? (y/n)")
    } else {
        format!("Reload the workloads and the rulesets of {app}: 2 requests? (y/n)")
    };
    model.overlay = Overlay::Confirm {
        reason,
        action: PendingAction::ReloadApp,
    };
}

/// `y` on the reload prompt: both panels past the cache, counted (the
/// rulesets not while a load of them is on the way); back to the overlay.
fn app_reload(model: &mut Model) -> Vec<Cmd> {
    model.overlay = Overlay::App;
    let policy_loading = model.policy.loading;
    let Some(v) = model.app_view.as_mut() else {
        model.overlay = Overlay::None;
        return Vec::new();
    };
    let Some(hrefs) = v.label_hrefs.clone() else {
        return Vec::new();
    };
    v.workloads = Loadable::Loading;
    v.policy = Loadable::Loading;
    v.policy_scope = model.policy.scope;
    v.cached = 0;
    let mut cmds = vec![Cmd::LoadAppWorkloads {
        seq: v.seq,
        filter: pce::app::app_workload_filter(&hrefs),
        mode: CacheMode::Bypass,
    }];
    v.policy_sent = !policy_loading;
    if !policy_loading {
        cmds.extend(load_rule_sets(model, CacheMode::Bypass));
    }
    if let Some(v) = model.app_view.as_mut() {
        v.requests += u32::try_from(cmds.len()).unwrap_or(u32::MAX);
    }
    cmds
}

/// `Enter`: a peer closes the overlay and focuses that pair in Flows; a
/// workload opens its detail on the Workloads screen; a ruleset opens it on
/// the Policy screen (those screens load what they lack, as always).
fn app_enter(model: &mut Model) -> Vec<Cmd> {
    let Some(v) = model.app_view.as_ref() else {
        return Vec::new();
    };
    match v.panel {
        AppPanel::Inbound | AppPanel::Outbound => {
            let Some((side, p)) = v.selected_peer() else {
                return Vec::new();
            };
            let (src, dst) = match side {
                Direction::In => (p.peer.clone(), v.focus.app.clone()),
                Direction::Out => (v.focus.app.clone(), p.peer.clone()),
            };
            let service = p.service.clone();
            let focus = v.focus.clone();
            model.overlay = Overlay::None;
            model.screen = Screen::Traffic;
            let t = &mut model.traffic;
            t.drill = Some(Drill::Service(src, dst, service, Decisions::All));
            t.view = TrafficView::Flows;
            t.show_detail = false;
            t.set_focus(focus); // rebuilds every view, the drill included
            model.status = focus_status(&model.traffic);
            Vec::new()
        }
        AppPanel::Workloads => {
            let Some(href) = v.selected_workload().map(|w| w.href.clone()) else {
                return Vec::new();
            };
            model.overlay = Overlay::None;
            let cmds = switch(model, Screen::Workloads);
            let w = &mut model.workloads;
            w.show_detail = w.select_href(&href);
            w.open_on_load = None;
            if !w.show_detail {
                // Not in the list yet: opened when the list arrives.
                w.open_on_load = Some(href);
                model.status = StatusLine::info("loading the workloads: its detail opens then");
            }
            cmds
        }
        AppPanel::Policy => {
            let Some(href) = v.selected_policy().map(|r| r.href.clone()) else {
                return Vec::new();
            };
            model.overlay = Overlay::None;
            // Select first, so the deny-rules fallback `switch` fires is
            // for this ruleset, not the row selected before.
            model.policy.pane = PolicyPane::RuleSets;
            model.policy.rulesets.select_where(|r| r.href == href);
            let mut cmds = switch(model, Screen::Policy);
            if let Some(idx) = model.index().cloned() {
                model
                    .policy
                    .rebuild_rules(&idx, model.workloads.impact_source());
                cmds.extend(deny_fallback(model, CacheMode::Use));
            }
            cmds
        }
    }
}

/// `w` on a blocked (or potentially blocked) peer row: the draft allow rule
/// from one of the flows behind it, as `w` on a traffic row (writes armed).
/// The form replaces the overlay; cancelling it, or the write done, comes
/// back to the overlay with nothing sent.
fn app_rule_key(model: &mut Model) -> Vec<Cmd> {
    if writes_refused(model) {
        return Vec::new();
    }
    let Some(v) = model.app_view.as_ref() else {
        return Vec::new();
    };
    let Some((side, p)) = v.selected_peer() else {
        model.status = StatusLine::warn("select a peer row (Inbound or Outbound)");
        return Vec::new();
    };
    if !matches!(
        p.decision,
        PolicyDecision::Blocked | PolicyDecision::PotentiallyBlocked
    ) {
        model.status = StatusLine::warn("this row is allowed; nothing to add");
        return Vec::new();
    }
    let focus = &v.focus;
    let flow = model
        .traffic
        .flows_for(focus)
        .iter()
        .find(|f| {
            let (app_ep, peer_ep) = match side {
                Direction::In => (&f.dst, &f.src),
                Direction::Out => (&f.src, &f.dst),
            };
            f.policy_decision == p.decision
                && f.service_text() == p.service
                && pce::traffic::group_key(app_ep, &focus.keys) == focus.app
                && pce::traffic::group_key(peer_ep, &focus.keys) == p.peer
        })
        .cloned();
    let Some(flow) = flow else {
        model.status = StatusLine::warn("no loaded flow behind this row");
        return Vec::new();
    };
    let cmds = begin_edit(model, EditIntent::RuleFromFlow(Box::new(flow)));
    model.form_from_app = matches!(model.overlay, Overlay::Form(_));
    cmds
}

/// Why `Ctrl-R` must not create `name`: a loaded draft ruleset (any case)
/// has it already, or several do. `None` when none does, or while the
/// drafts are not loaded (the plan then reads them fresh).
fn existing_ruleset(model: &Model, name: &str) -> Option<String> {
    let hits = edit::rulesets_named(model.draft_rulesets()?, name);
    let name = name.trim();
    match hits.as_slice() {
        [] => None,
        [_] => Some(format!(
            "ruleset {name:?} exists: Enter adds the rule to it"
        )),
        many => Some(format!(
            "{name:?} names several rulesets: {}; use the href",
            many.iter()
                .map(|rs| rs.href.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        )),
    }
}

/// The plan or write a form led to has ended: back where the form came
/// from when nothing else is open (`close_form`); the way back is
/// forgotten either way.
fn finish_form(model: &mut Model) {
    if model.overlay == Overlay::None {
        close_form(model);
    } else {
        model.form_from_app = false;
    }
}

/// A form is done, or the plan and confirm it led to: back to the app
/// overlay it was opened from (its view is kept: no request), else to the
/// screen behind it.
fn close_form(model: &mut Model) {
    let back = std::mem::take(&mut model.form_from_app) && model.app_view.is_some();
    model.overlay = if back { Overlay::App } else { Overlay::None };
}

/// The overlay's policy panel from the rulesets that just arrived (`hit`:
/// from the cache), or their error, while it waits for them. Fills the
/// view only: never reopens a closed overlay.
fn refresh_app_policy(model: &mut Model, scope: PolicyScope, outcome: Result<bool, &str>) {
    let idx = model.index().cloned();
    let Some(v) = model.app_view.as_mut() else {
        return;
    };
    if !matches!(v.policy, Loadable::Loading) || v.policy_scope != scope {
        return;
    }
    match (outcome, idx, v.label_hrefs.as_ref()) {
        (Ok(hit), Some(idx), Some(hrefs)) => {
            if v.policy_sent && hit {
                v.cached += 1;
            }
            v.policy = Loadable::Loaded(pce::app::app_policy(&model.policy.raw, hrefs, &idx));
        }
        (Err(e), _, _) => v.policy = Loadable::Failed(e.to_string()),
        _ => v.policy = Loadable::Unavailable("the name index is not loaded".into()),
    }
}

fn start_focused(
    model: &mut Model,
    filter: TrafficFilter,
    focus: Focus,
    mode: CacheMode,
) -> Vec<Cmd> {
    // The filter shows what runs, as for an unfocused query (a focused
    // cap step changes its cap).
    model.traffic.filter = filter.clone();
    let (seq, cancel) = model
        .traffic
        .start_focused_query(Instant::now(), focus.clone());
    let n = focus.direction.query_count();
    model.status = StatusLine::info(format!(
        "focused query running: {} · {}",
        focus.app_text(),
        queries_text(n)
    ));
    model.traffic.auto.rearm();
    vec![Cmd::RunFocused {
        seq,
        filter,
        focus,
        mode,
        cancel,
    }]
}

/// `+`: the next window step, re-running the whole window as one query
/// through the cost guard (Explorer's records overlap the window, so an
/// older slice cannot be added to the loaded ones).
fn widen_window(model: &mut Model) -> Vec<Cmd> {
    if model.traffic.is_running() {
        model.status = StatusLine::info("query already running");
        return Vec::new();
    }
    let cur = match model.traffic.filter.window_duration() {
        Ok(d) => d,
        Err(e) => {
            model.status = StatusLine::error(e);
            return Vec::new();
        }
    };
    let Some(step) = pce::traffic::next_window(cur) else {
        let top = pce::traffic::WINDOW_STEPS[pce::traffic::WINDOW_STEPS.len() - 1];
        model.status = StatusLine::info(format!("window already at {top}"));
        return Vec::new();
    };
    let mut f = model.traffic.filter.clone();
    f.window = step.to_string();
    request_query(model, f)
}

/// `B`: the same window again at the next backfill cap (10000 from below
/// it, then 25000, 50000, 100000, 200000), one query whose answer replaces
/// the truncated result. Above 10000 (or past the window threshold) it asks
/// first; `B` again climbs one more rung.
fn backfill_key(model: &mut Model) -> Vec<Cmd> {
    if model.traffic.is_running() {
        model.status = StatusLine::info("query already running");
        return Vec::new();
    }
    let Some(d) = model
        .traffic
        .data
        .as_ref()
        .filter(|d| d.filter == model.traffic.filter)
    else {
        model.status = StatusLine::info("nothing to backfill: r runs the query first");
        return Vec::new();
    };
    if !d.truncated {
        model.status = StatusLine::info("nothing to backfill: the result is complete");
        return Vec::new();
    }
    let old = d.filter.max_results;
    let Some(cap) = pce::traffic::backfill_cap(old) else {
        model.status = StatusLine::info(format!(
            "cap already at the maximum ({old}): narrow the filter"
        ));
        return Vec::new();
    };
    let mut f = d.filter.clone();
    f.max_results = cap;
    if let Some(reason) = f.needs_confirmation() {
        model.overlay = Overlay::Confirm {
            reason: format!(
                "{reason}. Re-run the window at cap {cap} (1 query, replaces the {} flows capped at {old})? y/n",
                d.flows.len()
            ),
            action: PendingAction::RunQuery(f),
        };
        return Vec::new();
    }
    update(model, Msg::ApplyFilter(f))
}

/// `]`, `[` or `B` while a focused result covering the focus is on screen:
/// the focused query again at the next (`]`, or `B` on a truncated result:
/// `backfill_cap`) or previous cap, one query per side, through the cost
/// guard. `None` when no such result is shown (the keys then act on the
/// unfocused result).
fn focused_cap_key(model: &mut Model, key: char) -> Option<Vec<Cmd>> {
    let t = &model.traffic;
    let focus = t.focus.clone()?;
    if !t.focus_result.as_ref().is_some_and(|r| r.covers(&focus)) {
        return None;
    }
    if t.is_running() {
        model.status = StatusLine::info("query already running");
        return Some(Vec::new());
    }
    let d = t.data.as_ref()?;
    let old = d.filter.max_results;
    let next = match key {
        'B' if !d.truncated => {
            model.status = StatusLine::info("nothing to backfill: the result is complete");
            return Some(Vec::new());
        }
        'B' => pce::traffic::backfill_cap(old),
        ']' => pce::traffic::next_cap(old, model.limits.traffic_max_results),
        _ => pce::traffic::prev_cap(old, model.limits.traffic_max_results),
    };
    let Some(cap) = next else {
        model.status = StatusLine::info(if key == '[' {
            "cap already at the minimum".to_string()
        } else {
            format!("cap already at the maximum ({old}): narrow the filter")
        });
        return Some(Vec::new());
    };
    let mut f = d.filter.clone();
    f.max_results = cap;
    let n = d.flows.len();
    if let Some(reason) = f.needs_confirmation() {
        let q = queries_text(focus.direction.query_count());
        model.overlay = Overlay::Confirm {
            reason: format!(
                "{reason}. Re-run {} at cap {cap} ({q}, replaces the {n} flows at cap {old})? y/n",
                focus.app_text()
            ),
            action: PendingAction::RunFocused(f, focus, CacheMode::Use),
        };
        return Some(Vec::new());
    }
    Some(start_focused(model, f, focus, CacheMode::Use))
}

/// The profile's opt-in after a truncated result below 10000: the same
/// window again at 10000, without `B` (above it `B` asks as usual).
fn auto_backfill(model: &mut Model, n: usize, cap: u32) -> Vec<Cmd> {
    let Some(mut f) = model.traffic.data.as_ref().map(|d| d.filter.clone()) else {
        return Vec::new();
    };
    f.max_results = pce::traffic::CONFIRM_MAX_RESULTS;
    model.traffic.filter = f.clone();
    let cmds = start_query(model, f, CacheMode::Use);
    model.status = StatusLine::info(format!(
        "{n} flows, capped at {cap}: re-running at cap {} (profile traffic_auto_backfill)",
        pce::traffic::CONFIRM_MAX_RESULTS
    ));
    cmds
}

/// `-`: the previous window step, no request. Flows last seen before the new
/// start are dropped (the rest overlap the narrower window, which is what
/// Explorer would return for it) and the views rebuilt; a query on the way
/// is cancelled (its answer is stale). A later `+` queries again.
fn narrow_window(model: &mut Model) -> Vec<Cmd> {
    let cur = match model.traffic.filter.window_duration() {
        Ok(d) => d,
        Err(e) => {
            model.status = StatusLine::error(e);
            return Vec::new();
        }
    };
    let Some(step) = pce::traffic::prev_window(cur) else {
        model.status = StatusLine::info(format!(
            "window already at {}",
            pce::traffic::WINDOW_STEPS[0]
        ));
        return Vec::new();
    };
    let t = &mut model.traffic;
    t.cancel_query();
    t.filter.window = step.to_string();
    let mut dropped = None;
    if let Some(d) = t.data.as_mut() {
        // A cancelled `]`, `[` or `f` query never ran: the filter goes back
        // to the one the flows were fetched with, at the stepped window.
        let window = std::mem::take(&mut t.filter.window);
        t.filter = TrafficFilter {
            window,
            ..d.filter.clone()
        };
        let span = t
            .filter
            .window_duration()
            .ok()
            .and_then(|w| jiff::Span::try_from(w).ok());
        // Only a range that covers the new window narrows; a wider window
        // than the flows cover waits for `r`.
        if let Some(r) = d.range.as_mut()
            && let Some(start) = span.and_then(|span| r.end.checked_sub(span).ok())
            && start >= r.start
        {
            let before = d.flows.len();
            d.flows.retain(|f| {
                f.last_detected
                    .as_deref()
                    .and_then(|l| l.parse::<jiff::Timestamp>().ok())
                    .is_none_or(|l| l >= start)
            });
            r.start = start;
            d.filter.window = step.to_string();
            dropped = Some(before - d.flows.len());
            // No longer what its async query returned: `D` cannot
            // re-evaluate that query for it, and the draft counts shrink.
            d.hrefs.clear();
            if d.draft.is_some() {
                d.draft = Some(pce::traffic::DraftSummary::of(&d.flows));
            }
        }
    }
    t.rebuild_views();
    model.status = StatusLine::info(match dropped {
        Some(n) => format!("window {step} · {n} older flows dropped"),
        None => format!("window {step} · r runs it"),
    });
    Vec::new()
}

/// Auto-refresh cannot run `f` unattended when it is past a cost threshold:
/// each run needs a confirm. The status that says so; `None` otherwise.
fn auto_off_status(f: &TrafficFilter) -> Option<StatusLine> {
    f.needs_confirmation()?;
    let what = if f
        .window_duration()
        .is_ok_and(|w| w > pce::traffic::CONFIRM_WINDOW)
    {
        format!("window {}", f.window)
    } else {
        format!("cap {}", f.max_results)
    };
    Some(StatusLine::warn(format!(
        "auto off: {what} needs a confirm each run"
    )))
}

/// `]` / `[`: the next or previous cap step, re-running the whole query
/// (the PCE picks which rows fit the cap) through the cost guard.
fn step_cap(model: &mut Model, up: bool) -> Vec<Cmd> {
    if model.traffic.is_running() {
        model.status = StatusLine::info("query already running");
        return Vec::new();
    }
    let cur = model.traffic.filter.max_results;
    let start = model.limits.traffic_max_results;
    let next = if up {
        pce::traffic::next_cap(cur, start)
    } else {
        pce::traffic::prev_cap(cur, start)
    };
    let Some(cap) = next else {
        model.status = StatusLine::info(if up {
            "cap already at the maximum"
        } else {
            "cap already at the minimum"
        });
        return Vec::new();
    };
    let mut f = model.traffic.filter.clone();
    f.max_results = cap;
    request_query(model, f)
}

/// Validated filter from the editor or a command: confirm if expensive, else apply.
fn request_query(model: &mut Model, f: TrafficFilter) -> Vec<Cmd> {
    if let Some(reason) = f.needs_confirmation() {
        model.overlay = Overlay::Confirm {
            reason: format!("{reason}. Run anyway? (y/n)"),
            action: PendingAction::RunQuery(f),
        };
        return Vec::new();
    }
    update(model, Msg::ApplyFilter(f))
}

/// Mouse: the wheel moves the focused table one row (also while a search is
/// active), a left click on the tabs row switches screen through the same path
/// as the number keys. Everything else is ignored; the wheel never requests anything.
fn on_mouse(model: &mut Model, me: crossterm::event::MouseEvent) -> Vec<Cmd> {
    use crossterm::event::{MouseButton, MouseEventKind};
    if model.overlay != Overlay::None {
        return Vec::new();
    }
    match me.kind {
        MouseEventKind::ScrollDown => scroll_focused(model, true),
        MouseEventKind::ScrollUp => scroll_focused(model, false),
        MouseEventKind::Down(MouseButton::Left) if me.row == 1 => {
            return match crate::tui::ui::tab_at(me.column, model.size.0) {
                Some(s) => switch(model, s),
                None => Vec::new(),
            };
        }
        _ => {}
    }
    Vec::new()
}

/// One row up or down in the current screen's focused table.
fn scroll_focused(model: &mut Model, down: bool) {
    fn step<T>(table: &mut crate::tui::table::TableView<T>, down: bool, page: usize) {
        if down {
            table.select_next();
        } else {
            table.select_prev();
        }
        table.ensure_visible(page);
    }
    match model.screen {
        Screen::Traffic => {
            let page = traffic_page(model);
            let t = &mut model.traffic;
            match t.view {
                TrafficView::Flows if t.flows_grouped() => step(&mut t.flows_grouped, down, page),
                TrafficView::Flows => step(&mut t.flows, down, page),
                TrafficView::Aggregate if t.apps_state.by == By::Pairs => {
                    step(t.pairs_table_mut(), down, page)
                }
                TrafficView::Aggregate => step(&mut t.apps, down, page),
                TrafficView::Mesh => step(&mut t.mesh, down, page),
                TrafficView::Blocked => step(&mut t.blocked, down, page),
                // The wheel moves table rows; the treemap has none.
                TrafficView::Treemap => {}
            }
        }
        Screen::Policy => {
            let page = policy_page(model);
            match model.policy.pane {
                PolicyPane::RuleSets => {
                    let before = model.policy.selected_href();
                    step(&mut model.policy.rulesets, down, page);
                    // The rules follow the ruleset; a missing deny list is not
                    // fetched from here (the wheel never requests).
                    if model.policy.selected_href() != before
                        && let Some(idx) = model.index().cloned()
                    {
                        model
                            .policy
                            .rebuild_rules(&idx, model.workloads.impact_source());
                    }
                }
                PolicyPane::Rules => step(&mut model.policy.rules, down, page),
            }
        }
        Screen::Objects => {
            let page = if model.objects.kind == ObjectKind::IpLists {
                ip_lists_page(model)
            } else {
                objects_page(model)
            };
            let o = &mut model.objects;
            match o.kind {
                ObjectKind::Labels => step(&mut o.labels, down, page),
                ObjectKind::LabelGroups => step(&mut o.groups, down, page),
                ObjectKind::Services => step(&mut o.services, down, page),
                ObjectKind::IpLists => step(&mut o.ip_lists.table, down, page),
            }
        }
        Screen::Workloads => {
            let page = workloads_page(model);
            let w = &mut model.workloads;
            if w.grouped {
                step(&mut w.sections, down, page);
            } else {
                step(&mut w.table, down, page);
            }
        }
        Screen::Events => {
            let page = events_page(model);
            step(&mut model.events.table, down, page);
        }
    }
}

fn switch(model: &mut Model, s: Screen) -> Vec<Cmd> {
    model.screen = s;
    match s {
        // Loaded data is kept: returning to the screen never requests again.
        Screen::Policy
            if model.policy.loaded_at.is_none()
                && !model.policy.loading
                && model.is_connected() =>
        {
            load_rule_sets(model, CacheMode::Use)
        }
        // Loaded off this screen: the selected ruleset's deny rules now.
        // Not during a reload: its answer asks for them, on fresh data.
        Screen::Policy if model.policy.loaded_at.is_some() && !model.policy.loading => {
            deny_fallback(model, CacheMode::Use)
        }
        Screen::Objects => load_missing_objects(model),
        Screen::Workloads => load_missing_workloads(model),
        Screen::Events => load_missing_events(model),
        _ => Vec::new(),
    }
}

fn load_rule_sets(model: &mut Model, mode: CacheMode) -> Vec<Cmd> {
    model.policy.loading = true;
    model.policy.auto.last_run = Some(Instant::now());
    model.policy.auto.from_tick = false;
    vec![Cmd::LoadRuleSets {
        scope: model.policy.scope,
        mode,
    }]
}

/// Deny-rule endpoint request for the selected ruleset when its inline list is empty.
fn deny_fallback(model: &mut Model, mode: CacheMode) -> Vec<Cmd> {
    model
        .policy
        .take_deny_fallback()
        .map(|href| vec![Cmd::LoadDenyRules { href, mode }])
        .unwrap_or_default()
}

fn policy_page(model: &Model) -> usize {
    let area = ratatui::layout::Rect::new(0, 0, model.size.0, model.size.1);
    crate::tui::ui::layout::chrome(area)
        .map(|c| crate::tui::ui::screens::policy::table_rows(c.body, model.policy.pane))
        .unwrap_or(10)
        .max(1)
}

fn policy_key(model: &mut Model, k: KeyEvent) -> Vec<Cmd> {
    let page = policy_page(model);
    if !model.policy.is_searching() {
        match k.code {
            KeyCode::Char('d') => {
                if model.policy.loading {
                    model.status = StatusLine::info("already loading");
                    return Vec::new();
                }
                model.policy.scope = match model.policy.scope {
                    PolicyScope::Draft => PolicyScope::Active,
                    PolicyScope::Active => PolicyScope::Draft,
                };
                model.policy.form_drafts_loading = false;
                model.policy.form_drafts_stale = false;
                model.policy.clear();
                model.objects.ip_lists.recount(&model.policy);
                return load_rule_sets(model, CacheMode::Use);
            }
            KeyCode::Char('r') => {
                if model.policy.loading {
                    // A tick result would consume the deny bypass meant for this one.
                    model.status = StatusLine::info("already loading");
                    return Vec::new();
                }
                model.policy.bypass_deny = true;
                model.policy.auto.rearm();
                return load_rule_sets(model, CacheMode::Bypass);
            }
            KeyCode::Char('p') => {
                if model.policy.pending_loading {
                    model.status = StatusLine::info("pending changes already loading");
                    return Vec::new();
                }
                model.policy.pending_loading = true;
                model.status = StatusLine::info("loading pending changes");
                return vec![Cmd::LoadPending(CacheMode::Use)];
            }
            KeyCode::Char('a') => {
                model.status = model.policy.auto.toggle();
                return Vec::new();
            }
            KeyCode::Char('i') => return impact_key(model),
            KeyCode::Char(c @ ('n' | 'e' | 'x')) => return policy_edit_key(model, c),
            // `N` (a deny rule) is bound on the rules pane only.
            KeyCode::Char('N') if model.policy.pane == PolicyPane::Rules => {
                return policy_edit_key(model, 'N');
            }
            KeyCode::Tab | KeyCode::BackTab => {
                model.policy.pane = match model.policy.pane {
                    PolicyPane::RuleSets => PolicyPane::Rules,
                    PolicyPane::Rules => PolicyPane::RuleSets,
                };
                return Vec::new();
            }
            KeyCode::Enter if model.policy.pane == PolicyPane::RuleSets => {
                return deny_fallback(model, CacheMode::Use);
            }
            KeyCode::Enter if model.policy.pane == PolicyPane::Rules => {
                if let Some(r) = model.policy.rules.selected_item() {
                    model.overlay = Overlay::RuleDetail(r.view.clone());
                }
                return Vec::new();
            }
            _ => {}
        }
    }
    let before = model.policy.selected_href();
    match model.policy.pane {
        PolicyPane::RuleSets => {
            let t = &mut model.policy.rulesets;
            if t.is_searching() {
                search_key(t, k);
            } else {
                table_key(t, k, page);
            }
            t.ensure_visible(page);
        }
        PolicyPane::Rules => {
            let t = &mut model.policy.rules;
            if t.is_searching() {
                search_key(t, k);
            } else {
                table_key(t, k, page);
            }
            t.ensure_visible(page);
        }
    }
    if model.policy.selected_href() != before
        && let Some(idx) = model.index().cloned()
    {
        model
            .policy
            .rebuild_rules(&idx, model.workloads.impact_source());
        return deny_fallback(model, CacheMode::Use);
    }
    Vec::new()
}

/// Loads every fetched kind that has neither arrived nor is in flight, and
/// the IP lists when their tab is the current one. Needs the name index: it
/// is built from the same four lists, so these are cache hits and the screen
/// costs no request.
fn load_missing_objects(model: &mut Model) -> Vec<Cmd> {
    if model.index().is_none() {
        return Vec::new();
    }
    let missing: Vec<ObjectKind> = ObjectKind::FETCHED
        .into_iter()
        .filter(|k| model.objects.ages[k.index()].is_none() && !model.objects.loading[k.index()])
        .collect();
    let mut cmds: Vec<Cmd> = missing
        .into_iter()
        .flat_map(|k| load_objects(model, k, CacheMode::Use))
        .collect();
    if model.objects.kind == ObjectKind::IpLists {
        cmds.extend(load_missing_ip_lists(model));
    }
    cmds
}

fn load_objects(model: &mut Model, kind: ObjectKind, mode: CacheMode) -> Vec<Cmd> {
    if kind == ObjectKind::IpLists {
        return load_ip_lists(model, mode);
    }
    model.objects.loading[kind.index()] = true;
    model.objects.auto.last_run = Some(Instant::now());
    model.objects.auto.from_tick = false;
    vec![Cmd::LoadObjects { kind, mode }]
}

/// Loads the workloads once. Waits for the name index like Objects does, so the
/// startup load never races a second request for the same data.
fn load_missing_workloads(model: &mut Model) -> Vec<Cmd> {
    if model.index().is_none() || model.workloads.loaded_at.is_some() || model.workloads.loading {
        return Vec::new();
    }
    load_workloads(model, CacheMode::Use)
}

fn load_workloads(model: &mut Model, mode: CacheMode) -> Vec<Cmd> {
    model.workloads.loading = true;
    model.workloads.auto.last_run = Some(Instant::now());
    model.workloads.auto.from_tick = false;
    vec![Cmd::LoadWorkloads(mode)]
}

/// `i` on Policy and in a rule detail (and, from Task 6, the confirm
/// overlay): the workload list behind the impact, read once. A list already cached or on its way costs
/// nothing.
fn impact_key(model: &mut Model) -> Vec<Cmd> {
    if !model.is_connected() {
        return Vec::new();
    }
    if model.workloads.loading {
        model.status = StatusLine::info("workloads already loading");
        return Vec::new();
    }
    if model.workloads.loaded_at.is_some() {
        model.status = StatusLine::info(format!(
            "{} workloads already loaded; r on Workloads (4) refreshes them",
            model.workloads.raw.len()
        ));
        return Vec::new();
    }
    model.workloads.error = None;
    model.status = StatusLine::info("loading workloads for impact (1 request)");
    load_workloads(model, CacheMode::Use)
}

fn ip_lists_page(model: &Model) -> usize {
    use crate::tui::ui::screens::{iplists, objects};
    let area = ratatui::layout::Rect::new(0, 0, model.size.0, model.size.1);
    crate::tui::ui::layout::chrome(area)
        .map(|c| {
            iplists::table_rows(
                objects::strip_split(c.body).1,
                model.objects.ip_lists.show_detail,
            )
        })
        .unwrap_or(10)
        .max(1)
}

/// Loads the IP lists once (the Objects screen's IP lists tab). Waits for the
/// name index like the other kinds: the index reads the same list, so after
/// it the load is a cache hit.
fn load_missing_ip_lists(model: &mut Model) -> Vec<Cmd> {
    if model.index().is_none()
        || model.objects.ip_lists.loaded_at.is_some()
        || model.objects.ip_lists.loading
    {
        return Vec::new();
    }
    load_ip_lists(model, CacheMode::Use)
}

fn load_ip_lists(model: &mut Model, mode: CacheMode) -> Vec<Cmd> {
    model.objects.ip_lists.loading = true;
    model.objects.ip_lists.auto.last_run = Some(Instant::now());
    model.objects.ip_lists.auto.from_tick = false;
    vec![Cmd::LoadIpLists(mode)]
}

fn ip_lists_key(model: &mut Model, k: KeyEvent) -> Vec<Cmd> {
    let page = ip_lists_page(model);
    if !model.objects.ip_lists.table.is_searching() {
        match k.code {
            KeyCode::Char('r') => {
                if model.index().is_none() {
                    if model.objects.index_failed {
                        model.objects.index_failed = false;
                        model.workloads.index_failed = false;
                        model.status = StatusLine::info("loading name index…");
                        return vec![Cmd::LoadIndex(CacheMode::Use)];
                    }
                    model.status = StatusLine::info("objects are still loading");
                    return Vec::new();
                }
                if model.objects.ip_lists.loading {
                    model.status = StatusLine::info("IP lists already loading");
                    return Vec::new();
                }
                model.objects.ip_lists.auto.rearm();
                return load_ip_lists(model, CacheMode::Bypass);
            }
            KeyCode::Enter => {
                let s = &mut model.objects.ip_lists;
                s.show_detail = !s.show_detail && s.table.selected_item().is_some();
                return Vec::new();
            }
            KeyCode::Char('a') => {
                model.status = model.objects.ip_lists.auto.toggle();
                return Vec::new();
            }
            KeyCode::Char(c @ ('n' | 'e' | 'x')) => return ip_lists_edit_key(model, c),
            _ => {}
        }
    }
    drive_table(&mut model.objects.ip_lists.table, k, page);
    Vec::new()
}

/// `n` / `e` / `x` on IP lists, through the Plan 4 intents.
fn ip_lists_edit_key(model: &mut Model, c: char) -> Vec<Cmd> {
    if writes_refused(model) {
        return Vec::new();
    }
    let selected = model
        .objects
        .ip_lists
        .table
        .selected_item()
        .map(|r| r.list.clone());
    let intent = match c {
        'n' => Some(EditIntent::CreateIpList),
        'e' => selected.map(EditIntent::UpdateIpList),
        _ => selected.map(EditIntent::DeleteIpList),
    };
    let Some(intent) = intent else {
        model.status = StatusLine::warn("no row selected");
        return Vec::new();
    };
    begin_edit(model, intent)
}

/// Sent but not answered, whether or not its audit entry was written.
fn is_unanswered(e: &WriteError) -> bool {
    match e {
        WriteError::Unanswered(_) => true,
        WriteError::AuditFailed { write, .. } => is_unanswered(write),
        _ => false,
    }
}

/// `q`, `Ctrl-C`, `:q`: refused while a batch is being sent, so no
/// request on the wire is dropped before its answer is audited.
fn quit(model: &mut Model) -> Vec<Cmd> {
    if model.batch_sending {
        model.status = StatusLine::warn("batch sending: wait for it to finish");
        return Vec::new();
    }
    model.quit = true;
    vec![Cmd::Quit]
}

/// Whether a planned batch was planned on this session's profile and PCE
/// (and no switch is on its way).
fn batch_for_session(model: &Model, batch: &pce::write::Batch) -> bool {
    let p = &model.profile;
    !model.switching
        && batch
            .origin
            .as_ref()
            .is_some_and(|o| o.profile == p.name && o.host == p.host && o.org_id == p.org_id)
}

/// Whether a planned batch may open its confirm: only over the generator
/// that planned it (still open, still planning).
fn batch_lands(model: &Model) -> bool {
    model.overlay == Overlay::Generator && model.gen_view.as_ref().is_some_and(|v| v.planning)
}

/// A batch could not be planned: nothing was sent; the generator that
/// planned it (still open) keeps its state and shows why.
fn batch_plan_failed(model: &mut Model, e: &str, planned_here: bool) {
    match model.gen_view.as_mut() {
        Some(v) if planned_here => {
            v.error = Some(e.to_string());
            model.status = StatusLine::error(format!("plan: {e}"));
        }
        _ => fail(model, "plan", e, false),
    }
}

/// The batch confirm or its report is done: back to the generator (the
/// drafts it reads load again after a write: one request, counted), else
/// to the screen.
fn batch_closed(model: &mut Model) -> Vec<Cmd> {
    if !std::mem::take(&mut model.batch_from_gen) || model.gen_view.is_none() {
        model.overlay = Overlay::None;
        return Vec::new();
    }
    model.overlay = Overlay::Generator;
    // The batch's outcome stays on the status line, whatever loads.
    let status = model.status.clone();
    let cmds = generator::ensure_drafts(model);
    // Rows, gap and keys again: traffic may have landed meanwhile.
    generator::rebuild_gen(model);
    if model.overlay == Overlay::Generator {
        model.status = status;
    }
    cmds
}

/// A written step whose audit entry could not be written (why).
fn not_audited(r: &pce::write::StepResult) -> Option<&str> {
    match r {
        pce::write::StepResult::Written(w) => w.audit_failed.as_deref(),
        _ => None,
    }
}

/// `wrote 5 of 7 (pending); step 6 failed: …; 1 not sent`, then what the
/// PCE dropped or warned about per written step, and whether the batch
/// stopped (`Some(true)`), only warned (`Some(false)`) or neither.
fn batch_status(results: &[pce::write::StepResult]) -> (String, Option<bool>) {
    use pce::write::StepResult as R;
    let count = |f: fn(&R) -> bool| results.iter().filter(|r| f(r)).count();
    let written = count(|r| matches!(r, R::Written(_)));
    let reused = count(|r| matches!(r, R::Skipped { .. }));
    let not_sent = count(|r| matches!(r, R::NotSent));
    let mut s = format!("wrote {written} of {} (pending)", results.len());
    if reused > 0 {
        s.push_str(&format!("; reused {reused}"));
    }
    let failed = results.iter().enumerate().find_map(|(i, r)| match r {
        R::Failed(e) => Some((i, e)),
        _ => None,
    });
    if let Some((i, e)) = failed {
        if is_unanswered(e) {
            s.push_str(&format!(
                "; step {} not answered: it may have applied; reloading",
                i + 1
            ));
        } else {
            s.push_str(&format!("; step {} failed: {e}", i + 1));
        }
    }
    let unaudited = results
        .iter()
        .enumerate()
        .find_map(|(i, r)| not_audited(r).map(|why| (i, why)));
    if let Some((i, why)) = unaudited {
        s.push_str(&format!(
            "; step {} written but not audited ({why}): batch stopped",
            i + 1
        ));
    }
    if not_sent > 0 {
        s.push_str(&format!("; {not_sent} not sent"));
    }
    let mut noted = false;
    for (i, r) in results.iter().enumerate() {
        let R::Written(w) = r else { continue };
        if !w.dropped_fields.is_empty() {
            noted = true;
            s.push_str(&format!(
                "; step {}: the PCE dropped {}",
                i + 1,
                w.dropped_fields.join(", ")
            ));
        }
        for warning in &w.warnings {
            if w.audit_failed.as_ref() == Some(warning) {
                continue;
            }
            noted = true;
            s.push_str(&format!("; step {}: {warning}", i + 1));
        }
    }
    let kind = match (failed.is_some() || unaudited.is_some(), noted) {
        (true, _) => Some(true),
        (false, true) => Some(false),
        (false, false) => None,
    };
    (s, kind)
}

/// The batch report: each step's summary and what became of it (the href
/// written or reused, the failure, not sent), then how to undo and resume.
fn batch_report_lines(
    target: &str,
    summaries: &[String],
    results: &[pce::write::StepResult],
) -> Vec<String> {
    use pce::write::StepResult as R;
    let n = results.len();
    let mut out = vec![format!("sent to {target}")];
    for (i, r) in results.iter().enumerate() {
        let what = summaries.get(i).map_or("?", String::as_str);
        out.push(format!("{}/{n} {what}", i + 1));
        out.push(match r {
            R::Written(w) => match &w.audit_failed {
                Some(why) => format!("    written {}, NOT AUDITED: {why}", w.href),
                None => format!("    written {}", w.href),
            },
            R::Skipped { existing_href } => format!("    reused {existing_href}"),
            R::Failed(e) if is_unanswered(e) => {
                format!("    NOT ANSWERED, it may have applied: {e}")
            }
            R::Failed(e) => format!("    FAILED: {e}"),
            R::NotSent => "    not sent".into(),
        });
    }
    let touched = results.iter().any(|r| match r {
        R::Written(_) => true,
        R::Failed(e) => is_unanswered(e),
        _ => false,
    });
    let stopped = results
        .iter()
        .any(|r| matches!(r, R::Failed(_) | R::NotSent) || not_audited(r).is_some());
    out.push(String::new());
    if touched {
        out.push(
            "These are draft changes, not provisioned: p on Policy lists them, R reverts the marked ones."
                .into(),
        );
    }
    if stopped {
        out.push("Planning the batch again resumes it: what was written then exists.".into());
    }
    out
}

/// A sent batch's answer: the status line and the report (opened when the
/// batch stopped, else `:batch` shows it), then one reload of the
/// rulesets (and of the IP lists when one was written, so the index
/// follows) and one draft re-evaluation, whatever the step count. An
/// overlay opened while it was sent stays.
fn batch_done(
    model: &mut Model,
    kinds: &[pce::write::ObjectKind],
    summaries: &[String],
    target: &str,
    results: &[pce::write::StepResult],
) -> Vec<Cmd> {
    use pce::write::StepResult as R;
    model.writes.in_flight = false;
    model.batch_sending = false;
    // The runtime dropped `policy:pending`; the next `p` fetches again.
    model.policy.pending = None;
    model.policy.pending_marked.clear();
    let (text, kind) = batch_status(results);
    let stopped = kind == Some(true) || results.iter().any(|r| matches!(r, R::NotSent));
    model.batch_report = Some(batch_report_lines(target, summaries, results));
    model.status = match kind {
        Some(true) => StatusLine::error(text),
        Some(false) => StatusLine::warn(text),
        None => StatusLine::info(text),
    };
    // Written, or sent without an answer (it may have applied).
    let touched = |r: &R| match r {
        R::Written(_) => true,
        R::Failed(e) => is_unanswered(e),
        R::Skipped { .. } | R::NotSent => false,
    };
    let mut cmds = Vec::new();
    if results.iter().any(touched) {
        // Once per batch, not per step.
        let redraft = redraft_after_write(model, true, false);
        let lists = kinds
            .iter()
            .zip(results)
            .any(|(k, r)| *k == pce::write::ObjectKind::IpList && touched(r));
        if lists {
            cmds.extend(reload_kind_after_write(
                model,
                pce::write::ObjectKind::IpList,
            ));
        }
        cmds.extend(reload_after_write(model, pce::write::ObjectKind::Rule));
        cmds.extend(redraft);
    }
    if model.overlay != Overlay::None {
        // Whatever opened meanwhile closes onto the screen.
        model.batch_from_gen = false;
        model.status.text.push_str(" · :batch shows each step");
    } else if stopped {
        model.overlay = Overlay::BatchReport { scroll: 0 };
    } else {
        model.status.text.push_str(" · :batch shows each step");
        cmds.extend(batch_closed(model));
    }
    cmds
}

/// One reload of the written kind's table, past the cache (the writer
/// already invalidated it).
fn reload_after_write(model: &mut Model, kind: pce::write::ObjectKind) -> Vec<Cmd> {
    let mut cmds = reload_kind_after_write(model, kind);
    cmds.extend(usage_after_write(model));
    cmds
}

/// Any write may change what holds a label (a workload's labels, a rule's
/// actors, a group's members): the PCE's usage flags are forgotten, and
/// with `u` on asked for again, past the cache. A PCE without flags stays so.
fn usage_after_write(model: &mut Model) -> Vec<Cmd> {
    let o = &mut model.objects;
    if o.pce_usage == PceUsage::Unsupported {
        return Vec::new();
    }
    o.pce_usage = PceUsage::Unknown;
    o.rebuild_labels(&model.limits.key_order());
    if !o.unused_only {
        return Vec::new();
    }
    o.pce_usage_loading = true;
    vec![Cmd::LoadLabelUsage(CacheMode::Bypass)]
}

/// `r` on the Labels tab while the PCE's flags are held: they are read
/// again with the labels, past the cache.
fn reload_usage_with_labels(model: &mut Model) -> Option<Cmd> {
    let o = &mut model.objects;
    if !matches!(o.pce_usage, PceUsage::Flags(_)) || o.pce_usage_loading {
        return None;
    }
    o.pce_usage_loading = true;
    Some(Cmd::LoadLabelUsage(CacheMode::Bypass))
}

/// A policy write changed the drafts: the held ones go, and the answer to
/// a form's load still on its way (read before the write) is dropped.
fn forget_form_drafts(model: &mut Model) {
    model.policy.form_drafts = None;
    model.policy.form_drafts_stale = model.policy.form_drafts_loading;
}

/// The reload of the written kind's own list.
fn reload_kind_after_write(model: &mut Model, kind: pce::write::ObjectKind) -> Vec<Cmd> {
    match kind {
        pce::write::ObjectKind::Label => load_objects(model, ObjectKind::Labels, CacheMode::Bypass),
        // Nothing writes label groups yet; the tab has no reload of its own.
        pce::write::ObjectKind::LabelGroup => Vec::new(),
        pce::write::ObjectKind::IpList => load_ip_lists(model, CacheMode::Bypass),
        pce::write::ObjectKind::Service => {
            load_objects(model, ObjectKind::Services, CacheMode::Bypass)
        }
        pce::write::ObjectKind::Workload => load_workloads(model, CacheMode::Bypass),
        // A draft write changes the draft view only; the reload keeps the
        // write's report in the status line.
        pce::write::ObjectKind::RuleSet
        | pce::write::ObjectKind::Rule
        | pce::write::ObjectKind::DenyRule => {
            // The next `w` form loads the changed drafts (one GET).
            forget_form_drafts(model);
            if model.policy.scope != PolicyScope::Draft {
                return Vec::new();
            }
            model.policy.quiet_reload = true;
            load_rule_sets(model, CacheMode::Bypass)
        }
        // Provisioning changes both scopes and the pending list; a revert
        // changes the draft and the pending list.
        pce::write::ObjectKind::Provision | pce::write::ObjectKind::Revert => {
            forget_form_drafts(model);
            model.policy.quiet_reload = true;

            let mut cmds = load_rule_sets(model, CacheMode::Bypass);
            model.policy.pending_loading = true;
            cmds.push(Cmd::LoadPending(CacheMode::Bypass));
            cmds
        }
    }
}

/// `Enter` on the pending overlay: one diff request for an IP list, a
/// service or a ruleset; anything else says why there is none.
fn pending_diff(model: &mut Model) -> Vec<Cmd> {
    let Some(change) = model
        .policy
        .pending
        .as_ref()
        .and_then(|p| p.get(model.policy.pending_selected))
        .cloned()
    else {
        return Vec::new();
    };
    let Some(kind) = crate::tui::diff::diff_object_kind(&change.object_type) else {
        model.status = StatusLine::info(match change.object_type.as_str() {
            // A rule is provisioned, and diffed, through its ruleset.
            "sec_rules" | "deny_rules" => "rule changes show in their ruleset's diff".to_string(),
            other => format!("no diff for {other}"),
        });
        return Vec::new();
    };
    if model.policy.diff_loading {
        model.status = StatusLine::info("diff already loading");
        return Vec::new();
    }
    model.policy.diff_loading = true;
    model.status = StatusLine::info(format!(
        "loading the active version of {}",
        change.name.as_deref().unwrap_or(&change.href)
    ));
    // The deny rules the Policy screen fetched for a ruleset whose list
    // payload had none: the diff reads them from the model, not the PCE.
    let fallback_deny = (kind == pce::write::ObjectKind::RuleSet)
        .then(|| model.policy.fallback_deny.get(&change.href).cloned())
        .flatten();
    vec![Cmd::LoadDiff {
        change,
        kind,
        fallback_deny,
    }]
}

const PROVISION_DESCRIPTION: &str = "illumio-tui provision";

/// `P` in the pending overlay: provision the marked entries, or everything
/// pending when none is marked. The plan reads the pending list once more,
/// past the cache, and the confirm overlay shows the exact body.
fn provision_key(model: &mut Model) -> Vec<Cmd> {
    if writes_refused(model) {
        return Vec::new();
    }
    if model.policy.pending.as_ref().is_none_or(Vec::is_empty) {
        model.status = StatusLine::info("nothing pending");
        return Vec::new();
    }
    let marked = &model.policy.pending_marked;
    let subset = if marked.is_empty() {
        ProvisionSubset::All
    } else {
        ProvisionSubset::Hrefs(marked.iter().cloned().collect())
    };
    model.overlay = Overlay::None;
    model.status = StatusLine::info("planning…");
    model.writes.in_flight = true;
    vec![Cmd::PlanWrite(WriteIntent::Provision(ProvisionRequest {
        description: Some(PROVISION_DESCRIPTION.into()),
        subset,
    }))]
}

/// `R` in the pending overlay: revert the marked entries to their active
/// version (a draft-only object is deleted). Never everything: with nothing
/// marked it only says how. The plan reads the pending list once more, past
/// the cache, and the confirm names each object and its fate.
fn revert_key(model: &mut Model) -> Vec<Cmd> {
    if writes_refused(model) {
        return Vec::new();
    }
    if model.policy.pending_marked.is_empty() {
        model.status = StatusLine::info("mark changes with Space first");
        return Vec::new();
    }
    let targets = model.policy.pending_marked.iter().cloned().collect();
    model.overlay = Overlay::None;
    model.status = StatusLine::info("planning…");
    model.writes.in_flight = true;
    vec![Cmd::PlanWrite(WriteIntent::Revert(RevertRequest {
        all: false,
        targets,
    }))]
}

/// A revert's object lines name a ruleset pending only through its rules
/// by href (the pending list has no name for it): the loaded rulesets
/// (either scope) give the name when they hold it. Display only: the lines
/// are neither sent nor audited.
fn name_rulesets(plan: &mut Plan, sets: &[pce::models::RuleSet]) {
    let Plan::Send { request, .. } = plan else {
        return;
    };
    for line in &mut request.objects {
        let Some(rest) = line.strip_prefix("rule_set /") else {
            continue;
        };
        let href = format!("/{}", rest.split(' ').next().unwrap_or_default());
        let named = sets.iter().find(|rs| {
            rs.href
                .replacen("/sec_policy/active/", "/sec_policy/draft/", 1)
                == href
        });
        if let Some(rs) = named {
            *line = line.replacen(&href, &format!("{:?}", rs.name), 1);
        }
    }
}

/// The word typed into a revert confirm before `Enter` sends it.
pub const REVERT_WORD: &str = "revert";

/// Keys of the revert confirm: letters type the word, `Enter` sends only
/// when it reads `revert`, `Esc` cancels, arrows scroll. `y` is a letter.
fn revert_confirm_key(model: &mut Model, plan: Plan, scroll: u16, code: KeyCode) -> Vec<Cmd> {
    let kind = pce::write::ObjectKind::Revert;
    let area = ratatui::layout::Rect::new(0, 0, model.size.0, model.size.1);
    let last = confirm_write_last_scroll(&plan, kind, area);
    let last = u16::try_from(last).unwrap_or(u16::MAX);
    let typed = &mut model.policy.revert_typed;
    match code {
        KeyCode::Enter if typed == REVERT_WORD => {
            typed.clear();
            model.policy.provision_counts =
                Some(counts_text(plan.request().and_then(|r| r.body.as_ref())));
            model.overlay = Overlay::None;
            model.status = StatusLine::info("sending …");
            model.writes.in_flight = true;
            return vec![Cmd::ExecuteWrite { plan, kind }];
        }
        KeyCode::Enter => {
            model.status = StatusLine::warn("type revert, then Enter, to send it");
        }
        KeyCode::Esc => {
            typed.clear();
            model.overlay = Overlay::None;
            model.status = StatusLine::info("cancelled, nothing sent");
        }
        KeyCode::Backspace => {
            typed.pop();
        }
        KeyCode::Char(c) if typed.chars().count() < REVERT_WORD.chars().count() + 8 => {
            typed.push(c)
        }
        KeyCode::Down => {
            model.overlay = Overlay::ConfirmWrite {
                plan,
                kind,
                scroll: (scroll + 1).min(last),
            };
        }
        KeyCode::Up => {
            model.overlay = Overlay::ConfirmWrite {
                plan,
                kind,
                scroll: scroll.saturating_sub(1),
            };
        }
        _ => {}
    }
    Vec::new()
}

/// `ip_lists=1, rule_sets=2`: what a provision body lists, per type, as the
/// CLI prints it.
fn counts_text(body: Option<&serde_json::Value>) -> String {
    body.map(provision_counts)
        .unwrap_or_default()
        .iter()
        .map(|(k, n)| format!("{k}={n}"))
        .collect::<Vec<_>>()
        .join(", ")
}

fn workloads_page(model: &Model) -> usize {
    let area = ratatui::layout::Rect::new(0, 0, model.size.0, model.size.1);
    crate::tui::ui::layout::chrome(area)
        .map(|c| crate::tui::ui::screens::workloads::table_rows(c.body, &model.workloads))
        .unwrap_or(10)
        .max(1)
}

/// What the traffic filter calls this workload: its readable name when that
/// resolves to exactly this workload, else its href id. `,` and `|` separate
/// filter terms, and the resolver takes the first hostname, name or href-suffix
/// match, so a name with those characters or shared with another loaded
/// workload is addressed by the id.
fn workload_filter_ref(w: &pce::models::Workload, loaded: &[pce::models::Workload]) -> String {
    let name = w.display_name();
    let ambiguous = loaded.iter().any(|o| {
        o.href != w.href
            && (o
                .hostname
                .as_deref()
                .is_some_and(|h| h.eq_ignore_ascii_case(&name))
                || o.name
                    .as_deref()
                    .is_some_and(|n| n.eq_ignore_ascii_case(&name))
                || o.href
                    .to_ascii_lowercase()
                    .ends_with(&name.to_ascii_lowercase()))
    });
    let name = if ambiguous || name.contains([',', '|']) {
        pce::models::Ref::new(&w.href).id().to_string()
    } else {
        name
    };
    format!("wl:{name}")
}

fn workloads_key(model: &mut Model, k: KeyEvent) -> Vec<Cmd> {
    let page = workloads_page(model);
    if !model.workloads.table.is_searching() {
        match k.code {
            KeyCode::Char('r') => {
                if model.index().is_none() {
                    if model.workloads.index_failed {
                        model.objects.index_failed = false;
                        model.workloads.index_failed = false;
                        model.status = StatusLine::info("loading name index…");
                        return vec![Cmd::LoadIndex(CacheMode::Use)];
                    }
                    model.status = StatusLine::info("workloads are still loading");
                } else if model.workloads.loading {
                    model.status = StatusLine::info("workloads already loading");
                } else {
                    model.workloads.auto.rearm();
                    return load_workloads(model, CacheMode::Bypass);
                }
                return Vec::new();
            }
            KeyCode::Enter => {
                if let Some(g) = model.workloads.selected_group().cloned() {
                    return narrow_to_group(model, &g);
                }
                let w = &mut model.workloads;
                w.show_detail = !w.show_detail && w.selected_row().is_some();
                return Vec::new();
            }
            KeyCode::Char('x') => {
                let keys = model.workload_group_keys();
                let w = &mut model.workloads;
                w.toggle_grouped(keys);
                model.status = StatusLine::info(if model.workloads.grouped {
                    format!("grouped: {}", model.workloads.group_keys.join(","))
                } else {
                    "flat".to_string()
                });
                let page = workloads_page(model);
                model.workloads.ensure_visible(page);
                return Vec::new();
            }
            KeyCode::Char('f') => {
                let mut e = LabelFilterEditor::new(&model.workloads.labels.to_string());
                label_editor_unknown(model, &mut e);
                model.overlay = Overlay::LabelFilter(e);
                return Vec::new();
            }
            KeyCode::Char(' ') if model.workloads.grouped => {
                model.workloads.toggle_fold();
                model.workloads.ensure_visible(page);
                return Vec::new();
            }
            KeyCode::Char('z') if model.workloads.grouped => {
                model.workloads.toggle_fold_all();
                model.workloads.ensure_visible(page);
                return Vec::new();
            }
            KeyCode::Char('s') if model.workloads.grouped => {
                let order = model.workloads.cycle_group_order();
                model.status = StatusLine::info(format!("groups {}", order.label()));
                model.workloads.ensure_visible(page);
                return Vec::new();
            }
            KeyCode::Char(c @ ('h' | 'E' | 'm' | 'o' | 'u' | 'c')) => {
                // Local filters over the loaded list: no request.
                let order = model.limits.key_order();
                if c == 'u' && !model.workloads.filters.unlabeled && order.first().is_empty() {
                    model.status = StatusLine::info("no label model keys");
                    return Vec::new();
                }
                let w = &mut model.workloads;
                let href = w.selected_row().map(|r| r.workload.href.clone());
                match c {
                    'h' => w.unhealthy_only = !w.unhealthy_only,
                    'E' => w.filters.next_enforcement(),
                    'm' => w.filters.next_managed(),
                    'o' => w.filters.offline = !w.filters.offline,
                    'u' => w.filters.unlabeled = !w.filters.unlabeled,
                    _ => w.clear_filters(),
                }
                w.rebuild(&order);
                // The same workload while it is still shown.
                if let Some(href) = href {
                    w.select_href(&href);
                }
                w.show_detail &= w.selected_row().is_some();
                // The filter line comes and goes: the page size with it.
                let page = workloads_page(model);
                model.workloads.ensure_visible(page);
                return Vec::new();
            }
            KeyCode::Char('H') => {
                model.workloads.sort_by_health();
                return Vec::new();
            }
            KeyCode::Char('a') => {
                model.status = model.workloads.auto.toggle();
                return Vec::new();
            }
            KeyCode::Char(c @ ('t' | 'T')) => {
                let Some(w) = model.workloads.selected_row() else {
                    return Vec::new();
                };
                let expr = workload_filter_ref(&w.workload, &model.workloads.raw);
                let mut f = model.traffic.filter.clone();
                if c == 't' {
                    f.src = expr;
                } else {
                    f.dst = expr;
                }
                // The switch to Traffic happens when the query starts (`ApplyFilter`),
                // so declining the cost guard leaves the user on Workloads.
                return request_query(model, f);
            }
            // `x` groups here: delete is `X`.
            KeyCode::Char(c @ ('n' | 'e' | 'X')) => return workloads_edit_key(model, c),
            _ => {}
        }
    }
    let w = &mut model.workloads;
    if !w.grouped {
        drive_table(&mut w.table, k, page);
        return Vec::new();
    }
    // Grouped: `/` searches the workloads (the sections follow), the
    // rest moves in the sections.
    if w.table.is_searching() {
        search_key(&mut w.table, k);
        w.regroup();
    } else if k.code == KeyCode::Char('/') {
        w.table.start_search();
    } else {
        table_key(&mut w.sections, k, page);
    }
    w.sections.ensure_visible(page);
    Vec::new()
}

/// The editor's unknown keys and values for its current text (none while
/// it does not parse): once per edit, never per frame.
fn label_editor_unknown(model: &Model, e: &mut LabelFilterEditor) {
    let idx = model.index().cloned();
    e.unknown = e
        .parse()
        .map(|f| model.workloads.unknown_labels(&f, idx.as_deref()))
        .unwrap_or_default();
}

/// `Enter` on a section header: the label filter narrowed to the group
/// (`app=shop env=prod`; a missing label as `key=-`), other terms kept.
fn narrow_to_group(model: &mut Model, g: &crate::tui::wl_groups::WlGroup) -> Vec<Cmd> {
    let pairs = pce::label_filter::group_pairs(&g.key, &model.workloads.group_keys);
    let labels = model.workloads.labels.narrowed(&pairs);
    let order = model.limits.key_order();
    let idx = model.index().cloned();
    let w = &mut model.workloads;
    w.set_labels(labels, idx.as_deref(), &order);
    w.sections.home();
    model.status = StatusLine::info(format!("labels {}", model.workloads.labels));
    let page = workloads_page(model);
    model.workloads.ensure_visible(page);
    Vec::new()
}

/// Applies a label filter typed in `f` or `:filter` on Workloads: local,
/// never a request. Unknown keys or values are kept and named.
pub(crate) fn apply_workload_labels(model: &mut Model, expr: &str) -> Result<(), String> {
    let labels = pce::LabelFilter::parse(expr)?;
    let order = model.limits.key_order();
    let idx = model.index().cloned();
    let w = &mut model.workloads;
    let href = w.selected_row().map(|r| r.workload.href.clone());
    w.set_labels(labels, idx.as_deref(), &order);
    if let Some(href) = href {
        w.select_href(&href);
    }
    w.show_detail &= w.selected_row().is_some();
    let w = &model.workloads;
    model.status = if w.labels.is_empty() {
        StatusLine::info("label filter cleared")
    } else if w.label_warnings.is_empty() {
        StatusLine::info(format!("labels {} · {} workloads", w.labels, w.table.len()))
    } else {
        StatusLine::warn(format!(
            "labels {} · no match: {}",
            w.labels,
            w.label_warnings.join(", ")
        ))
    };
    let page = workloads_page(model);
    model.workloads.ensure_visible(page);
    Ok(())
}

/// Loads the events once. Unlike the other lists they are not in the startup
/// index, and they need no name index: a connection is enough.
fn load_missing_events(model: &mut Model) -> Vec<Cmd> {
    if !model.is_connected() || model.events.loaded_at.is_some() || model.events.loading {
        return Vec::new();
    }
    load_events(model, CacheMode::Use)
}

/// Requests the events for the applied filter. A filter that no longer
/// validates (it was validated when applied) is reported, never sent.
fn load_events(model: &mut Model, mode: CacheMode) -> Vec<Cmd> {
    match model.events.filter.to_filter(jiff::Timestamp::now()) {
        Ok(filter) => request_events(model, filter, mode),
        Err(e) => {
            model.status = StatusLine::error(e);
            Vec::new()
        }
    }
}

fn request_events(model: &mut Model, filter: EventFilter, mode: CacheMode) -> Vec<Cmd> {
    model.events.loading = true;
    model.events.auto.last_run = Some(Instant::now());
    model.events.auto.from_tick = false;
    vec![Cmd::LoadEvents { filter, mode }]
}

fn events_page(model: &Model) -> usize {
    let area = ratatui::layout::Rect::new(0, 0, model.size.0, model.size.1);
    crate::tui::ui::layout::chrome(area)
        .map(|c| crate::tui::ui::screens::events::table_rows(c.body))
        .unwrap_or(10)
        .max(1)
}

fn events_key(model: &mut Model, k: KeyEvent) -> Vec<Cmd> {
    let page = events_page(model);
    if !model.events.table.is_searching() {
        match k.code {
            KeyCode::Char('f') => {
                let f = &model.events.filter;
                model.overlay = Overlay::EventsEditor {
                    values: [f.severity.clone(), f.event_type.clone(), f.window.clone()],
                    active: 0,
                    error: None,
                };
                return Vec::new();
            }
            KeyCode::Char('r') => {
                if model.events.loading {
                    model.status = StatusLine::info("events already loading");
                    return Vec::new();
                }
                model.events.auto.rearm();
                return load_events(model, CacheMode::Bypass);
            }
            KeyCode::Enter => {
                if let Some(e) = model.events.table.selected_item() {
                    model.overlay = Overlay::EventDetail {
                        event: e.clone(),
                        scroll: 0,
                        raw: false,
                    };
                }
                return Vec::new();
            }
            KeyCode::Char('S') => {
                model.events.sort_by_severity();
                return Vec::new();
            }
            KeyCode::Char('a') => {
                model.status = model.events.auto.toggle();
                return Vec::new();
            }
            _ => {}
        }
    }
    drive_table(&mut model.events.table, k, page);
    Vec::new()
}

fn objects_page(model: &Model) -> usize {
    let area = ratatui::layout::Rect::new(0, 0, model.size.0, model.size.1);
    crate::tui::ui::layout::chrome(area)
        .map(|c| crate::tui::ui::screens::objects::table_rows(c.body, model.objects.show_detail))
        .unwrap_or(10)
        .max(1)
}

/// Search editing, navigation and sort on one table, keeping the selection in view.
fn drive_table<T>(table: &mut crate::tui::table::TableView<T>, k: KeyEvent, page: usize) {
    if table.is_searching() {
        search_key(table, k);
    } else {
        table_key(table, k, page);
    }
    table.ensure_visible(page);
}

fn objects_key(model: &mut Model, k: KeyEvent) -> Vec<Cmd> {
    let page = objects_page(model);
    if !model.objects.is_searching() {
        let kind = model.objects.kind;
        match k.code {
            KeyCode::Tab | KeyCode::BackTab => {
                let o = &mut model.objects;
                o.kind = if k.code == KeyCode::Tab {
                    kind.next()
                } else {
                    kind.prev()
                };
                o.show_detail = false;
                o.ip_lists.show_detail = false;
                // The first visit to the IP lists tab loads them (a cache hit).
                return if o.kind == ObjectKind::IpLists {
                    load_missing_ip_lists(model)
                } else {
                    Vec::new()
                };
            }
            _ if kind == ObjectKind::IpLists => return ip_lists_key(model, k),
            KeyCode::Char('r') => {
                if model.index().is_none() {
                    if model.objects.index_failed {
                        model.objects.index_failed = false;
                        model.workloads.index_failed = false;
                        model.status = StatusLine::info("loading name index…");
                        return vec![Cmd::LoadIndex(CacheMode::Use)];
                    }
                    model.status = StatusLine::info("objects are still loading");
                } else if model.objects.loading[kind.index()] {
                    model.status = StatusLine::info(format!("{} already loading", kind.title()));
                } else {
                    model.objects.auto.rearm();
                    let mut cmds = load_objects(model, kind, CacheMode::Bypass);
                    if kind == ObjectKind::Labels {
                        cmds.extend(reload_usage_with_labels(model));
                    }
                    return cmds;
                }
                return Vec::new();
            }
            KeyCode::Enter => {
                let o = &mut model.objects;
                o.show_detail = !o.show_detail && o.has_selection();
                return Vec::new();
            }
            KeyCode::Char('a') => {
                model.status = model.objects.auto.toggle();
                return Vec::new();
            }
            KeyCode::Char(c @ ('n' | 'e' | 'x')) => return objects_edit_key(model, c),
            KeyCode::Char('f') if kind == ObjectKind::Labels => {
                model.overlay = Overlay::LabelKeys(open_label_key_picker(model));
                return Vec::new();
            }
            KeyCode::Char('i') if kind == ObjectKind::Labels => {
                toggle_ignored_labels(model);
                return Vec::new();
            }
            KeyCode::Char('u') if kind == ObjectKind::Labels => return unused_labels_key(model),
            _ => {}
        }
    }
    let o = &mut model.objects;
    match o.kind {
        ObjectKind::Labels => drive_table(&mut o.labels, k, page),
        ObjectKind::LabelGroups => drive_table(&mut o.groups, k, page),
        ObjectKind::Services => drive_table(&mut o.services, k, page),
        // A search on the IP lists tab: its own keys and page size.
        ObjectKind::IpLists => return ip_lists_key(model, k),
    }
    Vec::new()
}

/// `f` on the Labels tab: the profile's scope and tier keys, then every
/// key the index knows with its label count (ignored keys only while `i`
/// shows them); the current filter checked.
fn open_label_key_picker(model: &Model) -> KeyPicker {
    let order = model.limits.key_order();
    let hide = model.objects.hide_ignored;
    let seen: Vec<(String, usize)> = model
        .index()
        .map(|idx| {
            idx.label_keys()
                .into_iter()
                // Keys `i` hides are not offered while hidden.
                .filter(|k| !hide || order.shows(k))
                .map(|k| {
                    let n = idx.values_for_key(&k).len();
                    (k, n)
                })
                .collect()
        })
        .unwrap_or_default();
    let keys = &model.objects.label_keys;
    KeyPicker::new(
        &model.limits.scope_keys,
        &model.limits.tier_key,
        &seen,
        keys,
        keys.is_empty(),
    )
}

/// Enter in the label key picker: the Labels tab shows these keys (every
/// key when empty). No request.
fn apply_label_keys(model: &mut Model, keys: Vec<String>) {
    model.status = StatusLine::info(if keys.is_empty() {
        "labels: all keys".to_string()
    } else {
        format!("labels: key {}", keys.join(","))
    });
    model.objects.label_keys = keys;
    model.objects.rebuild_labels(&model.limits.key_order());
}

/// `i` on the Labels tab: show or hide the labels of the profile's `ignore_keys`.
fn toggle_ignored_labels(model: &mut Model) {
    if model.limits.ignore_keys.is_empty() {
        model.status = StatusLine::info("the profile has no ignore_keys");
        return;
    }
    let o = &mut model.objects;
    o.hide_ignored = !o.hide_ignored;
    o.rebuild_labels(&model.limits.key_order());
    let keys = model.limits.ignore_keys.join(",");
    model.status = StatusLine::info(if model.objects.hide_ignored {
        format!("ignored keys hidden ({keys})")
    } else {
        format!("ignored keys shown ({keys})")
    });
}

/// `u` on the Labels tab: unused labels only, or all again. The PCE's own
/// usage flags decide: one `usage=true` labels request per `u` (cached like
/// the labels), the filter applying when it answers (at once with flags
/// already in hand). A PCE without the flags falls back to our counts.
fn unused_labels_key(model: &mut Model) -> Vec<Cmd> {
    let o = &mut model.objects;
    if o.unused_only {
        o.unused_only = false;
        o.rebuild_labels(&model.limits.key_order());
        model.status = StatusLine::info("labels: all, used or not");
        return Vec::new();
    }
    if o.pce_usage == PceUsage::Unsupported {
        return counted_unused_key(model);
    }
    if !model.is_connected() {
        model.status = StatusLine::warn("unused labels: not connected");
        return Vec::new();
    }
    let o = &mut model.objects;
    o.unused_only = true;
    o.rebuild_labels(&model.limits.key_order());
    model.status = unused_status(&model.objects);
    if std::mem::replace(&mut model.objects.pce_usage_loading, true) {
        return Vec::new(); // already asked
    }
    vec![Cmd::LoadLabelUsage(CacheMode::Use)]
}

/// The PCE's usage answer: its flags decide `u`; without them (an older
/// PCE) `u` goes on with our own counts, asking for what they need.
fn label_usage_loaded(
    model: &mut Model,
    result: Result<Vec<pce::models::Label>, String>,
) -> Vec<Cmd> {
    let o = &mut model.objects;
    o.pce_usage_loading = false;
    match result {
        Ok(labels) => {
            o.pce_usage = PceUsage::from_labels(labels);
            if o.pce_usage == PceUsage::Unsupported && o.unused_only {
                o.unused_only = false;
                o.rebuild_labels(&model.limits.key_order());
                return counted_unused_key(model);
            }
            o.rebuild_labels(&model.limits.key_order());
            if o.unused_only {
                model.status = unused_status(&model.objects);
            }
        }
        Err(e) => {
            if o.unused_only && !o.unused_decided() {
                o.unused_only = false;
                o.rebuild_labels(&model.limits.key_order());
            }
            fail(model, "label usage", &e, false);
        }
    }
    Vec::new()
}

/// What `u` shows in the status line once on.
fn unused_status(o: &ObjectsState) -> StatusLine {
    use crate::cli::output::USAGE_FROM_COUNTS;
    let n = o.labels.len();
    match (&o.pce_usage, &o.label_usage, o.usage_truncated) {
        (PceUsage::Flags(_), ..) => StatusLine::info(format!("{n} unused labels")),
        (PceUsage::Unknown, ..) => {
            StatusLine::info("unused labels: asking the PCE what holds each label")
        }
        (PceUsage::Unsupported, None, _) => {
            StatusLine::info("unused labels: waiting for workloads and policy")
        }
        (PceUsage::Unsupported, Some(_), true) => StatusLine::warn(format!(
            "{n} unused labels; the workload list is truncated, some may be in use"
        )),
        (PceUsage::Unsupported, Some(_), false) => StatusLine::info(format!(
            "{n} unused labels (usage from {USAGE_FROM_COUNTS})"
        )),
    }
}

/// `u` from a PCE without usage flags: our counts need the workloads and
/// the rulesets; when one is neither cached nor on its way the reads wait
/// for a `y` (no request before it).
fn counted_unused_key(model: &mut Model) -> Vec<Cmd> {
    let missing = |loaded: bool, loading: bool| !loaded && !loading;
    let workloads = missing(model.workloads.loaded_at.is_some(), model.workloads.loading);
    let policy = missing(model.policy.loaded_at.is_some(), model.policy.loading);
    if workloads || policy {
        let reason = match (workloads, policy) {
            (true, false) => "unused needs workloads loaded: press y to load workloads".to_string(),
            (false, _) => "unused needs policy loaded: press y to load policy \
                 (it may also fetch the selected ruleset's deny rules)"
                .to_string(),
            (true, true) => "unused needs workloads and policy loaded: press y to load both \
                 (policy may also fetch the selected ruleset's deny rules)"
                .to_string(),
        };
        model.status = StatusLine::info(reason.clone());
        model.overlay = Overlay::Confirm {
            reason,
            action: PendingAction::LoadLabelUsage,
        };
        return Vec::new();
    }
    apply_unused(model);
    Vec::new()
}

/// Turns `unused_only` on once usage can be counted or is on its way;
/// otherwise leaves it off and says what is missing.
fn apply_unused(model: &mut Model) {
    if let Some(gap) = usage_gap(model) {
        model.status = StatusLine::warn(gap);
        return;
    }
    model.objects.unused_only = true;
    model.recount_labels();
    model.status = unused_status(&model.objects);
}

/// Why label usage cannot be counted: what is neither cached nor loading.
/// `None` when usage is ready or every missing part is in flight.
fn usage_gap(model: &Model) -> Option<String> {
    if model.usage_ready() {
        return None;
    }
    let groups = ObjectKind::LabelGroups.index();
    let parts = [
        (
            "workloads",
            model.workloads.loaded_at.is_some(),
            model.workloads.loading,
        ),
        (
            "policy",
            model.policy.loaded_at.is_some(),
            model.policy.loading,
        ),
        (
            "label groups",
            model.objects.ages[groups].is_some(),
            model.objects.loading[groups],
        ),
    ];
    let missing: Vec<&str> = parts
        .iter()
        .filter(|(_, loaded, loading)| !loaded && !loading)
        .map(|(name, _, _)| *name)
        .collect();
    if missing.is_empty() {
        return None; // all on their way
    }
    let index = if model.index().is_none() {
        " (no name index yet)"
    } else {
        ""
    };
    Some(format!(
        "unused labels need {} loaded{index}",
        missing.join(" and ")
    ))
}

/// `y` on the `u` confirm: the workloads and the rulesets, each through its
/// screen's own load unless cached or on its way; the filter applies when
/// they have arrived. Nothing to load and nothing coming: the filter stays
/// off and the status line names the gap.
fn load_label_usage(model: &mut Model) -> Vec<Cmd> {
    if !model.is_connected() {
        model.status = StatusLine::warn("unused labels: not connected");
        return Vec::new();
    }
    if model.index().is_none() {
        // The workloads wait for the index: load nothing rather than half.
        if let Some(gap) = usage_gap(model) {
            model.status = StatusLine::warn(gap);
        }
        return Vec::new();
    }
    let mut cmds = load_missing_workloads(model);
    if model.policy.loaded_at.is_none() && !model.policy.loading {
        cmds.extend(load_rule_sets(model, CacheMode::Use));
    }
    apply_unused(model);
    if !cmds.is_empty() && model.objects.unused_only {
        model.status = StatusLine::info("unused labels: loading workloads and policy");
    }
    cmds
}

/// `writes on`, or `writes off: <reason>` (just the reason when it already
/// says so, as `WRITES_OFF` does).
fn writes_status(writes: &WritesState) -> String {
    if writes.enabled {
        return "writes on".into();
    }
    let reason = writes.refusal();
    if reason.starts_with("writes off") {
        reason
    } else {
        format!("writes off: {reason}")
    }
}

/// Writes off, or a write already on its way: the reason goes to the status
/// line and the key does nothing else.
fn writes_refused(model: &mut Model) -> bool {
    if !model.writes.enabled {
        model.status = StatusLine::warn(model.writes.refusal());
        return true;
    }
    if model.writes.in_flight {
        model.status = StatusLine::warn("a write is already in progress");
        return true;
    }
    false
}

/// An open `w` form again from the draft rulesets that just arrived: its
/// ruleset hint, and the pick while the field is empty. Nothing for another
/// overlay (a form closed meanwhile is gone). No request.
fn refill_rule_form(model: &mut Model) {
    let Some(n) = model.draft_rulesets().map(<[_]>::len) else {
        return;
    };
    if model.status.text == FORM_DRAFTS_LOADING {
        model.status = StatusLine::info(format!("{n} draft rulesets for the form"));
    }
    let mut overlay = std::mem::replace(&mut model.overlay, Overlay::None);
    if let Overlay::Form(form) = &mut overlay {
        let empty = pce::NameIndex::default();
        let idx = model.index().cloned();
        edit::refill_ruleset(form, model.flow_prefill(), idx.as_deref().unwrap_or(&empty));
    }
    model.overlay = overlay;
}

/// A `w` form needs the draft rulesets: one load (cached) when they are
/// neither held nor on their way. The Policy screen on the draft loads its
/// own list (it shows it); on the active one the drafts are kept apart.
fn load_form_drafts(model: &mut Model) -> Vec<Cmd> {
    if !model.is_connected() || model.draft_rulesets().is_some() {
        return Vec::new();
    }
    let cmds = if model.policy.scope == PolicyScope::Draft {
        if model.policy.loading {
            return Vec::new();
        }
        // Its answer must not replace the status line's form hint.
        model.policy.quiet_reload = true;
        load_rule_sets(model, CacheMode::Use)
    } else {
        if model.policy.form_drafts_loading {
            return Vec::new();
        }
        model.policy.form_drafts_loading = true;
        vec![Cmd::LoadRuleSets {
            scope: PolicyScope::Draft,
            mode: CacheMode::Use,
        }]
    };
    model.status = StatusLine::info(FORM_DRAFTS_LOADING);
    cmds
}

/// The status line while a `w` form's draft rulesets load.
const FORM_DRAFTS_LOADING: &str = "loading the draft rulesets for the form · 1 req";

/// A create or update opens its form; a delete goes straight to planning.
fn begin_edit(model: &mut Model, intent: EditIntent) -> Vec<Cmd> {
    let start = edit::start(
        intent,
        model.index().map(|i| i.as_ref()),
        model.flow_prefill(),
    );
    begin_start(model, start)
}

fn begin_start(model: &mut Model, start: Start) -> Vec<Cmd> {
    // `app_rule_key` sets it again for a form the app overlay opened.
    model.form_from_app = false;
    match start {
        Start::Form(f) => {
            let flow = matches!(f.intent, EditIntent::RuleFromFlow(_));
            model.overlay = Overlay::Form(f);
            if flow {
                load_form_drafts(model)
            } else {
                Vec::new()
            }
        }
        Start::Plan(w) => {
            model.status = StatusLine::info("planning…");
            model.writes.in_flight = true;
            vec![Cmd::PlanWrite(w)]
        }
    }
}

/// `n` / `e` / `x` on Objects: labels and services; label groups are read-only.
fn objects_edit_key(model: &mut Model, c: char) -> Vec<Cmd> {
    if writes_refused(model) {
        return Vec::new();
    }
    let o = &model.objects;
    let intent = match (o.kind, c) {
        (ObjectKind::IpLists, _) => return ip_lists_edit_key(model, c),
        (ObjectKind::LabelGroups, _) => {
            model.status = StatusLine::warn("label groups are read-only in this version");
            return Vec::new();
        }
        (ObjectKind::Labels, 'n') => Some(EditIntent::CreateLabel),
        (ObjectKind::Services, 'n') => Some(EditIntent::CreateService),
        (ObjectKind::Labels, 'e') => o
            .labels
            .selected_item()
            .map(|r| EditIntent::UpdateLabel(r.label.clone())),
        (ObjectKind::Labels, _) => o
            .labels
            .selected_item()
            .map(|r| EditIntent::DeleteLabel(r.label.clone())),
        (ObjectKind::Services, 'e') => o
            .services
            .selected_item()
            .cloned()
            .map(EditIntent::UpdateService),
        (ObjectKind::Services, _) => o
            .services
            .selected_item()
            .cloned()
            .map(EditIntent::DeleteService),
    };
    let Some(intent) = intent else {
        model.status = StatusLine::warn("no row selected");
        return Vec::new();
    };
    begin_edit(model, intent)
}

/// `n` / `e` / `x` on Workloads. Edit and delete only for workloads the PCE
/// reports as unmanaged with no VEN (the library's rule, checked again there).
fn workloads_edit_key(model: &mut Model, c: char) -> Vec<Cmd> {
    if writes_refused(model) {
        return Vec::new();
    }
    if c == 'n' {
        return begin_edit(model, EditIntent::CreateWorkload);
    }
    let Some(w) = model.workloads.selected_row().map(|r| r.workload.clone()) else {
        model.status = StatusLine::warn("no row selected");
        return Vec::new();
    };
    // The write guard's test (pce::write::workloads::require_unmanaged), not
    // Workload::is_unmanaged: offer an edit only where the write would pass.
    if w.managed != Some(false) || w.ven.is_some() {
        model.status = StatusLine::warn("only unmanaged workloads can be edited here");
        return Vec::new();
    }
    let intent = if c == 'e' {
        EditIntent::UpdateWorkload(w)
    } else {
        EditIntent::DeleteWorkload(w)
    };
    begin_edit(model, intent)
}

/// `n` / `e` / `x` on the rulesets pane: draft only.
fn policy_edit_key(model: &mut Model, c: char) -> Vec<Cmd> {
    if writes_refused(model) {
        return Vec::new();
    }
    if model.policy.scope == PolicyScope::Active {
        model.status = StatusLine::warn("switch to draft to edit policy");
        return Vec::new();
    }
    let intent = match model.policy.pane {
        PolicyPane::RuleSets => ruleset_intent(&model.policy, c),
        PolicyPane::Rules => rule_intent(&model.policy, c),
    };
    let Some(intent) = intent else {
        model.status = StatusLine::warn("no row selected");
        return Vec::new();
    };
    begin_edit(model, intent)
}

/// `n` a new ruleset; `e` / `x` the selected one.
fn ruleset_intent(p: &PolicyState, c: char) -> Option<EditIntent> {
    if c == 'n' {
        return Some(EditIntent::CreateRuleSet);
    }
    let rs = p.selected_rule_set()?.clone();
    Some(if c == 'e' {
        EditIntent::UpdateRuleSet(rs)
    } else {
        EditIntent::DeleteRuleSet(rs)
    })
}

/// `n` / `N` a new allow / deny rule in the selected ruleset; `e` / `x` the
/// selected rule, allow or deny by its kind, found by href in what was loaded.
fn rule_intent(p: &PolicyState, c: char) -> Option<EditIntent> {
    let ruleset_href = p.selected_href()?;
    match c {
        'n' => return Some(EditIntent::CreateRule { ruleset_href }),
        'N' => return Some(EditIntent::CreateDenyRule { ruleset_href }),
        _ => {}
    }
    let view = p.rules.selected_item()?;
    let href = view.href.clone();
    match (c, view.kind == RuleKind::Allow) {
        ('x', true) => Some(EditIntent::DeleteRule {
            href,
            name: rule_line(view),
        }),
        ('x', false) => Some(EditIntent::DeleteDenyRule {
            href,
            name: rule_line(view),
        }),
        (_, true) => p
            .find_rule(&href)
            .map(|(r, rs)| EditIntent::UpdateRule(r, rs)),
        (_, false) => p
            .find_deny_rule(&href)
            .map(|(d, rs)| EditIntent::UpdateDenyRule(d, rs)),
    }
}

/// `w` on Traffic: a draft allow rule from the selected blocked or
/// potentially blocked flow (Flows view or a drilled pair), or from the
/// first such flow behind a section's peer row (App-to-app `in` / `out` /
/// `both`, Blocked) or a Mesh row. App-to-app `pairs` refuses: a pair holds
/// flows of every decision; so does a section header.
fn flow_rule_key(model: &mut Model) -> Vec<Cmd> {
    if writes_refused(model) {
        return Vec::new();
    }
    let t = &model.traffic;
    let flow = match t.view {
        TrafficView::Flows
            if t.flows_grouped
                .selected_item()
                .is_some_and(SectionRow::is_header) =>
        {
            model.status = StatusLine::warn("select a flow row (a header holds a whole app)");
            return Vec::new();
        }
        TrafficView::Flows => t.selected_flow().map(|r| (*r.flow).clone()),
        TrafficView::Blocked | TrafficView::Aggregate if t.section_table().is_some() => {
            match t.section_drill() {
                Some(d @ Drill::Peer { .. }) => {
                    let keys = t.effective_keys();
                    t.first_flow(|f| d.matches(f, &keys))
                }
                Some(_) => {
                    model.status =
                        StatusLine::warn("select a peer row (a header holds a whole app)");
                    return Vec::new();
                }
                None => None,
            }
        }
        TrafficView::Mesh => t.mesh.selected_item().and_then(|r| {
            let keys = t.effective_keys();
            t.first_flow(|f| r.contains(f, &keys, t.mesh_cols))
        }),
        TrafficView::Aggregate if t.apps_state.by_service => return service_pair_rule_key(model),
        TrafficView::Blocked | TrafficView::Aggregate | TrafficView::Treemap => {
            model.status = StatusLine::warn("select a flow in the Flows view (Enter on a pair)");
            return Vec::new();
        }
    };
    let Some(flow) = flow else {
        model.status = StatusLine::warn("no row selected");
        return Vec::new();
    };
    match flow.policy_decision {
        PolicyDecision::Blocked | PolicyDecision::PotentiallyBlocked => {
            begin_edit(model, EditIntent::RuleFromFlow(Box::new(flow)))
        }
        PolicyDecision::Allowed => {
            model.status = StatusLine::warn("this flow is allowed; nothing to add");
            Vec::new()
        }
        PolicyDecision::Unknown => {
            model.status = StatusLine::warn("this flow has no policy decision; nothing to add");
            Vec::new()
        }
    }
}

/// `w` on an App-to-app pair by service (`o`): the allow rule for the
/// row's tuple, drafted from one of its blocked flows (one the decision
/// filter keeps) with the row's groups (under the `g` keys) as providers
/// and consumers and its service.
fn service_pair_rule_key(model: &mut Model) -> Vec<Cmd> {
    let t = &model.traffic;
    let Some(g) = t.pairs_table().selected_item() else {
        model.status = StatusLine::warn("no row selected");
        return Vec::new();
    };
    if g.blocked + g.potentially_blocked == 0 {
        model.status = StatusLine::warn("every flow of this row is allowed; nothing to add");
        return Vec::new();
    }
    let keys = t.effective_keys();
    let decisions = t.apps_state.decisions;
    let Some(flow) = t.first_flow(|f| decisions.keeps(f) && g.contains(f, &keys)) else {
        model.status = StatusLine::warn("no row selected");
        return Vec::new();
    };
    let prefill = edit::FlowPrefill {
        group_keys: Some(&keys),
        ..model.flow_prefill()
    };
    let start = edit::start(
        EditIntent::RuleFromFlow(Box::new(flow)),
        model.index().map(|i| i.as_ref()),
        prefill,
    );
    begin_start(model, start)
}

/// `:draft` / `:active`: like `d` on the Policy screen, but explicit. The scope
/// already shown is not reloaded (like `2`); a load in flight is not doubled.
fn show_policy(model: &mut Model, scope: PolicyScope) -> Vec<Cmd> {
    if !model.is_connected() {
        model.status = StatusLine::error("not connected");
        return Vec::new();
    }
    if model.policy.scope != scope {
        if model.policy.loading {
            model.screen = Screen::Policy;
            model.status = StatusLine::info("already loading");
            return Vec::new();
        }
        model.policy.scope = scope;
        model.policy.form_drafts_loading = false;
        model.policy.form_drafts_stale = false;
        model.policy.clear();
    }
    switch(model, Screen::Policy)
}

pub fn on_command(model: &mut Model, cmd: Command) -> Vec<Cmd> {
    match cmd {
        Command::Screen(s) => switch(model, s),
        Command::IpLists => {
            model.objects.kind = ObjectKind::IpLists;
            model.objects.show_detail = false;
            model.objects.ip_lists.show_detail = false;
            switch(model, Screen::Objects)
        }
        Command::Quit => quit(model),
        Command::Batch => {
            if model.batch_report.is_some() {
                model.overlay = Overlay::BatchReport { scroll: 0 };
            } else {
                model.status = StatusLine::info("no batch sent in this session");
            }
            Vec::new()
        }
        Command::Help => {
            open_help(model);
            Vec::new()
        }
        Command::Marks(action) => marks::open_marks(model, action.as_deref()),
        // On Workloads `:filter` sets the label filter (local, no request).
        Command::Filter(expr) if model.screen == Screen::Workloads => {
            if let Err(e) = apply_workload_labels(model, &expr) {
                model.status = StatusLine::error(format!("label filter: {e}"));
            }
            Vec::new()
        }
        Command::Filter(expr) => {
            let mut f = model.traffic.filter.clone();
            f.src = expr;
            match f.validate() {
                Ok(()) => request_query(model, f),
                Err(e) => {
                    model.status = StatusLine::error(e);
                    Vec::new()
                }
            }
        }
        Command::Unicast => {
            // Through the cost guard like any filter change: one query.
            let mut f = model.traffic.filter.clone();
            f.toggle_unicast();
            request_query(model, f)
        }
        Command::Window(w) => {
            let mut f = model.traffic.filter.clone();
            f.window = w;
            match f.validate() {
                Ok(()) => request_query(model, f),
                Err(e) => {
                    model.status = StatusLine::error(e);
                    Vec::new()
                }
            }
        }
        Command::Focus(Some(name)) => {
            focus_by_name(model, &name);
            Vec::new()
        }
        Command::Focus(None) => {
            if model.traffic.data.is_none() {
                model.status = StatusLine::info("no result to focus yet: r runs the query");
            } else {
                model.screen = Screen::Traffic;
                open_app_picker(model, AppPickerPurpose::Focus, "");
            }
            Vec::new()
        }
        Command::App(Some(name)) => open_app_by_name(model, &name),
        Command::App(None) => {
            open_app_picker(model, AppPickerPurpose::Overlay, "");
            Vec::new()
        }
        Command::Gen(Some(name)) => gen_by_name(model, &name),
        Command::Gen(None) => {
            open_app_picker(model, AppPickerPurpose::Generator, "");
            Vec::new()
        }
        Command::Profile(name) => {
            if model.batch_sending {
                model.status = StatusLine::warn("batch sending: wait for it to finish");
                return Vec::new();
            }
            if model.writes.in_flight {
                model.status = StatusLine::warn("finish or cancel the write first");
                return Vec::new();
            }
            if model.switching {
                model.status = StatusLine::info("profile switch in progress");
                return Vec::new();
            }
            model.switching = true;
            vec![Cmd::SwitchProfile(name)]
        }
        Command::Writes(None) => {
            model.status = StatusLine::info(writes_status(&model.writes));
            Vec::new()
        }
        Command::Writes(Some(on)) => {
            if model.batch_sending {
                model.status = StatusLine::warn("batch sending: wait for it to finish");
                return Vec::new();
            }
            if model.writes.in_flight {
                model.status = StatusLine::warn("finish or cancel the write first");
                return Vec::new();
            }
            if model.switching {
                model.status = StatusLine::info("profile switch in progress");
                return Vec::new();
            }
            vec![Cmd::SetWrites(on)]
        }
        Command::Export { path, force } => match crate::tui::export::current_table(model) {
            Ok((headers, rows)) => vec![Cmd::Export {
                path,
                force,
                headers,
                rows,
            }],
            Err(e) => {
                model.status = StatusLine::error(e);
                Vec::new()
            }
        },
        Command::Draft => show_policy(model, PolicyScope::Draft),
        Command::Active => show_policy(model, PolicyScope::Active),
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
mod label_filter_tests {
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
            serde_json::from_str(include_str!("../../../../fixtures/workloads.json")).unwrap();
        let rule_sets: Vec<pce::models::RuleSet> =
            serde_json::from_str(include_str!("../../../../fixtures/rule_sets.json")).unwrap();
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
            "../../../../fixtures/labels_usage.json"
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
            serde_json::from_str(include_str!("../../../../fixtures/labels.json")).unwrap();
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
            serde_json::from_str(include_str!("../../../../fixtures/labels.json")).unwrap();
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
}

#[cfg(test)]
mod window_step_tests {
    use super::*;
    use crate::tui::model::{Model, Overlay, PendingAction, Range};
    use crate::tui::msg::QueryOutcome;
    use crate::tui::test_support::{FIXTURE_END, model_with_flows, model_with_window};
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use jiff::{Span, Timestamp};
    use pce::traffic::ResolvedFlow;
    use std::time::Duration;
    use tokio_util::sync::CancellationToken;

    fn key(c: char) -> Msg {
        Msg::Key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE))
    }

    #[track_caller]
    fn quiet(m: &mut Model, msg: Msg) {
        assert_eq!(update(m, msg), Vec::<Cmd>::new());
    }

    fn end() -> Timestamp {
        FIXTURE_END.parse().unwrap()
    }

    fn ago(hours: i64) -> Timestamp {
        end().checked_sub(Span::new().hours(hours)).unwrap()
    }

    fn with_window(m: &Model, w: &str) -> TrafficFilter {
        let mut f = m.traffic.filter.clone();
        f.window = w.into();
        f
    }

    fn query_cmd(m: &Model, w: &str) -> Cmd {
        Cmd::RunQuery {
            seq: m.traffic.seq,
            filter: with_window(m, w),
            mode: CacheMode::Use,
            cancel: CancellationToken::new(),
        }
    }

    /// The answer to the query in flight for the window `hours` back.
    fn query_done(m: &Model, flows: Vec<ResolvedFlow>, hours: i64) -> Msg {
        Msg::QueryDone {
            seq: m.traffic.seq,
            result: Ok(QueryOutcome {
                flows,
                truncated: false,
                incomplete: false,
                elapsed: Duration::from_millis(900),
                hit: false,
                filter: m.traffic.filter.clone(),
                bounds: Some((ago(hours), end())),
                hrefs: Vec::new(),
            }),
        }
    }

    fn range(m: &Model) -> Range {
        m.traffic.data.as_ref().unwrap().range.unwrap()
    }

    #[test]
    fn app_to_app_state_survives_plus_and_minus() {
        use crate::tui::model::By;
        let mut m = model_with_window("1d");
        m.traffic.view = TrafficView::Aggregate;
        quiet(&mut m, key('x'));
        quiet(&mut m, key('x'));
        quiet(&mut m, key('d'));
        let st = m.traffic.apps_state.clone();
        assert_eq!(st.by, By::Out);
        let cmds = update(&mut m, key('+'));
        assert_eq!(cmds, vec![query_cmd(&m, "2d")]);
        let flows = m.traffic.loaded_flows().to_vec();
        let done = query_done(&m, flows, 48);
        quiet(&mut m, done);
        assert_eq!(m.traffic.apps_state, st);
        update(&mut m, key('-'));
        assert_eq!(m.traffic.apps_state, st);
    }

    /// Explorer returns the records that overlap the window, with lifetime
    /// counters: an older slice cannot be added to the loaded rows, so `+`
    /// runs the wider window as one query whose answer replaces them.
    #[test]
    fn plus_reruns_the_whole_wider_window_and_the_answer_replaces_the_rows() {
        let mut m = model_with_window("1d");
        let cmds = update(&mut m, key('+'));
        assert_eq!(cmds, vec![query_cmd(&m, "2d")]);
        assert_eq!(m.traffic.filter.window, "2d");
        quiet(&mut m, key('+')); // running: no second request
        let few = m.traffic.loaded_flows()[..2].to_vec();
        let done = query_done(&m, few, 48);
        quiet(&mut m, done);
        assert_eq!(m.traffic.loaded_flows().len(), 2, "replaced, not merged");
        assert_eq!(
            range(&m),
            Range {
                start: ago(48),
                end: end()
            }
        );
        let out = crate::tui::test_support::render_to_string(&m, 80, 24);
        assert!(out.contains("window 2d · cap 1000"), "{out}");
        assert!(!out.contains("slice"), "{out}");
    }

    #[test]
    fn minus_during_a_cap_query_puts_the_loaded_filter_back() {
        let mut m = model_with_window("1d");
        m.limits.traffic_max_results = 20_000;
        assert_eq!(update(&mut m, key(']')).len(), 1);
        assert_eq!(m.traffic.filter.max_results, 2000);
        quiet(&mut m, key('-'));
        assert!(!m.traffic.is_running());
        assert_eq!(m.traffic.filter.max_results, 1000, "the unrun cap is gone");
        assert_eq!(m.traffic.filter.window, "12h");
        assert_eq!(m.traffic.data.as_ref().unwrap().filter, m.traffic.filter);
        // The same for an editor query with another source.
        let mut m = model_with_window("1d");
        let mut f = m.traffic.filter.clone();
        f.src = "app=shop".into();
        assert_eq!(update(&mut m, Msg::ApplyFilter(f)).len(), 1);
        quiet(&mut m, key('-'));
        assert_eq!(m.traffic.filter.src, "");
        assert_eq!(m.traffic.filter.window, "12h");
    }

    #[test]
    fn minus_sends_nothing_and_drops_exactly_the_older_flows() {
        let mut m = model_with_window("2d");
        let loaded = m.traffic.loaded_flows().len();
        if let Some(d) = m.traffic.data.as_mut() {
            for f in &mut d.flows[..3] {
                f.last_detected = Some("2026-10-09T01:00:00Z".into()); // before end - 1d
            }
            d.flows[3].last_detected = Some("2026-10-09T08:00:00Z".into()); // exactly the start
            d.flows[4].last_detected = None;
        }
        m.traffic.rebuild_views();
        quiet(&mut m, key('-'));
        assert_eq!(m.traffic.filter.window, "1d");
        let d = m.traffic.data.as_ref().unwrap();
        assert_eq!(d.filter.window, "1d");
        assert_eq!(d.flows.len(), loaded - 3);
        assert_eq!(m.traffic.flows.rows().len(), loaded - 3, "views rebuilt");
        assert!(
            d.flows.iter().any(|f| f.last_detected.is_none()),
            "undated kept"
        );
        assert_eq!(range(&m).start, ago(24));
        assert_eq!(m.status.text, "window 1d · 3 older flows dropped");
    }

    #[test]
    fn a_late_answer_after_f_window_or_minus_is_ignored() {
        let mut m = model_with_window("1d");
        let loaded = m.traffic.loaded_flows().len();
        update(&mut m, key('+'));
        let late = query_done(&m, Vec::new(), 48);
        // `:window` (the editor's Enter takes the same path) starts a new query.
        let f = with_window(&m, "6h");
        assert_eq!(update(&mut m, Msg::ApplyFilter(f)).len(), 1);
        quiet(&mut m, late);
        assert_eq!(m.traffic.loaded_flows().len(), loaded);
        assert_eq!(range(&m).start, ago(24));

        let mut m = model_with_window("1d");
        update(&mut m, key('+'));
        let late = query_done(&m, Vec::new(), 48);
        quiet(&mut m, key('-'));
        assert!(!m.traffic.is_running(), "the query was cancelled");
        assert_eq!(m.traffic.filter.window, "1d");
        quiet(&mut m, late);
        assert_eq!(m.traffic.data.as_ref().unwrap().filter.window, "1d");
        assert_eq!(range(&m).start, ago(24), "the 2d answer was dropped");
    }

    #[test]
    fn plus_past_the_threshold_confirms_the_whole_window() {
        let mut m = model_with_window("1w");
        quiet(&mut m, key('+'));
        match &m.overlay {
            Overlay::Confirm {
                reason,
                action: PendingAction::RunQuery(f),
            } => {
                assert!(reason.contains("window 2w exceeds 7d"), "{reason}");
                assert_eq!(f.window, "2w");
            }
            o => panic!("{o:?}"),
        }
        quiet(
            &mut m,
            Msg::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)),
        );
        assert_eq!(m.traffic.filter.window, "1w", "nothing changed");
        quiet(&mut m, key('+'));
        quiet(&mut m, key('n'));
        assert_eq!(m.overlay, Overlay::None);
        quiet(&mut m, key('+'));
        let cmds = update(&mut m, key('y'));
        assert_eq!(cmds, vec![query_cmd(&m, "2w")]);
    }

    #[test]
    fn plus_without_a_known_range_runs_one_full_query() {
        let mut m = model_with_flows(); // no bounds
        let cmds = update(&mut m, key('+'));
        assert_eq!(
            cmds,
            vec![Cmd::RunQuery {
                seq: m.traffic.seq,
                filter: with_window(&m, "2d"),
                mode: CacheMode::Use,
                cancel: CancellationToken::new(),
            }]
        );
        quiet(&mut m, key('+')); // running: no second request
        assert_eq!(m.status.text, "query already running");
    }

    #[test]
    fn the_ladder_ends_send_nothing() {
        let mut m = model_with_window("90d");
        quiet(&mut m, key('+'));
        assert_eq!(m.status.text, "window already at 90d");
        let mut m = model_with_window("1h");
        quiet(&mut m, key('-'));
        assert_eq!(m.status.text, "window already at 1h");
        let mut m = model_with_window("1d");
        m.traffic.filter.max_results = 200_000;
        quiet(&mut m, key(']'));
        assert_eq!(m.status.text, "cap already at the maximum");
        m.traffic.filter.max_results = 500;
        quiet(&mut m, key('['));
        assert_eq!(m.status.text, "cap already at the minimum");
    }

    #[test]
    fn brackets_rerun_the_whole_query_once_with_the_next_cap() {
        let mut m = model_with_window("1d");
        m.limits.traffic_max_results = 20_000;
        let mut f = m.traffic.filter.clone();
        f.max_results = 2000;
        let cmds = update(&mut m, key(']'));
        assert_eq!(
            cmds,
            vec![Cmd::RunQuery {
                seq: m.traffic.seq,
                filter: f,
                mode: CacheMode::Use,
                cancel: CancellationToken::new(),
            }]
        );
        assert_eq!(m.traffic.filter.max_results, 2000);
        quiet(&mut m, key(']')); // running
        let mut m = model_with_window("1d");
        let mut f = m.traffic.filter.clone();
        f.max_results = 500;
        let cmds = update(&mut m, key('['));
        assert_eq!(cmds.len(), 1);
        assert!(matches!(&cmds[0], Cmd::RunQuery { filter, .. } if *filter == f));
    }

    #[test]
    fn raising_the_cap_past_the_threshold_confirms() {
        let mut m = model_with_window("1d");
        m.limits.traffic_max_results = 20_000;
        m.traffic.filter.max_results = 10_000;
        quiet(&mut m, key(']'));
        assert!(
            matches!(&m.overlay, Overlay::Confirm { reason, .. } if reason.contains("cap 20000 exceeds 10000")),
            "{:?}",
            m.overlay
        );
        quiet(&mut m, key('n'));
        assert_eq!(m.traffic.filter.max_results, 10_000, "n keeps the cap");
        quiet(&mut m, key(']'));
        quiet(
            &mut m,
            Msg::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)),
        );
        quiet(&mut m, key(']'));
        let cmds = update(&mut m, key('y'));
        assert_eq!(cmds.len(), 1);
        assert!(matches!(&cmds[0], Cmd::RunQuery { filter, .. } if filter.max_results == 20_000));
    }

    /// The profile's cap is only where `]` starts: it steps on to 200000,
    /// every rung past 10000 behind the `y`.
    #[test]
    fn the_cap_steps_past_the_profile_value_to_the_hard_maximum() {
        let mut m = model_with_window("1d");
        m.limits.traffic_max_results = 3000;
        m.traffic.filter.max_results = 3000;
        let mut seen = Vec::new();
        loop {
            let cmds = update(&mut m, key(']'));
            if let Overlay::Confirm { reason, .. } = &m.overlay {
                assert!(reason.contains("exceeds 10000"), "{reason}");
                assert!(cmds.is_empty());
                assert_eq!(update(&mut m, key('y')).len(), 1, "y runs it");
            } else if cmds.is_empty() {
                break;
            }
            let cap = m.traffic.filter.max_results;
            seen.push(cap);
            m.traffic.query = QueryState::Idle;
        }
        assert_eq!(
            seen,
            [5000, 10_000, 25_000, 50_000, 100_000, 200_000],
            "{}",
            m.status.text
        );
        assert_eq!(m.status.text, "cap already at the maximum");
        m.traffic.filter.max_results = 5000;
        m.traffic.query = QueryState::Idle;
        assert_eq!(update(&mut m, key('[')).len(), 1);
        assert_eq!(m.traffic.filter.max_results, 3000, "the profile's own rung");
    }
}

#[cfg(test)]
mod app_sections_tests {
    use super::*;
    use crate::tui::model::{AppState, Model, Overlay, RowKind};
    use crate::tui::test_support::model_with_rich_flows;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use pce::traffic::{Decisions, Direction};

    fn key(c: char) -> Msg {
        Msg::Key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE))
    }

    fn kc(code: KeyCode) -> Msg {
        Msg::Key(KeyEvent::new(code, KeyModifiers::NONE))
    }

    #[track_caller]
    fn quiet(m: &mut Model, msg: Msg) {
        assert_eq!(update(m, msg), Vec::<Cmd>::new());
    }

    fn on_apps() -> Model {
        let mut m = model_with_rich_flows();
        quiet(&mut m, key('v'));
        assert_eq!(m.traffic.view, TrafficView::Aggregate);
        m
    }

    fn headers(m: &Model) -> Vec<String> {
        m.traffic
            .apps
            .rows()
            .iter()
            .filter(|r| r.is_header())
            .map(|r| r.app.join("/"))
            .collect()
    }

    #[test]
    fn x_and_capital_x_cycle_the_state_and_d_the_decision_filter_without_requests() {
        let mut m = on_apps();
        assert_eq!(m.traffic.apps_state.by, By::Pairs);
        assert!(m.traffic.apps.is_empty(), "pairs builds no sections");
        let mut seen = Vec::new();
        for _ in 0..4 {
            quiet(&mut m, key('x'));
            seen.push(m.traffic.apps_state.by);
        }
        assert_eq!(seen, vec![By::In, By::Out, By::Both, By::Pairs]);
        let mut seen = Vec::new();
        for _ in 0..4 {
            quiet(&mut m, key('X'));
            seen.push(m.traffic.apps_state.by);
        }
        assert_eq!(seen, vec![By::Both, By::Out, By::In, By::Pairs]);
        quiet(&mut m, key('x'));
        let all = m.traffic.apps.len();
        let mut seen = Vec::new();
        for _ in 0..3 {
            quiet(&mut m, key('d'));
            seen.push((m.traffic.apps_state.decisions, m.traffic.apps.len()));
        }
        assert_eq!(seen[0].0, Decisions::Blocked);
        assert_eq!(seen[1].0, Decisions::BlockedOnly);
        assert_eq!(seen[2], (Decisions::All, all));
        assert!(seen[1].1 < seen[0].1 && seen[0].1 < all, "{seen:?}");
        assert_eq!(m.status.text, "decision all");
    }

    #[test]
    fn enter_on_a_header_drills_to_the_app_and_on_a_peer_row_to_its_flows() {
        let mut m = on_apps();
        quiet(&mut m, key('x'));
        quiet(&mut m, key('d')); // blocked
        let first = m.traffic.apps.selected_item().unwrap().clone();
        let RowKind::Header { totals, .. } = &first.kind else {
            panic!("{first:?}")
        };
        let expected = totals[0].1.flows as usize;
        quiet(&mut m, kc(KeyCode::Enter));
        assert_eq!(m.traffic.view, TrafficView::Flows);
        assert_eq!(
            m.traffic.drill,
            Some(Drill::App {
                app: first.app.clone(),
                direction: Some(Direction::In),
                decisions: Decisions::Blocked,
            })
        );
        assert_eq!(m.traffic.flows.len(), expected);
        assert!(
            m.traffic
                .drill
                .as_ref()
                .unwrap()
                .title()
                .ends_with(" · blocked")
        );
        quiet(&mut m, kc(KeyCode::Esc));
        quiet(&mut m, key('v'));
        assert_eq!(m.traffic.view, TrafficView::Aggregate);
        assert_eq!(
            m.traffic.apps_state.by,
            By::In,
            "the state outlives the drill"
        );
        quiet(&mut m, key('j'));
        let row = m.traffic.apps.selected_item().unwrap().clone();
        let (direction, peer) = row.peer().unwrap();
        let (flows, peer_key, service) = (peer.flows, peer.peer.clone(), peer.service.clone());
        quiet(&mut m, kc(KeyCode::Enter));
        assert_eq!(
            m.traffic.drill,
            Some(Drill::Peer {
                app: row.app.clone(),
                direction,
                peer: peer_key,
                service,
                decisions: Decisions::Blocked,
            })
        );
        assert_eq!(m.traffic.flows.len() as u64, flows);
        assert!(
            m.traffic
                .flows
                .rows()
                .iter()
                .all(|r| r.flow.policy_decision != PolicyDecision::Allowed)
        );
    }

    #[test]
    fn both_header_drills_to_either_side() {
        let mut m = on_apps();
        m.traffic.apps_state.by = By::Both;
        m.traffic.rebuild_views();
        let app = m.traffic.apps.selected_item().unwrap().app.clone();
        quiet(&mut m, kc(KeyCode::Enter));
        let Some(Drill::App { direction, .. }) = &m.traffic.drill else {
            panic!("{:?}", m.traffic.drill)
        };
        assert_eq!(*direction, None);
        let keys = m.traffic.effective_keys();
        let n = m
            .traffic
            .loaded_flows()
            .iter()
            .filter(|f| {
                pce::traffic::group_key(&f.src, &keys) == app
                    || pce::traffic::group_key(&f.dst, &keys) == app
            })
            .count();
        assert_eq!(m.traffic.flows.len(), n);
    }

    #[test]
    fn space_folds_a_section_and_z_folds_or_unfolds_them_all() {
        let mut m = on_apps();
        quiet(&mut m, key('x'));
        let all = m.traffic.apps.len();
        let apps = headers(&m);
        quiet(&mut m, key('j'));
        quiet(&mut m, key('j'));
        quiet(&mut m, key(' '));
        let sel = m.traffic.apps.selected_item().unwrap();
        assert!(sel.is_header(), "the cursor lands on the folded header");
        assert!(m.traffic.apps_state.folded.contains(&sel.app));
        assert!(m.traffic.apps.len() < all);
        assert_eq!(headers(&m), apps, "every section keeps its header");
        quiet(&mut m, key(' '));
        assert_eq!(m.traffic.apps.len(), all);
        quiet(&mut m, key('z'));
        assert_eq!(m.traffic.apps.len(), apps.len(), "headers only");
        quiet(&mut m, key('z'));
        assert_eq!(m.traffic.apps.len(), all);
        quiet(&mut m, key('s'));
        assert_eq!(
            m.traffic.apps.sort(),
            None,
            "s orders the sections, no column"
        );
        assert_eq!(m.status.text, "by app A→Z");
    }

    #[test]
    fn search_keeps_a_section_when_any_of_its_rows_matches() {
        let mut m = on_apps();
        quiet(&mut m, key('x'));
        quiet(&mut m, key('/'));
        for c in "53/udp".chars() {
            quiet(&mut m, key(c));
        }
        quiet(&mut m, kc(KeyCode::Enter));
        let shown: Vec<_> = m
            .traffic
            .apps
            .visible()
            .iter()
            .map(|&i| m.traffic.apps.rows()[i].clone())
            .collect();
        assert!(!shown.is_empty());
        assert!(shown.iter().any(|r| r.is_header()));
        assert!(shown.len() < m.traffic.apps.rows().len());
        let apps: std::collections::BTreeSet<_> = shown.iter().map(|r| r.app.clone()).collect();
        for app in &apps {
            let whole = m
                .traffic
                .apps
                .rows()
                .iter()
                .filter(|r| &r.app == app)
                .count();
            let kept = shown.iter().filter(|r| &r.app == app).count();
            assert_eq!(whole, kept, "the whole section stays");
        }
    }

    #[test]
    fn the_state_survives_g_and_r_and_a_profile_switch_resets_it() {
        let mut m = on_apps();
        quiet(&mut m, key('x'));
        quiet(&mut m, key('d'));
        quiet(&mut m, key('z'));
        quiet(&mut m, key('s'));
        let st = m.traffic.apps_state.clone();
        assert!(!st.folded.is_empty());
        assert_eq!(st.order, pce::traffic::SectionOrder::Name);
        quiet(&mut m, key('g'));
        quiet(&mut m, kc(KeyCode::Enter));
        assert_eq!(m.overlay, Overlay::None);
        assert_eq!(m.traffic.apps_state, st, "g keeps it");
        assert!(!m.traffic.apps.is_empty());
        let cmds = update(&mut m, key('r'));
        let filter = m.traffic.filter.clone();
        assert_eq!(
            cmds,
            vec![Cmd::RunQuery {
                seq: m.traffic.seq,
                filter: filter.clone(),
                mode: pce::CacheMode::Bypass,
                cancel: tokio_util::sync::CancellationToken::new(),
            }],
            "r runs the query"
        );
        let flows = crate::tui::test_support::fixture_flows_rich();
        let seq = m.traffic.seq;
        quiet(
            &mut m,
            Msg::QueryDone {
                seq,
                result: Ok(crate::tui::msg::QueryOutcome {
                    flows,
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
        assert_eq!(m.traffic.apps_state, st, "r keeps it");
        assert_eq!(m.traffic.apps.len(), headers(&m).len(), "still folded");
        m.reset_data();
        assert_eq!(
            m.traffic.apps_state,
            AppState::default(),
            ":profile resets it"
        );
    }

    #[test]
    fn w_on_a_peer_row_opens_the_rule_form_and_on_a_header_explains() {
        let mut m = on_apps();
        m.writes = crate::tui::model::WritesState::on();
        quiet(&mut m, key('x'));
        quiet(&mut m, key('d'));
        quiet(&mut m, key('w'));
        assert_eq!(m.overlay, Overlay::None);
        assert_eq!(
            m.status.text,
            "select a peer row (a header holds a whole app)"
        );
        quiet(&mut m, key('j'));
        assert_eq!(update(&mut m, key('w')).len(), 1, "the draft rulesets");
        let Overlay::Form(f) = &m.overlay else {
            panic!("{:?}", m.overlay)
        };
        assert_eq!(f.title, "Allow rule from flow");
    }

    #[test]
    fn blocked_ignores_x_and_d_and_says_why() {
        let mut m = model_with_rich_flows();
        m.traffic.view = TrafficView::Blocked;
        let before = (m.traffic.apps_state.clone(), m.traffic.blocked.len());
        for c in ['x', 'X', 'd'] {
            quiet(&mut m, key(c));
            assert_eq!(
                m.status.text,
                "Blocked is App-to-app with in · blocked; press v until App-to-app"
            );
            assert_eq!(
                (m.traffic.apps_state.clone(), m.traffic.blocked.len()),
                before
            );
        }
        quiet(&mut m, key('z'));
        assert!(m.traffic.blocked.rows().iter().all(|r| r.is_header()));
        assert!(m.traffic.apps_state.folded.is_empty(), "its own folds");
        quiet(&mut m, key('z'));
        quiet(&mut m, key('j'));
        quiet(&mut m, kc(KeyCode::Enter));
        assert!(matches!(
            m.traffic.drill,
            Some(Drill::Peer {
                direction: Direction::In,
                decisions: Decisions::Blocked,
                ..
            })
        ));
    }
}

#[cfg(test)]
mod backfill_tests {
    use super::*;
    use crate::tui::model::{Model, Overlay, PendingAction, StatusKind};
    use crate::tui::msg::QueryOutcome;
    use crate::tui::test_support::{
        FIXTURE_END, fixture_flows_rich, model_fixture, render_to_string,
    };
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use jiff::{Span, Timestamp};
    use std::time::Duration;
    use tokio_util::sync::CancellationToken;

    fn key(c: char) -> Msg {
        Msg::Key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE))
    }

    #[track_caller]
    fn quiet(m: &mut Model, msg: Msg) {
        assert_eq!(update(m, msg), Vec::<Cmd>::new());
    }

    fn end() -> Timestamp {
        FIXTURE_END.parse().unwrap()
    }

    fn cap(m: &Model) -> u32 {
        m.traffic.filter.max_results
    }

    /// The answer to the query in flight (or to a `1d` query at `cap`
    /// started here): the rich fixture, `truncated` or not.
    fn answer(m: &mut Model, cap: u32, truncated: bool) -> Vec<Cmd> {
        if !m.traffic.is_running() {
            let f = TrafficFilter::defaults("1d", cap);
            assert_eq!(update(m, Msg::ApplyFilter(f)).len(), 1);
        }
        let filter = m.traffic.filter.clone();
        assert_eq!(filter.max_results, cap, "the query in flight");
        update(
            m,
            Msg::QueryDone {
                seq: m.traffic.seq,
                result: Ok(QueryOutcome {
                    flows: fixture_flows_rich(),
                    truncated,
                    incomplete: false,
                    elapsed: Duration::from_millis(900),
                    hit: false,
                    filter,
                    bounds: Some((end().checked_sub(Span::new().hours(24)).unwrap(), end())),
                    hrefs: Vec::new(),
                }),
            },
        )
    }

    fn run_at(m: &Model, cap: u32) -> Cmd {
        let mut filter = m.traffic.filter.clone();
        filter.max_results = cap;
        Cmd::RunQuery {
            seq: m.traffic.seq,
            filter,
            mode: CacheMode::Use,
            cancel: CancellationToken::new(),
        }
    }

    fn n() -> usize {
        fixture_flows_rich().len()
    }

    #[test]
    fn nothing_to_backfill_without_a_result_or_on_a_complete_one() {
        let mut m = model_fixture();
        quiet(&mut m, key('B'));
        assert_eq!(m.status.text, "nothing to backfill: r runs the query first");
        let mut m = model_fixture();
        quiet_answer(&mut m, 1000, false);
        quiet(&mut m, key('B'));
        assert_eq!(m.status.text, "nothing to backfill: the result is complete");
        assert_eq!(m.overlay, Overlay::None);
    }

    fn quiet_answer(m: &mut Model, cap: u32, truncated: bool) {
        assert_eq!(answer(m, cap, truncated), Vec::<Cmd>::new());
    }

    /// A truncated result names its cap and claims no time span; `B`
    /// re-runs the window at 10000 without asking (below the threshold
    /// nothing is asked), and the answer replaces the result.
    #[test]
    fn b_below_10000_reruns_the_window_at_10000_and_replaces_the_result() {
        let mut m = model_fixture();
        quiet_answer(&mut m, 120, true);
        assert_eq!(m.status.kind, StatusKind::Warn);
        assert_eq!(
            m.status.text,
            format!("{} flows, capped at 120; B raises the cap", n())
        );
        let out = render_to_string(&m, 120, 30);
        assert!(
            out.contains(&format!("{} flows, capped at 120; B raises the cap", n())),
            "{out}"
        );
        assert!(!out.contains("covers"), "no time span: {out}");
        let cmds = update(&mut m, key('B'));
        assert_eq!(m.overlay, Overlay::None, "nothing asked up to 10000");
        assert_eq!(cmds, vec![run_at(&m, 10_000)]);
        assert_eq!(cap(&m), 10_000);
        quiet(&mut m, key('B')); // running: no second request
        assert_eq!(m.status.text, "query already running");
        quiet_answer(&mut m, 10_000, false);
        let d = m.traffic.data.as_ref().unwrap();
        assert!(!d.truncated);
        assert_eq!(d.filter.max_results, 10_000);
        assert_eq!(d.flows.len(), n(), "replaced, never merged or summed");
        let out = render_to_string(&m, 120, 30);
        assert!(out.contains(&format!("{} flows · ", n())), "{out}");
        assert!(!out.contains("capped"), "{out}");
    }

    /// Above 10000 every rung asks first, even though the last result
    /// proved there is more; `B` climbs one rung per `y`.
    #[test]
    fn above_10000_b_asks_for_each_rung() {
        let mut m = model_fixture();
        quiet_answer(&mut m, 10_000, true);
        quiet(&mut m, key('B'));
        match &m.overlay {
            Overlay::Confirm {
                reason,
                action: PendingAction::RunQuery(f),
            } => {
                assert_eq!(
                    reason,
                    &format!(
                        "cap 25000 exceeds 10000. Re-run the window at cap 25000 (1 query, replaces the {} flows capped at 10000)? y/n",
                        n()
                    )
                );
                assert_eq!(f.max_results, 25_000);
            }
            o => panic!("{o:?}"),
        }
        quiet(&mut m, key('n'));
        assert_eq!(cap(&m), 10_000, "n keeps the cap");
        quiet(&mut m, key('B'));
        let cmds = update(&mut m, key('y'));
        assert_eq!(cmds, vec![run_at(&m, 25_000)]);
        quiet_answer(&mut m, 25_000, true);
        quiet(&mut m, key('B'));
        assert!(
            matches!(&m.overlay, Overlay::Confirm { action: PendingAction::RunQuery(f), .. } if f.max_results == 50_000),
            "{:?}",
            m.overlay
        );
        // At the hard maximum there is no rung left.
        let mut m = model_fixture();
        quiet_answer(&mut m, 200_000, true);
        quiet(&mut m, key('B'));
        assert_eq!(m.overlay, Overlay::None);
        assert_eq!(
            m.status.text,
            "cap already at the maximum (200000): narrow the filter"
        );
    }

    #[test]
    fn the_opt_in_raises_to_10000_without_b_and_never_above() {
        let mut m = model_fixture();
        m.limits.traffic_auto_backfill = true;
        let cmds = answer(&mut m, 120, true);
        assert_eq!(cmds, vec![run_at(&m, 10_000)]);
        assert_eq!(
            m.status.text,
            format!(
                "{} flows, capped at 120: re-running at cap 10000 (profile traffic_auto_backfill)",
                n()
            )
        );
        // Truncated again at 10000: the opt-in stops, B asks as usual.
        quiet_answer(&mut m, 10_000, true);
        assert_eq!(m.overlay, Overlay::None);
        assert!(m.status.text.contains("capped at 10000; B raises the cap"));
        // A complete result is left alone.
        let mut m = model_fixture();
        m.limits.traffic_auto_backfill = true;
        quiet_answer(&mut m, 120, false);
    }

    /// The opt-in never overrides a cap the user stepped down to.
    #[test]
    fn lowering_the_cap_under_the_opt_in_stays_at_the_lower_cap() {
        let mut m = model_fixture();
        m.limits.traffic_auto_backfill = true;
        m.limits.traffic_max_results = 10_000;
        quiet_answer(&mut m, 10_000, true);
        let cmds = update(&mut m, key('['));
        assert_eq!(cmds, vec![run_at(&m, 5000)], "exactly one query");
        quiet_answer(&mut m, 5000, true);
        assert_eq!(cap(&m), 5000);
        assert!(!m.traffic.is_running());
        assert!(m.status.text.contains("capped at 5000; B raises the cap"));
        // `r` on the same filter does not raise it either.
        assert_eq!(update(&mut m, key('r')).len(), 1);
        quiet_answer(&mut m, 5000, true);
        assert_eq!(cap(&m), 5000);
    }

    #[test]
    fn a_late_answer_to_b_is_dropped_after_another_query() {
        let mut m = model_fixture();
        quiet_answer(&mut m, 120, true);
        assert_eq!(update(&mut m, key('B')).len(), 1);
        let stale = m.traffic.seq;
        let f = TrafficFilter::defaults("6h", 500);
        assert_eq!(update(&mut m, Msg::ApplyFilter(f)).len(), 1);
        quiet(
            &mut m,
            Msg::QueryDone {
                seq: stale,
                result: Ok(QueryOutcome {
                    flows: Vec::new(),
                    truncated: false,
                    incomplete: false,
                    elapsed: Duration::ZERO,
                    hit: false,
                    filter: TrafficFilter::defaults("1d", 10_000),
                    bounds: None,
                    hrefs: Vec::new(),
                }),
            },
        );
        let d = m.traffic.data.as_ref().unwrap();
        assert_eq!((d.flows.len(), d.filter.max_results), (n(), 120));
        assert!(
            m.traffic.is_running(),
            "the 6h query is still the one in flight"
        );
    }

    #[test]
    fn an_incomplete_result_says_so_in_the_status_and_the_header() {
        let mut m = model_fixture();
        let f = TrafficFilter::defaults("1d", 1000);
        assert_eq!(update(&mut m, Msg::ApplyFilter(f.clone())).len(), 1);
        let seq = m.traffic.seq;
        quiet(
            &mut m,
            Msg::QueryDone {
                seq,
                result: Ok(QueryOutcome {
                    flows: fixture_flows_rich(),
                    truncated: false,
                    incomplete: true,
                    elapsed: Duration::ZERO,
                    hit: false,
                    filter: f,
                    bounds: None,
                    hrefs: Vec::new(),
                }),
            },
        );
        assert_eq!(m.status.kind, StatusKind::Warn);
        assert_eq!(
            m.status.text,
            format!("{} flows · incomplete: a PCE region did not respond", n())
        );
        let out = render_to_string(&m, 120, 30);
        assert!(out.contains("incomplete"), "{out}");
    }
}

#[cfg(test)]
mod filter_completion_tests {
    use super::*;
    use crate::tui::filter::FilterEditor;
    use crate::tui::model::{Model, Overlay};
    use crate::tui::msg::{Cmd, Msg};
    use crate::tui::test_support::{model_fixture, with_workloads};
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    /// Sends the key and asserts it asked for nothing.
    #[track_caller]
    fn send(m: &mut Model, code: KeyCode, mods: KeyModifiers) {
        assert_eq!(
            update(m, Msg::Key(KeyEvent::new(code, mods))),
            Vec::<Cmd>::new()
        );
    }

    #[track_caller]
    fn press(m: &mut Model, code: KeyCode) {
        send(m, code, KeyModifiers::NONE);
    }

    #[track_caller]
    fn ctrl(m: &mut Model, c: char) {
        send(m, KeyCode::Char(c), KeyModifiers::CONTROL);
    }

    #[track_caller]
    fn type_text(m: &mut Model, text: &str) {
        for c in text.chars() {
            press(m, KeyCode::Char(c));
        }
    }

    /// The fixture model with the filter editor open on field `field`, its
    /// text replaced by `text`.
    fn editing(field: usize, text: &str) -> Model {
        let mut m = model_fixture();
        let mut e = FilterEditor::from_filter(&m.traffic.filter);
        e.active = field;
        e.values[field] = text.into();
        m.overlay = Overlay::FilterEditor(e);
        m
    }

    #[track_caller]
    fn editor(m: &Model) -> &FilterEditor {
        match &m.overlay {
            Overlay::FilterEditor(e) => e,
            other => panic!("{other:?}"),
        }
    }

    #[track_caller]
    fn value(m: &Model, field: usize) -> String {
        editor(m).values[field].clone()
    }

    #[test]
    fn src_cycles_dives_into_a_key_and_esc_restores_then_cancels() {
        let mut m = editing(1, "app=shop, r");
        ctrl(&mut m, 'n');
        assert_eq!(value(&m, 1), "app=shop, role=", "the key first");
        ctrl(&mut m, 'n');
        assert_eq!(value(&m, 1), "app=shop, role=db", "then its values");
        ctrl(&mut m, 'n');
        assert_eq!(value(&m, 1), "app=shop, role=web");
        ctrl(&mut m, 'n');
        assert_eq!(value(&m, 1), "app=shop, role=db", "wraps");
        ctrl(&mut m, 'p');
        assert_eq!(value(&m, 1), "app=shop, role=web");
        assert_eq!(
            editor(&m).completion_strip(),
            Some(vec![
                ("role=db".to_string(), false),
                ("role=web".to_string(), true)
            ])
        );
        press(&mut m, KeyCode::Esc);
        assert_eq!(value(&m, 1), "app=shop, r", "the typed text is back");
        assert_eq!(editor(&m).completion, None, "Esc ended the completion only");
        press(&mut m, KeyCode::Esc);
        assert_eq!(m.overlay, Overlay::None, "a second Esc cancels the editor");
    }

    #[test]
    fn any_other_key_accepts_and_operators_and_earlier_tokens_stay() {
        let mut m = editing(2, "app=shop | ro");
        ctrl(&mut m, 'n');
        assert_eq!(value(&m, 2), "app=shop | role=");
        ctrl(&mut m, 'n');
        assert_eq!(value(&m, 2), "app=shop | role=db");
        press(&mut m, KeyCode::Char(','));
        assert_eq!(value(&m, 2), "app=shop | role=db,", "accepted, then typed");
        assert_eq!(editor(&m).completion, None);
        type_text(&mut m, "ipl:c");
        ctrl(&mut m, 'n');
        assert_eq!(value(&m, 2), "app=shop | role=db, ipl:Corp");
        press(&mut m, KeyCode::Tab);
        assert_eq!(
            (value(&m, 2).as_str(), editor(&m).active),
            ("app=shop | role=db, ipl:Corp", 3),
            "Tab accepts and moves on"
        );
        assert_eq!(editor(&m).completion, None);
        // Backspace accepts and then deletes.
        let mut m = editing(1, "ipl:a");
        ctrl(&mut m, 'n');
        assert_eq!(value(&m, 1), "ipl:Any (0.0.0.0/0 and ::/0)");
        press(&mut m, KeyCode::Backspace);
        assert_eq!(value(&m, 1), "ipl:Any (0.0.0.0/0 and ::/0");
    }

    #[test]
    fn window_service_and_decision_complete_their_words() {
        let mut m = editing(0, "");
        ctrl(&mut m, 'p');
        assert_eq!(value(&m, 0), "90d", "Ctrl-P opens on the last step");
        ctrl(&mut m, 'p');
        assert_eq!(value(&m, 0), "4w");
        let mut m = editing(0, "24h");
        ctrl(&mut m, 'n');
        assert_eq!(value(&m, 0), "2d", "the pre-filled window: where `+` goes");
        ctrl(&mut m, 'n');
        assert_eq!(value(&m, 0), "3d");
        let mut m = editing(1, "!ro");
        let before = m.overlay.clone();
        ctrl(&mut m, 'n');
        assert_eq!(
            m.overlay, before,
            "no negation in the filter: left as typed"
        );
        let mut m = editing(3, "post");
        ctrl(&mut m, 'n');
        assert_eq!(value(&m, 3), "5432/tcp");
        let mut m = editing(3, "443/");
        ctrl(&mut m, 'n');
        assert_eq!(value(&m, 3), "443/", "a port is left as typed");
        let mut m = editing(4, "allowed,b");
        ctrl(&mut m, 'n');
        assert_eq!(value(&m, 4), "allowed, blocked");
        ctrl(&mut m, 'n');
        assert_eq!(value(&m, 4), "allowed, potentially_blocked");
        press(&mut m, KeyCode::Tab);
        let f = editor(&m).to_filter().expect("a valid filter");
        assert_eq!(f.decisions_text(), "allowed,potentially_blocked");
    }

    #[test]
    fn workloads_are_offered_only_once_the_workloads_screen_has_loaded() {
        let mut m = editing(2, "wl:");
        ctrl(&mut m, 'n');
        assert_eq!(value(&m, 2), "wl:", "nothing loaded, nothing offered");
        assert_eq!(editor(&m).completion, None);
        let mut m = with_workloads(m);
        ctrl(&mut m, 'n');
        assert_eq!(value(&m, 2), "wl:batch04");
        ctrl(&mut m, 'n');
        assert_eq!(value(&m, 2), "wl:host-1.example");
    }

    #[test]
    fn a_text_field_or_no_index_completes_nothing() {
        let mut m = editing(5, "100");
        let before = m.overlay.clone();
        ctrl(&mut m, 'n');
        assert_eq!(m.overlay, before, "max results: not typed, not completed");
        let mut m = editing(1, "ro");
        m.connection = Connection::Connected {
            index: None,
            index_age_secs: 0,
        };
        let before = m.overlay.clone();
        ctrl(&mut m, 'n');
        ctrl(&mut m, 'p');
        assert_eq!(m.overlay, before);
    }
}

#[cfg(test)]
mod focus_tests {
    use super::*;
    use crate::tui::model::{Model, Overlay, PendingAction, TrafficView};
    use crate::tui::msg::FocusedOutcome;
    use crate::tui::test_support::{model_fixture, model_with_flows, model_with_rich_flows};
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use pce::traffic::FocusDirection;
    use std::time::Duration;

    fn key(c: char) -> Msg {
        Msg::Key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE))
    }

    fn kc(code: KeyCode) -> Msg {
        Msg::Key(KeyEvent::new(code, KeyModifiers::NONE))
    }

    #[track_caller]
    fn quiet(m: &mut Model, msg: Msg) {
        assert_eq!(update(m, msg), Vec::<Cmd>::new());
    }

    fn focus_cmd(name: &str) -> Msg {
        Msg::Command(Command::Focus(Some(name.into())))
    }

    fn focused(m: &Model) -> Option<(String, FocusDirection)> {
        m.traffic
            .focus
            .as_ref()
            .map(|f| (f.app_text(), f.direction))
    }

    #[test]
    fn x_and_esc_cycle_and_clear_the_focus_on_every_view_without_requests() {
        for view in [
            TrafficView::Flows,
            TrafficView::Aggregate,
            TrafficView::Mesh,
            TrafficView::Blocked,
            TrafficView::Treemap,
        ] {
            let mut m = model_with_rich_flows();
            m.traffic.view = view;
            quiet(&mut m, focus_cmd("crm/prod"));
            assert_eq!(
                focused(&m),
                Some(("crm/prod".into(), FocusDirection::Both)),
                "{view:?}"
            );
            quiet(&mut m, key('X'));
            assert_eq!(focused(&m).unwrap().1, FocusDirection::In, "{view:?}");
            let t = &m.traffic;
            let crm = |k: &Vec<String>| k.join("/") == "crm/prod";
            match view {
                TrafficView::Flows => {
                    assert_eq!(t.flows.len(), 7);
                    assert!(t.flows.rows().iter().all(|r| &*r.dst_group == "crm/prod"));
                }
                TrafficView::Aggregate => {
                    assert!(!t.groups.is_empty() && t.groups.rows().iter().all(|g| crm(&g.dst)))
                }
                TrafficView::Mesh => {
                    assert!(!t.mesh.is_empty() && t.mesh.rows().iter().all(|r| crm(&r.dst)))
                }
                TrafficView::Blocked => {
                    assert!(!t.blocked.is_empty() && t.blocked.rows().iter().all(|r| crm(&r.app)))
                }
                TrafficView::Treemap => assert!(
                    !t.treemap.tiles.is_empty() && t.treemap.groups.iter().all(|g| crm(&g.dst))
                ),
            }
            quiet(&mut m, key('X'));
            assert_eq!(focused(&m).unwrap().1, FocusDirection::Out);
            quiet(&mut m, key('X'));
            assert_eq!(focused(&m).unwrap().1, FocusDirection::Both);
            quiet(&mut m, kc(KeyCode::Esc));
            assert_eq!(focused(&m), None, "{view:?}");
            assert_eq!(m.traffic.flows.len(), 30, "{view:?}: every flow again");
            assert_eq!(m.status.text, "focus cleared");
        }
    }

    #[test]
    fn without_a_focus_x_and_esc_keep_their_old_meaning() {
        let mut m = model_with_rich_flows();
        quiet(&mut m, key('v'));
        assert_eq!(m.traffic.view, TrafficView::Aggregate);
        quiet(&mut m, key('X'));
        assert_eq!(m.traffic.apps_state.by, By::Both, "X steps back from pairs");
        m.traffic.view = TrafficView::Flows;
        m.traffic.drill = Some(Drill::Pair(
            vec!["crm".into(), "prod".into()],
            vec!["crm".into(), "prod".into()],
            Decisions::All,
        ));
        m.traffic.rebuild_views();
        quiet(&mut m, focus_cmd("crm/prod"));
        quiet(&mut m, kc(KeyCode::Esc));
        assert!(
            m.traffic.drill.is_some(),
            "the first Esc clears the focus only"
        );
        quiet(&mut m, kc(KeyCode::Esc));
        assert!(
            m.traffic.drill.is_none(),
            "then Esc clears the drill as before"
        );
    }

    #[test]
    fn f_on_a_row_takes_its_source_then_its_destination() {
        let mut m = model_with_flows();
        m.traffic
            .flows
            .select_where(|r| r.flow.dst.display_name() == "fqdn-1.example");
        quiet(&mut m, key('F'));
        assert_eq!(
            focused(&m),
            Some(("shop/prod".into(), FocusDirection::Both))
        );
        assert_eq!(m.status.text, "focus shop/prod · both · 4 flows");
        m.traffic
            .flows
            .select_where(|r| r.flow.dst.display_name() == "fqdn-1.example");
        quiet(&mut m, key('F'));
        assert_eq!(focused(&m).unwrap().0, "fqdn-1.example/-");
        quiet(&mut m, kc(KeyCode::Esc));
        m.traffic.view = TrafficView::Blocked;
        m.traffic.blocked.select_where(|r| !r.is_header());
        quiet(&mut m, key('F'));
        assert_eq!(
            focused(&m).unwrap().0,
            "shop/prod",
            "Blocked: the destination first"
        );
    }

    #[test]
    fn f_without_a_row_opens_the_picker_or_clears_and_the_picker_focuses() {
        let mut m = model_with_flows();
        m.traffic.flows.set_rows(Vec::new());
        quiet(&mut m, key('F'));
        let Overlay::AppPicker(p) = &m.overlay else {
            panic!("{:?}", m.overlay)
        };
        assert_eq!(p.apps, vec!["shop/prod"]);
        quiet(&mut m, key('s'));
        quiet(&mut m, key('q'));
        quiet(&mut m, kc(KeyCode::Backspace));
        assert!(!m.quit, "q is typed into the picker");
        quiet(&mut m, kc(KeyCode::Enter));
        assert_eq!(m.overlay, Overlay::None);
        assert_eq!(focused(&m).unwrap().0, "shop/prod");
        m.traffic.flows.set_rows(Vec::new());
        quiet(&mut m, key('F'));
        assert_eq!(focused(&m), None, "F on nothing with a focus set clears it");
        quiet(&mut m, Msg::Command(Command::Focus(None)));
        assert!(matches!(m.overlay, Overlay::AppPicker(_)));
        quiet(&mut m, kc(KeyCode::Esc));
        assert_eq!(m.overlay, Overlay::None);
        assert_eq!(focused(&m), None);
    }

    #[test]
    fn focus_names_an_unknown_app_and_its_closest_match_and_sends_nothing() {
        let mut m = model_with_rich_flows();
        quiet(&mut m, focus_cmd("crm/prd"));
        assert_eq!(focused(&m), None);
        assert_eq!(
            m.status.text,
            "unknown app \"crm/prd\"; closest: crm/prod; nothing sent"
        );
        let mut m = model_fixture();
        quiet(&mut m, focus_cmd("crm/prod"));
        assert_eq!(m.status.text, "no result to focus yet: r runs the query");
        quiet(&mut m, key('F'));
        assert_eq!(m.status.text, "no result to focus yet: r runs the query");
    }

    #[test]
    fn focus_from_another_screen_shows_the_traffic_screen() {
        let mut m = model_with_rich_flows();
        m.screen = Screen::Workloads;
        quiet(&mut m, focus_cmd("crm/prod"));
        assert_eq!(m.screen, Screen::Traffic);
    }

    #[test]
    fn g_clears_the_focus_and_says_so() {
        let mut m = model_with_rich_flows();
        quiet(&mut m, focus_cmd("crm/prod"));
        apply_group_keys(&mut m, vec!["app".into()], false);
        assert_eq!(focused(&m), None);
        assert_eq!(
            m.status.text,
            "grouped by app · focus cleared: the app is not defined under the new keys"
        );
        assert_eq!(m.traffic.flows.len(), 30);
        apply_group_keys(&mut m, vec!["app".into(), "env".into()], false);
        assert_eq!(
            m.status.text, "grouped by app,env",
            "no focus, nothing to say"
        );
    }

    fn focused_data(m: &Model, n: usize) -> TrafficData {
        TrafficData {
            flows: m.traffic.loaded_flows()[..n].to_vec(),
            truncated: false,
            incomplete: false,
            elapsed: Duration::from_millis(900),
            cache_hit: false,
            fetched_at: Instant::now(),
            filter: m.traffic.filter.clone(),
            range: None,
            hrefs: Vec::new(),
            draft: None,
        }
    }

    #[test]
    fn a_truncated_result_offers_r_and_r_sends_one_query_per_side() {
        let mut m = model_with_flows();
        m.traffic.data.as_mut().unwrap().truncated = true;
        quiet(&mut m, focus_cmd("shop/prod"));
        assert_eq!(
            m.status.text,
            "focus shop/prod · both · 4 flows · the result is truncated: r re-queries shop/prod (2 queries)"
        );
        let cmds = update(&mut m, key('r'));
        let [
            Cmd::RunFocused {
                seq,
                filter,
                focus,
                mode,
                ..
            },
        ] = cmds.as_slice()
        else {
            panic!("{cmds:?}")
        };
        assert_eq!(*seq, m.traffic.seq);
        assert_eq!(filter, &m.traffic.filter);
        assert_eq!(focus.app_text(), "shop/prod");
        assert_eq!(*mode, CacheMode::Use);
        assert_eq!(
            m.status.text,
            "focused query running: shop/prod · 2 queries"
        );
        let flows = crate::tui::test_support::fixture_flows();
        let outcome = QueryOutcome {
            flows: flows[..3].to_vec(),
            truncated: false,
            incomplete: false,
            elapsed: Duration::from_millis(900),
            hit: false,
            filter: m.traffic.filter.clone(),
            bounds: None,
            hrefs: Vec::new(),
        };
        let seq = m.traffic.seq;
        quiet(
            &mut m,
            Msg::FocusedDone {
                seq,
                result: Ok(FocusedOutcome {
                    outcome,
                    focus: focus.clone(),
                    queries: 2,
                }),
            },
        );
        assert!(!m.traffic.is_running());
        assert_eq!(m.traffic.loaded_flows().len(), 3);
        assert_eq!(
            m.traffic.focus_meta().as_deref(),
            Some("focus shop/prod · both · 2 queries")
        );
        assert_eq!(m.status.text, "3 flows for shop/prod in 0.9s · 2 queries");
        let cmds = update(&mut m, key('r'));
        assert!(
            matches!(
                cmds.as_slice(),
                [Cmd::RunFocused {
                    mode: CacheMode::Bypass,
                    ..
                }]
            ),
            "r on the focused result refreshes it past the cache: {cmds:?}"
        );
        m.traffic.cancel_query();
        quiet(&mut m, kc(KeyCode::Esc));
        assert_eq!(focused(&m), None);
        assert_eq!(
            m.traffic.loaded_flows().len(),
            4,
            "the unfocused result is back"
        );
        assert!(m.traffic.data.as_ref().unwrap().truncated);
    }

    /// A focused result on screen: `B` and `]` re-run the focused query at
    /// the next cap, one query per side, asking above 10000; the answer
    /// says how `B` goes on.
    #[test]
    fn leaving_a_focus_puts_the_unfocused_cap_back() {
        let mut m = model_with_flows();
        m.traffic.data.as_mut().unwrap().truncated = true;
        let unfocused_cap = m.traffic.data.as_ref().unwrap().filter.max_results;
        quiet(&mut m, focus_cmd("shop/prod"));
        let focus = m.traffic.focus.clone().unwrap();
        let mut data = focused_data(&m, 2);
        data.truncated = true;
        m.traffic.apply_focused(focus, 2, data);
        let _ = update(&mut m, key('B')); // a focused cap step in flight
        assert_ne!(m.traffic.filter.max_results, unfocused_cap);
        quiet(&mut m, kc(KeyCode::Esc)); // cancels it, the focus goes
        assert_eq!(m.traffic.filter.max_results, unfocused_cap);
        // `B` now offers the unfocused result's next rung, not "r first".
        let cmds = update(&mut m, key('B'));
        assert!(
            !m.status.text.contains("nothing to backfill"),
            "{}",
            m.status.text
        );
        assert!(
            !cmds.is_empty() || matches!(m.overlay, Overlay::Confirm { .. }),
            "{cmds:?}"
        );
    }

    #[test]
    fn b_and_brackets_raise_the_cap_of_a_focused_result() {
        let mut m = model_with_flows();
        m.traffic.data.as_mut().unwrap().truncated = true;
        quiet(&mut m, focus_cmd("shop/prod"));
        let focus = m.traffic.focus.clone().unwrap();
        let mut data = focused_data(&m, 2);
        data.truncated = true;
        m.traffic.apply_focused(focus.clone(), 2, data);
        let cmds = update(&mut m, key('B'));
        let [
            Cmd::RunFocused {
                filter,
                focus: f,
                mode,
                ..
            },
        ] = cmds.as_slice()
        else {
            panic!("{cmds:?}")
        };
        assert_eq!((filter.max_results, *mode), (10_000, CacheMode::Use));
        assert_eq!(f, &focus);
        assert_eq!(m.traffic.filter.max_results, 10_000);
        assert!(m.traffic.focus_result.is_some(), "still focused");
        let outcome = QueryOutcome {
            flows: m.traffic.loaded_flows().to_vec(),
            truncated: true,
            incomplete: false,
            elapsed: Duration::from_millis(900),
            hit: false,
            filter: filter.clone(),
            bounds: None,
            hrefs: Vec::new(),
        };
        let seq = m.traffic.seq;
        quiet(
            &mut m,
            Msg::FocusedDone {
                seq,
                result: Ok(FocusedOutcome {
                    outcome,
                    focus: focus.clone(),
                    queries: 2,
                }),
            },
        );
        assert_eq!(
            m.status.text,
            "2 flows for shop/prod · 2 queries, capped at 10000; B raises the cap"
        );
        let out = crate::tui::test_support::render_to_string(&m, 160, 40);
        assert!(out.contains("capped at 10000; B raises the cap"), "{out}");
        // Above 10000 the next rung asks, naming the queries; n changes nothing.
        quiet(&mut m, key(']'));
        match &m.overlay {
            Overlay::Confirm {
                reason,
                action: PendingAction::RunFocused(f, _, CacheMode::Use),
            } => {
                assert_eq!(f.max_results, 25_000);
                assert!(
                    reason.starts_with(
                        "cap 25000 exceeds 10000. Re-run shop/prod at cap 25000 (2 queries"
                    ),
                    "{reason}"
                );
            }
            o => panic!("{o:?}"),
        }
        quiet(&mut m, key('n'));
        assert_eq!(m.traffic.filter.max_results, 10_000);
        quiet(&mut m, key('B'));
        let cmds = update(&mut m, key('y'));
        assert!(
            matches!(cmds.as_slice(), [Cmd::RunFocused { filter, .. }] if filter.max_results == 25_000),
            "{cmds:?}"
        );
    }

    #[test]
    fn a_stale_focused_answer_is_dropped() {
        let mut m = model_with_flows();
        m.traffic.data.as_mut().unwrap().truncated = true;
        quiet(&mut m, focus_cmd("shop/prod"));
        let _ = update(&mut m, key('r'));
        let stale = m.traffic.seq;
        m.traffic.cancel_query();
        let outcome = QueryOutcome {
            flows: Vec::new(),
            truncated: false,
            incomplete: false,
            elapsed: Duration::ZERO,
            hit: false,
            filter: m.traffic.filter.clone(),
            bounds: None,
            hrefs: Vec::new(),
        };
        let focus = m.traffic.focus.clone().unwrap();
        quiet(
            &mut m,
            Msg::FocusedDone {
                seq: stale,
                result: Ok(FocusedOutcome {
                    outcome,
                    focus,
                    queries: 2,
                }),
            },
        );
        assert!(m.traffic.focus_result.is_none());
        assert_eq!(m.traffic.loaded_flows().len(), 4);
    }

    #[test]
    fn an_expensive_focused_query_asks_first_and_names_the_count() {
        let mut m = model_with_flows();
        m.traffic.data.as_mut().unwrap().truncated = true;
        m.traffic.filter.max_results = 20_000;
        quiet(&mut m, focus_cmd("shop/prod"));
        quiet(&mut m, key('X'));
        quiet(&mut m, key('r'));
        let Overlay::Confirm { reason, .. } = &m.overlay else {
            panic!("{:?}", m.overlay)
        };
        assert_eq!(
            reason,
            "cap 20000 exceeds 10000. Run 1 query for shop/prod anyway? (y/n)"
        );
        let cmds = update(&mut m, key('y'));
        assert!(
            matches!(cmds.as_slice(), [Cmd::RunFocused { focus, .. }] if focus.direction == FocusDirection::In),
            "{cmds:?}"
        );
    }

    #[test]
    fn a_group_without_labels_is_not_re_queried() {
        let mut m = model_with_flows();
        m.traffic.data.as_mut().unwrap().truncated = true;
        m.traffic.view = TrafficView::Blocked;
        let nas = |r: &crate::tui::model::SectionRow| {
            r.peer()
                .is_some_and(|(_, p)| p.peer_text() == "unmanaged-nas/-")
        };
        m.traffic.blocked.select_where(nas);
        quiet(&mut m, key('F'));
        m.traffic.blocked.select_where(nas);
        quiet(&mut m, key('F'));
        assert_eq!(focused(&m).unwrap().0, "unmanaged-nas/-");
        quiet(&mut m, key('r'));
        assert_eq!(
            m.status.text,
            "unmanaged-nas/- is not a label group: it cannot be re-queried by its labels; Esc clears the focus, then r reruns the whole query"
        );
    }

    #[test]
    fn r_without_a_partial_focus_reruns_the_unfocused_query() {
        let mut m = model_with_flows();
        quiet(&mut m, focus_cmd("shop/prod"));
        let cmds = update(&mut m, key('r'));
        assert!(
            matches!(
                cmds.as_slice(),
                [Cmd::RunQuery {
                    mode: CacheMode::Bypass,
                    ..
                }]
            ),
            "{cmds:?}"
        );
    }

    #[test]
    fn window_and_cap_keys_work_on_the_unfocused_result() {
        let mut m = model_with_flows();
        m.traffic.data.as_mut().unwrap().truncated = true;
        quiet(&mut m, focus_cmd("shop/prod"));
        let f = m.traffic.focus.clone().unwrap();
        let data = focused_data(&m, 2);
        m.traffic.apply_focused(f, 2, data);
        let _ = update(&mut m, key('-'));
        assert!(m.traffic.unfocused.is_none() && m.traffic.focus_result.is_none());
        assert_eq!(m.traffic.loaded_flows().len(), 4);
        assert!(
            m.traffic.focus.is_some(),
            "the focus stays and narrows the unfocused result"
        );
        // A new unfocused result also drops a focused one kept aside.
        let f = m.traffic.focus.clone().unwrap();
        let data = focused_data(&m, 1);
        m.traffic.apply_focused(f, 2, data);
        let seq = m.traffic.seq;
        let outcome = QueryOutcome {
            flows: crate::tui::test_support::fixture_flows(),
            truncated: true,
            incomplete: false,
            elapsed: Duration::from_millis(100),
            hit: false,
            filter: m.traffic.filter.clone(),
            bounds: None,
            hrefs: Vec::new(),
        };
        let _ = update(
            &mut m,
            Msg::QueryDone {
                seq,
                result: Ok(outcome),
            },
        );
        assert!(m.traffic.unfocused.is_none() && m.traffic.focus_result.is_none());
        assert!(
            m.traffic.focus_partial(),
            "the new result is truncated: partial again"
        );
    }

    /// A focused run in flight on the rich fixture (truncated), focused on
    /// crm/prod: its seq and focus.
    fn in_flight() -> (Model, u64, pce::traffic::Focus) {
        let mut m = model_with_rich_flows();
        m.traffic.data.as_mut().unwrap().truncated = true;
        quiet(&mut m, focus_cmd("crm/prod"));
        let cmds = update(&mut m, key('r'));
        let [Cmd::RunFocused { seq, focus, .. }] = cmds.as_slice() else {
            panic!("{cmds:?}")
        };
        assert!(m.traffic.is_running());
        assert_eq!(m.traffic.running_focus(), Some(focus));
        (m, *seq, focus.clone())
    }

    /// The late answer of the run `in_flight` started.
    fn late(m: &mut Model, seq: u64, focus: pce::traffic::Focus) {
        let outcome = QueryOutcome {
            flows: m.traffic.loaded_flows()[..2].to_vec(),
            truncated: false,
            incomplete: false,
            elapsed: Duration::from_millis(900),
            hit: false,
            filter: m.traffic.filter.clone(),
            bounds: None,
            hrefs: Vec::new(),
        };
        quiet(
            m,
            Msg::FocusedDone {
                seq,
                result: Ok(FocusedOutcome {
                    outcome,
                    focus,
                    queries: 2,
                }),
            },
        );
        assert!(
            m.traffic.focus_result.is_none(),
            "the late answer is dropped"
        );
        assert!(m.traffic.unfocused.is_none());
        assert_eq!(
            m.traffic.loaded_flows().len(),
            30,
            "the unfocused result stays"
        );
    }

    #[test]
    fn esc_during_a_focused_run_cancels_it_and_its_answer_never_revives_the_focus() {
        let (mut m, seq, focus) = in_flight();
        quiet(&mut m, kc(KeyCode::Esc));
        assert!(!m.traffic.is_running(), "Esc cancelled the focused run");
        assert_eq!(focused(&m), None);
        late(&mut m, seq, focus);
        assert_eq!(focused(&m), None);
        assert_eq!(m.traffic.flows.len(), 30);
    }

    #[test]
    fn another_app_during_a_focused_run_cancels_it() {
        let (mut m, seq, focus) = in_flight();
        quiet(&mut m, focus_cmd("shop/prod"));
        assert!(!m.traffic.is_running());
        late(&mut m, seq, focus);
        assert_eq!(
            focused(&m),
            Some(("shop/prod".into(), FocusDirection::Both))
        );
    }

    #[test]
    fn x_during_a_focused_run_cancels_it() {
        let (mut m, seq, focus) = in_flight();
        quiet(&mut m, key('X'));
        assert!(!m.traffic.is_running());
        late(&mut m, seq, focus);
        assert_eq!(focused(&m), Some(("crm/prod".into(), FocusDirection::In)));
        assert_eq!(
            m.traffic.flows.len(),
            7,
            "still narrowed by the new direction"
        );
    }

    #[test]
    fn g_during_a_focused_run_cancels_it() {
        let (mut m, seq, focus) = in_flight();
        apply_group_keys(&mut m, vec!["app".into()], false);
        assert!(!m.traffic.is_running());
        late(&mut m, seq, focus);
        assert_eq!(focused(&m), None, "the old keys' focus is not revived");
    }

    #[test]
    fn a_focused_answer_for_another_focus_is_dropped_even_on_the_current_seq() {
        let (mut m, seq, focus) = in_flight();
        // Bypass the cancel: the outcome guard alone must hold.
        m.traffic.focus = Some(focus.clone().with_direction(FocusDirection::Out));
        late(&mut m, seq, focus);
        assert!(!m.traffic.is_running());
        assert_eq!(focused(&m).unwrap().1, FocusDirection::Out);
    }

    #[test]
    fn f_on_an_app_to_app_pair_takes_the_source_then_the_destination() {
        let mut m = model_with_flows();
        quiet(&mut m, key('v'));
        assert_eq!(m.traffic.view, TrafficView::Aggregate);
        assert_eq!(m.traffic.apps_state.by, By::Pairs);
        let fqdn = |g: &pce::traffic::FlowGroup| g.dst.join("/") == "fqdn-1.example/-";
        m.traffic.groups.select_where(fqdn);
        quiet(&mut m, key('F'));
        assert_eq!(focused(&m).unwrap().0, "shop/prod");
        m.traffic.groups.select_where(fqdn);
        quiet(&mut m, key('F'));
        assert_eq!(
            focused(&m).unwrap().0,
            "fqdn-1.example/-",
            "F again: the other side"
        );
    }

    #[test]
    fn f_on_a_mesh_row_takes_the_source_then_the_destination() {
        let mut m = model_with_flows();
        m.traffic.view = TrafficView::Mesh;
        let nas = |r: &pce::traffic::MeshRow| r.src.join("/") == "unmanaged-nas/-";
        m.traffic.mesh.select_where(nas);
        quiet(&mut m, key('F'));
        assert_eq!(focused(&m).unwrap().0, "unmanaged-nas/-");
        m.traffic.mesh.select_where(nas);
        quiet(&mut m, key('F'));
        assert_eq!(focused(&m).unwrap().0, "shop/prod");
    }

    #[test]
    fn f_on_a_treemap_tile_takes_its_source_group() {
        let mut m = model_with_rich_flows();
        m.traffic.view = TrafficView::Treemap;
        let tm = &m.traffic.treemap;
        let target = tm.current().expect("a tile under the cursor");
        assert!(target.inner.is_none() && tm.zoom.is_none());
        let label = tm.tile(target).label.clone();
        assert!(!tm.tile(target).is_other(), "{label}");
        quiet(&mut m, key('F'));
        assert_eq!(
            focused(&m).unwrap().0,
            label,
            "the outer tile is a source group"
        );
        assert!(
            m.traffic
                .treemap
                .groups
                .iter()
                .all(|g| g.src.join("/") == label || g.dst.join("/") == label),
            "the treemap is narrowed to it"
        );
    }
}
#[cfg(test)]
mod app_tests {
    use super::*;
    use crate::tui::app_view::{AppPanel, Loadable};
    use crate::tui::model::{Model, Overlay, TrafficView, WritesState};
    use crate::tui::msg::AppWorkloadsPayload;
    use crate::tui::test_support::{model_with_flows, with_policy, with_workloads};
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use pce::models::Workload;
    use pce::traffic::FocusDirection;

    fn key(c: char) -> Msg {
        Msg::Key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE))
    }

    fn kc(code: KeyCode) -> Msg {
        Msg::Key(KeyEvent::new(code, KeyModifiers::NONE))
    }

    #[track_caller]
    fn quiet(m: &mut Model, msg: Msg) {
        assert_eq!(update(m, msg), Vec::<Cmd>::new());
    }

    /// Every command that reaches the PCE (all but writes-off, export, quit).
    fn requests(cmds: &[Cmd]) -> u32 {
        cmds.iter()
            .filter(|c| !matches!(c, Cmd::SetWrites(_) | Cmd::Export { .. } | Cmd::Quit))
            .count() as u32
    }

    /// The fixture rulesets with Global-Infra (no inline deny rules) first:
    /// selected on the Policy screen, it would ask for its deny rules.
    fn rulesets_answer(hit: bool) -> Msg {
        let mut rs: Vec<pce::models::RuleSet> =
            serde_json::from_str(include_str!("../../../../fixtures/rule_sets.json")).unwrap();
        rs.reverse();
        assert!(rs[0].deny_rules.is_empty());
        Msg::RuleSetsLoaded {
            scope: PolicyScope::Draft,
            result: Ok((rs, u64::from(hit) * 3, hit)),
        }
    }

    fn view(m: &Model) -> &crate::tui::app_view::AppView {
        m.app_view.as_ref().expect("the overlay's state")
    }

    fn shop_labels() -> Vec<String> {
        vec!["/orgs/5/labels/3".into(), "/orgs/5/labels/4".into()]
    }

    fn fixture_workloads() -> Vec<Workload> {
        serde_json::from_str(include_str!("../../../../fixtures/workloads.json")).unwrap()
    }

    /// Flows, workloads and rulesets all loaded: the overlay costs nothing.
    fn everything() -> Model {
        with_policy(with_workloads(model_with_flows()))
    }

    #[test]
    fn a_opens_the_overlay_for_the_row_app_with_at_most_two_requests() {
        let mut m = model_with_flows();
        m.traffic
            .flows
            .select_where(|r| r.flow.dst.display_name() == "fqdn-1.example");
        let cmds = update(&mut m, key('A'));
        assert_eq!(
            cmds,
            vec![
                Cmd::LoadAppWorkloads {
                    seq: 1,
                    filter: pce::app::app_workload_filter(&shop_labels()),
                    mode: CacheMode::Use,
                },
                Cmd::LoadRuleSets {
                    scope: PolicyScope::Draft,
                    mode: CacheMode::Use,
                },
            ]
        );
        assert_eq!(m.overlay, Overlay::App);
        let v = view(&m);
        assert_eq!(v.requests, 2);
        assert_eq!(v.focus.app_text(), "shop/prod");
        assert_eq!(v.focus.direction, FocusDirection::Both, "no focus: both");
        assert_eq!((v.inbound.len(), v.outbound.len()), (3, 2));
        assert_eq!(v.header_title(), "shop/prod · in+out · last 24h");
        assert!(matches!(v.workloads, Loadable::Loading) && matches!(v.policy, Loadable::Loading));
    }

    #[test]
    fn loaded_screens_make_the_overlay_free() {
        let mut m = everything();
        quiet(&mut m, key('A'));
        let v = view(&m);
        assert_eq!(v.requests, 0);
        assert_eq!(v.header_meta(), "0 req · cached");
        let Loadable::Loaded((rows, s)) = &v.workloads else {
            panic!("{:?}", v.workloads)
        };
        assert_eq!(
            rows.iter().map(|w| w.name.as_str()).collect::<Vec<_>>(),
            vec!["host-1.example", "host-2.example"]
        );
        assert_eq!(s.enforcement.text(), "enforced 1 · visibility 1");
        assert_eq!(s.ven_text(), "1 active · 1 stale");
        let Loadable::Loaded(p) = &v.policy else {
            panic!("{:?}", v.policy)
        };
        assert_eq!(
            p.iter().map(|r| r.name.as_str()).collect::<Vec<_>>(),
            vec!["Shop-Prod", "Global-Infra"]
        );
    }

    #[test]
    fn the_overlay_takes_the_focus_direction_and_x_cycles_it_without_requests() {
        let mut m = everything();
        quiet(
            &mut m,
            Msg::Command(Command::Focus(Some("shop/prod".into()))),
        );
        quiet(&mut m, key('X')); // in
        quiet(&mut m, key('A'));
        assert_eq!(view(&m).focus.direction, FocusDirection::In);
        assert_eq!(view(&m).outbound.len(), 0);
        quiet(&mut m, key('X'));
        assert_eq!(view(&m).focus.direction, FocusDirection::Out);
        assert_eq!((view(&m).inbound.len(), view(&m).outbound.len()), (0, 2));
        assert_eq!(
            m.traffic.focus.as_ref().unwrap().direction,
            FocusDirection::In,
            "the overlay's X leaves the traffic focus alone"
        );
    }

    #[test]
    fn esc_closes_and_tab_and_j_k_move_without_requests() {
        let mut m = everything();
        quiet(&mut m, key('A'));
        quiet(&mut m, key('j'));
        assert_eq!(view(&m).cursor[0], 1);
        quiet(&mut m, kc(KeyCode::Tab));
        assert_eq!(view(&m).panel, AppPanel::Outbound);
        quiet(&mut m, kc(KeyCode::BackTab));
        assert_eq!(view(&m).panel, AppPanel::Inbound);
        quiet(&mut m, kc(KeyCode::Esc));
        assert_eq!(m.overlay, Overlay::None);
        assert_eq!(m.screen, Screen::Traffic);
        assert_eq!(
            m.traffic.view,
            TrafficView::Flows,
            "back to the view it was opened from"
        );
    }

    #[test]
    fn enter_on_a_peer_focuses_that_pair_in_flows() {
        let mut m = everything();
        quiet(&mut m, key('A'));
        quiet(&mut m, kc(KeyCode::Enter)); // inbound row 0: shop/prod -> shop/prod 5432/tcp
        assert_eq!(m.overlay, Overlay::None);
        assert_eq!(m.traffic.view, TrafficView::Flows);
        assert_eq!(
            m.traffic.focus.as_ref().map(|f| f.app_text()).as_deref(),
            Some("shop/prod")
        );
        assert_eq!(
            m.traffic.drill,
            Some(Drill::Service(
                vec!["shop".into(), "prod".into()],
                vec!["shop".into(), "prod".into()],
                "5432/tcp".into(),
                Decisions::All
            ))
        );
        assert_eq!(m.traffic.flows.len(), 1);
    }

    #[test]
    fn enter_on_a_workload_or_a_ruleset_opens_it_on_its_screen() {
        let mut m = everything();
        quiet(&mut m, key('A'));
        for _ in 0..3 {
            quiet(&mut m, kc(KeyCode::Tab));
        }
        assert_eq!(view(&m).panel, AppPanel::Workloads);
        quiet(&mut m, key('j'));
        let _ = update(&mut m, kc(KeyCode::Enter));
        assert_eq!(m.screen, Screen::Workloads);
        assert!(m.workloads.show_detail);
        assert_eq!(
            m.workloads
                .table
                .selected_item()
                .map(|r| r.workload.href.as_str()),
            Some("/orgs/5/workloads/aaaa-2")
        );
        m.screen = Screen::Traffic;
        quiet(&mut m, key('A'));
        assert_eq!(
            view(&m).panel,
            AppPanel::Workloads,
            "reopened where it was left"
        );
        quiet(&mut m, kc(KeyCode::BackTab));
        // Policy's own selection sits on a ruleset left to the fallback.
        m.policy.rulesets.select_where(|r| r.name != "Shop-Prod");
        let cmds = update(&mut m, kc(KeyCode::Enter));
        assert_eq!(m.screen, Screen::Policy);
        let chosen = m.policy.rulesets.selected_item().map(|r| r.href.clone());
        assert_eq!(
            m.policy.rulesets.selected_item().map(|r| r.name.as_str()),
            Some("Shop-Prod")
        );
        // No deny rules fetched for the row selected before Enter.
        for c in &cmds {
            if let Cmd::LoadDenyRules { href, .. } = c {
                assert_eq!(Some(href), chosen.as_ref(), "{cmds:?}");
            }
        }
    }

    #[test]
    fn entering_policy_during_a_rulesets_reload_waits_for_its_answer() {
        let mut m = everything();
        m.policy.rulesets.select_where(|r| r.name != "Shop-Prod");
        m.policy.loading = true;
        let cmds = switch(&mut m, Screen::Policy);
        assert!(
            !cmds.iter().any(|c| matches!(c, Cmd::LoadDenyRules { .. })),
            "{cmds:?}"
        );
        m.policy.loading = false;
        m.screen = Screen::Traffic;
        let cmds = switch(&mut m, Screen::Policy);
        assert!(
            cmds.iter().any(|c| matches!(c, Cmd::LoadDenyRules { .. })),
            "{cmds:?}"
        );
    }

    #[test]
    fn w_drafts_an_allow_rule_from_a_blocked_peer_only() {
        let mut m = everything();
        m.writes = WritesState::on();
        quiet(&mut m, key('A'));
        quiet(&mut m, kc(KeyCode::Enter)); // leave via a peer...
        quiet(&mut m, key('A')); // ...and reopen: nothing sent
        quiet(&mut m, key('w'));
        assert_eq!(m.status.text, "this row is allowed; nothing to add");
        quiet(&mut m, key('j'));
        quiet(&mut m, key('j')); // unmanaged-nas/- icmp, blocked
        let _ = update(&mut m, key('w'));
        assert!(matches!(m.overlay, Overlay::Form(_)), "{:?}", m.overlay);
    }

    #[test]
    fn a_w_form_from_the_overlay_returns_to_it_after_a_cancel_or_the_write() {
        use crate::tui::test_support::label_post_plan;
        let mut m = everything();
        m.writes = WritesState::on();
        quiet(&mut m, key('A'));
        quiet(&mut m, key('j'));
        quiet(&mut m, key('j')); // a blocked peer
        let open = |m: &mut Model| {
            quiet(m, key('w'));
            let Overlay::Form(f) = &mut m.overlay else {
                panic!("{:?}", m.overlay)
            };
            for (label, value) in [("Ruleset", "Shop-Prod"), ("Services", "443/tcp")] {
                f.fields
                    .iter_mut()
                    .find(|x| x.label == label)
                    .unwrap()
                    .value = value.into();
            }
        };
        // Esc in the form.
        open(&mut m);
        quiet(&mut m, kc(KeyCode::Esc));
        assert_eq!(m.overlay, Overlay::App, "back, nothing sent");
        assert_eq!(view(&m).panel, AppPanel::Inbound, "where it was");
        // `n` on the confirm.
        open(&mut m);
        assert_eq!(update(&mut m, kc(KeyCode::Enter)).len(), 1);
        let planned = || Msg::Planned(Ok((label_post_plan(), pce::write::ObjectKind::Rule)));
        quiet(&mut m, planned());
        assert!(matches!(m.overlay, Overlay::ConfirmWrite { .. }));
        quiet(&mut m, key('n'));
        assert_eq!(m.overlay, Overlay::App);
        // The write done.
        open(&mut m);
        update(&mut m, kc(KeyCode::Enter));
        quiet(&mut m, planned());
        assert_eq!(update(&mut m, key('y')).len(), 1);
        let requests_before = view(&m).requests;
        update(
            &mut m,
            Msg::WriteDone {
                kind: pce::write::ObjectKind::Rule,
                method: Some(pce::WriteMethod::Post),
                result: Ok(pce::write::Outcome::Written(pce::write::Written {
                    href: "/orgs/5/sec_policy/draft/rule_sets/100/sec_rules/9".into(),
                    pending: true,
                    status: 201,
                    object: serde_json::json!({}),
                    dropped_fields: vec![],
                    warnings: vec![],
                    audit_failed: None,
                })),
            },
        );
        assert_eq!(m.overlay, Overlay::App);
        assert_eq!(view(&m).requests, requests_before, "the view is cached");
        // A form opened on Traffic still closes to the screen.
        quiet(&mut m, kc(KeyCode::Esc));
        m.traffic.view = TrafficView::Flows;
        m.traffic.flows.select_where(|r| {
            matches!(
                r.flow.policy_decision,
                PolicyDecision::Blocked | PolicyDecision::PotentiallyBlocked
            )
        });
        quiet(&mut m, key('w'));
        assert!(matches!(m.overlay, Overlay::Form(_)));
        quiet(&mut m, kc(KeyCode::Esc));
        assert_eq!(m.overlay, Overlay::None);
    }

    #[test]
    fn a_w_write_from_the_overlay_returns_to_it_with_one_draft_run() {
        use crate::tui::test_support::rule_post_plan;
        let mut m = everything();
        m.traffic.data.as_mut().unwrap().hrefs =
            vec!["/orgs/5/traffic_flows/async_queries/q-1".into()];
        m.writes = WritesState::on();
        quiet(&mut m, key('A'));
        quiet(&mut m, key('j'));
        quiet(&mut m, key('j')); // a blocked peer
        quiet(&mut m, key('w'));
        let Overlay::Form(f) = &mut m.overlay else {
            panic!("{:?}", m.overlay)
        };
        for (label, value) in [("Ruleset", "Shop-Prod"), ("Services", "443/tcp")] {
            f.fields
                .iter_mut()
                .find(|x| x.label == label)
                .unwrap()
                .value = value.into();
        }
        assert_eq!(update(&mut m, kc(KeyCode::Enter)).len(), 1);
        quiet(
            &mut m,
            Msg::Planned(Ok((rule_post_plan(), pce::write::ObjectKind::Rule))),
        );
        assert_eq!(update(&mut m, key('y')).len(), 1);
        let cmds = update(
            &mut m,
            Msg::WriteDone {
                kind: pce::write::ObjectKind::Rule,
                method: Some(pce::WriteMethod::Post),
                result: Ok(pce::write::Outcome::Written(pce::write::Written {
                    href: "/orgs/5/sec_policy/draft/rule_sets/100/sec_rules/9".into(),
                    pending: true,
                    status: 201,
                    object: serde_json::json!({}),
                    dropped_fields: vec![],
                    warnings: vec![],
                    audit_failed: None,
                })),
            },
        );
        let runs = cmds
            .iter()
            .filter(|c| matches!(c, Cmd::RunDraft { .. }))
            .count();
        assert_eq!(runs, 1, "{cmds:?}");
        assert_eq!(m.overlay, Overlay::App, "back to the overlay");
        assert!(
            m.status
                .text
                .ends_with("draft re-evaluation running · Esc on Traffic cancels"),
            "{}",
            m.status.text
        );
    }

    #[test]
    fn the_way_back_to_the_overlay_never_outlives_its_write() {
        use crate::tui::test_support::label_post_plan;
        let mut m = everything();
        m.writes = WritesState::on();
        quiet(&mut m, key('A'));
        quiet(&mut m, key('j'));
        quiet(&mut m, key('j'));
        quiet(&mut m, key('w'));
        let Overlay::Form(f) = &mut m.overlay else {
            panic!("{:?}", m.overlay)
        };
        for (label, value) in [("Ruleset", "Shop-Prod"), ("Services", "443/tcp")] {
            f.fields
                .iter_mut()
                .find(|x| x.label == label)
                .unwrap()
                .value = value.into();
        }
        update(&mut m, kc(KeyCode::Enter));
        quiet(
            &mut m,
            Msg::Planned(Ok((label_post_plan(), pce::write::ObjectKind::Rule))),
        );
        assert_eq!(update(&mut m, key('y')).len(), 1);
        // Help opened while the write is on its way: it stays, the way back goes.
        m.overlay = Overlay::Help;
        update(
            &mut m,
            Msg::WriteDone {
                kind: pce::write::ObjectKind::Rule,
                method: Some(pce::WriteMethod::Post),
                result: Ok(pce::write::Outcome::Skipped {
                    existing_href: "/orgs/5/sec_policy/draft/rule_sets/100/sec_rules/9".into(),
                    reason: "exists".into(),
                }),
            },
        );
        assert_eq!(m.overlay, Overlay::Help);
        assert!(!m.form_from_app);
        // A later provision's `n` closes to the screen, not the overlay.
        m.overlay = Overlay::None;
        quiet(
            &mut m,
            Msg::Planned(Ok((label_post_plan(), pce::write::ObjectKind::Provision))),
        );
        quiet(&mut m, key('n'));
        assert_eq!(m.overlay, Overlay::None);
    }

    #[test]
    fn the_peer_panel_in_force_is_one_the_direction_shows() {
        let mut m = everything();
        m.writes = WritesState::on();
        quiet(&mut m, key('A'));
        assert_eq!(view(&m).panel, AppPanel::Inbound);
        quiet(&mut m, key('X')); // in
        quiet(&mut m, key('X')); // out: Inbound now says "X shows inbound"
        assert_eq!(view(&m).panel, AppPanel::Outbound);
        quiet(&mut m, key('w'));
        assert_eq!(m.status.text, "this row is allowed; nothing to add");
        quiet(&mut m, key('X')); // both: stays where it is
        assert_eq!(view(&m).panel, AppPanel::Outbound);
        quiet(&mut m, key('X')); // in
        assert_eq!(view(&m).panel, AppPanel::Inbound);
        // Opened at the traffic focus's direction (`out`): on Outbound.
        quiet(&mut m, kc(KeyCode::Esc));
        m.app_view = None;
        m.traffic.set_focus(
            Focus::new(vec!["shop".into(), "prod".into()], Vec::new())
                .with_direction(FocusDirection::Out),
        );
        quiet(&mut m, key('A'));
        assert_eq!(view(&m).focus.direction, FocusDirection::Out);
        assert_eq!(view(&m).panel, AppPanel::Outbound);
        // The other panels are left alone.
        quiet(&mut m, kc(KeyCode::Tab));
        quiet(&mut m, key('X'));
        assert_eq!(view(&m).panel, AppPanel::Policy);
    }

    #[test]
    fn r_asks_then_reloads_both_panels_past_the_cache() {
        let mut m = everything();
        quiet(&mut m, key('A'));
        quiet(&mut m, key('r'));
        let Overlay::Confirm { reason, .. } = &m.overlay else {
            panic!("{:?}", m.overlay)
        };
        assert_eq!(
            reason,
            "Reload the workloads and the rulesets of shop/prod: 2 requests? (y/n)"
        );
        quiet(&mut m, key('n'));
        assert_eq!(
            m.overlay,
            Overlay::App,
            "no: back to the overlay, nothing sent"
        );
        quiet(&mut m, key('r'));
        let cmds = update(&mut m, key('y'));
        assert_eq!(requests(&cmds), 2);
        assert!(cmds.iter().all(|c| matches!(
            c,
            Cmd::LoadAppWorkloads {
                mode: CacheMode::Bypass,
                ..
            } | Cmd::LoadRuleSets {
                mode: CacheMode::Bypass,
                ..
            }
        )));
        assert_eq!(m.overlay, Overlay::App);
        assert_eq!(view(&m).requests, 2);
    }

    #[test]
    fn the_overlay_never_sends_a_request_it_did_not_count() {
        let mut m = model_with_flows();
        let mut sent = 0;
        for msg in [
            key('A'),
            kc(KeyCode::Tab),
            key('j'),
            key('X'),
            key('X'),
            key('X'),
            kc(KeyCode::BackTab),
            key('k'),
            key('r'),
            key('y'),
            key('X'),
        ] {
            sent += requests(&update(&mut m, msg));
        }
        assert_eq!(
            sent, 3,
            "open: workloads and rulesets; r: the workloads again (the rulesets are still loading)"
        );
        assert_eq!(view(&m).requests, sent);
    }

    #[test]
    fn t_in_the_overlay_does_nothing_now_the_timeline_is_gone() {
        let mut m = everything();
        quiet(&mut m, key('A'));
        let before = m.status.text.clone();
        assert!(update(&mut m, key('t')).is_empty(), "no request");
        assert_eq!(m.overlay, Overlay::App, "no prompt");
        assert_eq!(m.status.text, before);
    }

    #[test]
    fn answers_fill_the_panels_and_stale_ones_are_dropped() {
        let mut m = model_with_flows();
        let _ = update(&mut m, key('A'));
        let seq = view(&m).seq;
        quiet(
            &mut m,
            Msg::AppWorkloadsLoaded {
                seq: seq + 7,
                result: Ok(AppWorkloadsPayload {
                    workloads: fixture_workloads(),
                    truncated: false,
                    hit: true,
                }),
            },
        );
        assert!(
            matches!(view(&m).workloads, Loadable::Loading),
            "a stale answer is dropped"
        );
        quiet(
            &mut m,
            Msg::AppWorkloadsLoaded {
                seq,
                result: Ok(AppWorkloadsPayload {
                    workloads: fixture_workloads(),
                    truncated: false,
                    hit: true,
                }),
            },
        );
        assert!(
            matches!(&view(&m).workloads, Loadable::Loaded((rows, _)) if rows.len() == 2),
            "filtered to the app"
        );
        assert_eq!(view(&m).cached, 1);
        quiet(&mut m, rulesets_answer(true));
        assert!(matches!(&view(&m).policy, Loadable::Loaded(rows) if rows.len() == 2));
        assert_eq!(view(&m).header_meta(), "2 req · cached");
    }

    #[test]
    fn a_stale_workloads_answer_is_dropped() {
        let mut m = model_with_flows();
        let _ = update(&mut m, key('A'));
        let old = view(&m).seq;
        quiet(&mut m, kc(KeyCode::Esc));
        // Another app (row 2's source): a new overlay, a new sequence number.
        m.traffic
            .flows
            .select_where(|r| r.flow.src.display_name() == "unmanaged-nas");
        quiet(&mut m, key('A'));
        assert_ne!(view(&m).seq, old);
        quiet(
            &mut m,
            Msg::AppWorkloadsLoaded {
                seq: old,
                result: Ok(AppWorkloadsPayload {
                    workloads: fixture_workloads(),
                    truncated: false,
                    hit: false,
                }),
            },
        );
        assert!(matches!(view(&m).workloads, Loadable::Unavailable(_)));
        assert_eq!(view(&m).focus.app_text(), "unmanaged-nas/-");
    }

    #[test]
    fn a_traffic_answer_under_the_open_overlay_rebuilds_its_peers() {
        let mut m = everything();
        quiet(&mut m, key('A'));
        assert_eq!(view(&m).inbound.len(), 3);
        let flows = m.traffic.loaded_flows()[..1].to_vec(); // shop/prod -> shop/prod
        let seq = m.traffic.seq;
        let filter = m.traffic.filter.clone();
        let _ = update(
            &mut m,
            Msg::QueryDone {
                seq,
                result: Ok(crate::tui::msg::QueryOutcome {
                    flows,
                    truncated: false,
                    incomplete: false,
                    elapsed: std::time::Duration::ZERO,
                    hit: false,
                    filter,
                    bounds: None,
                    hrefs: Vec::new(),
                }),
            },
        );
        assert_eq!(m.overlay, Overlay::App);
        assert_eq!((view(&m).inbound.len(), view(&m).outbound.len()), (1, 1));
    }

    #[test]
    fn the_request_budget_holds_end_to_end_answers_included() {
        let mut m = model_with_flows();
        let mut sent = requests(&update(&mut m, key('A')));
        let seq = view(&m).seq;
        let workloads = || Msg::AppWorkloadsLoaded {
            seq,
            result: Ok(AppWorkloadsPayload {
                workloads: fixture_workloads(),
                truncated: false,
                hit: false,
            }),
        };
        sent += requests(&update(&mut m, workloads()));
        sent += requests(&update(&mut m, rulesets_answer(false)));
        assert_eq!(
            (sent, view(&m).requests),
            (2, 2),
            "no deny rules off the Policy screen"
        );
        assert_eq!(view(&m).header_meta(), "2 req");
        for msg in [key('r'), key('y')] {
            sent += requests(&update(&mut m, msg));
        }
        sent += requests(&update(&mut m, workloads()));
        sent += requests(&update(&mut m, rulesets_answer(true)));
        assert_eq!((sent, view(&m).requests), (4, 4));
        assert_eq!(view(&m).header_meta(), "4 req");
        // The Policy screen asks for the selected ruleset's deny rules on entry.
        quiet(&mut m, kc(KeyCode::Esc));
        let cmds = update(&mut m, Msg::Command(Command::Screen(Screen::Policy)));
        assert!(
            matches!(cmds.as_slice(), [Cmd::LoadDenyRules { .. }]),
            "{cmds:?}"
        );
    }

    fn one_flow_answer(m: &Model) -> Msg {
        Msg::QueryDone {
            seq: m.traffic.seq,
            result: Ok(crate::tui::msg::QueryOutcome {
                flows: m.traffic.loaded_flows()[..1].to_vec(),
                truncated: false,
                incomplete: false,
                elapsed: std::time::Duration::ZERO,
                hit: false,
                filter: m.traffic.filter.clone(),
                bounds: None,
                hrefs: Vec::new(),
            }),
        }
    }

    #[test]
    fn a_on_pairs_mesh_and_treemap_opens_the_rows_app() {
        let mut m = everything();
        quiet(&mut m, key('v'));
        assert_eq!(m.traffic.apps_state.by, crate::tui::model::By::Pairs);
        m.traffic
            .groups
            .select_where(|g| g.dst.join("/") == "fqdn-1.example/-");
        quiet(&mut m, key('A'));
        assert_eq!(view(&m).focus.app_text(), "shop/prod", "a pair: its source");
        quiet(&mut m, kc(KeyCode::Esc));

        m.traffic.view = TrafficView::Mesh;
        m.traffic
            .mesh
            .select_where(|r| r.src.join("/") == "unmanaged-nas/-");
        quiet(&mut m, key('A'));
        assert_eq!(
            view(&m).focus.app_text(),
            "unmanaged-nas/-",
            "a mesh row: its source"
        );
        quiet(&mut m, kc(KeyCode::Esc));

        let mut m = with_policy(with_workloads(
            crate::tui::test_support::model_with_rich_flows(),
        ));
        m.traffic.view = TrafficView::Treemap;
        let tm = &m.traffic.treemap;
        let label = tm.tile(tm.current().expect("a tile")).label.clone();
        let cmds = update(&mut m, key('A'));
        assert_eq!(m.overlay, Overlay::App);
        assert_eq!(
            view(&m).focus.app_text(),
            label,
            "a treemap tile: its source group"
        );
        assert!(requests(&cmds) <= 2);
        assert_eq!(requests(&cmds), view(&m).requests);
    }

    #[test]
    fn a_traffic_answer_under_the_reload_prompt_rebuilds_the_peers() {
        let mut m = everything();
        quiet(&mut m, key('A'));
        quiet(&mut m, key('r'));
        assert!(matches!(m.overlay, Overlay::Confirm { .. }));
        let answer = one_flow_answer(&m);
        let _ = update(&mut m, answer);
        quiet(&mut m, key('n'));
        assert_eq!(m.overlay, Overlay::App);
        assert_eq!((view(&m).inbound.len(), view(&m).outbound.len()), (1, 1));
    }

    #[test]
    fn a_reused_view_drops_policy_rows_of_the_other_scope() {
        let mut m = everything();
        quiet(&mut m, key('A'));
        assert_eq!(view(&m).policy_scope, PolicyScope::Draft);
        quiet(&mut m, kc(KeyCode::Esc));
        let cmds = update(&mut m, Msg::Command(Command::Active));
        assert!(matches!(
            cmds.as_slice(),
            [Cmd::LoadRuleSets {
                scope: PolicyScope::Active,
                ..
            }]
        ));
        m.screen = Screen::Traffic;
        quiet(&mut m, key('A')); // the active load is on its way: not doubled
        let v = view(&m);
        assert_eq!(v.policy_scope, PolicyScope::Active);
        assert!(
            matches!(v.policy, Loadable::Loading),
            "no draft rows: {:?}",
            v.policy
        );
        let mut rs: Vec<pce::models::RuleSet> =
            serde_json::from_str(include_str!("../../../../fixtures/rule_sets.json")).unwrap();
        rs.truncate(1);
        let _ = update(
            &mut m,
            Msg::RuleSetsLoaded {
                scope: PolicyScope::Active,
                result: Ok((rs, 0, false)),
            },
        );
        assert!(matches!(&view(&m).policy, Loadable::Loaded(rows) if rows.len() == 1));
    }

    #[test]
    fn the_reload_prompt_counts_what_it_will_send() {
        let mut m = model_with_flows();
        let _ = update(&mut m, key('A')); // workloads and rulesets on their way
        quiet(&mut m, key('r'));
        let Overlay::Confirm { reason, .. } = &m.overlay else {
            panic!("{:?}", m.overlay)
        };
        assert_eq!(
            reason,
            "Reload the workloads of shop/prod (the rulesets are on their way): 1 request? (y/n)"
        );
        let cmds = update(&mut m, key('y'));
        assert_eq!(requests(&cmds), 1);
        assert_eq!(view(&m).requests, 3);
    }

    #[test]
    fn enter_on_a_workload_before_its_list_opens_it_when_the_list_arrives() {
        let mut m = model_with_flows();
        let _ = update(&mut m, key('A'));
        let seq = view(&m).seq;
        quiet(
            &mut m,
            Msg::AppWorkloadsLoaded {
                seq,
                result: Ok(AppWorkloadsPayload {
                    workloads: fixture_workloads(),
                    truncated: false,
                    hit: false,
                }),
            },
        );
        quiet(&mut m, kc(KeyCode::BackTab)); // Workloads
        quiet(&mut m, key('j'));
        let cmds = update(&mut m, kc(KeyCode::Enter));
        assert!(
            matches!(cmds.as_slice(), [Cmd::LoadWorkloads(_)]),
            "{cmds:?}"
        );
        assert_eq!(m.screen, Screen::Workloads);
        assert!(!m.workloads.show_detail);
        let _ = update(
            &mut m,
            Msg::WorkloadsLoaded(Ok((fixture_workloads(), 0, false))),
        );
        assert!(m.workloads.show_detail);
        assert_eq!(
            m.workloads
                .table
                .selected_item()
                .map(|r| r.workload.href.as_str()),
            Some("/orgs/5/workloads/aaaa-2")
        );
        assert_eq!(m.workloads.open_on_load, None);
    }

    #[test]
    fn a_late_answer_never_reopens_a_closed_overlay() {
        let mut m = model_with_flows();
        let _ = update(&mut m, key('A'));
        let seq = view(&m).seq;
        quiet(&mut m, kc(KeyCode::Esc));
        quiet(
            &mut m,
            Msg::AppWorkloadsLoaded {
                seq,
                result: Ok(AppWorkloadsPayload {
                    workloads: fixture_workloads(),
                    truncated: false,
                    hit: false,
                }),
            },
        );
        quiet(&mut m, rulesets_answer(false));
        assert_eq!(m.overlay, Overlay::None, "the answers do not reopen it");
    }

    #[test]
    fn a_reopens_the_same_app_from_its_view_without_requests() {
        let mut m = model_with_flows();
        m.writes = WritesState::on();
        let cmds = update(&mut m, key('A'));
        assert_eq!(requests(&cmds), 2);
        let seq = view(&m).seq;
        quiet(
            &mut m,
            Msg::AppWorkloadsLoaded {
                seq,
                result: Ok(AppWorkloadsPayload {
                    workloads: fixture_workloads(),
                    truncated: false,
                    hit: false,
                }),
            },
        );
        quiet(&mut m, key('j'));
        quiet(&mut m, key('j')); // unmanaged-nas/- icmp, blocked
        let _ = update(&mut m, key('w'));
        assert!(matches!(m.overlay, Overlay::Form(_)), "{:?}", m.overlay);
        m.overlay = Overlay::None; // the form cancelled
        quiet(&mut m, key('A'));
        assert_eq!(m.overlay, Overlay::App);
        let v = view(&m);
        assert_eq!(v.seq, seq, "the same view");
        assert_eq!(v.requests, 2, "nothing more sent");
        assert!(matches!(&v.workloads, Loadable::Loaded((rows, _)) if rows.len() == 2));
        assert!(
            matches!(v.policy, Loadable::Loading),
            "the rulesets are still on their way"
        );
        assert_eq!(v.cursor[0], 2, "the cursor is kept");
    }

    #[test]
    fn a_failed_workloads_request_shows_the_error_and_r_retries() {
        let mut m = model_with_flows();
        let _ = update(&mut m, key('A'));
        let seq = view(&m).seq;
        quiet(
            &mut m,
            Msg::AppWorkloadsLoaded {
                seq,
                result: Err("HTTP 500".into()),
            },
        );
        assert_eq!(view(&m).workloads, Loadable::Failed("HTTP 500".into()));
        assert_eq!(view(&m).inbound.len(), 3, "the other panels stay");
    }

    #[test]
    fn a_rulesets_load_in_flight_is_not_doubled() {
        let mut m = with_workloads(model_with_flows());
        m.policy.loading = true;
        let cmds = update(&mut m, key('A'));
        assert_eq!(
            requests(&cmds),
            0,
            "workloads are loaded; the rulesets are on their way"
        );
        assert!(matches!(view(&m).policy, Loadable::Loading));
        assert!(!view(&m).policy_sent);
    }

    #[test]
    fn a_name_keyed_app_requests_nothing_for_workloads_or_policy() {
        let mut m = model_with_flows();
        // Flows row 2: unmanaged-nas -> shop/prod; A takes the source.
        m.traffic
            .flows
            .select_where(|r| r.flow.src.display_name() == "unmanaged-nas");
        let cmds = update(&mut m, key('A'));
        assert_eq!(requests(&cmds), 0);
        let v = view(&m);
        assert_eq!(v.focus.app_text(), "unmanaged-nas/-");
        assert_eq!(v.outbound.len(), 1, "its one blocked flow");
        assert!(
            matches!(&v.workloads, Loadable::Unavailable(why) if why.contains("not a label group"))
        );
        assert!(matches!(&v.policy, Loadable::Unavailable(_)));
    }

    #[test]
    fn app_by_name_reaches_apps_without_flows_and_names_the_closest_otherwise() {
        let mut m = everything();
        m.traffic.data.as_mut().unwrap().flows.clear();
        m.traffic.rebuild_views();
        quiet(&mut m, Msg::Command(Command::App(Some("shop/prod".into()))));
        assert_eq!(m.overlay, Overlay::App, "its labels exist");
        assert!(view(&m).inbound.is_empty() && view(&m).outbound.is_empty());
        quiet(&mut m, kc(KeyCode::Esc));
        quiet(&mut m, Msg::Command(Command::App(Some("shop/prd".into()))));
        assert_eq!(m.overlay, Overlay::None);
        assert!(
            m.status.text.starts_with("unknown app \"shop/prd\""),
            "{}",
            m.status.text
        );
        quiet(&mut m, Msg::Command(Command::App(None)));
        assert!(
            matches!(&m.overlay, Overlay::AppPicker(p) if p.purpose == crate::tui::app_picker::AppPickerPurpose::Overlay)
        );
    }
}

#[cfg(test)]
mod draft_tests {
    use super::*;
    use crate::tui::model::{Model, Overlay, PendingAction, TrafficView};
    use crate::tui::msg::{DraftFailure, DraftOutcome, FocusedOutcome};
    use crate::tui::test_support::{model_with_rich_flows, with_draft};
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use pce::traffic::{Decisions, Focus, FocusDirection};
    use std::time::Duration;

    const HREF: &str = "/orgs/1/traffic_flows/async_queries/q-1";

    fn key(c: char) -> Msg {
        Msg::Key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE))
    }

    fn esc() -> Msg {
        Msg::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
    }

    /// The rich fixture as the answer of our own query `HREF`.
    fn model() -> Model {
        let mut m = model_with_rich_flows();
        m.traffic.data.as_mut().unwrap().hrefs = vec![HREF.into()];
        m
    }

    /// The draft answer for the result on screen of `m`.
    fn answer(m: &Model, seq: u64, hit: bool) -> Msg {
        let d = m.traffic.data.as_ref().unwrap();
        Msg::DraftDone {
            seq,
            result: Ok(DraftOutcome {
                outcome: QueryOutcome {
                    flows: with_draft(d.flows.clone()),
                    truncated: d.truncated,
                    incomplete: false,
                    elapsed: Duration::from_millis(700),
                    hit,
                    filter: d.filter.clone(),
                    bounds: None,
                    hrefs: d.hrefs.clone(),
                },
                focus: m.traffic.focus_result.as_ref().map(|r| r.focus.clone()),
                age: if hit {
                    Duration::from_secs(40)
                } else {
                    Duration::ZERO
                },
            }),
        }
    }

    /// `D` on `m`: the one `Cmd::RunDraft` it sends.
    fn press_d(m: &mut Model) -> (u64, Vec<String>, Option<Focus>, CancellationToken) {
        let cmds = update(m, key('D'));
        let [
            Cmd::RunDraft {
                seq,
                hrefs,
                focus,
                cancel,
                ..
            },
        ] = cmds.as_slice()
        else {
            panic!("{cmds:?}")
        };
        (*seq, hrefs.clone(), focus.clone(), cancel.clone())
    }

    use tokio_util::sync::CancellationToken;

    #[test]
    fn draft_decision_marks_go_active_with_draft_decisions_and_wait_again_without() {
        let dir = tempfile::tempdir().unwrap();
        let mut m = crate::tui::test_support::model_with_marks(
            dir.path(),
            "[[mark]]\naction = \"revisit\"\nsrc = \"any\"\ndst = \"any\"\ndecision = [\"blocked\", \"potentially_blocked\"]\ndraft_decision = \"allowed\"\ncomment = \"fixed by the pending policy\"\n",
        );
        m.traffic.data.as_mut().unwrap().hrefs = vec![HREF.into()];
        let counts = |m: &Model| {
            let mv = m.traffic.marked.as_ref().unwrap();
            (mv.active, mv.waiting)
        };
        assert_eq!(counts(&m), (0, 1));
        assert!(
            m.traffic.flows.marker.is_none(),
            "nothing active, no gutter"
        );
        let (seq, ..) = press_d(&mut m);
        let msg = answer(&m, seq, false);
        assert!(update(&mut m, msg).is_empty());
        assert_eq!(counts(&m), (1, 0), "draft decisions loaded: active");
        let revisit = m
            .traffic
            .flows
            .rows()
            .iter()
            .filter(|r| {
                r.mark
                    .is_some_and(|h| h.action == pce::marks::Action::Revisit)
            })
            .count();
        assert!(revisit > 0, "the flows the draft fixes are marked");
        assert!(m.traffic.flows.marker.is_some());
        assert!(m.traffic.marks_meta(TrafficView::Flows).is_none());
        assert!(m.traffic.clear_draft());
        assert_eq!(counts(&m), (0, 1), "draft decisions gone: waiting again");
        assert!(m.traffic.flows.rows().iter().all(|r| r.mark.is_none()));
        assert_eq!(
            m.traffic.marks_meta(TrafficView::Flows).as_deref(),
            Some("1 mark waits for the draft decision (D)")
        );
    }

    #[test]
    fn d_with_a_known_query_sends_one_draft_run_and_no_new_query() {
        let mut m = model();
        let (seq, hrefs, focus, _) = press_d(&mut m);
        assert_eq!(hrefs, vec![HREF.to_string()], "the PUT goes to our query");
        assert_eq!(focus, None);
        assert!(m.traffic.is_draft_running());
        assert!(m.status.text.contains("Esc cancels"), "{}", m.status.text);
        assert!(
            update(&mut m, key('D')).is_empty(),
            "a second D while it runs sends nothing"
        );
        let msg = answer(&m, seq, false);
        assert!(update(&mut m, msg).is_empty());
        assert!(m.traffic.draft_loaded());
        let s = m.traffic.data.as_ref().unwrap().draft.unwrap();
        assert!(s.fixed > 0 && s.newly_blocked > 0);
        assert_eq!(
            m.status.text,
            format!(
                "draft: {} blocked flows fixed · {} would be blocked · D clears",
                s.fixed, s.newly_blocked
            )
        );
        assert_eq!(m.traffic.flows.columns.last().unwrap().title, "draft");
        assert_eq!(m.traffic.groups.columns.last().unwrap().title, "draft");
        assert_eq!(m.traffic.apps.columns.last().unwrap().title, "draft");
        let row = m
            .traffic
            .flows
            .rows()
            .iter()
            .find(|r| r.flow.fixed_by_draft())
            .unwrap();
        let cell = row.flow.draft_text();
        assert!(cell.ends_with(" → allowed"), "{cell}");
    }

    #[test]
    fn d_again_clears_the_draft_with_no_request() {
        let mut m = model();
        let (seq, ..) = press_d(&mut m);
        let msg = answer(&m, seq, false);
        update(&mut m, msg);
        m.traffic.apps_state.decisions = Decisions::FixedByDraft;
        assert!(update(&mut m, key('D')).is_empty());
        assert!(!m.traffic.draft_loaded());
        assert!(
            m.traffic
                .loaded_flows()
                .iter()
                .all(|f| f.draft_policy_decision.is_none())
        );
        assert_eq!(m.traffic.flows.columns.last().unwrap().title, "decision");
        assert_eq!(
            m.traffic.apps_state.decisions,
            Decisions::All,
            "fixed by draft goes with the draft"
        );
        assert!(m.status.text.contains("no request"), "{}", m.status.text);
    }

    #[test]
    fn d_without_a_known_query_asks_then_runs_it_again_with_the_draft() {
        let mut m = model_with_rich_flows(); // no hrefs: a result we cannot re-evaluate
        assert!(update(&mut m, key('D')).is_empty());
        let Overlay::Confirm { reason, action } = m.overlay.clone() else {
            panic!("{:?}", m.overlay)
        };
        assert!(
            reason.contains("1 POST, 1 PUT, polls and 1 download"),
            "{reason}"
        );
        assert!(matches!(&action, PendingAction::RunDraft { hrefs, .. } if hrefs.is_empty()));
        let cmds = update(&mut m, key('y'));
        assert!(
            matches!(cmds.as_slice(), [Cmd::RunDraft { hrefs, .. }] if hrefs.is_empty()),
            "{cmds:?}"
        );
        let mut m = model_with_rich_flows();
        update(&mut m, key('D'));
        assert!(update(&mut m, key('n')).is_empty(), "n sends nothing");
        assert!(!m.traffic.is_running());
    }

    #[test]
    fn d_past_a_cost_threshold_asks_first() {
        let mut m = model();
        let d = m.traffic.data.as_mut().unwrap();
        d.filter.max_results = 25_000;
        m.traffic.filter.max_results = 25_000;
        assert!(update(&mut m, key('D')).is_empty());
        let Overlay::Confirm { reason, .. } = m.overlay.clone() else {
            panic!("{:?}", m.overlay)
        };
        assert!(
            reason.contains("1 status check, 1 PUT, polls and 1 download"),
            "{reason}"
        );
        assert!(matches!(
            update(&mut m, key('y')).as_slice(),
            [Cmd::RunDraft { .. }]
        ));
    }

    #[test]
    fn esc_cancels_a_draft_run_and_its_late_answer_is_dropped() {
        let mut m = model();
        let (seq, _, _, cancel) = press_d(&mut m);
        assert!(update(&mut m, esc()).is_empty());
        assert!(cancel.is_cancelled());
        assert!(!m.traffic.is_running());
        assert_eq!(m.status.text, "draft re-evaluation cancelled");
        let msg = answer(&m, seq, false);
        update(&mut m, msg);
        assert!(!m.traffic.draft_loaded(), "the late answer is dropped");
    }

    #[test]
    fn a_new_query_a_cap_step_or_a_profile_switch_drops_the_draft_answer() {
        // A new query (`]` steps the cap and re-runs).
        let mut m = model();
        let (seq, _, _, cancel) = press_d(&mut m);
        let cmds = update(&mut m, key(']'));
        assert!(
            matches!(cmds.as_slice(), [Cmd::RunQuery { .. }]),
            "{cmds:?}"
        );
        assert!(cancel.is_cancelled());
        let msg = answer(&m, seq, false);
        update(&mut m, msg);
        assert!(!m.traffic.draft_loaded());
        // A profile switch.
        let mut m = model();
        let (seq, _, _, cancel) = press_d(&mut m);
        m.reset_data();
        assert!(cancel.is_cancelled());
        let mut m2 = model();
        let msg = answer(&m2, seq, false);
        update(&mut m, msg);
        assert!(!m.traffic.draft_loaded());
        // The answer is checked against the result too, past the seq.
        let (seq, ..) = press_d(&mut m2);
        m2.traffic.data.as_mut().unwrap().filter.window = "2h".into();
        let msg = Msg::DraftDone {
            seq,
            result: Ok(DraftOutcome {
                outcome: QueryOutcome {
                    flows: Vec::new(),
                    truncated: false,
                    incomplete: false,
                    elapsed: Duration::ZERO,
                    hit: false,
                    filter: model().traffic.filter.clone(),
                    bounds: None,
                    hrefs: vec![HREF.into()],
                },
                focus: None,
                age: Duration::ZERO,
            }),
        };
        update(&mut m2, msg);
        assert!(!m2.traffic.draft_loaded());
        assert_eq!(m2.status.text, "draft answer dropped: the result changed");
    }

    #[test]
    fn a_focus_change_cancels_a_draft_run() {
        let mut m = model();
        let (_, _, _, cancel) = press_d(&mut m);
        update(
            &mut m,
            Msg::Command(Command::Focus(Some("crm/prod".into()))),
        );
        assert!(cancel.is_cancelled(), "setting a focus");
        let mut m = model();
        update(
            &mut m,
            Msg::Command(Command::Focus(Some("crm/prod".into()))),
        );
        let (_, _, _, cancel) = press_d(&mut m);
        update(&mut m, key('X'));
        assert!(cancel.is_cancelled(), "its direction");
        let (_, _, _, cancel) = press_d(&mut m);
        update(&mut m, esc());
        assert!(cancel.is_cancelled(), "Esc first cancels the run");
        assert!(m.traffic.focus.is_some(), "and leaves the focus");
    }

    #[test]
    fn a_focused_result_is_re_evaluated_by_its_hrefs_per_side() {
        let mut m = model();
        let focus = Focus::new(
            vec!["crm".into(), "prod".into()],
            vec!["app".into(), "env".into()],
        );
        update(
            &mut m,
            Msg::Command(Command::Focus(Some("crm/prod".into()))),
        );
        let cmds = update(&mut m, key('r'));
        let seq = match cmds.as_slice() {
            [Cmd::RunFocused { seq, .. }] => *seq,
            // A complete result needs no re-query: set the focused one directly.
            _ => m.traffic.seq,
        };
        let d = m.traffic.data.as_ref().unwrap();
        let hrefs = vec!["/q/in".to_string(), "/q/out".to_string()];
        let outcome = QueryOutcome {
            flows: d.flows[..10].to_vec(),
            truncated: false,
            incomplete: false,
            elapsed: Duration::ZERO,
            hit: false,
            filter: m.traffic.filter.clone(),
            bounds: None,
            hrefs: hrefs.clone(),
        };
        m.traffic.seq = seq;
        update(
            &mut m,
            Msg::FocusedDone {
                seq,
                result: Ok(FocusedOutcome {
                    outcome,
                    focus: focus.clone().with_direction(FocusDirection::Both),
                    queries: 2,
                }),
            },
        );
        assert!(m.traffic.focus_result.is_some());
        let (seq, sent, f, _) = press_d(&mut m);
        assert_eq!(sent, hrefs);
        assert_eq!(f.map(|f| f.app_text()), Some("crm/prod".into()));
        let msg = answer(&m, seq, true);
        update(&mut m, msg);
        assert!(m.traffic.draft_loaded());
        assert!(m.status.text.ends_with("(cached 40s)"), "{}", m.status.text);
    }

    #[test]
    fn a_dropped_query_asks_to_run_it_again_and_a_refusal_keeps_the_plain_result() {
        let mut m = model();
        let (seq, ..) = press_d(&mut m);
        update(
            &mut m,
            Msg::DraftDone {
                seq,
                result: Err(DraftFailure::Gone("not found: q-1".into())),
            },
        );
        let Overlay::Confirm { reason, action } = m.overlay.clone() else {
            panic!("{:?}", m.overlay)
        };
        assert!(
            reason.starts_with("The PCE no longer holds this query"),
            "{reason}"
        );
        assert!(matches!(action, PendingAction::RunDraft { hrefs, .. } if hrefs.is_empty()));
        let mut m = model();
        let (seq, ..) = press_d(&mut m);
        let n = m.traffic.loaded_flows().len();
        update(
            &mut m,
            Msg::DraftDone {
                seq,
                result: Err(DraftFailure::Failed("the PCE refused it".into())),
            },
        );
        assert!(!m.traffic.is_running() && !m.traffic.draft_loaded());
        assert_eq!(m.traffic.loaded_flows().len(), n, "the plain result stays");
        assert_eq!(m.status.text, "draft: the PCE refused it");
    }

    /// The answer of a successful rule delete.
    fn written() -> Msg {
        Msg::WriteDone {
            kind: pce::write::ObjectKind::Rule,
            method: Some(pce::WriteMethod::Delete),
            result: Ok(pce::write::Outcome::Written(pce::write::Written {
                href: "/orgs/1/sec_policy/draft/rule_sets/1/sec_rules/2".into(),
                pending: true,
                status: 204,
                object: serde_json::Value::Null,
                dropped_fields: vec![],
                warnings: vec![],
                audit_failed: None,
            })),
        }
    }

    /// The `Cmd::RunDraft`s among `cmds`: their hrefs.
    fn redrafts(cmds: &[Cmd]) -> Vec<Vec<String>> {
        cmds.iter()
            .filter_map(|c| match c {
                Cmd::RunDraft { hrefs, .. } => Some(hrefs.clone()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn a_write_under_draft_decisions_evaluates_them_again_once() {
        let mut m = model();
        let (seq, ..) = press_d(&mut m);
        let msg = answer(&m, seq, false);
        update(&mut m, msg);
        assert!(m.traffic.draft_loaded());
        m.writes.in_flight = true;
        let cmds = update(&mut m, written());
        assert_eq!(redrafts(&cmds), vec![vec![HREF.to_string()]], "{cmds:?}");
        assert!(!m.traffic.draft_loaded() && m.traffic.is_draft_running());
        assert_eq!(
            m.status.text,
            "deleted /orgs/1/sec_policy/draft/rule_sets/1/sec_rules/2 (pending) · draft re-evaluation running · Esc cancels"
        );
        // A run in flight at the write: cancelled and started once more.
        let mut m = model();
        let (.., cancel) = press_d(&mut m);
        m.writes.in_flight = true;
        let cmds = update(&mut m, written());
        assert!(cancel.is_cancelled(), "the stale run is cancelled");
        assert_eq!(redrafts(&cmds).len(), 1, "{cmds:?}");
        assert!(m.traffic.is_draft_running());
    }

    #[test]
    fn an_unrelated_write_without_draft_decisions_asks_nothing() {
        let mut m = model();
        m.writes.in_flight = true;
        let cmds = update(&mut m, written());
        assert!(redrafts(&cmds).is_empty(), "{cmds:?}");
        assert!(!m.traffic.is_running());
        assert_eq!(
            m.status.text,
            "deleted /orgs/1/sec_policy/draft/rule_sets/1/sec_rules/2 (pending)"
        );
    }

    #[test]
    fn a_failed_write_keeps_the_draft_and_asks_nothing() {
        let mut m = model();
        let (seq, ..) = press_d(&mut m);
        let msg = answer(&m, seq, false);
        update(&mut m, msg);
        m.writes.in_flight = true;
        let cmds = update(
            &mut m,
            Msg::WriteDone {
                kind: pce::write::ObjectKind::Rule,
                method: Some(pce::WriteMethod::Delete),
                result: Err(pce::write::WriteError::invalid("refused")),
            },
        );
        assert!(redrafts(&cmds).is_empty(), "{cmds:?}");
        assert!(m.traffic.draft_loaded() && !m.traffic.is_running());
    }

    #[test]
    fn a_write_drops_the_draft_and_the_next_d_asks_the_pce() {
        let mut m = model();
        let (seq, ..) = press_d(&mut m);
        let msg = answer(&m, seq, false);
        update(&mut m, msg);
        assert!(m.traffic.draft_loaded());
        // No query of this session behind it (a cached result): `D` would
        // ask first, so the write only drops the draft.
        m.traffic.data.as_mut().unwrap().hrefs.clear();
        m.writes.in_flight = true;
        let cmds = update(&mut m, written());
        assert!(redrafts(&cmds).is_empty(), "{cmds:?}");
        assert!(!m.traffic.draft_loaded() && !m.traffic.is_running());
        assert!(
            m.status.text.ends_with("draft cleared: D re-evaluates"),
            "{}",
            m.status.text
        );
        m.traffic.data.as_mut().unwrap().hrefs = vec![HREF.into()];
        let (_, hrefs, ..) = press_d(&mut m);
        assert_eq!(hrefs, vec![HREF.to_string()], "D runs again, not a clear");
    }

    #[test]
    fn a_draft_only_on_the_result_kept_aside_is_dropped_not_evaluated() {
        let mut m = model();
        let (seq, ..) = press_d(&mut m);
        let msg = answer(&m, seq, false);
        update(&mut m, msg);
        // The drafted result kept aside, a plain one on screen.
        m.traffic.unfocused = m.traffic.data.take();
        m.traffic.data = model().traffic.data;
        m.writes.in_flight = true;
        let cmds = update(&mut m, written());
        assert!(redrafts(&cmds).is_empty(), "{cmds:?}");
        assert!(m.traffic.unfocused.as_ref().unwrap().draft.is_none());
        assert!(
            m.status.text.ends_with("draft cleared: D re-evaluates"),
            "{}",
            m.status.text
        );
    }

    #[test]
    fn a_write_off_the_traffic_screen_says_where_esc_cancels() {
        let mut m = model();
        let (seq, ..) = press_d(&mut m);
        let msg = answer(&m, seq, false);
        update(&mut m, msg);
        m.screen = crate::tui::model::Screen::Policy;
        m.writes.in_flight = true;
        let cmds = update(&mut m, written());
        assert_eq!(redrafts(&cmds).len(), 1, "{cmds:?}");
        assert!(
            m.status
                .text
                .ends_with(" · draft re-evaluation running · Esc on Traffic cancels"),
            "{}",
            m.status.text
        );
    }

    #[test]
    fn a_provision_evaluates_the_draft_and_keeps_its_own_report_for_the_pending_list() {
        let mut m = model();
        let (seq, ..) = press_d(&mut m);
        let msg = answer(&m, seq, false);
        update(&mut m, msg);
        m.policy.provision_counts = Some("rule_sets=1".into());
        m.writes.in_flight = true;
        let cmds = update(
            &mut m,
            Msg::WriteDone {
                kind: pce::write::ObjectKind::Provision,
                method: Some(pce::WriteMethod::Post),
                result: Ok(pce::write::Outcome::Written(pce::write::Written {
                    href: "/orgs/1/sec_policy/7".into(),
                    pending: false,
                    status: 201,
                    object: serde_json::Value::Null,
                    dropped_fields: vec![],
                    warnings: vec![],
                    audit_failed: None,
                })),
            },
        );
        assert_eq!(redrafts(&cmds).len(), 1, "{cmds:?}");
        assert_eq!(
            m.policy.provisioned.as_deref(),
            Some("provisioned /orgs/1/sec_policy/7: rule_sets=1")
        );
        assert!(m.status.text.contains("draft re-evaluation running"));
    }

    #[test]
    fn an_unanswered_write_reloads_but_evaluates_nothing() {
        let mut m = model();
        let (seq, ..) = press_d(&mut m);
        let msg = answer(&m, seq, false);
        update(&mut m, msg);
        m.policy.write_from_flow = true;
        m.writes.in_flight = true;
        let cmds = update(
            &mut m,
            Msg::WriteDone {
                kind: pce::write::ObjectKind::Rule,
                method: Some(pce::WriteMethod::Delete),
                result: Err(pce::write::WriteError::Unanswered(pce::PceError::Timeout(
                    Duration::from_secs(30),
                ))),
            },
        );
        assert!(!cmds.is_empty(), "it reloads");
        assert!(redrafts(&cmds).is_empty(), "{cmds:?}");
        assert!(!m.policy.write_from_flow, "taken");
    }

    #[test]
    fn a_draft_re_run_takes_the_new_time_range() {
        let mut m = model_with_rich_flows(); // no hrefs: D re-runs the query
        update(&mut m, key('D'));
        let cmds = update(&mut m, key('y'));
        let [Cmd::RunDraft { seq, .. }] = cmds.as_slice() else {
            panic!("{cmds:?}")
        };
        let end: jiff::Timestamp = "2026-10-11T00:00:00Z".parse().unwrap();
        let start: jiff::Timestamp = "2026-10-10T00:00:00Z".parse().unwrap();
        let d = m.traffic.data.as_ref().unwrap();
        let msg = Msg::DraftDone {
            seq: *seq,
            result: Ok(DraftOutcome {
                outcome: QueryOutcome {
                    flows: with_draft(d.flows.clone()),
                    truncated: false,
                    incomplete: false,
                    elapsed: Duration::ZERO,
                    hit: false,
                    filter: d.filter.clone(),
                    bounds: Some((start, end)),
                    hrefs: vec!["/orgs/1/traffic_flows/async_queries/new".into()],
                },
                focus: None,
                age: Duration::ZERO,
            }),
        };
        update(&mut m, msg);
        let r = m.traffic.data.as_ref().unwrap().range.unwrap();
        assert_eq!((r.start, r.end), (start, end));
    }

    #[test]
    fn the_decision_filter_offers_fixed_by_draft_only_with_draft_data() {
        let mut m = model();
        m.traffic.view = TrafficView::Aggregate;
        for want in [Decisions::Blocked, Decisions::BlockedOnly, Decisions::All] {
            update(&mut m, key('d'));
            assert_eq!(m.traffic.apps_state.decisions, want);
        }
        let (seq, ..) = press_d(&mut m);
        let msg = answer(&m, seq, false);
        update(&mut m, msg);
        for _ in 0..3 {
            update(&mut m, key('d'));
        }
        assert_eq!(m.traffic.apps_state.decisions, Decisions::FixedByDraft);
        assert_eq!(m.status.text, "decision fixed by draft");
        let fixed = m.traffic.data.as_ref().unwrap().draft.unwrap().fixed;
        let total: u64 = m.traffic.groups.rows().iter().map(|g| g.flows).sum();
        assert_eq!(total, fixed, "the pairs count only the fixed flows");
        update(&mut m, key('d'));
        assert_eq!(m.traffic.apps_state.decisions, Decisions::All);
    }
}

#[cfg(test)]
mod rule_form_ruleset_tests {
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
        serde_json::from_str(include_str!("../../../../fixtures/rule_sets.json")).unwrap()
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
}

#[cfg(test)]
mod batch_tests {
    use super::*;
    use crate::tui::model::{Model, Overlay, StatusKind};
    use crate::tui::test_support::{batch_plan, model_with_flows, with_draft_loaded};
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use pce::write::{ObjectKind, StepResult, Written};

    fn key(c: char) -> Msg {
        Msg::Key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE))
    }

    fn written(href: &str) -> StepResult {
        StepResult::Written(Written {
            href: href.into(),
            pending: true,
            status: 201,
            object: serde_json::json!({"href": href}),
            dropped_fields: Vec::new(),
            warnings: Vec::new(),
            audit_failed: None,
        })
    }

    fn kinds() -> Vec<ObjectKind> {
        vec![ObjectKind::IpList, ObjectKind::RuleSet, ObjectKind::Rule]
    }

    fn summaries() -> Vec<String> {
        vec![
            "create ip_list shop-prod peer 203.0.113.7".into(),
            "create rule_set RF-crm-prod".into(),
            "create rule in rule_set RF-crm-prod".into(),
        ]
    }

    /// Writes on, the draft rulesets loaded, the generator open for
    /// shop/prod and planning.
    fn armed() -> Model {
        let mut m = crate::tui::test_support::with_policy(model_with_flows());
        m.writes = crate::tui::model::WritesState::on();
        m.writes.in_flight = true; // the plan is on its way
        let focus = pce::traffic::Focus::new(
            pce::traffic::parse_app("shop/prod"),
            vec!["app".into(), "env".into()],
        );
        let mut v = crate::tui::gen_view::GenView::new(focus, "24h".into(), 1);
        v.planning = true;
        m.gen_view = Some(v);
        m.overlay = Overlay::Generator;
        m
    }

    /// `m` after its plan landed and `y` sent it: the confirm closed.
    fn sent(mut m: Model) -> Model {
        update(&mut m, Msg::BatchPlanned(Ok(batch_plan())));
        update(&mut m, key('y'));
        assert!(m.batch_sending && m.overlay == Overlay::None);
        m
    }

    #[test]
    fn a_planned_batch_opens_its_confirm_with_every_request() {
        let mut m = armed();
        assert_eq!(
            update(&mut m, Msg::BatchPlanned(Ok(batch_plan()))),
            Vec::new()
        );
        assert!(!m.writes.in_flight);
        let Overlay::ConfirmBatch { batch, scroll } = &m.overlay else {
            panic!("{:?}", m.overlay)
        };
        assert_eq!(*scroll, 0);
        assert_eq!(
            batch.summary(),
            "3 requests: 1 ip_list, 1 rule_set, 1 rule · planned with 2 reads"
        );
        assert_eq!(
            m.status.text,
            "review the requests: y sends them all, n cancels"
        );
    }

    #[test]
    fn a_batch_landing_over_another_overlay_is_discarded() {
        let mut m = armed();
        m.overlay = Overlay::Prompt { input: "y".into() };
        update(&mut m, Msg::BatchPlanned(Ok(batch_plan())));
        assert_eq!(m.overlay, Overlay::Prompt { input: "y".into() });
        assert_eq!(m.status.text, "requests discarded, nothing sent");
        assert!(!m.writes.in_flight);
    }

    #[test]
    fn y_sends_exactly_one_execute_batch_and_n_sends_nothing() {
        let mut m = armed();
        update(&mut m, Msg::BatchPlanned(Ok(batch_plan())));
        let cmds = update(&mut m, key('y'));
        assert_eq!(cmds.len(), 1);
        assert!(matches!(&cmds[0], Cmd::ExecuteBatch(b) if b.steps.len() == 3));
        assert!(m.writes.in_flight);
        assert_eq!(m.overlay, Overlay::None);
        assert_eq!(m.status.text, "sending 3 requests …");
        let mut m = armed();
        update(&mut m, Msg::BatchPlanned(Ok(batch_plan())));
        assert_eq!(update(&mut m, key('n')), Vec::new());
        assert_eq!(m.overlay, Overlay::Generator, "back to the generator");
        assert_eq!(m.status.text, "cancelled, nothing sent");
    }

    #[test]
    fn j_scrolls_the_confirm_and_stops_at_the_end() {
        let mut m = armed();
        m.size = (80, 12);
        update(&mut m, Msg::BatchPlanned(Ok(batch_plan())));
        for _ in 0..100 {
            update(&mut m, key('j'));
        }
        let Overlay::ConfirmBatch { batch, scroll } = &m.overlay else {
            panic!()
        };
        let area = ratatui::layout::Rect::new(0, 0, 80, 12);
        assert_eq!(usize::from(*scroll), confirm_batch_last_scroll(batch, area));
        assert!(*scroll > 0);
    }

    #[test]
    fn a_failed_plan_sends_nothing_and_says_why() {
        let mut m = armed();
        assert_eq!(
            update(
                &mut m,
                Msg::BatchPlanned(Err("too many draft rulesets".into()))
            ),
            Vec::new()
        );
        assert_eq!(m.status.text, "plan: too many draft rulesets");
        assert_eq!(m.status.kind, StatusKind::Error);
    }

    #[test]
    fn a_partly_failed_batch_reports_each_step_and_reloads_once() {
        let mut m = sent(with_draft_loaded(armed()));
        // A known query: the draft is evaluated again without asking.
        m.traffic.data.as_mut().unwrap().hrefs =
            vec!["/orgs/5/traffic_flows/async_queries/q-1".into()];
        let results = vec![
            written("/orgs/5/sec_policy/draft/ip_lists/77"),
            StepResult::Failed(pce::write::WriteError::invalid("name in use")),
            StepResult::NotSent,
        ];
        let cmds = update(
            &mut m,
            Msg::BatchDone {
                kinds: kinds(),
                summaries: summaries(),
                target: "demo (pce.example, org 5)".into(),
                results,
            },
        );
        assert_eq!(
            m.status.text.split(" · ").next().unwrap(),
            "wrote 1 of 3 (pending); step 2 failed: name in use; 1 not sent"
        );
        assert_eq!(m.status.kind, StatusKind::Error);
        assert!(!m.writes.in_flight);
        let count = |f: fn(&Cmd) -> bool| cmds.iter().filter(|c| f(c)).count();
        assert_eq!(
            count(|c| matches!(c, Cmd::LoadRuleSets { .. })),
            1,
            "{cmds:?}"
        );
        assert_eq!(
            count(|c| matches!(c, Cmd::LoadIpLists(CacheMode::Bypass))),
            1
        );
        assert_eq!(
            count(|c| matches!(c, Cmd::RunDraft { .. })),
            1,
            "one redraft per batch"
        );
        // The report stays up: which object went where, what did not.
        assert_eq!(m.overlay, Overlay::BatchReport { scroll: 0 });
        assert_eq!(
            m.batch_report.as_deref().unwrap(),
            [
                "sent to demo (pce.example, org 5)",
                "1/3 create ip_list shop-prod peer 203.0.113.7",
                "    written /orgs/5/sec_policy/draft/ip_lists/77",
                "2/3 create rule_set RF-crm-prod",
                "    FAILED: name in use",
                "3/3 create rule in rule_set RF-crm-prod",
                "    not sent",
                "",
                "These are draft changes, not provisioned: p on Policy lists them, R reverts the marked ones.",
                "Planning the batch again resumes it: what was written then exists.",
            ]
        );
        assert_eq!(
            update(
                &mut m,
                Msg::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
            ),
            Vec::new()
        );
        assert_eq!(m.overlay, Overlay::Generator, "back to the generator");
        on_command(&mut m, crate::tui::commands::Command::Batch);
        assert_eq!(
            m.overlay,
            Overlay::BatchReport { scroll: 0 },
            ":batch reopens it"
        );
    }

    fn all_written() -> Vec<StepResult> {
        vec![
            written("/orgs/5/sec_policy/draft/ip_lists/77"),
            written("/orgs/5/sec_policy/draft/rule_sets/200"),
            written("/orgs/5/sec_policy/draft/rule_sets/200/sec_rules/1"),
        ]
    }

    fn done(m: &mut Model, results: Vec<StepResult>) -> Vec<Cmd> {
        update(
            m,
            Msg::BatchDone {
                kinds: kinds(),
                summaries: summaries(),
                target: "demo (pce.example, org 5)".into(),
                results,
            },
        )
    }

    #[test]
    fn quitting_waits_while_a_batch_is_sent() {
        let mut m = armed();
        update(&mut m, Msg::BatchPlanned(Ok(batch_plan())));
        update(&mut m, key('y'));
        assert!(m.batch_sending);
        let ctrl_c = || Msg::Key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL));
        assert_eq!(update(&mut m, ctrl_c()), Vec::new());
        assert_eq!(update(&mut m, key('q')), Vec::new());
        assert_eq!(
            on_command(&mut m, crate::tui::commands::Command::Quit),
            Vec::new()
        );
        assert!(!m.quit);
        assert_eq!(m.status.text, "batch sending: wait for it to finish");
        done(&mut m, all_written());
        assert!(!m.batch_sending);
        assert_eq!(update(&mut m, ctrl_c()), vec![Cmd::Quit]);
        assert!(m.quit);
    }

    #[test]
    fn nothing_can_switch_or_clear_the_write_guard_while_a_batch_is_sent() {
        use crate::tui::commands::Command;
        let mut m = armed();
        update(&mut m, Msg::BatchPlanned(Ok(batch_plan())));
        update(&mut m, key('y'));
        assert_eq!(
            on_command(&mut m, Command::Profile("prod-b".into())),
            Vec::new()
        );
        assert_eq!(m.status.text, "batch sending: wait for it to finish");
        assert!(!m.switching);
        assert_eq!(on_command(&mut m, Command::Writes(Some(false))), Vec::new());
        assert_eq!(m.status.text, "batch sending: wait for it to finish");
        // A plan landing now (from anywhere) leaves the batch's guard alone.
        update(&mut m, Msg::BatchPlanned(Ok(batch_plan())));
        assert!(m.writes.in_flight, "still the batch's");
        assert_eq!(m.overlay, Overlay::None);
        assert_eq!(
            m.status.text,
            "plan discarded: a batch is being sent, nothing sent"
        );
        update(&mut m, Msg::Planned(Err("x".into())));
        assert!(m.writes.in_flight);
        update(&mut m, Msg::BatchProgress { done: 2, of: 3 });
        assert_eq!(m.status.text, "sending: 2 of 3 answered …");
        done(&mut m, all_written());
        assert!(!m.writes.in_flight && !m.batch_sending);
        assert_eq!(
            on_command(&mut m, Command::Profile("prod-b".into())),
            vec![Cmd::SwitchProfile("prod-b".into())]
        );
    }

    #[test]
    fn a_batch_planned_on_another_profile_or_during_a_switch_is_discarded() {
        let mut m = armed();
        m.profile.name = "prod-b".into();
        update(&mut m, Msg::BatchPlanned(Ok(batch_plan())));
        assert_eq!(m.overlay, Overlay::Generator, "no confirm");
        assert_eq!(
            m.status.text,
            "requests discarded: planned on another profile, nothing sent"
        );
        let mut m = armed();
        m.switching = true;
        update(&mut m, Msg::BatchPlanned(Ok(batch_plan())));
        assert_eq!(m.overlay, Overlay::Generator, "no confirm");
        let mut m = armed();
        let mut b = batch_plan();
        b.origin = None;
        update(&mut m, Msg::BatchPlanned(Ok(b)));
        assert_eq!(m.overlay, Overlay::Generator, "no origin: never confirmed");
    }

    #[test]
    fn a_clean_batch_keeps_its_report_for_batch_and_an_open_overlay_stays() {
        let mut m = sent(armed());
        done(&mut m, all_written());
        assert_eq!(m.overlay, Overlay::Generator, "the generator reopens");
        assert!(
            m.status.text.contains(":batch shows each step"),
            "{}",
            m.status.text
        );
        assert!(m.batch_report.is_some());
        // Help opened while the batch was sent is not closed by its answer.
        let mut m = sent(armed());
        m.overlay = Overlay::Help;
        done(
            &mut m,
            vec![
                StepResult::Failed(pce::write::WriteError::invalid("bad")),
                StepResult::NotSent,
                StepResult::NotSent,
            ],
        );
        assert_eq!(m.overlay, Overlay::Help);
        assert!(m.status.text.contains(":batch shows each step"));
    }

    #[test]
    fn a_step_written_but_not_audited_stops_and_says_so() {
        let mut m = sent(armed());
        let StepResult::Written(mut w) = written("/orgs/5/sec_policy/draft/ip_lists/77") else {
            unreachable!()
        };
        let why = "audit entry could not be written to audit.jsonl: disk full".to_string();
        w.warnings = vec![why.clone()];
        w.audit_failed = Some(why);
        done(
            &mut m,
            vec![
                StepResult::Written(w),
                StepResult::NotSent,
                StepResult::NotSent,
            ],
        );
        assert!(
            m.status.text.starts_with(
                "wrote 1 of 3 (pending); step 1 written but not audited (audit entry could not be written to audit.jsonl: disk full): batch stopped; 2 not sent"
            ),
            "{}",
            m.status.text
        );
        assert_eq!(m.status.kind, StatusKind::Error);
        assert_eq!(m.overlay, Overlay::BatchReport { scroll: 0 });
        assert!(m.batch_report.as_ref().unwrap()[2].contains("NOT AUDITED"));
    }

    #[test]
    fn an_unanswered_step_says_it_may_have_applied_and_reloads() {
        let mut m = armed();
        let results = vec![
            StepResult::Failed(pce::write::WriteError::Unanswered(pce::PceError::Timeout(
                std::time::Duration::from_secs(30),
            ))),
            StepResult::NotSent,
            StepResult::NotSent,
        ];
        let cmds = update(
            &mut m,
            Msg::BatchDone {
                kinds: kinds(),
                summaries: summaries(),
                target: "demo (pce.example, org 5)".into(),
                results,
            },
        );
        assert!(
            m.status
                .text
                .starts_with("wrote 0 of 3 (pending); step 1 not answered: it may have applied; reloading; 2 not sent"),
            "{}",
            m.status.text
        );
        assert!(cmds.iter().any(|c| matches!(c, Cmd::LoadRuleSets { .. })));
    }

    #[test]
    fn what_the_pce_dropped_from_a_written_step_is_a_warning() {
        let mut m = armed();
        let StepResult::Written(mut w) = written("/orgs/5/sec_policy/draft/ip_lists/77") else {
            unreachable!()
        };
        w.dropped_fields = vec!["fqdns".into()];
        let results = vec![
            StepResult::Written(w),
            written("/orgs/5/sec_policy/draft/rule_sets/200"),
            written("/orgs/5/sec_policy/draft/rule_sets/200/sec_rules/1"),
        ];
        update(
            &mut m,
            Msg::BatchDone {
                kinds: kinds(),
                summaries: summaries(),
                target: "demo (pce.example, org 5)".into(),
                results,
            },
        );
        assert_eq!(
            m.status.text.split(" · ").next().unwrap(),
            "wrote 3 of 3 (pending); step 1: the PCE dropped fqdns"
        );
        assert_eq!(m.status.kind, StatusKind::Warn);
    }

    #[test]
    fn a_batch_that_failed_first_reloads_nothing() {
        let mut m = armed();
        let results = vec![
            StepResult::Failed(pce::write::WriteError::invalid("bad")),
            StepResult::NotSent,
            StepResult::NotSent,
        ];
        let cmds = update(
            &mut m,
            Msg::BatchDone {
                kinds: kinds(),
                summaries: summaries(),
                target: "demo (pce.example, org 5)".into(),
                results,
            },
        );
        assert_eq!(cmds, Vec::new());
    }
}

#[cfg(test)]
mod section_order_tests {
    use super::*;
    use crate::tui::model::{FlowsBy, Model, RowKind, SectionRow};
    use crate::tui::test_support::model_with_rich_flows;
    use crate::tui::traffic_marks::tests::{RICH_MARKS, marked_model};
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use pce::traffic::{Decisions, Direction, SectionOrder, group_key};

    fn key(c: char) -> Msg {
        Msg::Key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE))
    }

    fn kc(code: KeyCode) -> Msg {
        Msg::Key(KeyEvent::new(code, KeyModifiers::NONE))
    }

    #[track_caller]
    fn quiet(m: &mut Model, msg: Msg) {
        assert_eq!(update(m, msg), Vec::<Cmd>::new());
    }

    /// Each section of `rows`: its app, its header's (flows, conns), and
    /// the rows under it.
    fn sections(rows: &[SectionRow]) -> Vec<(String, u64, u64, Vec<&SectionRow>)> {
        let mut out: Vec<(String, u64, u64, Vec<&SectionRow>)> = Vec::new();
        for r in rows {
            match &r.kind {
                RowKind::Header { totals, .. } => {
                    let flows = totals.iter().map(|(_, t)| t.flows).sum();
                    let conns = totals.iter().map(|(_, t)| t.connections).sum();
                    out.push((r.app.join("/"), flows, conns, Vec::new()));
                }
                _ => out.last_mut().unwrap().3.push(r),
            }
        }
        out
    }

    fn by_name(names: &[String]) -> Vec<String> {
        let mut v = names.to_vec();
        v.sort_by_key(|n| n.to_lowercase());
        v
    }

    #[test]
    fn s_cycles_app_to_app_sections_by_conns_name_flows_and_keeps_their_rows() {
        let mut m = model_with_rich_flows();
        quiet(&mut m, key('v'));
        for x in 0..3 {
            quiet(&mut m, key('x'));
            let both = x == 2;
            let start = sections(m.traffic.apps.rows())
                .into_iter()
                .map(|(a, f, c, rows)| (a, f, c, rows.into_iter().cloned().collect::<Vec<_>>()))
                .collect::<Vec<_>>();
            assert!(
                start.windows(2).all(|w| w[0].2 >= w[1].2),
                "by conns at first"
            );
            let mut seen = Vec::new();
            for _ in 0..3 {
                quiet(&mut m, key('s'));
                seen.push(m.traffic.apps_state.order);
                let now = sections(m.traffic.apps.rows());
                assert_eq!(now.len(), start.len());
                for (app, f, c, rows) in &now {
                    let was = start.iter().find(|s| &s.0 == app).unwrap();
                    assert_eq!((was.1, was.2), (*f, *c));
                    assert!(
                        was.3.iter().eq(rows.iter().copied()),
                        "{app}: the rows keep their order (both: {both})"
                    );
                }
                let names: Vec<String> = now.iter().map(|s| s.0.clone()).collect();
                match m.traffic.apps_state.order {
                    SectionOrder::Name => assert_eq!(names, by_name(&names)),
                    SectionOrder::Flows => assert!(now.windows(2).all(|w| w[0].1 >= w[1].1)),
                    SectionOrder::Connections => {
                        let start_names: Vec<String> = start.iter().map(|s| s.0.clone()).collect();
                        assert_eq!(names, start_names, "back to the start");
                    }
                }
                assert_eq!(m.traffic.apps.sort(), None);
                assert_eq!(m.status.text, m.traffic.apps_state.order.label());
            }
            assert_eq!(
                seen,
                vec![
                    SectionOrder::Name,
                    SectionOrder::Flows,
                    SectionOrder::Connections
                ]
            );
        }
    }

    #[test]
    fn s_keeps_the_cursor_on_its_section_and_pairs_keep_the_table_sort() {
        let mut m = model_with_rich_flows();
        quiet(&mut m, key('v'));
        quiet(&mut m, key('s'));
        assert_eq!(
            m.traffic.groups.sort(),
            Some((0, false)),
            "pairs: a column sort"
        );
        assert_eq!(m.traffic.apps_state.order, SectionOrder::Connections);
        quiet(&mut m, key('x'));
        quiet(&mut m, key('j'));
        quiet(&mut m, key('j'));
        let app = m.traffic.apps.selected_item().unwrap().app.clone();
        quiet(&mut m, key('s'));
        let sel = m.traffic.apps.selected_item().unwrap();
        assert!(sel.is_header() && sel.app == app, "on its app's header");
    }

    #[test]
    fn blocked_keeps_its_order() {
        let mut m = model_with_rich_flows();
        m.traffic.view = TrafficView::Blocked;
        let before = m.traffic.blocked.rows().to_vec();
        quiet(&mut m, key('s'));
        assert_eq!(m.traffic.blocked.rows(), before.as_slice());
        assert_eq!(m.status.text, "sections keep their order (by connections)");
    }

    fn grouped(by: char) -> Model {
        let mut m = model_with_rich_flows();
        assert_eq!(m.traffic.view, TrafficView::Flows);
        for c in by.to_string().chars() {
            quiet(&mut m, key(c));
        }
        // The cursor on the first header (`x` keeps it on its flow).
        m.traffic.flows_grouped.home();
        m
    }

    #[test]
    fn x_and_capital_x_cycle_flat_by_src_by_dst_without_requests() {
        let mut m = model_with_rich_flows();
        assert!(
            m.traffic.flows_grouped.is_empty(),
            "flat builds no sections"
        );
        let mut seen = Vec::new();
        for _ in 0..3 {
            quiet(&mut m, key('x'));
            seen.push(m.traffic.flows_state.by);
        }
        assert_eq!(seen, vec![FlowsBy::Src, FlowsBy::Dst, FlowsBy::Flat]);
        let mut seen = Vec::new();
        for _ in 0..3 {
            quiet(&mut m, key('X'));
            seen.push(m.traffic.flows_state.by);
        }
        assert_eq!(seen, vec![FlowsBy::Dst, FlowsBy::Src, FlowsBy::Flat]);
        assert!(m.traffic.flows_grouped.is_empty());
        assert_eq!(
            m.traffic.apps_state.by,
            crate::tui::model::By::Pairs,
            "App-to-app untouched"
        );
    }

    #[test]
    fn by_src_and_by_dst_put_every_flow_under_its_app_once() {
        for (c, dir) in [('x', Direction::Out), ('X', Direction::In)] {
            let m = grouped(c);
            let t = &m.traffic;
            let keys = t.effective_keys();
            let secs = sections(t.flows_grouped.rows());
            assert!(secs.windows(2).all(|w| w[0].2 >= w[1].2), "by conns");
            let mut n = 0;
            for (app, flows, conns, rows) in &secs {
                assert_eq!(rows.len() as u64, *flows, "{app}");
                let sum: u64 = rows.iter().map(|r| r.flow().unwrap().num_connections).sum();
                assert_eq!(sum, *conns, "{app}");
                for r in rows {
                    let f = &r.flow().unwrap().flow;
                    let side = match dir {
                        Direction::Out => &f.src,
                        Direction::In => &f.dst,
                    };
                    assert_eq!(group_key(side, &keys).join("/"), *app);
                }
                n += rows.len();
            }
            assert_eq!(n, t.flows.rows().len(), "every flow once");
            // The headers count flows, not groups.
            let first = t.flows_grouped.rows()[0].header_line().unwrap();
            assert!(
                first.contains(" flows · ") && !first.contains("group"),
                "{first}"
            );
        }
    }

    #[test]
    fn a_sections_flows_follow_the_flat_tables_sort() {
        let mut m = model_with_rich_flows();
        // conns, descending: column 6 of the flat table.
        m.traffic.flows.sort_by(6, true);
        quiet(&mut m, key('x'));
        for (app, _, _, rows) in sections(m.traffic.flows_grouped.rows()) {
            let conns: Vec<u64> = rows
                .iter()
                .map(|r| r.flow().unwrap().num_connections)
                .collect();
            assert!(conns.windows(2).all(|w| w[0] >= w[1]), "{app}: {conns:?}");
        }
        assert_eq!(
            m.traffic.flows_grouped.sort(),
            None,
            "no sort across sections"
        );
        quiet(&mut m, key('s'));
        assert_eq!(m.traffic.flows_state.order, SectionOrder::Name);
        assert_eq!(
            m.traffic.flows.sort(),
            Some((6, true)),
            "s orders the sections"
        );
        let names: Vec<String> = sections(m.traffic.flows_grouped.rows())
            .into_iter()
            .map(|s| s.0)
            .collect();
        assert_eq!(names, by_name(&names));
        quiet(&mut m, key('s'));
        let secs = sections(m.traffic.flows_grouped.rows());
        assert!(secs.windows(2).all(|w| w[0].1 >= w[1].1), "by flows");
    }

    #[test]
    fn space_and_z_fold_flows_sections_and_the_folds_survive_a_rebuild() {
        let mut m = grouped('x');
        let all = m.traffic.flows_grouped.len();
        let headers = sections(m.traffic.flows_grouped.rows()).len();
        quiet(&mut m, key('j'));
        quiet(&mut m, key(' '));
        let sel = m.traffic.flows_grouped.selected_item().unwrap().clone();
        assert!(sel.is_header());
        assert!(m.traffic.flows_state.folded().contains(&sel.app));
        assert!(m.traffic.apps_state.folded.is_empty(), "its own folds");
        assert!(m.traffic.flows_grouped.len() < all);
        m.traffic.rebuild_views();
        assert!(m.traffic.flows_grouped.len() < all, "kept");
        quiet(&mut m, key(' '));
        assert_eq!(m.traffic.flows_grouped.len(), all);
        quiet(&mut m, key('z'));
        assert_eq!(m.traffic.flows_grouped.len(), headers, "headers only");
        quiet(&mut m, key('z'));
        assert_eq!(m.traffic.flows_grouped.len(), all);
    }

    #[test]
    fn enter_on_a_flow_opens_the_detail_and_on_a_header_drills_to_the_app() {
        let mut m = grouped('x');
        quiet(&mut m, key('j'));
        assert!(m.traffic.selected_flow().is_some());
        quiet(&mut m, kc(KeyCode::Enter));
        assert!(m.traffic.show_detail);
        assert_eq!(m.traffic.drill, None, "a flow row opens its detail");
        quiet(&mut m, kc(KeyCode::Enter));
        assert!(!m.traffic.show_detail);
        quiet(&mut m, key('k'));
        let app = m.traffic.flows_grouped.selected_item().unwrap().app.clone();
        let (_, flows, _, _) = sections(m.traffic.flows_grouped.rows())[0].clone();
        quiet(&mut m, kc(KeyCode::Enter));
        assert_eq!(
            m.traffic.drill,
            Some(Drill::App {
                app: app.clone(),
                direction: Some(Direction::Out),
                decisions: Decisions::All,
            })
        );
        assert_eq!(m.traffic.view, TrafficView::Flows);
        assert_eq!(
            m.traffic.flows.rows().len() as u64,
            flows,
            "the app's flows"
        );
        let secs = sections(m.traffic.flows_grouped.rows());
        assert_eq!(secs.len(), 1);
        assert_eq!(secs[0].0, app.join("/"));
        assert!(m.traffic.flows_grouped.selected_item().unwrap().is_header());
        quiet(&mut m, kc(KeyCode::Esc));
        assert_eq!(m.traffic.drill, None);
        assert!(sections(m.traffic.flows_grouped.rows()).len() > 1);
    }

    #[test]
    fn f_w_and_m_read_the_grouped_row() {
        let mut m = grouped('x');
        m.writes = crate::tui::model::WritesState::on();
        let app = m.traffic.flows_grouped.selected_item().unwrap().app.clone();
        assert_eq!(m.traffic.row_apps(), vec![app.clone()], "a header: its app");
        quiet(&mut m, key('w'));
        assert_eq!(
            m.status.text,
            "select a flow row (a header holds a whole app)"
        );
        quiet(&mut m, key('j'));
        let f = m.traffic.selected_flow().unwrap().flow.clone();
        let keys = m.traffic.effective_keys();
        let mut want = vec![group_key(&f.src, &keys), group_key(&f.dst, &keys)];
        want.dedup();
        assert_eq!(m.traffic.row_apps(), want, "a flow: its two sides");
        quiet(&mut m, key('k'));
        quiet(&mut m, key('F'));
        assert_eq!(m.traffic.focus.as_ref().map(|f| f.app.clone()), Some(app));
    }

    #[test]
    fn search_in_flows_sections_keeps_whole_sections_once_per_section() {
        let mut m = grouped('x');
        quiet(&mut m, key('/'));
        for c in "shop-db-p".chars() {
            quiet(&mut m, key(c));
        }
        quiet(&mut m, kc(KeyCode::Enter));
        let t = &m.traffic;
        let shown: Vec<SectionRow> = t
            .flows_grouped
            .visible()
            .iter()
            .map(|&i| t.flows_grouped.rows()[i].clone())
            .collect();
        let secs = sections(&shown);
        assert!(!secs.is_empty());
        for (app, flows, _, rows) in &secs {
            assert_eq!(rows.len() as u64, *flows, "{app}: the whole section");
            assert!(
                rows.iter()
                    .any(|r| r.flow().unwrap().dst.display_name() == "shop-db-p.example"),
                "{app}"
            );
        }
        assert!(secs.len() < sections(t.flows_grouped.rows()).len());
    }

    #[test]
    fn enter_on_a_flow_keeps_the_cursor_on_screen_with_the_mark_line() {
        let (mut m, _dir) = marked_model(RICH_MARKS);
        m.size = (80, 24);
        quiet(&mut m, key('x'));
        for _ in 0..3 {
            quiet(&mut m, key('j'));
        }
        assert!(m.traffic.selected_flow().is_some());
        quiet(&mut m, kc(KeyCode::Enter));
        assert!(m.traffic.show_detail);
        let page = traffic_page(&m);
        let t = &m.traffic.flows_grouped;
        assert!(
            t.selected_pos() >= t.offset() && t.selected_pos() < t.offset() + page,
            "row {} not in {}..{}",
            t.selected_pos(),
            t.offset(),
            t.offset() + page
        );
    }

    #[test]
    fn the_flows_sections_are_built_only_while_the_flows_view_shows() {
        let mut m = grouped('x');
        let n = m.traffic.flows_grouped.len();
        quiet(&mut m, key('v'));
        m.traffic.rebuild_views();
        assert!(
            m.traffic.flows_grouped.is_empty(),
            "App-to-app on screen: none"
        );
        // App-to-app's own keys leave them alone.
        quiet(&mut m, key('x'));
        quiet(&mut m, key(' '));
        assert!(m.traffic.flows_grouped.is_empty());
        for _ in 0..4 {
            quiet(&mut m, key('v'));
        }
        assert_eq!(m.traffic.view, TrafficView::Flows);
        assert_eq!(
            m.traffic.flows_grouped.len(),
            n,
            "built again on the way back"
        );
    }

    #[test]
    fn a_flat_search_and_sort_carry_into_the_sections_and_the_meta() {
        let mut m = model_with_rich_flows();
        quiet(&mut m, key('/'));
        for c in "5432".chars() {
            quiet(&mut m, key(c));
        }
        quiet(&mut m, kc(KeyCode::Enter));
        let kept = m.traffic.flows.len();
        assert!(kept > 0 && kept < 30);
        quiet(&mut m, key('x'));
        let secs = sections(m.traffic.flows_grouped.rows());
        assert_eq!(
            secs.iter().map(|s| s.3.len()).sum::<usize>(),
            kept,
            "the flat search applies"
        );
        m.traffic.flows.sort_by(6, true);
        m.traffic.rebuild_views();
        let out = crate::tui::test_support::render_to_string(&m, 120, 30);
        assert!(
            out.contains("by src app · sort conns ↓ · flows /5432"),
            "{out}"
        );
    }

    #[test]
    fn x_keeps_the_cursor_on_its_flow_and_folds_are_per_grouping() {
        let mut m = model_with_rich_flows();
        for _ in 0..5 {
            quiet(&mut m, key('j'));
        }
        let flow = m.traffic.selected_flow().unwrap().flow.clone();
        quiet(&mut m, key('x'));
        assert!(std::sync::Arc::ptr_eq(
            &m.traffic.selected_flow().unwrap().flow,
            &flow
        ));
        quiet(&mut m, key('x'));
        assert!(std::sync::Arc::ptr_eq(
            &m.traffic.selected_flow().unwrap().flow,
            &flow
        ));
        // Fold by destination app; by source app has its own folds.
        quiet(&mut m, key(' '));
        let dst_folds = m.traffic.flows_state.folded_dst.clone();
        assert_eq!(dst_folds.len(), 1);
        quiet(&mut m, key('X'));
        assert_eq!(m.traffic.flows_state.by, FlowsBy::Src);
        assert!(m.traffic.flows_state.folded().is_empty());
        assert!(
            m.traffic
                .flows_grouped
                .rows()
                .iter()
                .all(|r| !matches!(r.kind, RowKind::Header { folded: true, .. }))
        );
        quiet(&mut m, key('x'));
        assert_eq!(m.traffic.flows_state.folded(), &dst_folds, "kept");
        // Back to flat: the cursor still on a flow of the table.
        quiet(&mut m, key('x'));
        assert_eq!(m.traffic.flows_state.by, FlowsBy::Flat);
        assert!(m.traffic.selected_flow().is_some());
    }

    #[test]
    fn f_on_a_flow_by_dst_app_focuses_the_sections_app_first() {
        let mut m = grouped('X');
        quiet(&mut m, key('j'));
        let r = m.traffic.flows_grouped.selected_item().unwrap().clone();
        let f = &r.flow().unwrap().flow;
        let keys = m.traffic.effective_keys();
        assert_eq!(group_key(&f.dst, &keys), r.app);
        assert_eq!(m.traffic.row_apps()[0], r.app);
        quiet(&mut m, key('F'));
        assert_eq!(m.traffic.focus.as_ref().map(|x| x.app.clone()), Some(r.app));
    }

    #[test]
    fn enter_on_a_header_under_a_drill_narrows_and_never_widens() {
        // A drill to one destination app, grouped by source app: a
        // header narrows to the pair.
        let mut m = model_with_rich_flows();
        let crm = vec!["crm".to_string(), "prod".to_string()];
        m.traffic.drill = Some(Drill::App {
            app: crm.clone(),
            direction: Some(Direction::In),
            decisions: Decisions::All,
        });
        m.traffic.rebuild_views();
        quiet(&mut m, key('x'));
        m.traffic.flows_grouped.home();
        let src = m.traffic.flows_grouped.selected_item().unwrap().app.clone();
        let n = sections(m.traffic.flows_grouped.rows())[0].1;
        quiet(&mut m, kc(KeyCode::Enter));
        assert_eq!(
            m.traffic.drill,
            Some(Drill::Pair(src.clone(), crm.clone(), Decisions::All))
        );
        assert_eq!(m.traffic.flows.rows().len() as u64, n);
        // One app on each side already: nothing changes.
        quiet(&mut m, kc(KeyCode::Enter));
        assert_eq!(
            m.traffic.drill,
            Some(Drill::Pair(src, crm.clone(), Decisions::All))
        );
        assert_eq!(m.status.text, "the drill holds only this app already");
        // Either side (`both`): no single drill says it; the header folds.
        m.traffic.drill = Some(Drill::App {
            app: crm.clone(),
            direction: None,
            decisions: Decisions::All,
        });
        m.traffic.rebuild_views();
        m.traffic.flows_grouped.home();
        let before = m.traffic.drill.clone();
        quiet(&mut m, kc(KeyCode::Enter));
        assert_eq!(m.traffic.drill, before, "never widened");
        assert_eq!(m.status.text, "drill active: Esc first");
        assert!(matches!(
            m.traffic.flows_grouped.selected_item().unwrap().kind,
            RowKind::Header { folded: true, .. }
        ));
    }

    #[test]
    fn m_on_a_flows_header_marks_the_app_side() {
        let dir = tempfile::tempdir().unwrap();
        let mut m = crate::tui::test_support::model_with_marks(dir.path(), "");
        quiet(&mut m, key('x'));
        m.traffic.flows_grouped.home();
        let app = m
            .traffic
            .flows_grouped
            .selected_item()
            .unwrap()
            .app
            .join("/");
        quiet(&mut m, key('m'));
        let crate::tui::model::Overlay::MarkForm(f) = &m.overlay else {
            panic!("{:?}", m.overlay)
        };
        assert_eq!(f.ctx.rows, crate::tui::mark_form::RowsOf::Flows);
        let value = |l: &str| {
            f.form
                .fields
                .iter()
                .find(|x| x.label == l)
                .unwrap()
                .value
                .clone()
        };
        assert_eq!(value("Destination"), "any", "the app is the source");
        assert_eq!(
            value("Source"),
            "ipl:Corp",
            "{app}: the IP list by its name"
        );
        assert_eq!(value("Service"), "");
    }

    #[test]
    fn a_section_whose_flows_are_all_squelched_disappears_and_the_counts_add_up() {
        // Every flow from hr/prod squelched; erp/prod's to crm/prod only.
        let marks = r#"
[[mark]]
id = "m-hrsq"
action = "squelch"
src = { labels = { app = "hr", env = "prod" } }
dst = "any"
comment = "hr out"

[[mark]]
id = "m-erpsq"
action = "squelch"
src = { labels = { app = "erp", env = "prod" } }
dst = { labels = { app = "crm" } }
comment = "erp to crm"

[[mark]]
id = "m-crit"
action = "critical"
src = "any"
dst = { labels = { app = "shop", env = "prod", role = "db" } }
decision = "blocked"
comment = "payments db"
"#;
        let (mut m, _dir) = marked_model(marks);
        let hr_out = m
            .traffic
            .loaded_flows()
            .iter()
            .filter(|f| f.src.label_value("app") == Some("hr"))
            .count();
        assert!(hr_out > 0);
        let flat = m.traffic.marked.as_ref().unwrap().flows;
        quiet(&mut m, key('x'));
        let t = &m.traffic;
        let secs = sections(t.flows_grouped.rows());
        assert!(
            secs.iter().all(|s| s.0 != "hr/prod"),
            "every flow from hr/prod is squelched: no section"
        );
        let shown: usize = secs.iter().map(|s| s.3.len()).sum();
        let mv = t.marked.as_ref().unwrap();
        assert_eq!(mv.flows, flat, "the flat view's counts");
        assert_eq!(shown, t.flows.rows().len());
        assert_eq!(shown as u64 + mv.flows.flows, 30, "kept + squelched = all");
        assert_eq!(
            t.marks_meta(TrafficView::Flows),
            mv.flows.meta(false, true),
            "the flat view's meta"
        );
        // erp/prod keeps its other flows: a partly squelched header with
        // its hidden records.
        let erp = t
            .flows_grouped
            .rows()
            .iter()
            .find(|r| r.is_header() && r.app.join("/") == "erp/prod")
            .unwrap();
        let RowKind::Header {
            marks: Some(rm), ..
        } = &erp.kind
        else {
            panic!("{erp:?}")
        };
        assert!(rm.hidden > 0, "{rm:?}");
        assert_eq!(
            crate::tui::traffic_marks::section_marker(erp),
            Some(pce::marks::Marker::PartlySquelched(rm.hidden))
        );
        // A header sums its flows' hits; the gutter follows them.
        assert!(t.flows_grouped.marker.is_some());
        let crit = t
            .flows_grouped
            .rows()
            .iter()
            .find(|r| {
                r.flow().is_some_and(|f| {
                    f.mark
                        .is_some_and(|h| h.action == pce::marks::Action::Critical)
                })
            })
            .unwrap();
        let header = t
            .flows_grouped
            .rows()
            .iter()
            .find(|r| r.is_header() && r.app == crit.app)
            .unwrap();
        assert_eq!(
            crate::tui::traffic_marks::section_marker(header),
            Some(pce::marks::Marker::Critical)
        );
        // `M`: the squelched flows back, dimmed, hr's section with them.
        quiet(&mut m, key('M'));
        let secs = sections(m.traffic.flows_grouped.rows());
        let hr = m
            .traffic
            .flows_grouped
            .rows()
            .iter()
            .find(|r| r.is_header() && r.app.join("/") == "hr/prod")
            .unwrap();
        assert_eq!(
            crate::tui::traffic_marks::section_marker(hr),
            Some(pce::marks::Marker::Squelched)
        );
        assert_eq!(secs.iter().map(|s| s.3.len()).sum::<usize>(), 30);
    }

    #[test]
    fn export_writes_the_grouped_flows_with_a_section_column() {
        let mut m = grouped('X');
        quiet(&mut m, key('z')); // folded sections still export every flow
        let (headers, rows) = crate::tui::export::current_table(&m).unwrap();
        let (flat_headers, _) = m.traffic.flows.export();
        assert_eq!(headers[0], "section");
        assert_eq!(&headers[1..], flat_headers.as_slice());
        assert_eq!(rows.len(), m.traffic.flows.rows().len());
        let secs = sections(&m.traffic.unfolded_section_rows().unwrap())
            .into_iter()
            .map(|s| s.0)
            .collect::<Vec<_>>();
        let mut seen: Vec<String> = rows.iter().map(|r| r[0].clone()).collect();
        seen.dedup();
        assert_eq!(seen, secs, "the sections in order, each whole");
        // The dst group column matches the section (by dst app).
        let dst = headers.iter().position(|h| *h == "dst group").unwrap();
        // (A non-workload's group cell is `-/-`; its section is its name.)
        assert!(
            rows.iter()
                .filter(|r| !r[dst].starts_with('-'))
                .all(|r| r[0] == r[dst]),
            "{rows:?}"
        );
    }

    #[test]
    fn the_flows_state_survives_g_and_a_profile_switch_resets_it() {
        let mut m = grouped('x');
        quiet(&mut m, key('s'));
        quiet(&mut m, key('z'));
        let st = m.traffic.flows_state.clone();
        quiet(&mut m, key('g'));
        quiet(&mut m, kc(KeyCode::Enter));
        assert_eq!(m.traffic.flows_state, st, "g keeps it");
        assert!(!m.traffic.flows_grouped.is_empty());
        m.reset_data();
        assert_eq!(
            m.traffic.flows_state,
            crate::tui::model::FlowsState::default(),
            ":profile resets it"
        );
    }
}

#[cfg(test)]
mod workload_group_tests {
    use super::*;
    use crate::tui::model::{Model, Screen, StatusKind};
    use crate::tui::msg::{Cmd, Msg};
    use crate::tui::test_support::model_with_workloads;
    use crate::tui::wl_groups::{GroupOrder, WlGroupRow};
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    fn key(c: char) -> Msg {
        Msg::Key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE))
    }

    fn code(c: KeyCode) -> Msg {
        Msg::Key(KeyEvent::new(c, KeyModifiers::NONE))
    }

    /// Sends `msg` and asserts it asked for nothing: everything here is
    /// local to the loaded list.
    #[track_caller]
    fn quiet(m: &mut Model, msg: Msg) {
        assert_eq!(update(m, msg), Vec::<Cmd>::new());
    }

    fn on_workloads() -> Model {
        let mut m = model_with_workloads();
        m.screen = Screen::Workloads;
        m
    }

    fn headers(m: &Model) -> Vec<(String, bool)> {
        m.workloads
            .sections
            .rows()
            .iter()
            .filter_map(|r| match r {
                WlGroupRow::Header { group, folded } => Some((group.title.clone(), *folded)),
                WlGroupRow::Member { .. } => None,
            })
            .collect()
    }

    fn type_text(m: &mut Model, text: &str) {
        for c in text.chars() {
            quiet(m, key(c));
        }
    }

    #[test]
    fn x_groups_by_the_traffic_keys_and_back_without_a_request() {
        let mut m = on_workloads();
        quiet(&mut m, key('x'));
        assert!(m.workloads.grouped);
        assert_eq!(m.workloads.group_keys, ["app", "env"]);
        assert_eq!(
            headers(&m),
            [
                ("shop | prod".to_string(), false),
                ("(no app) | (no env)".to_string(), false)
            ]
        );
        assert_eq!(m.workloads.sections.rows().len(), 6);
        // The cursor stays on its workload: host-1 is the first member.
        assert_eq!(
            m.workloads.selected_row().unwrap().workload.href,
            "/orgs/5/workloads/aaaa-1"
        );
        quiet(&mut m, key('x'));
        assert!(!m.workloads.grouped);
        assert!(
            m.workloads.sections.rows().is_empty(),
            "nothing kept while flat"
        );
        assert_eq!(
            m.workloads.selected_row().unwrap().workload.href,
            "/orgs/5/workloads/aaaa-1"
        );
    }

    #[test]
    fn g_keys_regroup_the_workloads() {
        let mut m = on_workloads();
        quiet(&mut m, key('x'));
        apply_group_keys(&mut m, vec!["role".into()], false);
        assert_eq!(m.workloads.group_keys, ["role"]);
        let titles: Vec<String> = headers(&m).into_iter().map(|(t, _)| t).collect();
        assert_eq!(titles, ["(no role)", "db", "web"]);
    }

    #[test]
    fn space_folds_z_folds_all_and_s_orders_the_groups() {
        let mut m = on_workloads();
        quiet(&mut m, key('x'));
        quiet(&mut m, key('j')); // host-1 under shop | prod
        quiet(&mut m, key(' '));
        assert_eq!(headers(&m)[0], ("shop | prod".to_string(), true));
        assert!(m.workloads.selected_group().is_some(), "on its header");
        assert_eq!(m.workloads.sections.rows().len(), 4);
        quiet(&mut m, key(' '));
        assert_eq!(m.workloads.sections.rows().len(), 6);
        quiet(&mut m, key('z'));
        assert!(headers(&m).iter().all(|(_, f)| *f));
        assert_eq!(m.workloads.sections.rows().len(), 2);
        quiet(&mut m, key('z'));
        assert!(headers(&m).iter().all(|(_, f)| !*f));
        // `s` orders the groups, not the table; the cursor keeps its header.
        let sort = m.workloads.table.sort();
        quiet(&mut m, key('s'));
        assert_eq!(m.workloads.group_order, GroupOrder::Name);
        assert_eq!(m.workloads.table.sort(), sort);
        assert_eq!(m.status.text, "groups by name A→Z");
        quiet(&mut m, key('s'));
        assert_eq!(m.workloads.group_order, GroupOrder::Unhealthy);
        assert_eq!(
            m.workloads.selected_group().unwrap().title,
            "shop | prod",
            "the cursor follows its header"
        );
        // Flat, `s` sorts the table again.
        quiet(&mut m, key('x'));
        quiet(&mut m, key('s'));
        assert_eq!(m.workloads.table.sort(), Some((0, false)));
    }

    #[test]
    fn enter_on_a_header_narrows_and_on_a_workload_opens_the_detail() {
        let mut m = on_workloads();
        quiet(&mut m, key('x'));
        quiet(&mut m, code(KeyCode::End)); // host-4, the last member
        quiet(&mut m, code(KeyCode::Enter));
        assert!(m.workloads.show_detail);
        quiet(&mut m, code(KeyCode::Enter));
        assert!(!m.workloads.show_detail);
        // The (no app) | (no env) header: missing labels narrow as key=-.
        quiet(&mut m, key('k'));
        quiet(&mut m, key('k'));
        assert_eq!(
            m.workloads.selected_group().unwrap().title,
            "(no app) | (no env)"
        );
        quiet(&mut m, code(KeyCode::Enter));
        assert_eq!(m.workloads.labels.to_string(), "app=- env=-");
        assert_eq!(m.workloads.table.len(), 2);
        assert_eq!(headers(&m).len(), 1);
        assert!(
            m.workloads
                .filter_line()
                .unwrap()
                .starts_with("labels app=- env=- · 2 of 4")
        );
        // Another group's header replaces those keys, other terms stay.
        quiet(&mut m, key('c'));
        assert!(m.workloads.labels.is_empty(), "c clears the label filter");
        apply_workload_labels(&mut m, "!role=db").unwrap();
        // (no app) has two workloads now, shop one: shop's header is second.
        quiet(&mut m, code(KeyCode::End));
        quiet(&mut m, key('k'));
        quiet(&mut m, code(KeyCode::Enter));
        assert_eq!(m.workloads.labels.to_string(), "!role=db app=shop env=prod");
        assert_eq!(m.workloads.table.len(), 1);
    }

    #[test]
    fn search_filters_the_sections_and_filters_keep_working_grouped() {
        let mut m = on_workloads();
        quiet(&mut m, key('x'));
        quiet(&mut m, key('/'));
        type_text(&mut m, "host-4");
        assert!(m.workloads.table.is_searching());
        assert_eq!(headers(&m).len(), 1);
        assert_eq!(headers(&m)[0].0, "(no app) | (no env)");
        quiet(&mut m, code(KeyCode::Esc));
        assert_eq!(headers(&m).len(), 2);
        // `m` (managed only), then `h` keep their sections current.
        quiet(&mut m, key('m'));
        let members: usize = m.workloads.groups.iter().map(|g| g.count()).sum();
        assert_eq!(members, 3);
        quiet(&mut m, key('c'));
        assert_eq!(m.workloads.sections.rows().len(), 6);
    }

    #[test]
    fn f_edits_the_label_filter_locally() {
        let mut m = on_workloads();
        quiet(&mut m, key('f'));
        assert!(matches!(m.overlay, Overlay::LabelFilter(_)));
        type_text(&mut m, "app=shop,crm !role=db");
        quiet(&mut m, code(KeyCode::Enter));
        assert_eq!(m.overlay, Overlay::None);
        assert_eq!(m.workloads.labels.to_string(), "app=shop,crm !role=db");
        assert_eq!(m.workloads.table.len(), 1, "host-1: shop, role web");
        assert_eq!(m.status.kind, StatusKind::Warn, "crm is no label");
        assert_eq!(m.workloads.label_warnings, ["unknown value app=crm"]);
        // Malformed: the error shows in the editor and the old filter holds.
        quiet(&mut m, key('f'));
        type_text(&mut m, " env=");
        quiet(&mut m, code(KeyCode::Enter));
        match &m.overlay {
            Overlay::LabelFilter(e) => assert!(e.error.as_deref().unwrap().contains("env=-")),
            o => panic!("{o:?}"),
        }
        assert_eq!(m.workloads.labels.to_string(), "app=shop,crm !role=db");
        quiet(&mut m, code(KeyCode::Esc));
        assert_eq!(m.overlay, Overlay::None);
        // An empty filter clears it.
        quiet(&mut m, key('f'));
        for _ in 0.."app=shop,crm !role=db".len() {
            quiet(&mut m, code(KeyCode::Backspace));
        }
        quiet(&mut m, code(KeyCode::Enter));
        assert!(m.workloads.labels.is_empty());
        assert_eq!(m.workloads.table.len(), 4);
    }

    #[test]
    fn ctrl_n_completes_keys_then_values_of_the_label_filter() {
        let mut m = on_workloads();
        quiet(&mut m, key('f'));
        type_text(&mut m, "ap");
        let ctrl_n = || Msg::Key(KeyEvent::new(KeyCode::Char('n'), KeyModifiers::CONTROL));
        let value = |m: &Model| match &m.overlay {
            Overlay::LabelFilter(e) => e.value.clone(),
            o => panic!("{o:?}"),
        };
        quiet(&mut m, ctrl_n());
        assert_eq!(value(&m), "app=");
        quiet(&mut m, ctrl_n());
        assert_eq!(value(&m), "app=shop", "dives into the key's values");
        quiet(&mut m, key(','));
        quiet(&mut m, ctrl_n());
        assert_eq!(
            value(&m),
            "app=shop,-",
            "the listed value is left out; - last"
        );
        type_text(&mut m, " !ro");
        quiet(&mut m, ctrl_n());
        assert_eq!(value(&m), "app=shop,- !role=");
    }

    #[test]
    fn colon_filter_sets_the_label_filter_on_workloads_only() {
        use crate::tui::commands::Command;
        let mut m = on_workloads();
        quiet(&mut m, Msg::Command(Command::Filter("env=prod".into())));
        assert_eq!(m.workloads.labels.to_string(), "env=prod");
        assert_eq!(m.workloads.table.len(), 2);
        assert_eq!(m.traffic.filter.src, "", "the traffic filter is untouched");
        quiet(&mut m, Msg::Command(Command::Filter("app=".into())));
        assert_eq!(m.status.kind, StatusKind::Error);
        assert_eq!(m.workloads.labels.to_string(), "env=prod");
        // Elsewhere `:filter` still edits the traffic source.
        m.screen = Screen::Traffic;
        update(&mut m, Msg::Command(Command::Filter("app=shop".into())));
        assert_eq!(m.traffic.filter.src, "app=shop");
    }

    #[test]
    fn x_on_workloads_no_longer_deletes() {
        let mut m = writes_on_workloads();
        quiet(&mut m, key('j'));
        quiet(&mut m, key('j')); // unmanaged-nas
        quiet(&mut m, key('x'));
        assert!(m.workloads.grouped);
        assert_eq!(m.overlay, Overlay::None);
    }

    #[test]
    fn shift_x_deletes_the_grouped_workload_and_refuses_a_header() {
        use crate::tui::edit::WriteIntent;
        let mut m = writes_on_workloads();
        quiet(&mut m, key('x'));
        // shop | prod, host-1, host-2, (no app) | (no env), unmanaged-nas, host-4.
        quiet(&mut m, code(KeyCode::End));
        quiet(&mut m, key('k'));
        assert_eq!(
            m.workloads.selected_row().unwrap().workload.href,
            "/orgs/5/workloads/aaaa-3"
        );
        assert_eq!(
            update(&mut m, key('X')),
            vec![Cmd::PlanWrite(WriteIntent::DeleteWorkload {
                href: "/orgs/5/workloads/aaaa-3".into(),
                name: "unmanaged-nas".into(),
            })],
            "the same plan, then the same confirm, as flat"
        );
        let mut m = writes_on_workloads();
        quiet(&mut m, key('x'));
        quiet(&mut m, code(KeyCode::Home));
        assert!(m.workloads.selected_group().is_some());
        quiet(&mut m, key('X'));
        assert_eq!(m.status.text, "no row selected");
        assert_eq!(m.overlay, Overlay::None);
    }

    #[test]
    fn label_warnings_follow_a_reload() {
        let mut m = on_workloads();
        apply_workload_labels(&mut m, "app=crm").unwrap();
        assert_eq!(m.workloads.label_warnings, ["unknown value app=crm"]);
        let mut raw = m.workloads.raw.clone();
        raw[0].labels[1].value = Some("crm".into());
        quiet(&mut m, Msg::WorkloadsLoaded(Ok((raw, 0, false))));
        assert!(m.workloads.label_warnings.is_empty());
        assert_eq!(m.workloads.table.len(), 1);
    }

    fn writes_on_workloads() -> Model {
        let mut m = on_workloads();
        m.writes = crate::tui::model::WritesState::on();
        m
    }
}
