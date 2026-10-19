//! The Explain overlay (`E`): which rule allows or blocks a flow. Its
//! content is `ExplainView::rows`; this draws it.
use crate::tui::explain::{ExplainView, Row};
use crate::tui::model::Model;
use crate::tui::ui::layout::centered;
use crate::tui::ui::widgets::panel::panel;
use pce::explain::MatchKind;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph, Wrap};

/// The key line, the last row inside the box.
pub const EXPLAIN_KEYS: &str =
    "j/k rule · Enter rule in Policy (draft) · D evaluate · w rule · Esc";

const LABEL: usize = 10;

pub fn render(model: &Model, v: &ExplainView, f: &mut Frame, area: Rect) {
    let t = &model.theme;
    let width = area.width.saturating_sub(4).clamp(20, 96);
    let inner = usize::from(width.saturating_sub(2)).max(1);
    let mut lines: Vec<Line> = Vec::new();
    let mut selected_line = None;
    for row in v.rows() {
        match row {
            Row::Field(label, text) => {
                let style = match label {
                    "verdict" => verdict_style(model, v),
                    _ => t.base(),
                };
                lines.push(Line::from(vec![
                    Span::styled(format!("{label:<LABEL$}"), t.header_style()),
                    Span::styled(text, style),
                ]));
            }
            Row::Rule {
                index,
                head,
                detail,
            } => {
                let r = &v.explanation.rules[index];
                let selected = index == v.selected;
                let marker = if selected { "▸ " } else { "  " };
                let kind = match r.kind {
                    MatchKind::Allow | MatchKind::Essential => Style::new().fg(t.ok),
                    MatchKind::Deny | MatchKind::OverrideDeny => Style::new().fg(t.bad),
                    MatchKind::Unknown => t.dim_style(),
                };
                let head_style = if selected { t.selected_style() } else { kind };
                if selected {
                    selected_line = Some(lines.len());
                }
                lines.push(Line::from(vec![
                    Span::styled(
                        format!("{:<LABEL$}", if index == 0 { "rules" } else { "" }),
                        t.header_style(),
                    ),
                    Span::styled(marker, kind),
                    Span::styled(head, head_style),
                ]));
                lines.push(Line::from(vec![
                    Span::raw(" ".repeat(LABEL + 2)),
                    Span::styled(detail, t.dim_style()),
                ]));
            }
            Row::Hint(h) => lines.push(Line::from(vec![
                Span::raw(" ".repeat(LABEL)),
                Span::styled(h, Style::new().fg(t.accent)),
            ])),
        }
    }
    let keys = Line::from(Span::styled(EXPLAIN_KEYS, t.dim_style()));
    // Rows once wrapped; the box fits the area, the key line stays at its
    // foot and the body scrolls to keep the selected rule in view.
    let heights: Vec<usize> = lines
        .iter()
        .map(|l| l.width().div_ceil(inner).max(1))
        .collect();
    let key_rows = keys.width().div_ceil(inner).max(1);
    let total = heights.iter().sum::<usize>() + key_rows + 2;
    let height = u16::try_from(total).unwrap_or(u16::MAX).min(area.height);
    let r = centered(area, width, height);
    f.render_widget(Clear, r);
    let block = panel("explain", "no request", t, true);
    let inside = block.inner(r);
    f.render_widget(block.style(t.base()), r);
    let key_h = u16::try_from(key_rows)
        .unwrap_or(u16::MAX)
        .min(inside.height);
    let body = Rect {
        height: inside.height - key_h,
        ..inside
    };
    let foot = Rect {
        y: inside.y + body.height,
        height: key_h,
        ..inside
    };
    let view = usize::from(body.height);
    let scroll = selected_line.map_or(0, |i| {
        let top: usize = heights[..i].iter().sum();
        let bottom = top + heights[i] + heights.get(i + 1).copied().unwrap_or(0);
        bottom.saturating_sub(view).min(top)
    });
    f.render_widget(
        Paragraph::new(lines)
            .style(t.base())
            .wrap(Wrap { trim: false })
            .scroll((u16::try_from(scroll).unwrap_or(u16::MAX), 0)),
        body,
    );
    f.render_widget(
        Paragraph::new(keys)
            .style(t.base())
            .wrap(Wrap { trim: false }),
        foot,
    );
}

fn verdict_style(model: &Model, v: &ExplainView) -> Style {
    let t = &model.theme;
    if !v.explanation.evaluated {
        Style::new().fg(t.warn)
    } else if v.allowed_by_rule()
        && !v
            .explanation
            .rules
            .iter()
            .any(|r| r.kind == MatchKind::OverrideDeny)
    {
        Style::new().fg(t.ok)
    } else {
        Style::new().fg(t.bad)
    }
}
