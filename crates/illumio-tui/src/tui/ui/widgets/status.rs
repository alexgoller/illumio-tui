//! Bottom status line: query/connection state on the left, key hints right.
use crate::tui::model::{Model, StatusKind};
use crate::tui::ui::widgets::overlay::screen_hint;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

pub fn render(model: &Model, f: &mut Frame, area: Rect) {
    let t = &model.theme;
    let colour = match model.status.kind {
        StatusKind::Info => t.fg,
        StatusKind::Warn => t.warn,
        StatusKind::Error => t.bad,
    };
    let hints = format!(
        " {}  : cmd  ? help ",
        screen_hint(model.screen, model.writes.enabled)
    );
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
