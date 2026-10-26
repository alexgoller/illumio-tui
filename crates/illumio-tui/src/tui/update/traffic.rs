//! The Traffic screen's transitions: queries, results, views, the treemap,
//! keys, focus, the window and cap steps, and the policy-decision drafts.
use super::*;

/// A query's answer as the loaded result (its range from the bounds).
pub(super) fn outcome_data(o: QueryOutcome) -> TrafficData {
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
pub(super) fn result_status(d: &TrafficData, head: &str, queries: &str, hint: &str) -> StatusLine {
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

pub(super) fn fail(model: &mut Model, context: &str, e: &str, auto_off: bool) {
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
pub(super) fn run_now(model: &mut Model, f: TrafficFilter) -> Vec<Cmd> {
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

pub(super) fn start_query(model: &mut Model, filter: TrafficFilter, mode: CacheMode) -> Vec<Cmd> {
    let (seq, cancel) = model.traffic.start_query(Instant::now());
    model.status = StatusLine::info("query running");
    vec![Cmd::RunQuery {
        seq,
        filter,
        mode,
        cancel,
    }]
}

pub(super) fn refresh_running_status(model: &mut Model) {
    refresh_running_status_at(model, Instant::now());
}

pub(super) fn refresh_running_status_at(model: &mut Model, now: Instant) {
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
pub(super) fn draft_cost(queries: u32, fresh: bool) -> String {
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
pub(super) fn draft_key(model: &mut Model) -> Vec<Cmd> {
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
pub(super) struct DraftTarget {
    pub(super) filter: TrafficFilter,
    pub(super) focus: Option<Focus>,
    pub(super) hrefs: Vec<String>,
    pub(super) queries: u32,
}

/// `D`'s target, `None` without a result on screen.
pub(super) fn draft_target(t: &TrafficState) -> Option<DraftTarget> {
    let d = t.data.as_ref()?;
    Some(DraftTarget {
        filter: d.filter.clone(),
        focus: t.focus_result.as_ref().map(|r| r.focus.clone()),
        hrefs: d.hrefs.clone(),
        queries: t.focus_result.as_ref().map_or(1, |r| r.queries),
    })
}

/// Why `D` asks before it runs.
pub(super) enum DraftAsk {
    /// No query of this session behind each part of the result (a cached
    /// or dropped one): the queries run again with the draft.
    Rerun,
    /// The filter's own reason (a long window, a high cap).
    Costly(String),
}

/// Whether `D` asks first on this target; `None`: it runs straight away.
pub(super) fn draft_ask(
    filter: &TrafficFilter,
    hrefs: &[String],
    queries: u32,
) -> Option<DraftAsk> {
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
pub(super) fn redraft_after_write(model: &mut Model, from_flow: bool, waits: bool) -> Vec<Cmd> {
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
pub(super) fn start_draft(
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
pub(super) fn draft_done(
    model: &mut Model,
    seq: u64,
    result: Result<DraftOutcome, DraftFailure>,
) -> Vec<Cmd> {
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

pub(super) fn traffic_page(model: &Model) -> usize {
    let area = ratatui::layout::Rect::new(0, 0, model.size.0, model.size.1);
    crate::tui::ui::layout::chrome(area)
        .map(|c| {
            use crate::tui::ui::screens::traffic::{detail_extra, table_rows};
            table_rows(c.body, detail_extra(&model.traffic))
        })
        .unwrap_or(10)
        .max(1)
}

pub(super) fn traffic_key(model: &mut Model, k: KeyEvent) -> Vec<Cmd> {
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
        KeyCode::Char('c') => return compare_key(model),
        KeyCode::Char('C') => leave_compare(model),
        KeyCode::Char('n') => cycle_changes(model),
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
        // Which rule allows or blocks the flow: no request.
        KeyCode::Char('E') => return explain_key(model),
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
pub(super) fn narrow_drill(old: &Drill, header: &Drill) -> Option<Drill> {
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
pub(super) fn treemap_enter(model: &mut Model) {
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
pub(super) fn treemap_out(model: &mut Model) {
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
pub(super) fn open_key_picker(model: &Model) -> KeyPicker {
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
pub(super) fn apply_group_keys(model: &mut Model, keys: Vec<String>, all: bool) {
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
pub(super) fn focus_key(model: &mut Model) -> Vec<Cmd> {
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
pub(super) fn open_app_picker(model: &mut Model, purpose: AppPickerPurpose, input: &str) {
    model.overlay =
        Overlay::AppPicker(AppPicker::new(model.traffic.loaded_apps(), purpose).with_input(input));
}

/// A new focus on `app` under the keys in force, at `both`. No request.
pub(super) fn focus_on(model: &mut Model, app: GroupKey) {
    let focus = Focus::new(app, model.traffic.keys.clone());
    model.traffic.set_focus(focus);
    model.status = focus_status(&model.traffic);
}

/// `focus shop/prod · in · 7 flows`; on a partial focus also what `r`
/// would send (asked once, here).
pub(super) fn focus_status(t: &TrafficState) -> StatusLine {
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
pub(super) fn focus_by_name(model: &mut Model, name: &str) {
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
pub(super) fn requery_focus(model: &mut Model, focus: Focus) -> Vec<Cmd> {
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

pub(super) fn start_focused(
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
pub(super) fn widen_window(model: &mut Model) -> Vec<Cmd> {
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
pub(super) fn backfill_key(model: &mut Model) -> Vec<Cmd> {
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
        if let Some(cmds) = backfill_baseline(model) {
            return cmds;
        }
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
pub(super) fn focused_cap_key(model: &mut Model, key: char) -> Option<Vec<Cmd>> {
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
pub(super) fn auto_backfill(model: &mut Model, n: usize, cap: u32) -> Vec<Cmd> {
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
pub(super) fn narrow_window(model: &mut Model) -> Vec<Cmd> {
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
    let ended = model.traffic.end_compare();
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
    if ended {
        model.status = StatusLine::info(format!("{} · {COMPARE_ENDED}", model.status.text));
    }
    Vec::new()
}

/// Auto-refresh cannot run `f` unattended when it is past a cost threshold:
/// each run needs a confirm. The status that says so; `None` otherwise.
pub(super) fn auto_off_status(f: &TrafficFilter) -> Option<StatusLine> {
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
pub(super) fn step_cap(model: &mut Model, up: bool) -> Vec<Cmd> {
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
pub(super) fn request_query(model: &mut Model, f: TrafficFilter) -> Vec<Cmd> {
    if let Some(reason) = f.needs_confirmation() {
        model.overlay = Overlay::Confirm {
            reason: format!("{reason}. Run anyway? (y/n)"),
            action: PendingAction::RunQuery(f),
        };
        return Vec::new();
    }
    update(model, Msg::ApplyFilter(f))
}
