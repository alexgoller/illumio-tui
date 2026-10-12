//! Bottom status line: query/connection state on the left, key hints right.
use crate::tui::model::{Model, StatusKind};
use crate::tui::ui::widgets::overlay::{screen_hint, screen_hint_more};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

/// Columns the status text keeps before the extra hints are shown.
const STATUS_ROOM: usize = 40;

pub fn render(model: &Model, f: &mut Frame, area: Rect) {
    let t = &model.theme;
    let colour = match model.status.kind {
        StatusKind::Info => t.fg,
        StatusKind::Warn => t.warn,
        StatusKind::Error => t.bad,
    };
    let base = screen_hint(model.screen, model.writes.enabled);
    let more = screen_hint_more(model.screen, model.traffic.view);
    let mut hints = format!(" {base}  : cmd  ? help ");
    if !more.is_empty() && usize::from(area.width) >= hints.len() + more.len() + 2 + STATUS_ROOM {
        hints = format!(" {base}  {more}  : cmd  ? help ");
    }
    let left_width = area.width.saturating_sub(hints.len() as u16) as usize;
    let text: String = model.status.text.chars().take(left_width).collect();
    let pad = left_width.saturating_sub(text.chars().count());
    let line = Line::from(vec![
        Span::styled(text, Style::new().fg(colour)),
        Span::raw(" ".repeat(pad)),
        Span::styled(hints, t.dim_style()),
    ]);
    f.render_widget(Paragraph::new(line).style(t.base()), area);
}
