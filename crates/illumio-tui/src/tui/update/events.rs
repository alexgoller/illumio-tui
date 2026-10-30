//! The Events screen's transitions: loading, the filter, its keys, the
//! type and actor summary and the detail's height. Filtering by type, by
//! actor (and by a `X+` severity) is local: it never sends a request.
use super::*;
use crate::tui::events_editor::{ACTOR, EventsEditor, SEVERITY, TYPE, WINDOW};
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
                    f.actor.clone(),
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
            KeyCode::Char('u') | KeyCode::Char('U')
                if !k.modifiers.contains(KeyModifiers::CONTROL) =>
            {
                let only = k.code == KeyCode::Char('u');
                match model.events.table.selected_item().map(|e| e.actor()) {
                    Some(actor) => actor_action(model, &actor, only),
                    None => model.status = StatusLine::info("no event to filter by"),
                }
                return Vec::new();
            }
            KeyCode::Char('t') => {
                if model.events.raw.is_empty() {
                    model.status = StatusLine::info("no events loaded");
                } else {
                    model.overlay = Overlay::EventTypes {
                        selected: 0,
                        actors: false,
                    };
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

/// `U` (hide) or `u` (only this) on the actor `actor`, and the same in
/// the summary's Actors view: the Actor filter changes and the loaded
/// events are filtered again; no request.
/// An actor name the term syntax cannot hold as one term (`,` `!` `=`
/// `*`) is refused and the filter kept: it is typed in `f` instead.
fn actor_action(model: &mut Model, actor: &str, only: bool) {
    if !model.events.actor_action(actor, only) {
        model.status = StatusLine::warn("actor name has , ! = or *: type a filter in f");
        return;
    }
    let what = if only { "only" } else { "hiding" };
    model.status = StatusLine::info(format!("{what} {actor} · {}", shown_text(&model.events)));
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
/// `Ctrl-P` complete the Type and Actor fields from the loaded events,
/// `Esc` closes (first puts a completed token back), `Enter` applies. A
/// change of the type or actor terms alone, or of a `X+` severity,
/// filters the loaded events again; only a change of what the PCE is
/// asked (one severity, the window) sends a request.
pub(super) fn events_editor_key(model: &mut Model, mut e: EventsEditor, k: KeyEvent) -> Vec<Cmd> {
    let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
    if ctrl && matches!(k.code, KeyCode::Char('n' | 'p')) {
        if e.active == TYPE || e.active == ACTOR {
            complete_field(model, &mut e, k.code == KeyCode::Char('n'));
            if !e.completing() {
                let what = if e.active == TYPE {
                    "event type"
                } else {
                    "actor"
                };
                model.status =
                    StatusLine::info(format!("no {what} to complete from the loaded events"));
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
        severity: e.values[SEVERITY].trim().to_string(),
        event_type: e.values[TYPE].trim().to_string(),
        actor: e.values[ACTOR].trim().to_string(),
        window: e.values[WINDOW].trim().to_string(),
    };
    let before = model.events.filter.clone();
    match apply_events_edit(model, edit) {
        Ok(cmds) => {
            // Sent: a PCE error for it comes back into this editor.
            if !cmds.is_empty() {
                model.events.reopen = Some(Box::new((e, before)));
            }
            cmds
        }
        Err(err) => {
            e.error = Some(err);
            model.overlay = Overlay::EventsEditor(e);
            Vec::new()
        }
    }
}

/// The editor's `Enter` (and a saved events filter): `edit` checked, then
/// in force. A change of the type or actor terms alone, or of a `X+` severity,
/// filters the loaded events again; only a change of what the PCE is asked
/// (one severity, the window) sends a request. Closes the overlay.
pub(super) fn apply_events_edit(
    model: &mut Model,
    edit: EventsFilterEdit,
) -> Result<Vec<Cmd>, String> {
    let filter = edit.to_filter(jiff::Timestamp::now())?;
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
        return Ok(Vec::new());
    }
    if ev.loading {
        return Err("events are still loading".into());
    }
    model.overlay = Overlay::None;
    model.events.filter = edit;
    model.events.rebuild();
    Ok(request_events(model, filter, CacheMode::Use))
}

/// A failed events load: when the editor's `Enter` sent it (and nothing
/// else is open on Events), the editor comes back with the PCE's error
/// inline and its fields editable, and the filter before it holds; else
/// the status line (and the error box for a long error) as before.
pub(super) fn events_failed(model: &mut Model, e: &str, tripped: bool) {
    let reopen = model.events.reopen.take();
    if let Some(r) = reopen
        && model.overlay == Overlay::None
        && model.screen == Screen::Events
    {
        let (mut editor, before) = *r;
        editor.error = Some(e.to_string());
        model.events.filter = before;
        model.events.rebuild();
        model.overlay = Overlay::EventsEditor(editor);
    }
    fail(model, "could not load events", e, tripped);
}

/// Keys of the summary (`actors`: its Actors view, else Types): `j`/`k`
/// move, `Tab` switches the view, `Enter` or `o` shows only the type or
/// actor (and closes), `x`/`!` hides it (the list stays open for more),
/// `Esc` or `q` closes. Never a request.
pub(super) fn event_types_key(
    model: &mut Model,
    selected: usize,
    actors: bool,
    k: KeyEvent,
) -> Vec<Cmd> {
    let values = if actors {
        model.events.actor_counts()
    } else {
        model.events.type_counts()
    };
    let last = values.len().saturating_sub(1);
    let value = values.get(selected.min(last)).map(|(v, _)| v.clone());
    let (selected, actors) = match k.code {
        KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('t') => {
            model.overlay = Overlay::None;
            return Vec::new();
        }
        KeyCode::Char('?') => {
            open_help(model);
            return Vec::new();
        }
        KeyCode::Tab | KeyCode::BackTab => (0, !actors),
        KeyCode::Down | KeyCode::Char('j') => ((selected + 1).min(last), actors),
        KeyCode::Up | KeyCode::Char('k') => (selected.saturating_sub(1), actors),
        KeyCode::PageDown => ((selected + 10).min(last), actors),
        KeyCode::PageUp => (selected.saturating_sub(10), actors),
        KeyCode::Home | KeyCode::Char('g') => (0, actors),
        KeyCode::End | KeyCode::Char('G') => (last, actors),
        KeyCode::Enter | KeyCode::Char('o') | KeyCode::Char('x') | KeyCode::Char('!') => {
            let only = matches!(k.code, KeyCode::Enter | KeyCode::Char('o'));
            match value.filter(|v| !v.is_empty()) {
                Some(a) if actors => actor_action(model, &a, only),
                Some(ty) => type_action(model, &ty, only),
                None => model.status = StatusLine::info("events without a type cannot be named"),
            }
            if only {
                model.overlay = Overlay::None;
                return Vec::new();
            }
            (selected, actors)
        }
        _ => (selected, actors),
    };
    model.overlay = Overlay::EventTypes { selected, actors };
    Vec::new()
}
