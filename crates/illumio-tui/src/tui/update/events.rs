//! The Events screen's transitions: loading, the filter, its keys and the
//! detail's height.
use super::*;

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
