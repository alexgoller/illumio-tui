//! The btop-style titled panel: rounded border, title on the left, a dim
//! meta string (age, countdown, counts) on the right.
use crate::tui::ui::theme::Theme;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType};

pub fn panel<'a>(title: &'a str, meta: &'a str, theme: &Theme, focused: bool) -> Block<'a> {
    let meta = if meta.is_empty() {
        Vec::new()
    } else {
        vec![Span::styled(meta, theme.dim_style())]
    };
    panel_spans(title, meta, theme, focused)
}

/// `panel` with the meta given as styled spans, for a meta part of which
/// reads differently (Volume's `b bytes` when no flow has bytes).
pub fn panel_spans<'a>(
    title: &'a str,
    meta: Vec<Span<'a>>,
    theme: &Theme,
    focused: bool,
) -> Block<'a> {
    let border = if focused {
        theme.border_focus
    } else {
        theme.border
    };
    let mut b = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(border))
        .title(Line::from(vec![
            Span::raw(" "),
            Span::styled(title, theme.title_style()),
            Span::raw(" "),
        ]));
    if !meta.is_empty() {
        let mut spans = vec![Span::raw(" ")];
        spans.extend(meta);
        spans.push(Span::raw(" "));
        b = b.title_top(Line::from(spans).right_aligned());
    }
    b
}
