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
    lines.push(Line::from(Span::styled(EXPLAIN_KEYS, t.dim_style())));
    // Rows once wrapped, plus the borders.
    let rows: usize = lines.iter().map(|l| l.width().div_ceil(inner).max(1)).sum();
    let height = u16::try_from(rows + 2).unwrap_or(u16::MAX);
    let r = centered(area, width, height);
    f.render_widget(Clear, r);
    f.render_widget(
        Paragraph::new(lines)
            .block(panel("explain", "no request", t, true))
            .style(t.base())
            .wrap(Wrap { trim: false }),
        r,
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
