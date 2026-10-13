//! Pure state transitions. Never performs I/O; returns `Cmd`s instead.
use crate::tui::app_picker::{AppPicker, AppPickerOutcome, AppPickerPurpose};
use crate::tui::commands::{self, Command};
use crate::tui::edit::{self, EditIntent, RulesetTarget, Start, WriteIntent};
use crate::tui::filter::{FilterEditor, TrafficFilter};
use crate::tui::form::{Completing, FormEvent};
use crate::tui::model::{
    By, Connection, Dir, Drill, EventsFilterEdit, Model, ObjectKind, Overlay, PendingAction,
    PolicyPane, PolicyState, QueryState, Screen, Slices, StatusLine, TrafficData, TrafficState,
    TrafficView, WritesState, queries_text, rule_line,
};
use crate::tui::msg::{Cmd, FocusedOutcome, Msg, ObjectsPayload, QueryOutcome};
use crate::tui::picker::{KeyPicker, PickerOutcome};
use crate::tui::ui::widgets::overlay::{confirm_write_last_scroll, help_last_scroll};
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use pce::models::{EventFilter, PolicyDecision, PolicyScope};
use pce::traffic::treemap::Weight;
use pce::traffic::{Decisions, Focus, GroupKey, QueryProgress};
use pce::write::{
    Outcome, Plan, ProvisionRequest, ProvisionSubset, RevertRequest, WriteError, provision_counts,
};
use pce::{CacheMode, RuleKind, WriteMethod};
use std::time::Instant;

pub fn update(model: &mut Model, msg: Msg) -> Vec<Cmd> {
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
                && model
                    .traffic
                    .data
                    .as_ref()
                    .is_some_and(|d| d.backfill.is_none())
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
                cmds.extend(deny_fallback(model, CacheMode::Use));
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
            // Key auto-repeat on `t` would otherwise restart (and re-POST) the same query.
            if model.traffic.is_running() && model.traffic.filter == f {
                model.status = StatusLine::info("query already running");
                return Vec::new();
            }
            model.traffic.filter = f.clone();
            start_query(model, f, CacheMode::Use)
        }
        Msg::RunNow => {
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
            // Merged slices were confirmed slice by slice: one query for the
            // whole window is a different (larger) request.
            // A backfill was confirmed slice by slice too, even with one slice.
            let fetched_with = model.traffic.data.as_ref().is_some_and(|d| {
                d.filter == f
                    && d.backfill.is_none()
                    && d.slices.as_ref().is_none_or(|s| s.count() <= 1)
            });
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
                    let QueryOutcome {
                        flows,
                        truncated,
                        matches_count,
                        elapsed,
                        hit,
                        filter,
                        bounds,
                    } = o;
                    // A new unfocused result: a focused one kept aside is stale.
                    model.traffic.unfocused = None;
                    model.traffic.focus_result = None;
                    let n = flows.len();
                    model.traffic.data = Some(TrafficData {
                        flows,
                        truncated,
                        matches_count,
                        elapsed,
                        cache_hit: hit,
                        fetched_at: Instant::now(),
                        filter,
                        slices: bounds.map(|(start, end)| Slices::new(start, end)),
                        backfill: None,
                    });
                    model.traffic.drill = None;
                    model.traffic.treemap.zoom = None;
                    model.traffic.rebuild_views();
                    model.traffic.show_detail = false;
                    model.traffic.query = QueryState::Idle;
                    model.traffic.auto.from_tick = false;
                    if let Some(status) = low_coverage_status(model) {
                        model.status = status;
                        if model.limits.traffic_auto_backfill {
                            return auto_backfill(model);
                        }
                        return Vec::new();
                    }
                    let focus_hint = model
                        .traffic
                        .focus
                        .as_ref()
                        .map(|f| format!(" · focus is partial: r re-queries {}", f.app_text()))
                        .unwrap_or_default();
                    model.status = if truncated {
                        StatusLine::warn(format!(
                            "{n} flows (truncated: the PCE matched {}; raise the cap or narrow the filter){focus_hint}",
                            matches_count
                                .map(|m| m.to_string())
                                .unwrap_or_else(|| "more".into())
                        ))
                    } else {
                        StatusLine::info(format!(
                            "{n} flows in {:.1}s{}",
                            elapsed.as_secs_f64(),
                            if hit { " (cached)" } else { "" }
                        ))
                    };
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
                    let QueryOutcome {
                        flows,
                        truncated,
                        matches_count,
                        elapsed,
                        hit,
                        filter,
                        bounds,
                    } = outcome;
                    let data = TrafficData {
                        flows,
                        truncated,
                        matches_count,
                        elapsed,
                        cache_hit: hit,
                        fetched_at: Instant::now(),
                        filter,
                        slices: bounds.map(|(start, end)| Slices::new(start, end)),
                        backfill: None,
                    };
                    let app = focus.app_text();
                    model.traffic.query = QueryState::Idle;
                    model.traffic.show_detail = false;
                    model.traffic.apply_focused(focus, queries, data);
                    let n = model.traffic.view_flows().len();
                    let q = queries_text(queries);
                    model.status = if truncated {
                        StatusLine::warn(format!(
                            "{n} flows for {app} · {q} (truncated: raise the cap or narrow the filter)"
                        ))
                    } else {
                        StatusLine::info(format!(
                            "{n} flows for {app} in {:.1}s · {q}{}",
                            elapsed.as_secs_f64(),
                            if hit { " (cached)" } else { "" }
                        ))
                    };
                }
                Err(e) => {
                    model.traffic.query = QueryState::Failed(e.clone());
                    fail(model, "focused query", &e, false);
                }
            }
            Vec::new()
        }
        Msg::SliceDone { seq, result } => {
            if model.traffic.seq != seq {
                return Vec::new(); // `f`, `:window`, `-`, `r` or `:profile` moved on
            }
            if model.traffic.backfill.is_some() {
                return on_backfill_slice(model, result);
            }
            on_slice(model, result);
            Vec::new()
        }
        Msg::RuleSetsLoaded { scope, result } => {
            // A reload a write asked for keeps the write's report. Taken before
            // the scope check: an ignored answer must not leave the flag set.
            let quiet = std::mem::take(&mut model.policy.quiet_reload);
            if scope != model.policy.scope {
                return Vec::new(); // the user toggled draft/active meanwhile
            }
            model.policy.loading = false;
            match result {
                Ok((raw, age)) => {
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
                    deny_fallback(model, mode)
                }
                Err(e) => {
                    // The explicit refresh failed: its bypass must not leak into a later reload.
                    model.policy.bypass_deny = false;
                    let tripped = model.policy.auto.take_tick_failure();
                    fail(model, "rulesets", &e, tripped);
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
                    w.show_detail &= w.table.selected_item().is_some();
                    model.status =
                        StatusLine::info(format!("{} workloads", model.workloads.table.len()));
                    // The rules pane's P/C and impact follow; an open rule
                    // detail reads them from the rows, so no overlay is
                    // touched here.
                    model.rebuild_rules();
                    model.recount_labels();
                    model.refresh_confirm_impact();
                }
                Err(e) => {
                    model.workloads.error = Some(e.clone());
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
                    if kind.whole_policy() {
                        model.policy.provisioned = Some(model.status.text.clone());
                    }
                    let mut cmds = reload_after_write(model, kind);
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
                    model.status = StatusLine::info(format!("skipped: {reason}"));
                    Vec::new()
                }
                // The write may have applied: say so once (the error's own
                // text carries the advice) and reload to show the real state.
                // A failed audit names its log in the error text.
                Err(e) if is_unanswered(&e) => {
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
                    model.status = StatusLine::error("write failed; nothing reloaded");
                    model.overlay = Overlay::Error(format!("write failed: {e}"));
                    Vec::new()
                }
            }
        }
        Msg::Planned(result) => {
            model.writes.in_flight = false;
            let flow_form = model.traffic.rule_form.take();
            // A ruleset `Ctrl-R` asked for keeps its rule only up to its confirm.
            if !matches!(&result, Ok((Plan::Send { .. }, _))) || model.overlay != Overlay::None {
                model.policy.rule_after_ruleset = None;
            }
            match result {
                // Keys typed while the plan was on its way belong to the
                // overlay they went to; a `y` there must not confirm this.
                Ok((Plan::Send { .. }, _)) if model.overlay != Overlay::None => {
                    model.status = StatusLine::warn("request discarded, nothing sent");
                }
                Ok((mut plan @ Plan::Send { .. }, kind)) => {
                    if kind == pce::write::ObjectKind::Revert {
                        name_rulesets(&mut plan, &model.policy.raw);
                    }
                    model.policy.revert_typed.clear();
                    model.status = StatusLine::info(if kind == pce::write::ObjectKind::Revert {
                        "review the revert: type revert and Enter to send it, Esc cancels"
                    } else {
                        "review the request: y sends it, n cancels"
                    });
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
                    model.status = StatusLine::info(reason);
                }
                Err(e) => {
                    // A `w` form comes back with the planner's answer (an
                    // unknown ruleset name lists similar ones), fields as typed.
                    if let Some(mut f) = flow_form
                        && model.overlay == Overlay::None
                    {
                        // An unknown name with a scoped destination offers Ctrl-R.
                        f.offers_ruleset =
                            edit::new_ruleset_offer(&f, &e, &model.limits.scope_keys);
                        f.error = Some(e.clone());
                        model.overlay = Overlay::Form(f);
                    }
                    fail(model, "write", &e, false);
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
fn complete_field(model: &Model, editor: &mut impl Completing, forward: bool) {
    let dives = forward && editor.completion_dives();
    if editor.completing() && !dives {
        editor.cycle(forward);
        return;
    }
    let Some(idx) = model.index() else {
        return;
    };
    let Some((kind, token, earlier)) = editor.completion_request() else {
        return;
    };
    let rulesets = model.flow_prefill().rulesets;
    let workloads = &model.workloads.raw;
    let candidates =
        crate::tui::complete::complete(kind, &token, &earlier, idx, rulesets, workloads);
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
pub fn event_detail_lines(event: &pce::models::Event) -> usize {
    serde_json::to_string_pretty(event)
        .map(|s| s.lines().count())
        .unwrap_or(0)
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
        started, status, ..
    } = &model.traffic.query
    {
        let secs = now.saturating_duration_since(*started).as_secs_f64();
        model.status = StatusLine::info(match &model.traffic.backfill {
            Some(bf) => format!("{} · {secs:.1}s · {status}", bf.status_text()),
            None => format!("query running {secs:.1}s · {status}"),
        });
    }
}

fn on_key(model: &mut Model, k: KeyEvent) -> Vec<Cmd> {
    if k.kind == KeyEventKind::Release {
        return Vec::new();
    }
    if k.code == KeyCode::Char('c') && k.modifiers == KeyModifiers::CONTROL {
        model.quit = true;
        return vec![Cmd::Quit];
    }
    match model.overlay.clone() {
        Overlay::Help => {
            let last = help_last_scroll(model.size.1);
            match k.code {
                KeyCode::Esc | KeyCode::Char('?') | KeyCode::Char('q') => {
                    model.overlay = Overlay::None;
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    model.help_scroll = (model.help_scroll + 1).min(last);
                }
                KeyCode::Up | KeyCode::Char('k') => {
                    model.help_scroll = model.help_scroll.saturating_sub(1);
                }
                KeyCode::End | KeyCode::Char('G') => model.help_scroll = last,
                KeyCode::Home | KeyCode::Char('g') => model.help_scroll = 0,
                _ => {}
            }
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
        Overlay::Confirm { action, .. } => match k.code {
            KeyCode::Char('y') | KeyCode::Char('Y') => {
                model.overlay = Overlay::None;
                match action {
                    PendingAction::RunQuery(f) => update(model, Msg::ApplyFilter(f)),
                    PendingAction::RunNow(f) => run_now(model, f),
                    PendingAction::LoadLabelUsage => load_label_usage(model),
                    PendingAction::RunSlice(f, start, end) => run_slice(model, f, start, end),
                    PendingAction::Backfill(plan) => start_backfill(model, plan, false),
                    PendingAction::RunFocused(f, focus, mode) => {
                        start_focused(model, f, focus, mode)
                    }
                }
            }
            KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
                model.overlay = Overlay::None;
                model.status = StatusLine::info("cancelled");
                // The opt-in asks for a cap past the cost threshold after
                // each truncated query: a no also stops the ticks asking.
                if let PendingAction::Backfill(plan) = action
                    && model.limits.traffic_auto_backfill
                    && plan.cap > pce::traffic::CONFIRM_MAX_RESULTS
                    && std::mem::take(&mut model.traffic.auto.enabled)
                {
                    model.status = StatusLine::info(
                        "cancelled · auto-refresh off, so the backfill is not asked again each tick",
                    );
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
        Overlay::EventDetail { event, scroll } => {
            // The renderer clamps to the last page; here only the top is a hard stop.
            let last = event_detail_lines(&event).saturating_sub(1);
            let next = match k.code {
                KeyCode::Esc | KeyCode::Enter | KeyCode::Char('q') => {
                    model.overlay = Overlay::None;
                    return Vec::new();
                }
                KeyCode::Down | KeyCode::Char('j') => (scroll + 1).min(last),
                KeyCode::Up | KeyCode::Char('k') => scroll.saturating_sub(1),
                KeyCode::PageDown | KeyCode::Char(' ') => (scroll + 10).min(last),
                KeyCode::PageUp => scroll.saturating_sub(10),
                KeyCode::Home | KeyCode::Char('g') => 0,
                KeyCode::End | KeyCode::Char('G') => last,
                _ => scroll,
            };
            model.overlay = Overlay::EventDetail {
                event,
                scroll: next,
            };
            Vec::new()
        }
        Overlay::Form(mut form)
            if k.modifiers.contains(KeyModifiers::CONTROL)
                && matches!(k.code, KeyCode::Char('n' | 'p')) =>
        {
            complete_field(model, &mut form, k.code == KeyCode::Char('n'));
            model.overlay = Overlay::Form(form);
            Vec::new()
        }
        Overlay::Form(mut form) => match form.on_key(k) {
            FormEvent::Continue => {
                model.overlay = Overlay::Form(form);
                Vec::new()
            }
            FormEvent::Cancel => {
                model.overlay = Overlay::None;
                model.status = StatusLine::info("cancelled, nothing sent");
                Vec::new()
            }
            // The ruleset is planned now; the rule waits for its href.
            FormEvent::NewRuleset => match edit::new_ruleset_intent(
                &form,
                model.index().map(|i| i.as_ref()),
                model.profile.org_id,
                &model.limits.scope_keys,
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
                        model.overlay = Overlay::None;
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
                    model.overlay = Overlay::None;
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
                    AppPickerPurpose::Focus => focus_by_name(model, &app),
                }
                Vec::new()
            }
        },
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
        (KeyCode::Char('q'), KeyModifiers::NONE) => {
            model.quit = true;
            vec![Cmd::Quit]
        }
        (KeyCode::Char(':'), _) => {
            model.overlay = Overlay::Prompt {
                input: String::new(),
            };
            Vec::new()
        }
        (KeyCode::Char('?'), _) => {
            model.help_scroll = 0;
            model.overlay = Overlay::Help;
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
        .map(|c| crate::tui::ui::screens::traffic::table_rows(c.body))
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
    if k.code == KeyCode::Esc
        && let Some(bf) = model.traffic.backfill.as_mut()
    {
        bf.cancel = true;
        model.status = StatusLine::info(bf.status_text());
        return Vec::new();
    }
    // With a focus set, Esc clears it first; today's Esc applies after.
    if k.code == KeyCode::Esc && model.traffic.focus.is_some() {
        model.traffic.clear_focus();
        model.status = StatusLine::info("focus cleared");
        return Vec::new();
    }
    // The window, cap and backfill keys work on the unfocused result.
    if matches!(k.code, KeyCode::Char('+' | '-' | ']' | '[' | 'B')) {
        model.traffic.back_to_unfocused();
    }
    match k.code {
        KeyCode::Char('f') => {
            model.overlay = Overlay::FilterEditor(FilterEditor::from_filter(&model.traffic.filter));
            return Vec::new();
        }
        KeyCode::Char('F') => return focus_key(model),
        KeyCode::Char('X') if model.traffic.focus.is_some() => {
            model.traffic.cycle_focus_direction();
            model.status = focus_status(&model.traffic);
        }
        KeyCode::Char('r') => return update(model, Msg::RunNow),
        KeyCode::Char('+') => return widen_window(model),
        KeyCode::Char('B') => return backfill_key(model),
        KeyCode::Char('-') => return narrow_window(model),
        KeyCode::Char(c @ (']' | '[')) => return step_cap(model, c == ']'),
        KeyCode::Char('v') => model.traffic.view = model.traffic.view.next(),
        KeyCode::Char('a')
            if !model.traffic.auto.enabled
                && model
                    .traffic
                    .data
                    .as_ref()
                    .is_some_and(|d| d.backfill.is_some()) =>
        {
            model.status = StatusLine::info(
                "auto-refresh stays off while a backfill is loaded: r runs the window as one query",
            );
        }
        KeyCode::Char('a') if !model.traffic.auto.enabled => {
            model.status = auto_off_status(&model.traffic.filter)
                .unwrap_or_else(|| model.traffic.auto.toggle());
        }
        KeyCode::Char('a') => {
            model.status = model.traffic.auto.toggle();
        }
        KeyCode::Char('w') => return flow_rule_key(model),
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
            t.rebuild_sections();
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
            t.apps_state.decisions = t.apps_state.decisions.next();
            t.rebuild_pairs();
            t.rebuild_sections();
            model.status = StatusLine::info(format!("decision {}", t.apps_state.decisions.label()));
        }
        KeyCode::Char(' ') if model.traffic.section_table().is_some() => {
            model.traffic.toggle_fold();
        }
        KeyCode::Char('z') if model.traffic.section_table().is_some() => {
            model.traffic.toggle_fold_all();
        }
        KeyCode::Char('s') if model.traffic.section_table().is_some() => {
            model.status = StatusLine::info("sections keep their order (by connections)");
        }
        KeyCode::Enter if model.traffic.section_table().is_some() => {
            let t = &mut model.traffic;
            if let Some(drill) = t.section_drill() {
                t.drill = Some(drill);
                t.view = TrafficView::Flows;
                t.rebuild_views();
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
            TrafficView::Flows => model.traffic.show_detail = !model.traffic.show_detail,
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
    let t = &mut model.traffic;
    match t.view {
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
    let n = t.view_flows().len();
    if t.focus_partial() {
        let q = u32::try_from(f.direction.sides().len()).unwrap_or(2);
        StatusLine::warn(format!(
            "{} · {n} flows · the result is truncated: r re-queries {} ({})",
            f.badge(),
            f.app_text(),
            crate::tui::model::queries_text(q)
        ))
    } else {
        StatusLine::info(format!("{} · {n} flows", f.badge()))
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
            "{} is not a label group: it cannot be re-queried by its labels",
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
        let n = u32::try_from(focus.direction.sides().len()).unwrap_or(2);
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

fn start_focused(
    model: &mut Model,
    filter: TrafficFilter,
    focus: Focus,
    mode: CacheMode,
) -> Vec<Cmd> {
    let (seq, cancel) = model.traffic.start_query(Instant::now());
    let n = u32::try_from(focus.direction.sides().len()).unwrap_or(2);
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

/// `+`: the next window step. With the loaded range known for this filter,
/// one request for the older slice only (merged by `on_slice`); otherwise a
/// full query with the wider window. The cost guard judges the merged window.
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
    if let Some(cmds) = widen_backfill(model, cur) {
        return cmds;
    }
    let Some(step) = pce::traffic::next_window(cur) else {
        let top = pce::traffic::WINDOW_STEPS[pce::traffic::WINDOW_STEPS.len() - 1];
        model.status = StatusLine::info(format!("window already at {top}"));
        return Vec::new();
    };
    let mut f = model.traffic.filter.clone();
    f.window = step.to_string();
    let loaded = model
        .traffic
        .data
        .as_ref()
        .filter(|d| d.filter == model.traffic.filter)
        .and_then(|d| d.slices.as_ref());
    let slice = loaded.and_then(|s| {
        let span = jiff::Span::try_from(f.window_duration().ok()?).ok()?;
        let start = s.end.checked_sub(span).ok()?;
        (start < s.start()).then_some((start, s.start()))
    });
    let Some((start, end)) = slice else {
        return request_query(model, f);
    };
    if let Some(reason) = f.needs_confirmation() {
        model.overlay = Overlay::Confirm {
            reason: format!(
                "{reason}. Fetch the slice {} to {}? (y/n)",
                pce::traffic::format_timestamp(start),
                pce::traffic::format_timestamp(end)
            ),
            action: PendingAction::RunSlice(f, start, end),
        };
        return Vec::new();
    }
    run_slice(model, f, start, end)
}

/// `+` on a backfill: one more slice of its length before the loaded
/// start; the window grows to reach it. `None` when the data on screen is
/// not a backfill of this filter.
fn widen_backfill(model: &mut Model, cur: std::time::Duration) -> Option<Vec<Cmd>> {
    let d = model
        .traffic
        .data
        .as_ref()
        .filter(|d| d.filter == model.traffic.filter)?;
    let slice = d.backfill?.slice;
    let s = d.slices.as_ref()?;
    let span = jiff::Span::try_from(slice).ok()?;
    let end = s.start();
    let start = end.checked_sub(span).ok()?;
    let reach = s.end.duration_since(start).unsigned_abs();
    let mut f = model.traffic.filter.clone();
    f.window = pce::traffic::window_text(cur.max(reach));
    if let Some(reason) = f.needs_confirmation() {
        model.overlay = Overlay::Confirm {
            reason: format!(
                "{reason}. Fetch the slice {} to {}? (y/n)",
                pce::traffic::format_timestamp(start),
                pce::traffic::format_timestamp(end)
            ),
            action: PendingAction::RunSlice(f, start, end),
        };
        return Some(Vec::new());
    }
    Some(run_slice(model, f, start, end))
}

/// The status after a truncated result covering under half its window:
/// how much it covers and what `]` and `B` would do. `None` otherwise.
fn low_coverage_status(model: &Model) -> Option<StatusLine> {
    let d = model.traffic.data.as_ref()?;
    let c = d.low_coverage()?;
    let lead = format!(
        "cap {} covers {} of {}: ] raises the cap",
        d.filter.max_results,
        pce::traffic::approx_duration(c.covered),
        d.filter.window
    );
    Some(StatusLine::warn(match d.backfill_plan() {
        Ok(p) => {
            let at = if p.cap > d.filter.max_results {
                format!(" at cap {}", p.cap)
            } else {
                String::new()
            };
            if p.partial() {
                format!(
                    "{lead}, B backfills the newest {} in {} slices{at}",
                    pce::traffic::compact_duration(p.batch_span()),
                    p.batch()
                )
            } else {
                format!("{lead}, B backfills the window in {} slices{at}", p.count)
            }
        }
        Err(_) => lead,
    }))
}

/// The `B` question: what the run fetches, at which cap, in how many
/// queries (each at least a submit, a status poll and a download). A cap
/// past the cost threshold says so first.
fn backfill_prompt(plan: &pce::traffic::BackfillPlan, cap: u32) -> String {
    use pce::traffic::compact_duration as dur;
    let raised = plan.cap > cap;
    let head = if raised {
        format!("raise the cap to {} and backfill", plan.cap)
    } else {
        "backfill".to_string()
    };
    let at = if raised {
        String::new()
    } else {
        format!(" at cap {}", plan.cap)
    };
    let n = plan.batch();
    let queries = format!("{n} queries (≥{} API calls)", 3 * n);
    let expensive = if plan.cap > pce::traffic::CONFIRM_MAX_RESULTS {
        format!(
            "cap {} exceeds {}. ",
            plan.cap,
            pce::traffic::CONFIRM_MAX_RESULTS
        )
    } else {
        String::new()
    };
    if plan.partial() {
        format!(
            "{expensive}{head} the next {n} slices of {}{at}: {} of the remaining {}, {queries}; B again continues. y/n?",
            dur(plan.slice),
            dur(plan.batch_span()),
            dur(plan.span)
        )
    } else {
        format!(
            "{expensive}{head} {} in {} slices of {}{at}: {queries}? y/n",
            dur(plan.span),
            plan.count,
            dur(plan.slice)
        )
    }
}

/// `B`: asks before fetching the window (or the rest of a stopped
/// backfill) in slices; never sends before the `y`.
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
    match d.backfill_plan() {
        Ok(plan) => {
            model.overlay = Overlay::Confirm {
                reason: backfill_prompt(&plan, d.filter.max_results),
                action: PendingAction::Backfill(plan),
            };
        }
        Err(why) => model.status = StatusLine::info(why),
    }
    Vec::new()
}

/// The profile's opt-in after a truncated, low-coverage result: one run
/// (one batch at most) as `B` then `y` would start. A cap past the cost
/// threshold is proposed with the `B` question instead.
fn auto_backfill(model: &mut Model) -> Vec<Cmd> {
    let Some(d) = model.traffic.data.as_ref() else {
        return Vec::new();
    };
    let Ok(plan) = d.backfill_plan() else {
        return Vec::new(); // the status already says why
    };
    if plan.cap > pce::traffic::CONFIRM_MAX_RESULTS {
        model.overlay = Overlay::Confirm {
            reason: backfill_prompt(&plan, d.filter.max_results),
            action: PendingAction::Backfill(plan),
        };
        return Vec::new();
    }
    start_backfill(model, plan, true)
}

/// Starts a run from `backfill_from` back to the window start at the
/// plan's slice and cap, turns auto-refresh off and sends the first slice.
fn start_backfill(model: &mut Model, plan: pce::traffic::BackfillPlan, auto: bool) -> Vec<Cmd> {
    let t = &mut model.traffic;
    let Some(d) = t.data.as_ref() else {
        return Vec::new();
    };
    let (Some(window_start), Some(from)) = (d.window_start(), d.backfill_from()) else {
        return Vec::new();
    };
    let filter = TrafficFilter {
        max_results: plan.cap,
        ..d.filter.clone()
    };
    let replace = d.backfill.is_none();
    let auto_was_on = std::mem::take(&mut t.auto.enabled);
    t.backfill = Some(crate::tui::model::ActiveBackfill {
        run: pce::traffic::BackfillRun::new(
            plan.slice,
            window_start,
            from,
            pce::traffic::MAX_SLICES,
        ),
        filter,
        replace,
        cancel: false,
        auto,
        note: None,
    });
    let cmds = next_backfill_slice(model);
    if auto_was_on {
        model.status = StatusLine::info(format!("{} · auto-refresh off", model.status.text));
    }
    cmds
}

/// Sends the next slice of the active run, or ends it at the window start
/// or the end of its batch.
fn next_backfill_slice(model: &mut Model) -> Vec<Cmd> {
    let Some(mut bf) = model.traffic.backfill.take() else {
        return Vec::new();
    };
    match bf.run.next_slice() {
        pce::traffic::Next::Slice(start, end) => {
            // Only the request carries the run's cap: `f`, `]` and `r` keep
            // acting on the filter of what is loaded until a slice merges.
            let filter = bf.filter.clone();
            let (seq, cancel) = model.traffic.start_query(Instant::now());
            model.status = if bf.note.is_some() {
                StatusLine::warn(bf.status_text())
            } else {
                StatusLine::info(bf.status_text())
            };
            model.traffic.backfill = Some(bf);
            vec![Cmd::RunSlice {
                seq,
                filter,
                start,
                end,
                cancel,
            }]
        }
        pce::traffic::Next::Done => {
            model.status = StatusLine::info(format!("backfill done: {}", loaded_text(model)));
            Vec::new()
        }
        pce::traffic::Next::OverBudget { remaining } => {
            model.status = StatusLine::warn(format!(
                "backfill batch done: {} · {remaining} slices of {} left, B again continues",
                loaded_text(model),
                pce::traffic::compact_duration(bf.run.slice)
            ));
            Vec::new()
        }
    }
}

/// `1234 flows · covers 23h of 24h · 10 slices · truncated`.
fn loaded_text(model: &Model) -> String {
    let Some(d) = model.traffic.data.as_ref() else {
        return String::new();
    };
    match d.coverage_meta() {
        Some(c) => format!("{} flows · {c}", d.flows.len()),
        None => format!("{} flows", d.flows.len()),
    }
}

/// A backfill slice answered. The first one replaces the truncated result
/// (whose rows are an unordered cut of the whole window), later ones
/// merge; then the next is sent. A truncated slice is dropped and its
/// range fetched again in halves; one that truncates at 1m is kept and the
/// run stops. `Esc` stops it here.
fn on_backfill_slice(model: &mut Model, result: Result<QueryOutcome, String>) -> Vec<Cmd> {
    model.traffic.query = QueryState::Idle;
    let Some(mut bf) = model.traffic.backfill.take() else {
        return Vec::new();
    };
    let Some(d) = model.traffic.data.as_mut() else {
        return Vec::new();
    };
    // The filter of what is loaded; the next slice sets the run's again.
    model.traffic.filter = d.filter.clone();
    let o = match result {
        Ok(o) => o,
        Err(e) => {
            let progress = bf.progress();
            fail(model, &format!("backfill stopped at {progress}"), &e, false);
            return Vec::new();
        }
    };
    let cap = o.filter.max_results;
    let progress = bf.progress();
    let step = bf.run.answered(o.truncated);
    if let pce::traffic::Step::Retry { slice } = step {
        let note = format!(
            "slice truncated at {cap}, halved to {}",
            pce::traffic::compact_duration(slice)
        );
        if bf.cancel {
            model.status = StatusLine::warn(format!("backfill stopped: {note}"));
            return Vec::new();
        }
        bf.note = Some(note);
        model.traffic.backfill = Some(bf);
        return next_backfill_slice(model);
    }
    let cut = step == pce::traffic::Step::TooSmall;
    if std::mem::take(&mut bf.replace) {
        d.flows = o.flows;
        d.matches_count = o.matches_count;
        d.elapsed = o.elapsed;
        d.cache_hit = o.hit;
        d.filter = o.filter;
        d.slices = o.bounds.map(|(start, end)| Slices::new(start, end));
    } else {
        merge_outcome(d, o);
    }
    d.backfill = Some(crate::tui::model::Backfilled {
        slice: bf.run.slice,
        cut: cut || d.backfill.is_some_and(|b| b.cut),
    });
    d.settle_backfill_truncated();
    model.traffic.filter = d.filter.clone();
    model.traffic.rebuild_views();
    if cut {
        let hint = if cap < pce::traffic::MAX_TRAFFIC_MAX_RESULTS {
            "] raises the cap"
        } else {
            "narrow the filter"
        };
        model.status =
            StatusLine::error(format!("cap {cap} is too small for this PCE at 1m: {hint}"));
        return Vec::new();
    }
    bf.note = None;
    if bf.cancel {
        model.status = StatusLine::info(format!(
            "backfill stopped after {progress}: {}",
            loaded_text(model)
        ));
        return Vec::new();
    }
    model.traffic.backfill = Some(bf);
    next_backfill_slice(model)
}

/// Folds a slice's flows into the loaded ones; the range grows by it.
fn merge_outcome(d: &mut TrafficData, o: QueryOutcome) {
    pce::traffic::merge_flows(&mut d.flows, o.flows);
    d.truncated |= o.truncated;
    d.matches_count = d.matches_count.zip(o.matches_count).map(|(a, b)| a + b);
    d.elapsed += o.elapsed;
    d.cache_hit &= o.hit;
    d.filter = o.filter;
    if let (Some(s), Some((start, _))) = (d.slices.as_mut(), o.bounds) {
        s.starts.push(start);
    }
}

/// One request for `start..end`; the filter shows the merged window from now on.
fn run_slice(
    model: &mut Model,
    f: TrafficFilter,
    start: jiff::Timestamp,
    end: jiff::Timestamp,
) -> Vec<Cmd> {
    model.traffic.filter = f.clone();
    let (seq, cancel) = model.traffic.start_query(Instant::now());
    model.status = StatusLine::info(format!("query running · slice for window {}", f.window));
    vec![Cmd::RunSlice {
        seq,
        filter: f,
        start,
        end,
        cancel,
    }]
}

/// A slice answer for the current `seq`: its flows join the loaded ones and
/// every view, drill and picker is rebuilt from the merged set. A failure
/// leaves the loaded flows and puts their filter back.
fn on_slice(model: &mut Model, result: Result<QueryOutcome, String>) {
    model.traffic.query = QueryState::Idle;
    let Some(d) = model.traffic.data.as_mut() else {
        return; // nothing to merge into: a reset bumps `seq`, so not reached
    };
    match result {
        Ok(o) => {
            let n = o.flows.len();
            let cap = o.filter.max_results;
            let truncated = o.truncated;
            merge_outcome(d, o);
            if let Some(b) = d.backfill.as_mut() {
                b.cut |= truncated;
            }
            d.settle_backfill_truncated();
            let total = d.flows.len();
            let window = d.filter.window.clone();
            // A tick re-runs the whole merged window as one query; past the
            // cost thresholds that needs a confirm each time, so it stops.
            let auto_off = model
                .traffic
                .auto
                .enabled
                .then(|| auto_off_status(&d.filter))
                .flatten();
            if auto_off.is_some() {
                model.traffic.auto.enabled = false;
            }
            model.traffic.rebuild_views();
            model.status = if let Some(status) = auto_off {
                status
            } else if truncated {
                StatusLine::warn(format!("slice truncated at {cap}: ] raises the cap"))
            } else {
                StatusLine::info(format!(
                    "{n} flows in the slice · {total} loaded · window {window}"
                ))
            };
        }
        Err(e) => {
            model.traffic.filter = d.filter.clone();
            fail(model, "slice", &e, false);
        }
    }
}

/// `-`: the previous window step, no request. Flows last seen before the new
/// start are dropped and the views rebuilt; a slice or query on the way is
/// cancelled (its answer is stale). Narrowed-away flows are not kept: a
/// later `+` fetches them again.
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
        if let Some(s) = d.slices.as_mut()
            && let Some(start) = span.and_then(|span| s.end.checked_sub(span).ok())
            && start >= s.start()
        {
            let before = d.flows.len();
            d.flows.retain(|f| {
                f.last_detected
                    .as_deref()
                    .and_then(|l| l.parse::<jiff::Timestamp>().ok())
                    .is_none_or(|l| l >= start)
            });
            s.narrow(start);
            d.filter.window = step.to_string();
            dropped = Some(before - d.flows.len());
            d.settle_backfill_truncated();
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
            step(&mut model.workloads.table, down, page);
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

/// One reload of the written kind's table, past the cache (the writer
/// already invalidated it).
fn reload_after_write(model: &mut Model, kind: pce::write::ObjectKind) -> Vec<Cmd> {
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
            if model.policy.scope != PolicyScope::Draft {
                return Vec::new();
            }
            model.policy.quiet_reload = true;
            load_rule_sets(model, CacheMode::Bypass)
        }
        // Provisioning changes both scopes and the pending list; a revert
        // changes the draft and the pending list.
        pce::write::ObjectKind::Provision | pce::write::ObjectKind::Revert => {
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
        .map(|c| {
            crate::tui::ui::screens::workloads::table_rows(c.body, model.workloads.show_detail)
        })
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
                let w = &mut model.workloads;
                w.show_detail = !w.show_detail && w.table.selected_item().is_some();
                return Vec::new();
            }
            KeyCode::Char('h') => {
                let order = model.limits.key_order();
                let w = &mut model.workloads;
                w.unhealthy_only = !w.unhealthy_only;
                w.rebuild(&order);
                w.show_detail &= w.table.selected_item().is_some();
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
                let Some(w) = model.workloads.table.selected_item() else {
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
            KeyCode::Char(c @ ('n' | 'e' | 'x')) => return workloads_edit_key(model, c),
            _ => {}
        }
    }
    drive_table(&mut model.workloads.table, k, page);
    Vec::new()
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
                    return load_objects(model, kind, CacheMode::Bypass);
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

/// `u` on the Labels tab: unused labels only, or all again. Usage needs the
/// workloads and the rulesets; when one is neither cached nor on its way
/// the reads wait for a `y` (no request before it).
fn unused_labels_key(model: &mut Model) -> Vec<Cmd> {
    let o = &mut model.objects;
    if o.unused_only {
        o.unused_only = false;
        o.rebuild_labels(&model.limits.key_order());
        model.status = StatusLine::info("labels: all, used or not");
        return Vec::new();
    }
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
    let o = &model.objects;
    model.status = match (&o.label_usage, o.usage_truncated) {
        (None, _) => StatusLine::info("unused labels: waiting for workloads and policy"),
        (Some(_), true) => StatusLine::warn(format!(
            "{} unused labels; the workload list is truncated, some may be in use",
            o.labels.len()
        )),
        (Some(_), false) => StatusLine::info(format!("{} unused labels", o.labels.len())),
    };
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
    match start {
        Start::Form(f) => {
            model.overlay = Overlay::Form(f);
            Vec::new()
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
    let Some(w) = model
        .workloads
        .table
        .selected_item()
        .map(|r| r.workload.clone())
    else {
        model.status = StatusLine::warn("no row selected");
        return Vec::new();
    };
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
        TrafficView::Flows => t.flows.selected_item().map(|r| r.flow.clone()),
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
    let Some(flow) = t.first_flow(|f| decisions.keeps(f.policy_decision) && g.contains(f, &keys))
    else {
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
        Command::Quit => {
            model.quit = true;
            vec![Cmd::Quit]
        }
        Command::Help => {
            model.help_scroll = 0;
            model.overlay = Overlay::Help;
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
        Command::Profile(name) => {
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
        assert_eq!(m.traffic.flows.rows()[0].src_group, "shop/prod");
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
        assert_eq!(m.traffic.flows.rows()[0].src_group, "shop/prod/web");
        assert_eq!(m.traffic.flows.rows()[0].dst_group, "shop/prod/db");
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
        assert_eq!(corp.src_group, "-/-/-");
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
        assert_eq!(m.traffic.flows.rows()[4].src_group, "crm/prod/web/fra1/us");
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
                .any(|r| r.src_group == "fra1" || r.dst_group == "fra1"),
            "grouping by an ignored key still reads its values"
        );
    }

    #[test]
    fn the_flow_detail_hides_ignored_keys() {
        use crate::tui::test_support::{model_with_flows, render_to_string};
        let mut m = model_with_flows();
        m.set_label_model(vec!["app".into()], "role".into(), vec!["env".into()]);
        m.traffic.show_detail = true;
        let screen = render_to_string(&m, 160, 50);
        let line = screen
            .lines()
            .find(|l| l.contains("src labels"))
            .unwrap_or_else(|| panic!("{screen}"));
        assert!(line.contains("app=shop role=web"), "{line}");
        assert!(!line.contains("env="), "{line}");
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
        assert_eq!(m.traffic.flows.rows()[0].src_group, "shop/prod/-");
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
        assert!(update(&mut m, key('w')).is_empty(), "no request");
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
        assert!(update(&mut m, key('w')).is_empty(), "no request");
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
                "src", "dst", "service", "flows", "conns", "bytes", "decision"
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
        assert!(update(m, key('w')).is_empty(), "no request");
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
                "blocked"
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

        // On screen: the compact decision column.
        let titles: Vec<&str> = m.traffic.services.columns.iter().map(|c| c.title).collect();
        assert_eq!(titles.last(), Some(&"decision"));
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
            matches_count: None,
            elapsed: std::time::Duration::from_secs(1),
            hit: false,
            filter: m.traffic.filter.clone(),
            bounds: None,
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
            matches_count: Some(4),
            elapsed: std::time::Duration::from_secs(1),
            hit: false,
            filter: f,
            bounds: None,
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
            matches_count: Some(4),
            elapsed: std::time::Duration::from_secs(1),
            hit: false,
            filter: f,
            bounds: None,
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
                "potentially_blocked"
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
            matches_count: Some(0),
            elapsed: std::time::Duration::from_millis(5),
            hit: false,
            filter: f.clone(),
            bounds: None,
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
            matches_count: Some(900),
            elapsed: std::time::Duration::from_secs(1),
            hit: false,
            filter: f,
            bounds: None,
        };
        update(
            &mut m,
            Msg::QueryDone {
                seq: 1,
                result: Ok(outcome),
            },
        );
        assert_eq!(m.status.kind, StatusKind::Warn);
        assert!(m.status.text.contains("truncated"), "{}", m.status.text);
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
                result: Ok((rs, 0)),
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
                result: Ok((Vec::new(), 0)),
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
                result: Ok((rs, 0)),
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
                result: Ok((rs.clone(), 0)),
            },
        );
        update(&mut m, key('d')); // Active now loading
        // A late draft answer (e.g. a duplicate) must not land in the active scope.
        update(
            &mut m,
            Msg::RuleSetsLoaded {
                scope: PolicyScope::Draft,
                result: Ok((rs, 0)),
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
                    result: Ok((rs, 0)),
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
                    result: Ok((rs, 0)),
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
                result: Ok((rs, 0)),
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
        assert_eq!(
            m.workloads
                .table
                .selected_item()
                .unwrap()
                .workload
                .hostname
                .as_deref(),
            Some("host-2.example"),
            "worst health first"
        );
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
        // The detail scrolls line by line and stops at the top and the last line.
        let total = event_detail_lines(m.events.table.selected_item().unwrap());
        assert!(total > 2, "{total}");
        assert!(update(&mut m, key('j')).is_empty());
        assert!(matches!(m.overlay, Overlay::EventDetail { scroll: 1, .. }));
        assert!(update(&mut m, kc(KeyCode::End)).is_empty());
        assert!(matches!(&m.overlay, Overlay::EventDetail { scroll, .. } if *scroll == total - 1));
        assert!(update(&mut m, key('j')).is_empty());
        assert!(matches!(&m.overlay, Overlay::EventDetail { scroll, .. } if *scroll == total - 1));
        assert!(update(&mut m, key('g')).is_empty());
        assert!(update(&mut m, key('k')).is_empty());
        assert!(matches!(m.overlay, Overlay::EventDetail { scroll: 0, .. }));
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
                    matches_count: Some(4),
                    elapsed: std::time::Duration::from_secs(1),
                    hit: false,
                    filter,
                    bounds: None,
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
        for c in ['n', 'e', 'x'] {
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
        for c in ['e', 'x'] {
            assert!(update(&mut m, key(c)).is_empty(), "{c}");
            assert_eq!(m.status.text, "only unmanaged workloads can be edited here");
            assert_eq!(m.overlay, Overlay::None);
        }
        update(&mut m, key('j'));
        update(&mut m, key('j')); // unmanaged-nas
        assert_eq!(
            update(&mut m, key('x')),
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
                result: Ok((raw, 0)),
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
        use crate::tui::test_support::{model_with_flows, model_with_policy};
        use crate::tui::ui::widgets::overlay::HELP_LINES;
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
        // Every new key is in the help.
        let keys: Vec<&str> = HELP_LINES.iter().map(|(k, _)| *k).collect();
        for k in [
            "Traffic  w",
            "w form   Ctrl-A Ctrl-R",
            "Form     Ctrl-N Ctrl-P",
            "Filter   Ctrl-N Ctrl-P",
            "Policy   N",
            "Pending  j/k Space Enter",
            "Pending  P",
            "Pending  R",
        ] {
            assert!(keys.contains(&k), "{k}");
        }
    }

    #[test]
    fn help_scrolls_to_its_last_line_and_reopens_at_the_top() {
        use crate::tui::ui::widgets::overlay::{HELP_LINES, help_last_scroll};
        let mut m = model_fixture(); // 80x24
        assert_eq!(HELP_LINES.len(), 50);
        assert!(HELP_LINES.iter().any(|(k, _)| *k == "Traffic  + / -"));
        assert!(HELP_LINES.iter().any(|(k, _)| *k == "Traffic  B"));
        assert!(HELP_LINES.iter().any(|(k, _)| *k == "Traffic  ] / ["));
        assert!(HELP_LINES.iter().any(|(k, _)| *k == "Labels   f / i / u"));
        assert!(
            HELP_LINES
                .iter()
                .any(|(k, _)| *k == "Treemap  hjkl Enter b")
        );
        assert!(HELP_LINES.iter().any(|(k, _)| *k == "Mesh     u / p"));
        assert!(
            HELP_LINES
                .iter()
                .any(|(_, d)| *d == ":writes [on|off]  :export[!] <path>")
        );
        assert!(HELP_LINES.iter().any(|(k, _)| *k == "IP lists Enter"));
        assert!(
            HELP_LINES
                .iter()
                .any(|(_, d)| *d == "new/edit/delete: Objects, IP lists, Workloads, Policy")
        );
        update(&mut m, key('?'));
        let last = help_last_scroll(24);
        assert_eq!(last, 28, "50 lines, 22 rows inside the box");
        assert!(HELP_LINES.iter().any(|(k, _)| *k == "App-to-app x / X"));
        assert!(HELP_LINES.iter().any(|(k, _)| *k == "App-to-app o"));
        assert!(HELP_LINES.iter().any(|(k, _)| *k == "App-to-app d"));
        assert!(HELP_LINES.iter().any(|(k, _)| *k == "Sections Space / z"));

        for _ in 0..40 {
            update(&mut m, key('j'));
        }
        assert_eq!(m.help_scroll, last);
        update(&mut m, key('k'));
        assert_eq!(m.help_scroll, last - 1);
        update(&mut m, key('G'));
        assert_eq!(m.help_scroll, last);
        update(&mut m, key('g'));
        assert_eq!(m.help_scroll, 0);
        update(&mut m, key('G'));
        update(&mut m, kc(KeyCode::Esc));
        update(&mut m, key('?'));
        assert_eq!(m.help_scroll, 0, "the help reopens at the top");
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
        for c in ['n', 'e', 'x'] {
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
                result: Ok((sets.clone(), 0)),
            },
        );
        assert_eq!(m.status.text, report, "the reload keeps the write's report");
        update(
            &mut m,
            Msg::RuleSetsLoaded {
                scope: PolicyScope::Draft,
                result: Ok((sets, 0)),
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
                result: Ok((sets.clone(), 0)),
            },
        );
        assert!(!m.policy.quiet_reload, "the ignored answer took the flag");
        update(
            &mut m,
            Msg::RuleSetsLoaded {
                scope: PolicyScope::Active,
                result: Ok((sets, 0)),
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
                result: Ok((sets, 0)),
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
        assert!(update(&mut m, key('w')).is_empty());
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
                assert_eq!(f.value("Ruleset"), "", "Shop-Prod is scoped by app and env");
                assert_eq!(f.fields[0].hint, "scope app=shop");
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
        match &m.overlay {
            Overlay::Form(f) => {
                assert_eq!(f.error.as_deref(), Some(e), "the planner's text only");
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
                assert_eq!(f.error.as_deref(), Some(r#"no ruleset named "New-App""#));
                assert!(f.ruleset_offer_armed().is_some());
            }
            other => panic!("{other:?}"),
        }
        let cmds = update(&mut m, ctrl_key('r'));
        let [Cmd::PlanWrite(WriteIntent::CreateRuleSet(d))] = cmds.as_slice() else {
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
    fn editing_the_ruleset_name_disarms_ctrl_r() {
        use crate::tui::test_support::model_with_flows;
        let mut m = writes_on(model_with_flows());
        unknown_ruleset_form(&mut m);
        update(&mut m, key('X'));
        match &m.overlay {
            Overlay::Form(f) => assert!(f.ruleset_offer_armed().is_none(), "the hint reverts"),
            other => panic!("{other:?}"),
        }
        assert!(update(&mut m, ctrl_key('r')).is_empty(), "nothing planned");
        match &m.overlay {
            Overlay::Form(f) => assert_eq!(
                f.error.as_deref(),
                Some("the ruleset name changed; press Enter to check it first")
            ),
            other => panic!("{other:?}"),
        }
        assert!(m.policy.rule_after_ruleset.is_none());
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
    fn ctrl_r_is_offered_only_for_an_unknown_name_and_a_scoped_destination() {
        use crate::tui::test_support::model_with_flows;
        // Ctrl-R on a fresh form does nothing.
        let mut m = writes_on(model_with_flows());
        update(&mut m, key('j'));
        update(&mut m, key('w'));
        assert!(update(&mut m, ctrl_key('r')).is_empty());
        assert!(matches!(m.overlay, Overlay::Form(_)));
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
            Overlay::Form(f) => assert_eq!(f.error.as_deref(), Some("authentication failed")),
            other => panic!("{other:?}"),
        }
        assert!(update(&mut m, ctrl_key('r')).is_empty());
        assert!(m.policy.rule_after_ruleset.is_none());
        // A destination without scope labels: the plain unknown-name error.
        let mut m = writes_on(model_with_flows());
        m.limits.scope_keys = vec!["site".into()];
        unknown_ruleset_form(&mut m);
        match &m.overlay {
            Overlay::Form(f) => {
                assert_eq!(f.error.as_deref(), Some(r#"no ruleset named "New-App""#))
            }
            other => panic!("{other:?}"),
        }
        assert!(update(&mut m, ctrl_key('r')).is_empty());
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
        let mut m = on_labels();
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
                result: Ok((rule_sets, 0)),
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
        assert_eq!(m.status.text, "2 unused labels");
        assert_eq!(m.objects.labels.len(), 2);
        press(&mut m, KeyCode::Char('u'));
        assert_eq!(m.objects.labels.len(), 6);
        // The other Labels-tab keys stay local too.
        press(&mut m, KeyCode::Char('f'));
        press(&mut m, KeyCode::Esc);
        press(&mut m, KeyCode::Char('i'));
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
        let mut m = all_cached();
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
        assert!(out.contains("· unused ·"), "{out}");
    }

    #[test]
    fn fallback_deny_rules_count_once_loaded() {
        let mut m = all_cached();
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
        let mut m = with_workloads(model_with_objects());
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
        let mut m = all_cached();
        m.objects.unused_only = false;
        m.objects.ages[ObjectKind::LabelGroups.index()] = None;
        m.recount_labels();
        press(&mut m, KeyCode::Char('u'));
        assert!(!m.objects.unused_only);
        assert_eq!(m.status.text, "unused labels need label groups loaded");
        assert_eq!(m.overlay, Overlay::None);

        // No name index: `y` loads nothing, so the filter stays off.
        let mut m = on_labels();
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
    use crate::tui::model::{Drill, Model, Overlay, PendingAction, Slices};
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

    fn slice_cmd(m: &Model, w: &str, start: Timestamp, end: Timestamp) -> Cmd {
        Cmd::RunSlice {
            seq: m.traffic.seq,
            filter: with_window(m, w),
            start,
            end,
            cancel: CancellationToken::new(),
        }
    }

    /// `n` fixture flows re-dated to `last` (an older slice's answer).
    /// Identical to loaded ones, so they fold into them.
    fn same_flows(m: &Model, n: usize, last: &str) -> Vec<ResolvedFlow> {
        m.traffic.loaded_flows()[..n]
            .iter()
            .cloned()
            .map(|mut f| {
                f.first_detected = Some("2026-10-08T00:00:00Z".into());
                f.last_detected = Some(last.into());
                f
            })
            .collect()
    }

    /// Like `same_flows` on ports nothing loaded uses: new rows.
    fn older_flows(m: &Model, n: usize, last: &str) -> Vec<ResolvedFlow> {
        let mut flows = same_flows(m, n, last);
        for (i, f) in flows.iter_mut().enumerate() {
            f.port = Some(60_000 + i as u16);
        }
        flows
    }

    fn slice_done(
        m: &Model,
        flows: Vec<ResolvedFlow>,
        truncated: bool,
        bounds: (Timestamp, Timestamp),
    ) -> Msg {
        Msg::SliceDone {
            seq: m.traffic.seq,
            result: Ok(QueryOutcome {
                flows,
                truncated,
                matches_count: None,
                elapsed: Duration::from_millis(900),
                hit: false,
                filter: m.traffic.filter.clone(),
                bounds: Some(bounds),
            }),
        }
    }

    /// The answer to the slice in flight, asserting it asks for nothing more.
    fn merge(m: &mut Model, flows: Vec<ResolvedFlow>, truncated: bool, b: (Timestamp, Timestamp)) {
        let msg = slice_done(m, flows, truncated, b);
        quiet(m, msg);
    }

    fn slices(m: &Model) -> Slices {
        m.traffic.data.as_ref().unwrap().slices.clone().unwrap()
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
        assert_eq!(cmds, vec![slice_cmd(&m, "2d", ago(48), ago(24))]);
        let older = older_flows(&m, 3, "2026-10-08T12:00:00Z");
        merge(&mut m, older, false, (ago(48), ago(24)));
        assert_eq!(m.traffic.apps_state, st);
        let rows: u64 = m
            .traffic
            .apps
            .rows()
            .iter()
            .filter_map(|r| r.peer().map(|(_, p)| p.flows))
            .sum();
        let kept = m
            .traffic
            .loaded_flows()
            .iter()
            .filter(|f| st.decisions.keeps(f.policy_decision))
            .count() as u64;
        assert_eq!(rows, kept, "rebuilt from the merged flows");
        update(&mut m, key('-'));
        assert_eq!(m.traffic.apps_state, st);
    }

    #[test]
    fn plus_fetches_only_the_older_slice_and_merges_it() {
        let mut m = model_with_window("1d");
        let loaded = m.traffic.loaded_flows().len();
        let groups_before: u64 = m.traffic.groups.rows().iter().map(|g| g.connections).sum();
        let keys = m.traffic.effective_keys();
        let first = m.traffic.loaded_flows()[0].clone();
        m.traffic.drill = Some(Drill::Pair(
            pce::traffic::group_key(&first.src, &keys),
            pce::traffic::group_key(&first.dst, &keys),
            Decisions::All,
        ));
        m.traffic.rebuild_views();
        let drilled = m.traffic.flows.rows().len();
        assert!(drilled > 0);
        let cmds = update(&mut m, key('+'));
        assert_eq!(cmds, vec![slice_cmd(&m, "2d", ago(48), ago(24))]);
        assert_eq!(
            m.traffic.filter.window, "2d",
            "the header shows the merged window"
        );
        assert!(m.traffic.is_running());

        let extra = older_flows(&m, 3, "2026-10-08T12:00:00Z");
        let extra_conns: u64 = extra.iter().map(|f| f.num_connections).sum();
        merge(&mut m, extra, false, (ago(48), ago(24)));
        let d = m.traffic.data.as_ref().unwrap();
        assert_eq!(d.flows.len(), loaded + 3);
        assert_eq!(d.filter.window, "2d");
        assert_eq!(slices(&m).starts, vec![ago(24), ago(48)]);
        assert!(!m.traffic.is_running());
        let groups_after: u64 = m.traffic.groups.rows().iter().map(|g| g.connections).sum();
        assert_eq!(groups_after, groups_before + extra_conns, "views rebuilt");
        assert!(m.traffic.drill.is_some(), "the drill stays");
        assert!(
            m.traffic.flows.rows().len() > drilled,
            "the drilled pair's new flows show"
        );
        assert!(
            m.status.text.contains("3 flows in the slice"),
            "{}",
            m.status.text
        );

        // A second `+` extends again, from the merged start.
        let cmds = update(&mut m, key('+'));
        assert_eq!(cmds, vec![slice_cmd(&m, "3d", ago(72), ago(48))]);
        merge(&mut m, Vec::new(), false, (ago(72), ago(48)));
        assert_eq!(slices(&m).count(), 3);
    }

    #[test]
    fn identical_flows_from_a_slice_fold_into_the_loaded_rows() {
        let mut m = model_with_window("1d");
        let loaded: Vec<ResolvedFlow> = m.traffic.loaded_flows().to_vec();
        update(&mut m, key('+'));
        let same = same_flows(&m, 3, "2026-10-08T12:00:00Z");
        merge(&mut m, same, false, (ago(48), ago(24)));
        let now = m.traffic.loaded_flows();
        assert_eq!(now.len(), loaded.len(), "no new rows");
        for (a, b) in loaded.iter().zip(now).take(3) {
            assert_eq!(b.num_connections, 2 * a.num_connections);
            assert_eq!(b.last_detected, a.last_detected, "the later one stays");
            assert_eq!(b.first_detected.as_deref(), Some("2026-10-08T00:00:00Z"));
        }
        assert_eq!(now[3].num_connections, loaded[3].num_connections);
    }

    #[test]
    fn r_after_widening_confirms_the_whole_merged_window() {
        let mut m = model_with_window("1w");
        quiet(&mut m, key('+'));
        assert_eq!(update(&mut m, key('y')).len(), 1);
        merge(&mut m, Vec::new(), false, (ago(14 * 24), ago(7 * 24)));
        quiet(&mut m, key('r'));
        assert!(
            matches!(&m.overlay, Overlay::Confirm { reason, action: PendingAction::RunNow(f) } if reason.contains("window 2w") && f.window == "2w"),
            "{:?}",
            m.overlay
        );
        quiet(&mut m, key('n'));
        // A cheap merged window runs straight away, as one query.
        let mut m = model_with_window("1d");
        update(&mut m, key('+'));
        merge(&mut m, Vec::new(), false, (ago(48), ago(24)));
        let cmds = update(&mut m, key('r'));
        assert!(
            matches!(&cmds[..], [Cmd::RunQuery { filter, mode: CacheMode::Bypass, .. }] if filter.window == "2d"),
            "{cmds:?}"
        );
    }

    #[test]
    fn widening_past_the_threshold_switches_auto_refresh_off() {
        let mut m = model_with_window("1w");
        m.traffic.auto.enabled = true;
        quiet(&mut m, key('+'));
        assert_eq!(update(&mut m, key('y')).len(), 1);
        merge(&mut m, Vec::new(), false, (ago(14 * 24), ago(7 * 24)));
        assert!(!m.traffic.auto.enabled);
        assert_eq!(
            m.status.text,
            "auto off: window 2w needs a confirm each run"
        );
        // Below the thresholds auto-refresh stays on.
        let mut m = model_with_window("1d");
        m.traffic.auto.enabled = true;
        update(&mut m, key('+'));
        merge(&mut m, Vec::new(), false, (ago(48), ago(24)));
        assert!(m.traffic.auto.enabled);
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
    fn header_counts_the_slices() {
        let mut m = model_with_window("1d");
        update(&mut m, key('+'));
        merge(&mut m, Vec::new(), false, (ago(48), ago(24)));
        let out = crate::tui::test_support::render_to_string(&m, 80, 24);
        assert!(out.contains("window 2d · cap 1000 · 2 slices"), "{out}");
    }

    #[test]
    fn a_truncated_slice_marks_the_result_and_says_so() {
        let mut m = model_with_window("1d");
        update(&mut m, key('+'));
        let extra = older_flows(&m, 2, "2026-10-08T12:00:00Z");
        merge(&mut m, extra, true, (ago(48), ago(24)));
        assert!(m.traffic.data.as_ref().unwrap().truncated);
        assert_eq!(m.status.text, "slice truncated at 1000: ] raises the cap");
    }

    #[test]
    fn a_failed_slice_keeps_the_flows_and_their_window() {
        let mut m = model_with_window("1d");
        let loaded = m.traffic.loaded_flows().len();
        update(&mut m, key('+'));
        let seq = m.traffic.seq;
        quiet(
            &mut m,
            Msg::SliceDone {
                seq,
                result: Err("boom".into()),
            },
        );
        assert_eq!(m.traffic.loaded_flows().len(), loaded);
        assert_eq!(m.traffic.filter.window, "1d");
        assert_eq!(slices(&m).count(), 1);
        assert!(m.status.text.contains("boom"));
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
        assert_eq!(slices(&m).starts, vec![ago(24)]);
        assert_eq!(m.status.text, "window 1d · 3 older flows dropped");
    }

    #[test]
    fn minus_after_plus_drops_the_slice_and_plus_queries_it_again() {
        let mut m = model_with_window("1d");
        let loaded = m.traffic.loaded_flows().len();
        update(&mut m, key('+'));
        let extra = older_flows(&m, 4, "2026-10-08T12:00:00Z");
        merge(&mut m, extra, false, (ago(48), ago(24)));
        quiet(&mut m, key('-'));
        assert_eq!(m.traffic.loaded_flows().len(), loaded);
        assert_eq!(slices(&m).starts, vec![ago(24)]);
        // Nothing is cached across narrowing.
        assert_eq!(
            update(&mut m, key('+')),
            vec![slice_cmd(&m, "2d", ago(48), ago(24))]
        );
    }

    #[test]
    fn narrowing_inside_a_slice_cuts_it() {
        let mut s = Slices::new(ago(24), end());
        s.starts.push(ago(48));
        s.starts.push(ago(72));
        s.narrow(ago(60));
        assert_eq!(s.starts, vec![ago(24), ago(48), ago(60)]);
        s.narrow(ago(36));
        assert_eq!(s.starts, vec![ago(24), ago(36)]);
        s.narrow(ago(40));
        assert_eq!(s.starts, vec![ago(24), ago(36)], "wider is a no-op");
        s.narrow(ago(6));
        assert_eq!(s.starts, vec![ago(6)]);
    }

    #[test]
    fn a_late_slice_after_f_window_or_minus_is_ignored() {
        let mut m = model_with_window("1d");
        let loaded = m.traffic.loaded_flows().len();
        update(&mut m, key('+'));
        let late = slice_done(
            &m,
            older_flows(&m, 3, "2026-10-08T12:00:00Z"),
            false,
            (ago(48), ago(24)),
        );
        // `:window` (the editor's Enter takes the same path) starts a new query.
        let f = with_window(&m, "6h");
        assert_eq!(update(&mut m, Msg::ApplyFilter(f)).len(), 1);
        quiet(&mut m, late);
        assert_eq!(m.traffic.loaded_flows().len(), loaded);
        assert_eq!(slices(&m).count(), 1);

        let mut m = model_with_window("1d");
        update(&mut m, key('+'));
        let late = slice_done(
            &m,
            older_flows(&m, 3, "2026-10-08T12:00:00Z"),
            false,
            (ago(48), ago(24)),
        );
        quiet(&mut m, key('-'));
        assert!(!m.traffic.is_running(), "the slice was cancelled");
        assert_eq!(m.traffic.filter.window, "1d");
        quiet(&mut m, late);
        assert_eq!(m.traffic.loaded_flows().len(), loaded);
        assert_eq!(m.traffic.data.as_ref().unwrap().filter.window, "1d");
    }

    #[test]
    fn plus_past_the_threshold_confirms_naming_the_slice() {
        let mut m = model_with_window("1w");
        quiet(&mut m, key('+'));
        match &m.overlay {
            Overlay::Confirm {
                reason,
                action: PendingAction::RunSlice(f, start, e),
            } => {
                assert!(reason.contains("window 2w exceeds 7d"), "{reason}");
                assert!(
                    reason.contains("2026-09-26T08:00:00Z to 2026-10-03T08:00:00Z"),
                    "{reason}"
                );
                assert_eq!(
                    (f.window.as_str(), *start, *e),
                    ("2w", ago(14 * 24), ago(7 * 24))
                );
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
        assert_eq!(cmds, vec![slice_cmd(&m, "2w", ago(14 * 24), ago(7 * 24))]);
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
        assert_eq!(m.traffic.apps.sort(), None, "sections keep their order");
        assert_eq!(m.status.text, "sections keep their order (by connections)");
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
        let st = m.traffic.apps_state.clone();
        assert!(!st.folded.is_empty());
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
                    matches_count: None,
                    elapsed: std::time::Duration::from_secs(1),
                    hit: false,
                    filter,
                    bounds: None,
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
        quiet(&mut m, key('w'));
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
    use crate::tui::model::{Backfilled, Model, Overlay, PendingAction, StatusKind};
    use crate::tui::msg::QueryOutcome;
    use crate::tui::test_support::{FIXTURE_END, fixture_flows_rich, model_fixture};
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use jiff::{Span, Timestamp};
    use pce::traffic::{BackfillPlan, ResolvedFlow};
    use std::time::Duration;
    use tokio_util::sync::CancellationToken;

    fn key(c: char) -> Msg {
        Msg::Key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE))
    }

    fn esc() -> Msg {
        Msg::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
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

    fn mins_ago(m: i64) -> Timestamp {
        end().checked_sub(Span::new().minutes(m)).unwrap()
    }

    fn at(t: Timestamp) -> String {
        pce::traffic::format_timestamp(t)
    }

    fn h(n: u64) -> Duration {
        Duration::from_secs(n * 3600)
    }

    /// The rich fixture `copies` times, re-dated: the first flow at
    /// `FIXTURE_END`, the last `span` before it, the rest a minute before
    /// the end.
    fn spread(span: Span, copies: usize) -> Vec<ResolvedFlow> {
        let mut flows: Vec<ResolvedFlow> = (0..copies).flat_map(|_| fixture_flows_rich()).collect();
        let n = flows.len();
        let oldest = end().checked_sub(span).unwrap();
        for (i, f) in flows.iter_mut().enumerate() {
            let t = match i {
                0 => end(),
                i if i == n - 1 => oldest,
                _ => end()
                    .checked_sub(Span::new().minutes(1))
                    .unwrap()
                    .max(oldest),
            };
            f.last_detected = Some(at(t));
        }
        flows
    }

    /// The `1d` query answers `flows`, truncated at `per_flow × flows`.
    fn answer_query(m: &mut Model, flows: Vec<ResolvedFlow>, per_flow: u32) -> Vec<Cmd> {
        let f = TrafficFilter::defaults("1d", per_flow * flows.len() as u32);
        if !m.traffic.is_running() {
            let cmds = update(m, Msg::ApplyFilter(f.clone()));
            assert_eq!(cmds.len(), 1, "{cmds:?}");
        }
        update(
            m,
            Msg::QueryDone {
                seq: m.traffic.seq,
                result: Ok(QueryOutcome {
                    flows,
                    truncated: true,
                    matches_count: None,
                    elapsed: Duration::from_millis(900),
                    hit: false,
                    filter: f,
                    bounds: Some((ago(24), end())),
                }),
            },
        )
    }

    fn truncated(m: &mut Model, span: Span, per_flow: u32) -> Vec<Cmd> {
        answer_query(m, spread(span, 1), per_flow)
    }

    /// Two hours covered at cap 4 × 30 = 120: 6h slices fill the day in 4.
    fn two_hours() -> Model {
        let mut m = model_fixture();
        let cmds = truncated(&mut m, Span::new().hours(2), 4);
        assert_eq!(cmds, Vec::<Cmd>::new(), "nothing without y or the opt-in");
        assert_eq!(cap(&m), 120);
        m
    }

    fn cap(m: &Model) -> u32 {
        m.traffic.filter.max_results
    }

    fn slice_cmd(m: &Model, start: Timestamp, end: Timestamp) -> Cmd {
        let filter = m
            .traffic
            .backfill
            .as_ref()
            .map_or_else(|| m.traffic.filter.clone(), |b| b.filter.clone());
        Cmd::RunSlice {
            seq: m.traffic.seq,
            filter,
            start,
            end,
            cancel: CancellationToken::new(),
        }
    }

    /// `n` rows on ports nothing else uses (`port..`), last seen at `last`.
    fn rows(n: usize, last: Timestamp, port: u16) -> Vec<ResolvedFlow> {
        fixture_flows_rich()
            .into_iter()
            .take(n)
            .enumerate()
            .map(|(i, mut f)| {
                f.port = Some(port + i as u16);
                f.last_detected = Some(at(last));
                f
            })
            .collect()
    }

    fn answer(
        m: &mut Model,
        flows: Vec<ResolvedFlow>,
        truncated: bool,
        b: (Timestamp, Timestamp),
    ) -> Vec<Cmd> {
        // The runtime answers with the filter the slice was sent with.
        let filter = m
            .traffic
            .backfill
            .as_ref()
            .map_or_else(|| m.traffic.filter.clone(), |bf| bf.filter.clone());
        let msg = Msg::SliceDone {
            seq: m.traffic.seq,
            result: Ok(QueryOutcome {
                flows,
                truncated,
                matches_count: None,
                elapsed: Duration::from_millis(500),
                hit: false,
                filter,
                bounds: Some(b),
            }),
        };
        update(m, msg)
    }

    fn progress(m: &Model) -> String {
        m.traffic
            .backfill
            .as_ref()
            .map(|b| b.progress())
            .unwrap_or_default()
    }

    fn meta(m: &Model) -> Option<String> {
        m.traffic.data.as_ref().unwrap().coverage_meta()
    }

    fn plan_of(m: &Model) -> BackfillPlan {
        match &m.overlay {
            Overlay::Confirm {
                action: PendingAction::Backfill(p),
                ..
            } => *p,
            o => panic!("no backfill question: {o:?}"),
        }
    }

    fn reason(m: &Model) -> String {
        match &m.overlay {
            Overlay::Confirm { reason, .. } => reason.clone(),
            o => panic!("no question: {o:?}"),
        }
    }

    #[test]
    fn a_truncated_low_coverage_result_says_how_much_it_covers() {
        let m = two_hours();
        assert_eq!(m.status.kind, StatusKind::Warn);
        assert_eq!(
            m.status.text,
            "cap 120 covers ~2 hours of 1d: ] raises the cap, B backfills the window in 4 slices"
        );
        assert_eq!(meta(&m).as_deref(), Some("covers 2h of 1d · truncated"));
        assert!(m.traffic.backfill.is_none());
    }

    #[test]
    fn half_covered_complete_or_unmeasurable_results() {
        let mut m = model_fixture();
        truncated(&mut m, Span::new().hours(13), 4);
        assert!(m.status.text.contains("truncated"), "{}", m.status.text);
        assert_eq!(meta(&m), None);
        quiet(&mut m, key('B'));
        assert_eq!(
            plan_of(&m),
            BackfillPlan::with_slice(h(24), h(24), 120),
            "B still offers the window"
        );
        let mut m = model_fixture();
        quiet(&mut m, key('B'));
        assert_eq!(m.status.text, "nothing to backfill: r runs the query first");
        let mut m = crate::tui::test_support::model_with_window("1d");
        quiet(&mut m, key('B'));
        assert_eq!(m.status.text, "nothing to backfill: the result is complete");
        // Truncated without a single last-seen time.
        let mut m = model_fixture();
        let mut flows = spread(Span::new().hours(2), 1);
        for f in &mut flows {
            f.last_detected = None;
        }
        quiet_query(&mut m, flows, 4);
        quiet(&mut m, key('B'));
        assert_eq!(m.status.text, "cannot measure coverage: no last-seen times");
        assert_eq!(m.overlay, Overlay::None);
        // Or with an unknown time range.
        let mut m = two_hours();
        m.traffic.data.as_mut().unwrap().slices = None;
        quiet(&mut m, key('B'));
        assert_eq!(m.status.text, "cannot measure coverage: no last-seen times");
    }

    fn quiet_query(m: &mut Model, flows: Vec<ResolvedFlow>, per_flow: u32) {
        assert_eq!(answer_query(m, flows, per_flow), Vec::<Cmd>::new());
    }

    #[test]
    fn b_asks_first_and_n_or_esc_send_nothing() {
        let mut m = two_hours();
        for no in [key('n'), esc()] {
            quiet(&mut m, key('B'));
            assert_eq!(
                reason(&m),
                "backfill 1d in 4 slices of 6h at cap 120: 4 queries (≥12 API calls)? y/n"
            );
            assert_eq!(plan_of(&m), BackfillPlan::with_slice(h(24), h(6), 120));
            quiet(&mut m, no);
            assert_eq!(m.overlay, Overlay::None);
            assert!(m.traffic.backfill.is_none());
            assert!(!m.traffic.is_running());
        }
        quiet(&mut m, key('B'));
        quiet(
            &mut m,
            Msg::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
        );
        assert!(m.traffic.backfill.is_none(), "Enter is not a yes");
    }

    #[test]
    fn y_walks_the_whole_window_newest_first_replacing_the_truncated_rows() {
        let mut m = two_hours();
        m.traffic.auto.enabled = true;
        quiet(&mut m, key('B'));
        let cmds = update(&mut m, key('y'));
        assert_eq!(cmds, vec![slice_cmd(&m, ago(6), end())]);
        assert_eq!(progress(&m), "slice 1 of 4");
        assert!(
            m.status.text.starts_with("backfill: slice 1 of 4"),
            "{}",
            m.status.text
        );
        assert!(
            m.status.text.ends_with("auto-refresh off"),
            "{}",
            m.status.text
        );
        assert!(!m.traffic.auto.enabled, "a backfill keeps auto-refresh off");
        // Keys that would send while a slice is in flight do not.
        quiet(&mut m, key('B'));
        quiet(&mut m, key('r'));
        quiet(&mut m, key('+'));
        // The first slice replaces the 30 truncated rows.
        let cmds = answer(&mut m, rows(2, ago(1), 50_000), false, (ago(6), end()));
        assert_eq!(cmds, vec![slice_cmd(&m, ago(12), ago(6))]);
        assert_eq!(m.traffic.loaded_flows().len(), 2);
        let d = m.traffic.data.as_ref().unwrap();
        assert!(
            d.truncated,
            "truncated until the walk reaches the window start"
        );
        assert_eq!(
            meta(&m).as_deref(),
            Some("covers 6h of 1d · 1 slice · truncated")
        );
        let next = [
            (ago(18), ago(12), "slice 3 of 4"),
            (ago(24), ago(18), "slice 4 of 4"),
        ];
        let mut range = (ago(12), ago(6));
        for (i, (s, e, p)) in next.into_iter().enumerate() {
            let cmds = answer(&mut m, rows(1, range.0, 51_000 + i as u16), false, range);
            assert_eq!(cmds, vec![slice_cmd(&m, s, e)], "{i}");
            assert_eq!(progress(&m), p);
            range = (s, e);
        }
        let cmds = answer(&mut m, rows(1, range.0, 52_000), false, range);
        assert_eq!(cmds, Vec::<Cmd>::new());
        assert!(m.traffic.backfill.is_none());
        let d = m.traffic.data.as_ref().unwrap();
        assert_eq!(d.flows.len(), 5, "the slices' rows only");
        assert_eq!(
            d.backfill,
            Some(Backfilled {
                slice: h(6),
                cut: false
            })
        );
        assert!(!d.truncated);
        assert_eq!(meta(&m).as_deref(), Some("1d · 4 slices"));
        assert_eq!(m.status.text, "backfill done: 5 flows · 1d · 4 slices");
        assert_eq!(m.traffic.filter.window, "1d", "the window is unchanged");
        quiet(&mut m, key('B'));
        assert_eq!(
            m.status.text,
            "nothing to backfill: the backfill reaches the window start"
        );
    }

    #[test]
    fn esc_stops_after_the_running_slice_and_b_continues() {
        let mut m = two_hours();
        quiet(&mut m, key('B'));
        update(&mut m, key('y'));
        quiet(&mut m, esc());
        assert!(
            m.status.text.ends_with("stops after this slice"),
            "{}",
            m.status.text
        );
        assert!(m.traffic.is_running(), "the running slice is not cancelled");
        let cmds = answer(&mut m, rows(2, ago(5), 50_000), false, (ago(6), end()));
        assert_eq!(cmds, Vec::<Cmd>::new());
        assert!(m.traffic.backfill.is_none());
        assert_eq!(
            meta(&m).as_deref(),
            Some("covers 6h of 1d · 1 slice · truncated")
        );
        assert!(
            m.status
                .text
                .starts_with("backfill stopped after slice 1 of 4: "),
            "{}",
            m.status.text
        );
        quiet(&mut m, key('B'));
        assert_eq!(
            reason(&m),
            "backfill 18h in 3 slices of 6h at cap 120: 3 queries (≥9 API calls)? y/n"
        );
        let cmds = update(&mut m, key('y'));
        assert_eq!(cmds, vec![slice_cmd(&m, ago(12), ago(6))]);
        assert_eq!(
            answer(&mut m, rows(1, ago(7), 51_000), false, (ago(12), ago(6))).len(),
            1
        );
        assert_eq!(m.traffic.loaded_flows().len(), 3, "a continued run merges");
    }

    #[test]
    fn a_truncated_slice_is_fetched_again_in_halves() {
        let mut m = two_hours();
        quiet(&mut m, key('B'));
        update(&mut m, key('y'));
        let cmds = answer(&mut m, rows(3, ago(3), 50_000), true, (ago(6), end()));
        assert_eq!(cmds, vec![slice_cmd(&m, ago(3), end())]);
        assert_eq!(m.status.kind, StatusKind::Warn);
        assert!(
            m.status
                .text
                .contains("slice truncated at 120, halved to 3h"),
            "{}",
            m.status.text
        );
        assert_eq!(progress(&m), "slice 1 of 8");
        assert_eq!(
            m.traffic.loaded_flows().len(),
            30,
            "the truncated slice is dropped"
        );
        let cmds = answer(&mut m, rows(1, ago(1), 50_000), false, (ago(3), end()));
        assert_eq!(cmds, vec![slice_cmd(&m, ago(6), ago(3))]);
        assert_eq!(progress(&m), "slice 2 of 8");
        assert_eq!(m.traffic.loaded_flows().len(), 1, "it replaced the result");
        assert_eq!(
            m.traffic.data.as_ref().unwrap().backfill.unwrap().slice,
            h(3)
        );
    }

    #[test]
    fn only_a_1m_slice_that_truncates_stops_the_backfill() {
        let mut m = two_hours();
        quiet(&mut m, key('B'));
        update(&mut m, key('y'));
        // 1m30s halves to the 1m floor, not to 45s.
        m.traffic.backfill.as_mut().unwrap().run.slice = Duration::from_secs(90);
        let cmds = answer(&mut m, rows(2, ago(3), 50_000), true, (ago(6), end()));
        assert_eq!(cmds, vec![slice_cmd(&m, mins_ago(1), end())]);
        let cmds = answer(
            &mut m,
            rows(2, mins_ago(0), 50_000),
            true,
            (mins_ago(1), end()),
        );
        assert_eq!(cmds, Vec::<Cmd>::new());
        assert_eq!(m.status.kind, StatusKind::Error);
        assert_eq!(
            m.status.text,
            "cap 120 is too small for this PCE at 1m: ] raises the cap"
        );
        assert!(m.traffic.backfill.is_none());
        let d = m.traffic.data.as_ref().unwrap();
        assert!(d.truncated);
        assert!(d.backfill.unwrap().cut);
        assert!(meta(&m).unwrap().ends_with(" · truncated"));
    }

    #[test]
    fn a_failed_slice_stops_and_keeps_what_is_loaded() {
        let mut m = two_hours();
        let filter = m.traffic.filter.clone();
        quiet(&mut m, key('B'));
        update(&mut m, key('y'));
        let seq = m.traffic.seq;
        quiet(
            &mut m,
            Msg::SliceDone {
                seq,
                result: Err("timeout".into()),
            },
        );
        assert!(m.traffic.backfill.is_none());
        assert_eq!(m.status.text, "backfill stopped at slice 1 of 4: timeout");
        assert_eq!(m.traffic.loaded_flows().len(), 30);
        assert_eq!(m.traffic.filter, filter);
    }

    /// 30 flows in 2 minutes at cap 30: 1440 slices of 1m; 48 of 30m at 1000.
    #[test]
    fn b_raises_the_cap_to_the_smallest_rung_that_fits() {
        let mut m = model_fixture();
        quiet_query(&mut m, spread(Span::new().minutes(2), 1), 1);
        assert!(
            m.status
                .text
                .ends_with(", B backfills the window in 48 slices at cap 1000"),
            "{}",
            m.status.text
        );
        quiet(&mut m, key('B'));
        assert_eq!(
            reason(&m),
            "raise the cap to 1000 and backfill 1d in 48 slices of 30m: 48 queries (≥144 API calls)? y/n"
        );
        let cmds = update(&mut m, key('y'));
        let mut f = m.traffic.data.as_ref().unwrap().filter.clone();
        f.max_results = 1000;
        assert_eq!(
            cmds,
            vec![Cmd::RunSlice {
                seq: m.traffic.seq,
                filter: f.clone(),
                start: mins_ago(30),
                end: end(),
                cancel: CancellationToken::new(),
            }]
        );
        answer(
            &mut m,
            rows(1, mins_ago(5), 50_000),
            false,
            (mins_ago(30), end()),
        );
        assert_eq!(m.traffic.data.as_ref().unwrap().filter.max_results, 1000);
    }

    /// 6000 flows in under a minute: even the hard maximum of 200000 needs
    /// 96 slices of 15m, so the newest 50, then `B` continues; the opt-in
    /// asks (past the cost threshold) and never chains a second batch.
    #[test]
    fn past_the_hard_maximum_a_batch_of_50_and_b_continues() {
        for auto in [false, true] {
            let mut m = model_fixture();
            m.limits.traffic_auto_backfill = auto;
            let cmds = answer_query(&mut m, spread(Span::new().seconds(30), 200), 1);
            assert!(cmds.is_empty());
            if !auto {
                assert!(
                    m.status
                        .text
                        .ends_with(", B backfills the newest 12h 30m in 50 slices at cap 200000"),
                    "{}",
                    m.status.text
                );
                quiet(&mut m, key('B'));
            }
            assert_eq!(
                reason(&m),
                "cap 200000 exceeds 10000. raise the cap to 200000 and backfill the next 50 \
                 slices of 15m: 12h 30m of the remaining 1d, 50 queries (≥150 API calls); B \
                 again continues. y/n?"
            );
            assert_eq!(update(&mut m, key('y')).len(), 1);
            assert_eq!(progress(&m), "slice 1 of 50");
            let mut sent = 1;
            loop {
                let Some(Cmd::RunSlice { start, end, .. }) = slice_in_flight(&m) else {
                    unreachable!()
                };
                let cmds = answer(&mut m, rows(1, start, 50_000 + sent), false, (start, end));
                if cmds.is_empty() {
                    break;
                }
                assert_eq!(cmds.len(), 1);
                sent += 1;
            }
            assert_eq!(sent, 50, "one batch, never chained");
            assert_eq!(m.status.kind, StatusKind::Warn);
            assert!(
                m.status
                    .text
                    .ends_with("· 46 slices of 15m left, B again continues"),
                "{}",
                m.status.text
            );
            assert_eq!(
                meta(&m).as_deref(),
                Some("covers 12h 30m of 1d · 50 slices · truncated")
            );
            quiet(&mut m, key('B'));
            assert_eq!(
                reason(&m),
                "cap 200000 exceeds 10000. backfill 11h 30m in 46 slices of 15m at cap 200000: 46 \
                 queries (≥138 API calls)? y/n"
            );
        }
    }

    /// The range of the slice in flight, rebuilt from the run.
    fn slice_in_flight(m: &Model) -> Option<Cmd> {
        let b = m.traffic.backfill.as_ref()?;
        let end = b.run.next_end;
        let start = end
            .checked_sub(Span::new().seconds(b.run.slice.as_secs() as i64))
            .unwrap()
            .max(b.run.window_start);
        Some(slice_cmd(m, start, end))
    }

    /// 3000 flows in under a minute: only a cap of 200000 fits the day; the
    /// opt-in asks instead of passing the cost threshold on its own.
    #[test]
    fn a_cap_past_the_cost_threshold_is_always_asked() {
        let mut m = model_fixture();
        m.limits.traffic_max_results = 200_000;
        m.limits.traffic_auto_backfill = true;
        let cmds = answer_query(&mut m, spread(Span::new().seconds(30), 100), 1);
        assert_eq!(cmds, Vec::<Cmd>::new());
        assert_eq!(
            reason(&m),
            "cap 200000 exceeds 10000. raise the cap to 200000 and backfill 1d in 48 slices \
             of 30m: 48 queries (≥144 API calls)? y/n"
        );
        assert!(m.traffic.backfill.is_none());
        let cmds = update(&mut m, key('y'));
        assert!(
            matches!(cmds.as_slice(), [Cmd::RunSlice { filter, .. }] if filter.max_results == 200_000),
            "{cmds:?}"
        );
    }

    /// The raised cap is only in the slice requests: until a slice merges,
    /// `f`, `]` and `r` act on the cap of what is loaded.
    #[test]
    fn a_raised_cap_is_not_the_filter_until_a_slice_merges() {
        let mut m = model_fixture();
        quiet_query(&mut m, spread(Span::new().minutes(2), 1), 1);
        quiet(&mut m, key('B'));
        let cmds = update(&mut m, key('y'));
        assert!(
            matches!(cmds.as_slice(), [Cmd::RunSlice { filter, .. }] if filter.max_results == 1000),
            "{cmds:?}"
        );
        assert_eq!(cap(&m), 30, "the loaded cap");
        quiet(&mut m, key('r'));
        assert_eq!(m.status.text, "query already running");
        quiet(&mut m, key('f'));
        assert!(
            matches!(&m.overlay, Overlay::FilterEditor(e) if e.values[5] == "30"),
            "{:?}",
            m.overlay
        );
        quiet(&mut m, esc());
        answer(
            &mut m,
            rows(1, mins_ago(5), 50_000),
            false,
            (mins_ago(30), end()),
        );
        assert_eq!(cap(&m), 1000, "merged: the filter is the run's");
    }

    /// A backfill stopped after one slice at a cap past 10000: `r` would run
    /// the whole window as one query, so it asks first.
    #[test]
    fn r_after_a_backfill_goes_through_the_cost_guard() {
        let mut m = model_fixture();
        m.limits.traffic_max_results = 200_000;
        quiet_query(&mut m, spread(Span::new().seconds(30), 100), 1);
        quiet(&mut m, key('B'));
        assert_eq!(plan_of(&m).cap, 200_000);
        let cmds = update(&mut m, key('y'));
        assert_eq!(cmds.len(), 1);
        quiet(&mut m, esc());
        let Some(Cmd::RunSlice { start, end, .. }) = slice_in_flight(&m) else {
            unreachable!()
        };
        quiet_answer(&mut m, (start, end));
        assert_eq!(
            m.traffic
                .data
                .as_ref()
                .unwrap()
                .slices
                .as_ref()
                .unwrap()
                .count(),
            1
        );
        assert_eq!(cap(&m), 200_000);
        quiet(&mut m, key('r'));
        assert!(
            matches!(&m.overlay, Overlay::Confirm { action: PendingAction::RunNow(_), reason } if reason.contains("200000")),
            "{:?}",
            m.overlay
        );
    }

    /// `n` to the opt-in's cap question turns auto-refresh off, so the
    /// next tick does not ask again.
    #[test]
    fn no_to_the_opt_in_cap_question_turns_auto_refresh_off() {
        let mut m = model_fixture();
        m.limits.traffic_max_results = 200_000;
        m.limits.traffic_auto_backfill = true;
        m.traffic.auto.enabled = true;
        quiet_query(&mut m, spread(Span::new().seconds(30), 100), 1);
        assert_eq!(plan_of(&m).cap, 200_000);
        quiet(&mut m, key('n'));
        assert!(!m.traffic.auto.enabled);
        assert_eq!(
            m.status.text,
            "cancelled · auto-refresh off, so the backfill is not asked again each tick"
        );
        quiet(
            &mut m,
            Msg::Tick(std::time::Instant::now() + Duration::from_secs(3600)),
        );
        // A plain B question declined leaves auto-refresh alone.
        let mut m = two_hours();
        m.traffic.auto.enabled = true;
        quiet(&mut m, key('B'));
        quiet(&mut m, key('n'));
        assert!(m.traffic.auto.enabled);
        assert_eq!(m.status.text, "cancelled");
    }

    #[test]
    fn the_opt_in_starts_without_y_and_esc_still_stops_it() {
        let mut m = model_fixture();
        m.limits.traffic_auto_backfill = true;
        let cmds = truncated(&mut m, Span::new().hours(2), 4);
        assert_eq!(cmds, vec![slice_cmd(&m, ago(6), end())]);
        assert_eq!(m.overlay, Overlay::None);
        assert!(
            m.status
                .text
                .starts_with("auto backfill: slice 1 of 4 (profile traffic_auto_backfill)"),
            "{}",
            m.status.text
        );
        quiet(&mut m, esc());
        let cmds = answer(&mut m, rows(1, ago(3), 50_000), false, (ago(6), end()));
        assert_eq!(cmds, Vec::<Cmd>::new());
        assert!(m.traffic.backfill.is_none());
    }

    /// Auto-refresh on with the opt-in: one tick query starts one run, auto
    /// turns off, and later ticks send nothing.
    #[test]
    fn a_tick_query_starts_one_run_and_auto_turns_off() {
        let mut m = two_hours();
        m.limits.traffic_auto_backfill = true;
        m.traffic.auto.enabled = true;
        let later = std::time::Instant::now() + Duration::from_secs(3600);
        let cmds = update(&mut m, Msg::Tick(later));
        assert!(
            matches!(cmds.as_slice(), [Cmd::RunQuery { .. }]),
            "{cmds:?}"
        );
        let cmds = truncated(&mut m, Span::new().hours(2), 4);
        assert_eq!(cmds, vec![slice_cmd(&m, ago(6), end())]);
        assert!(!m.traffic.auto.enabled);
        quiet(&mut m, Msg::Tick(later + Duration::from_secs(3600)));
        let mut range = (ago(6), end());
        for s in [ago(12), ago(18), ago(24)] {
            assert_eq!(
                answer(&mut m, rows(1, range.0, 50_000), false, range).len(),
                1
            );
            range = (s, range.0);
        }
        quiet(&mut m, Msg::Tick(later + Duration::from_secs(7200)));
        assert_eq!(
            answer(&mut m, rows(1, range.0, 51_000), false, range),
            Vec::<Cmd>::new()
        );
        quiet(&mut m, Msg::Tick(later + Duration::from_secs(9000)));
        assert!(!m.traffic.auto.enabled);
    }

    #[test]
    fn a_new_query_ends_the_backfill_and_its_late_slice_is_ignored() {
        let mut m = two_hours();
        quiet(&mut m, key('B'));
        update(&mut m, key('y'));
        let stale = m.traffic.seq;
        let mut f = m.traffic.filter.clone();
        f.window = "6h".into();
        assert_eq!(update(&mut m, Msg::ApplyFilter(f)).len(), 1);
        assert!(m.traffic.backfill.is_none());
        quiet(
            &mut m,
            Msg::SliceDone {
                seq: stale,
                result: Err("cancelled".into()),
            },
        );
        let mut m = two_hours();
        quiet(&mut m, key('B'));
        update(&mut m, key('y'));
        quiet(&mut m, key('-'));
        assert!(m.traffic.backfill.is_none() && !m.traffic.is_running());
    }

    /// After a full backfill: `+` adds one slice of its length, auto-refresh
    /// stays off and its tick sends nothing; `r` replaces it.
    #[test]
    fn plus_extends_a_backfill_by_its_slice_and_auto_stays_off() {
        let mut m = two_hours();
        quiet(&mut m, key('B'));
        update(&mut m, key('y'));
        let mut range = (ago(6), end());
        for s in [ago(12), ago(18), ago(24)] {
            assert_eq!(
                answer(&mut m, rows(1, range.0, 50_000), false, range).len(),
                1
            );
            range = (s, range.0);
        }
        quiet_answer(&mut m, range);
        let mut wide = m.traffic.filter.clone();
        wide.window = "30h".into();
        let cmds = update(&mut m, key('+'));
        assert_eq!(
            cmds,
            vec![Cmd::RunSlice {
                seq: m.traffic.seq,
                filter: wide.clone(),
                start: ago(30),
                end: ago(24),
                cancel: CancellationToken::new(),
            }]
        );
        quiet_answer(&mut m, (ago(30), ago(24)));
        let d = m.traffic.data.as_ref().unwrap();
        assert_eq!(d.filter.window, "30h");
        assert!(!d.truncated);
        assert_eq!(meta(&m).as_deref(), Some("30h · 5 slices"));
        quiet(&mut m, key('a'));
        assert!(!m.traffic.auto.enabled);
        assert!(
            m.status
                .text
                .starts_with("auto-refresh stays off while a backfill is loaded")
        );
        m.traffic.auto.enabled = true; // even if it were on, a tick sends nothing
        quiet(
            &mut m,
            Msg::Tick(std::time::Instant::now() + Duration::from_secs(3600)),
        );
        let cmds = update(&mut m, key('r'));
        assert!(
            matches!(cmds.as_slice(), [Cmd::RunQuery { .. }]),
            "{cmds:?}"
        );
    }

    fn quiet_answer(m: &mut Model, range: (Timestamp, Timestamp)) {
        assert_eq!(
            answer(m, rows(1, range.0, 52_000), false, range),
            Vec::<Cmd>::new()
        );
    }
}

/// `Ctrl-N`/`Ctrl-P` in the traffic filter editor (issue #14).
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
    use crate::tui::model::{Model, Overlay, TrafficView};
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
                    assert!(t.flows.rows().iter().all(|r| r.dst_group == "crm/prod"));
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
            matches_count: Some(n as u64),
            elapsed: Duration::from_millis(900),
            cache_hit: false,
            fetched_at: Instant::now(),
            filter: m.traffic.filter.clone(),
            slices: None,
            backfill: None,
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
            matches_count: Some(3),
            elapsed: Duration::from_millis(900),
            hit: false,
            filter: m.traffic.filter.clone(),
            bounds: None,
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
            matches_count: Some(0),
            elapsed: Duration::ZERO,
            hit: false,
            filter: m.traffic.filter.clone(),
            bounds: None,
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
            "unmanaged-nas/- is not a label group: it cannot be re-queried by its labels"
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
        let _ = update(&mut m, key(']'));
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
            matches_count: Some(40),
            elapsed: Duration::from_millis(100),
            hit: false,
            filter: m.traffic.filter.clone(),
            bounds: None,
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
}
