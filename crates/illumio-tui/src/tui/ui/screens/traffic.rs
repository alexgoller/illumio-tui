//! The Traffic screen. This task: layout and the filter bar. Tasks 7–9
//! add the stats header, the three result views and the detail pane.
use crate::tui::model::{Model, QueryState};
use crate::tui::ui::widgets::panel::panel;
use pce::models::PolicyDecision;
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

#[derive(Debug, Clone, Copy)]
pub struct TrafficLayout {
    pub filter: Rect,
    pub stats: Rect,
    pub table: Rect,
    pub detail: Rect,
}

/// Shared by `view` and `update` (for page sizes): filter bar 3, stats 3,
/// table takes the rest, detail 6 (0 when the terminal is short).
pub fn layout(body: Rect) -> TrafficLayout {
    let detail_h = if body.height >= 20 { 6 } else { 0 };
    let rows = Layout::vertical([
        Constraint::Length(3),
        Constraint::Length(3),
        Constraint::Min(4),
        Constraint::Length(detail_h),
    ])
    .split(body);
    TrafficLayout {
        filter: rows[0],
        stats: rows[1],
        table: rows[2],
        detail: rows[3],
    }
}

/// Rows of data visible in the flow table (minus border and header).
pub fn table_rows(body: Rect) -> usize {
    layout(body).table.height.saturating_sub(3) as usize
}

pub fn render(model: &Model, f: &mut Frame, body: Rect) {
    let l = layout(body);
    render_filter_bar(model, f, l.filter);
    let t = &model.theme;
    f.render_widget(
        Paragraph::new("").block(panel("Flows", &flows_meta(model), t, false)),
        l.table,
    );
}

/// Query state for the Flows panel title: running, result count and age, or failure.
fn flows_meta(model: &Model) -> String {
    match &model.traffic.query {
        QueryState::Running {
            started, status, ..
        } => format!("running {:.1}s · {status}", started.elapsed().as_secs_f64()),
        QueryState::Failed(_) => "failed".into(),
        QueryState::Idle => match &model.traffic.data {
            Some(d) => format!(
                "{} flows · {}s ago",
                d.flows.len(),
                d.fetched_at.elapsed().as_secs()
            ),
            None => "no query yet · press r".into(),
        },
    }
}

pub fn render_filter_bar(model: &Model, f: &mut Frame, area: Rect) {
    let t = &model.theme;
    let fl = &model.traffic.filter;
    let hint = fl.cost_hint();
    let block = panel("Filters", &hint, t, false);
    let or = |s: &str| {
        if s.trim().is_empty() {
            "*".to_string()
        } else {
            s.trim().to_string()
        }
    };
    let decision = if fl.decisions.len() == PolicyDecision::ALL.len() {
        "all".to_string()
    } else {
        fl.decisions
            .iter()
            .map(|d| d.as_str())
            .collect::<Vec<_>>()
            .join(",")
    };
    let line = Line::from(vec![
        Span::styled("window ", t.dim_style()),
        Span::styled(fl.window.clone(), Style::new().fg(t.accent)),
        Span::styled("  src ", t.dim_style()),
        Span::styled(or(&fl.src), t.base()),
        Span::styled("  dst ", t.dim_style()),
        Span::styled(or(&fl.dst), t.base()),
        Span::styled("  svc ", t.dim_style()),
        Span::styled(or(&fl.service), t.base()),
        Span::styled("  decision ", t.dim_style()),
        Span::styled(decision, t.base()),
    ]);
    f.render_widget(Paragraph::new(line).block(block).style(t.base()), area);
}
