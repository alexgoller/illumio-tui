//! The Workloads screen's transitions: loading, its keys and filters, the
//! label filter, the groups, the impact's workloads and the edit keys on
//! workloads.
use super::*;

/// Loads the workloads once. Waits for the name index like Objects does, so the
/// startup load never races a second request for the same data.
pub(super) fn load_missing_workloads(model: &mut Model) -> Vec<Cmd> {
    if model.index().is_none() || model.workloads.loaded_at.is_some() || model.workloads.loading {
        return Vec::new();
    }
    load_workloads(model, CacheMode::Use)
}

pub(super) fn load_workloads(model: &mut Model, mode: CacheMode) -> Vec<Cmd> {
    model.workloads.loading = true;
    model.workloads.auto.last_run = Some(Instant::now());
    model.workloads.auto.from_tick = false;
    vec![Cmd::LoadWorkloads(mode)]
}

/// `i` on Policy and in a rule detail (and, from Task 6, the confirm
/// overlay): the workload list behind the impact, read once. A list already cached or on its way costs
/// nothing.
pub(super) fn impact_key(model: &mut Model) -> Vec<Cmd> {
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

pub(super) fn workloads_page(model: &Model) -> usize {
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
pub(super) fn workload_filter_ref(
    w: &pce::models::Workload,
    loaded: &[pce::models::Workload],
) -> String {
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

pub(super) fn workloads_key(model: &mut Model, k: KeyEvent) -> Vec<Cmd> {
    let page = workloads_page(model);
    if !model.workloads.table.is_searching() {
        if wl_compare::refuse_gone(model, k) {
            return Vec::new();
        }
        match k.code {
            // Compare mode (#85): `c` clears the filters here, so `b` picks
            // the snapshot; `n` and `C` as on Traffic.
            KeyCode::Char('b') => return wl_compare::baseline_key(model),
            KeyCode::Char('n') if model.workloads.comparing() => {
                wl_compare::cycle_changes(model);
                return Vec::new();
            }
            KeyCode::Char('C') => {
                wl_compare::leave_compare(model);
                return Vec::new();
            }
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
            // Grouped, on a header: fold; on a workload (flat or grouped): mark.
            KeyCode::Char(' ') if model.workloads.selected_group().is_some() => {
                model.workloads.toggle_fold();
                model.workloads.ensure_visible(page);
                return Vec::new();
            }
            KeyCode::Char(' ') => {
                bulk::toggle_row(model);
                return Vec::new();
            }
            KeyCode::Char('V') => {
                bulk::toggle_scope(model);
                return Vec::new();
            }
            KeyCode::Char('L') => {
                bulk::open_labels(model);
                return Vec::new();
            }
            KeyCode::Char('M') => {
                bulk::open_enforcement(model);
                return Vec::new();
            }
            KeyCode::Esc if bulk::esc(model) => return Vec::new(),
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
            KeyCode::Char(c @ ('h' | 'E' | 'm' | 'o' | 'u' | 'v' | 'c')) => {
                // Local filters over the loaded list: no request.
                let order = model.limits.key_order();
                if c == 'u' && !model.workloads.filters.unlabeled && order.first().is_empty() {
                    model.status = StatusLine::info("no label model keys");
                    return Vec::new();
                }
                refilter(model, |w| match c {
                    'h' => w.toggle_unhealthy(),
                    'E' => w.filters.next_enforcement(),
                    'm' => w.filters.next_managed(),
                    'o' => w.toggle_offline(),
                    'u' => w.filters.unlabeled = !w.filters.unlabeled,
                    'v' => w.next_health(),
                    _ => w.clear_filters(),
                });
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

/// Changes the local filters with `change` and rebuilds the rows: the
/// cursor stays on its workload while it is shown. No request.
pub(super) fn refilter(
    model: &mut Model,
    change: impl FnOnce(&mut crate::tui::model::WorkloadsState),
) {
    let order = model.limits.key_order();
    let w = &mut model.workloads;
    let href = w.selected_row().map(|r| r.workload.href.clone());
    change(w);
    w.rebuild(&order);
    if let Some(href) = href {
        w.select_href(&href);
    }
    w.show_detail &= w.selected_row().is_some();
    // The filter line comes and goes: the page size with it.
    let page = workloads_page(model);
    model.workloads.ensure_visible(page);
}

/// `:ven`: the VEN fleet of the loaded workloads, on Workloads. Without
/// a loaded list it says so: it never loads one.
pub(super) fn open_ven(model: &mut Model) {
    if model.workloads.loaded_at.is_none() {
        model.status = StatusLine::info(if model.workloads.loading {
            "workloads are still loading"
        } else {
            "no workloads loaded: 4 (Workloads) loads them"
        });
        return;
    }
    model.screen = Screen::Workloads;
    model.overlay = Overlay::Ven { selected: 0 };
}

/// Keys of `:ven`: `j`/`k` move over the buckets, `Enter` filters the
/// list to the bucket (other filters kept) and closes, `Esc`/`q` close.
/// Never a request.
pub(super) fn ven_key(model: &mut Model, selected: usize, k: KeyEvent) -> Vec<Cmd> {
    let lines = crate::tui::ven_view::lines(&model.workloads.ven);
    let last = crate::tui::ven_view::bucket_lines(&lines)
        .len()
        .saturating_sub(1);
    let selected = match k.code {
        KeyCode::Esc | KeyCode::Char('q') => {
            model.overlay = Overlay::None;
            return Vec::new();
        }
        KeyCode::Char('?') => {
            open_help(model);
            return Vec::new();
        }
        KeyCode::Down | KeyCode::Char('j') => (selected + 1).min(last),
        KeyCode::Up | KeyCode::Char('k') => selected.saturating_sub(1),
        KeyCode::PageDown => (selected + 10).min(last),
        KeyCode::PageUp => selected.saturating_sub(10),
        KeyCode::Home | KeyCode::Char('g') => 0,
        KeyCode::End | KeyCode::Char('G') => last,
        KeyCode::Enter => {
            let Some(bucket) = crate::tui::ven_view::bucket_at(&lines, selected).cloned() else {
                return Vec::new();
            };
            let mut filters = model.workloads.filters.clone();
            if let Err(e) = bucket.apply(&mut filters) {
                model.status = StatusLine::warn(e);
                return Vec::new();
            }
            model.overlay = Overlay::None;
            refilter(model, |w| {
                // Health is one dimension: a health bucket replaces `o` and `h`.
                let health = filters.health;
                w.filters = filters;
                w.set_health(health);
            });
            let w = &model.workloads;
            model.status = StatusLine::info(format!(
                "{} · filtered locally, no request",
                w.filter_line().unwrap_or_default()
            ));
            return Vec::new();
        }
        _ => selected,
    };
    model.overlay = Overlay::Ven { selected };
    Vec::new()
}

/// The editor's unknown keys and values for its current text (none while
/// it does not parse): once per edit, never per frame.
pub(super) fn label_editor_unknown(model: &Model, e: &mut LabelFilterEditor) {
    let idx = model.index().cloned();
    e.unknown = e
        .parse()
        .map(|f| model.workloads.unknown_labels(&f, idx.as_deref()))
        .unwrap_or_default();
}

/// `Enter` on a section header: the label filter narrowed to the group
/// (`app=shop env=prod`; a missing label as `key=-`), other terms kept.
pub(super) fn narrow_to_group(model: &mut Model, g: &crate::tui::wl_groups::WlGroup) -> Vec<Cmd> {
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
    // Several labels a name matches without case: the editor says so.
    if let Some(i) = &idx {
        labels.resolved(i)?;
    }
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

/// `n` / `e` / `x` on Workloads. Edit and delete only for workloads the PCE
/// reports as unmanaged with no VEN (the library's rule, checked again there).
pub(super) fn workloads_edit_key(model: &mut Model, c: char) -> Vec<Cmd> {
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
