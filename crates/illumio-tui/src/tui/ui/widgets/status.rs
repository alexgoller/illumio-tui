//! Bottom status line: query/connection state on the left, key hints right
//! (only `? help`, or `Esc back` in the app overlay, when a warning or an
//! error needs the room).
use crate::tui::model::{Model, Overlay, StatusKind};
use crate::tui::ui::widgets::overlay::{screen_hint, screen_hint_more};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

/// Columns the status text keeps before the extra hints are shown.
const STATUS_ROOM: usize = 40;

/// The app overlay's keys: it takes every key, so the screen's do not apply.
const APP_HINTS: &str = " │ Tab panel  X dir  Enter open  w rule  r reload  Esc back ";
/// The generator's keys, for the same reason.
const GEN_HINTS: &str = " │ ←/→ stop  </> row  R pick  r re-query  Esc back ";
/// The where-used overlay's keys, for the same reason.
const USED_HINTS: &str = " │ Enter open  / search  Esc back ";

pub fn render(model: &Model, f: &mut Frame, area: Rect) {
    let t = &model.theme;
    let colour = match model.status.kind {
        StatusKind::Info => t.fg,
        StatusKind::Warn => t.warn,
        StatusKind::Error => t.bad,
    };
    let base = screen_hint(model.screen, model.writes.enabled);
    let more = screen_hint_more(model.screen, model.traffic.view);
    let width = usize::from(area.width);
    let app = matches!(
        model.overlay,
        Overlay::App | Overlay::Generator | Overlay::WhereUsed
    );
    let mut hints = if model.overlay == Overlay::Generator {
        GEN_HINTS.to_string()
    } else if model.overlay == Overlay::WhereUsed {
        USED_HINTS.to_string()
    } else if app {
        APP_HINTS.to_string()
    } else {
        format!(" {base}  : cmd  ? help ")
    };
    if model.status.kind != StatusKind::Info
        && model.status.text.chars().count() + hints.chars().count() > width
    {
        // A warning or an error says what to do next: it outranks the hints
        // (the overlay takes every key, so its way out stays instead of `?`).
        hints = if app { " Esc back " } else { " ? help " }.to_string();
    } else if !app && !more.is_empty() && width >= hints.len() + more.len() + 2 + STATUS_ROOM {
        hints = format!(" {base}  {more}  : cmd  ? help ");
    }
    let left_width = area.width.saturating_sub(hints.chars().count() as u16) as usize;
    let text: String = model.status.text.chars().take(left_width).collect();
    let pad = left_width.saturating_sub(text.chars().count());
    let line = Line::from(vec![
        Span::styled(text, Style::new().fg(colour)),
        Span::raw(" ".repeat(pad)),
        Span::styled(hints, t.dim_style()),
    ]);
    f.render_widget(Paragraph::new(line).style(t.base()), area);
}
