//! The Events screen's transitions: loading, the filter, its keys, the
//! type summary and the detail's height. Filtering by type (and by a
//! `X+` severity) is local: it never sends a request.
use super::*;
use crate::tui::events_editor::{EventsEditor, TYPE};
use crate::tui::model::EventsState;

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

/// Loads the events once. Unlike the other lists they are not in the startup
/// index, and they need no name index: a connection is enough.
pub(super) fn load_missing_events(model: &mut Model) -> Vec<Cmd> {
    if !model.is_connected() || model.events.loaded_at.is_some() || model.events.loading {
        return Vec::new();
    }
    load_events(model, CacheMode::Use)
}

/// Requests the events for the applied filter. A filter that no longer
/// validates (it was validated when applied) is reported, never sent.
pub(super) fn load_events(model: &mut Model, mode: CacheMode) -> Vec<Cmd> {
    match model.events.filter.to_filter(jiff::Timestamp::now()) {
        Ok(filter) => request_events(model, filter, mode),
        Err(e) => {
            model.status = StatusLine::error(e);
            Vec::new()
        }
    }
}

pub(super) fn request_events(model: &mut Model, filter: EventFilter, mode: CacheMode) -> Vec<Cmd> {
    model.events.requested = Some(model.events.filter.pce_side());
    model.events.loading = true;
    model.events.auto.last_run = Some(Instant::now());
    model.events.auto.from_tick = false;
    vec![Cmd::LoadEvents { filter, mode }]
}

pub(super) fn events_page(model: &Model) -> usize {
    let area = ratatui::layout::Rect::new(0, 0, model.size.0, model.size.1);
    crate::tui::ui::layout::chrome(area)
        .map(|c| crate::tui::ui::screens::events::table_rows(c.body))
        .unwrap_or(10)
        .max(1)
}

pub(super) fn events_key(model: &mut Model, k: KeyEvent) -> Vec<Cmd> {
    let page = events_page(model);
    if !model.events.table.is_searching() {
        match k.code {
            KeyCode::Char('f') => {
                let f = &model.events.filter;
                model.overlay = Overlay::EventsEditor(EventsEditor::new([
                    f.severity.clone(),
                    f.event_type.clone(),
                    f.window.clone(),
                ]));
                return Vec::new();
            }
            KeyCode::Char('x') | KeyCode::Char('!') | KeyCode::Char('o') => {
                let only = k.code == KeyCode::Char('o');
                match model
                    .events
                    .table
                    .selected_item()
                    .and_then(|e| e.event_type.clone())
                    .filter(|t| !t.is_empty())
                {
                    Some(ty) => type_action(model, &ty, only),
                    None => model.status = StatusLine::info("no event type to filter by"),
                }
                return Vec::new();
            }
            KeyCode::Char('t') => {
                if model.events.raw.is_empty() {
                    model.status = StatusLine::info("no events loaded");
                } else {
                    model.overlay = Overlay::EventTypes { selected: 0 };
                }
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

/// `x`/`!` (hide) or `o` (only this) on the type `ty`: the Type filter
/// changes and the loaded events are filtered again; no request.
fn type_action(model: &mut Model, ty: &str, only: bool) {
    let e = &mut model.events;
    if only {
        e.only_type(ty);
    } else {
        e.hide_type(ty);
    }
    let what = if only { "only" } else { "hiding" };
    model.status = StatusLine::info(format!("{what} {ty} · {}", shown_text(&model.events)));
}

/// `812 of 1,240 events shown` (or `4 events`), with `capped` when the
/// PCE returned a full page.
pub(super) fn shown_text(e: &EventsState) -> String {
    let (shown, all) = (e.table.rows().len(), e.raw.len());
    let mut s = if shown == all {
        format!("{all} events")
    } else {
        format!("{shown} of {all} events shown")
    };
    if e.truncated {
        s.push_str(" · capped: more may exist");
    }
    s
}

/// The status after a load.
pub(super) fn events_loaded_text(e: &EventsState) -> String {
    shown_text(e)
}

/// Keys of the events editor: Tab/↑/↓ move between fields, `Ctrl-N`/
/// `Ctrl-P` complete the Type field from the loaded events, `Esc` closes
/// (first puts a completed token back), `Enter` applies. A change of the
/// type terms alone, or of a `X+` severity, filters the loaded events
/// again; only a change of what the PCE is asked (one severity, the
/// window) sends a request.
pub(super) fn events_editor_key(model: &mut Model, mut e: EventsEditor, k: KeyEvent) -> Vec<Cmd> {
    let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
    if ctrl && matches!(k.code, KeyCode::Char('n' | 'p')) {
        if e.active == TYPE {
            complete_field(model, &mut e, k.code == KeyCode::Char('n'));
            if !e.completing() {
                model.status = StatusLine::info("no event type to complete from the loaded events");
            }
        }
        model.overlay = Overlay::EventsEditor(e);
        return Vec::new();
    }
    let esc = k.code == KeyCode::Esc;
    if e.end_completion(esc) && esc {
        model.overlay = Overlay::EventsEditor(e);
        return Vec::new();
    }
    let n = e.values.len();
    e.error = None;
    match k.code {
        KeyCode::Esc => {
            model.overlay = Overlay::None;
            return Vec::new();
        }
        KeyCode::Tab | KeyCode::Down => e.active = (e.active + 1) % n,
        KeyCode::BackTab | KeyCode::Up => e.active = (e.active + n - 1) % n,
        KeyCode::Backspace => {
            e.values[e.active].pop();
        }
        KeyCode::Char('u') if ctrl => e.values[e.active].clear(),
        KeyCode::Char(c) => e.values[e.active].push(c),
        KeyCode::Enter => return apply_events_filter(model, e),
        _ => {}
    }
    model.overlay = Overlay::EventsEditor(e);
    Vec::new()
}

fn apply_events_filter(model: &mut Model, mut e: EventsEditor) -> Vec<Cmd> {
    let edit = EventsFilterEdit {
        severity: e.values[0].trim().to_string(),
        event_type: e.values[1].trim().to_string(),
        window: e.values[2].trim().to_string(),
    };
    let filter = match edit.to_filter(jiff::Timestamp::now()) {
        Ok(f) => f,
        Err(err) => {
            e.error = Some(err);
            model.overlay = Overlay::EventsEditor(e);
            return Vec::new();
        }
    };
    let ev = &model.events;
    let side = edit.pce_side();
    // Refilter what is loaded (or about to be): only when it answers (or
    // will answer) the same question. After a failed request the edit
    // retries it.
    let answered = ev.loaded_at.is_some() && ev.answered.as_ref() == Some(&side);
    let in_flight = ev.loading && ev.requested.as_ref() == Some(&side);
    if answered || in_flight {
        model.overlay = Overlay::None;
        model.events.filter = edit;
        model.events.rebuild();
        model.status = StatusLine::info(format!(
            "{} · filtered locally, no request",
            shown_text(&model.events)
        ));
        return Vec::new();
    }
    if ev.loading {
        e.error = Some("events are still loading".into());
        model.overlay = Overlay::EventsEditor(e);
        return Vec::new();
    }
    model.overlay = Overlay::None;
    model.events.filter = edit;
    model.events.rebuild();
    request_events(model, filter, CacheMode::Use)
}

/// Keys of the type summary: `j`/`k` move, `Enter` shows only the type
/// (and closes), `x`/`!` hides it (the list stays open for more), `Esc`
/// or `q` closes. Never a request.
pub(super) fn event_types_key(model: &mut Model, selected: usize, k: KeyEvent) -> Vec<Cmd> {
    let types = model.events.type_counts();
    let last = types.len().saturating_sub(1);
    let ty = types.get(selected.min(last)).map(|(t, _)| t.clone());
    let selected = match k.code {
        KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('t') => {
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
        KeyCode::Enter | KeyCode::Char('x') | KeyCode::Char('!') => {
            let only = k.code == KeyCode::Enter;
            match ty.filter(|t| !t.is_empty()) {
                Some(ty) => type_action(model, &ty, only),
                None => model.status = StatusLine::info("events without a type cannot be named"),
            }
            if only {
                model.overlay = Overlay::None;
                return Vec::new();
            }
            selected
        }
        _ => selected,
    };
    model.overlay = Overlay::EventTypes { selected };
    Vec::new()
}
