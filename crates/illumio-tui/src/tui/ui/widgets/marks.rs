//! The mark form (`m`): one row per field, the value and its hint side by
//! side, then the live preview and the save line.
use crate::tui::form::{clip, visible_tail};
use crate::tui::mark_form::MarkForm;
use crate::tui::model::Model;
use crate::tui::ui::layout::centered;
use crate::tui::ui::screens::workloads::plain;
use crate::tui::ui::widgets::overlay::completion_line;
use crate::tui::ui::widgets::panel::panel;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph};

/// The label column.
const LABEL_W: usize = 13;
/// The value column.
const VALUE_W: usize = 34;

pub fn render_mark_form(model: &Model, mf: &MarkForm, f: &mut Frame, area: Rect) {
    let t = &model.theme;
    let form = &mf.form;
    let preview = mf.preview.len().max(1);
    let rows = form.fields.len() + 1 + preview + 1 + 2;
    let r = centered(area, 96, u16::try_from(rows).unwrap_or(u16::MAX));
    f.render_widget(Clear, r);
    let inner_w = (r.width as usize).saturating_sub(2);
    let value_w = VALUE_W.min(inner_w.saturating_sub(2 + LABEL_W + 4));
    let hint_w = inner_w.saturating_sub(2 + LABEL_W + value_w + 3);
    let block = panel(&form.title, "Tab next · Enter saves · Esc cancels", t, true);
    let mut lines: Vec<Line> = Vec::new();
    for (i, field) in form.fields.iter().enumerate() {
        let on = i == form.active;
        let shown = plain(&visible_tail(&field.value, value_w));
        let value = format!("{shown}{}", if on { "▏" } else { "" });
        let pad = (value_w + 1).saturating_sub(value.chars().count());
        let mut spans = vec![
            Span::styled(if on { "▸ " } else { "  " }, Style::new().fg(t.accent)),
            Span::styled(
                format!("{:<w$}", field.label, w = LABEL_W),
                t.header_style(),
            ),
            Span::styled(
                value,
                if on {
                    Style::new().fg(t.fg).bg(t.selected_bg)
                } else {
                    t.base()
                },
            ),
            Span::raw(" ".repeat(pad + 2)),
        ];
        match form.completion_strip().filter(|_| on) {
            Some(s) => spans.extend(completion_line(&s, 0, hint_w, t).spans),
            None => spans.push(Span::styled(
                clip(&plain(&field.hint), hint_w),
                t.dim_style(),
            )),
        }
        lines.push(Line::from(spans));
    }
    lines.push(Line::from(Span::styled("─".repeat(inner_w), t.dim_style())));
    if mf.preview.is_empty() {
        lines.push(Line::from(Span::styled(
            "preview: fix the fields above",
            t.dim_style(),
        )));
    }
    for p in &mf.preview {
        lines.push(Line::from(Span::styled(clip(p, inner_w), t.base())));
    }
    lines.push(match &form.error {
        Some(e) => Line::from(Span::styled(
            clip(&plain(e), inner_w),
            Style::new().fg(t.bad),
        )),
        None => Line::from(Span::styled(
            "Enter saves to marks.toml · Esc cancels",
            t.dim_style(),
        )),
    });
    f.render_widget(Paragraph::new(lines).block(block).style(t.base()), r);
}
