//! The btop-style titled panel: rounded border, title on the left, a dim
//! meta string (age, countdown, counts) on the right.
use crate::tui::ui::theme::Theme;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType};

pub fn panel<'a>(title: &'a str, meta: &'a str, theme: &Theme, focused: bool) -> Block<'a> {
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
        b = b.title_top(
            Line::from(vec![
                Span::raw(" "),
                Span::styled(meta, theme.dim_style()),
                Span::raw(" "),
            ])
            .right_aligned(),
        );
    }
    b
}
