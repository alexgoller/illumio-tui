//! The Traffic screen: filter bar, result stats, flow table and detail pane.
use crate::tui::model::{Model, QueryState, TrafficView};
use crate::tui::ui::widgets::bars;
use crate::tui::ui::widgets::panel::panel;
use pce::models::PolicyDecision;
use pce::traffic::{ResolvedEndpoint, ResolvedFlow, compact_count};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Wrap};
use std::time::Instant;

#[derive(Debug, Clone, Copy)]
pub struct TrafficLayout {
    pub filter: Rect,
    pub stats: Rect,
    pub table: Rect,
    pub detail: Rect,
}

/// Shared by `view` and `update` (for page sizes): filter bar 3, stats 3,
/// table takes the rest, detail 8 (0 when the terminal is short).
pub fn layout(body: Rect) -> TrafficLayout {
    let detail_h = if body.height >= 20 { 8 } else { 0 };
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
    render_stats(model, f, l.stats);
    match model.traffic.view {
        TrafficView::Flows => {
            render_flows(model, f, l.table);
            if l.detail.height > 0 {
                render_detail(model, f, l.detail);
            }
        }
        // The detail pane belongs to the Flows view; the others use its space.
        TrafficView::Aggregate => render_groups(model, f, l.table.union(l.detail)),
        TrafficView::Blocked => render_blocked(model, f, l.table.union(l.detail)),
    }
}

fn query_meta(model: &Model) -> String {
    match &model.traffic.query {
        QueryState::Running {
            started, status, ..
        } => format!("running {:.1}s · {status}", started.elapsed().as_secs_f64()),
        QueryState::Failed(_) => "failed".into(),
        QueryState::Idle => match &model.traffic.data {
            Some(d) => {
                let auto = &model.traffic.auto;
                format!(
                    "{} flows · {}{}{}",
                    d.flows.len(),
                    bars::age_text(d.fetched_at.elapsed()),
                    if d.cache_hit { " · cached" } else { "" },
                    auto.meta(Instant::now())
                )
            }
            None => "no query yet · press r".into(),
        },
    }
}

fn render_stats(model: &Model, f: &mut Frame, area: Rect) {
    let t = &model.theme;
    let meta = query_meta(model);
    let block = panel("Result", &meta, t, false);
    let inner = block.inner(area);
    f.render_widget(block, area);
    let Some(d) = &model.traffic.data else {
        f.render_widget(
            Paragraph::new(Span::styled(
                "run a query with r, edit the filter with f",
                t.dim_style(),
            )),
            inner,
        );
        return;
    };
    let (mut a, mut p, mut b) = (0u64, 0u64, 0u64);
    for fl in &d.flows {
        match fl.policy_decision {
            PolicyDecision::Allowed => a += 1,
            PolicyDecision::PotentiallyBlocked => p += 1,
            PolicyDecision::Blocked => b += 1,
            PolicyDecision::Unknown => {}
        }
    }
    let mut left = vec![
        Span::styled(format!("{} flows ", d.flows.len()), t.header_style()),
        Span::styled(format!("allowed {a} "), Style::new().fg(t.ok)),
        Span::styled(format!("pot.blocked {p} "), Style::new().fg(t.warn)),
        Span::styled(format!("blocked {b}"), Style::new().fg(t.bad)),
    ];
    if d.truncated {
        left.push(Span::styled("  ⚠ truncated", Style::new().fg(t.warn)));
    }
    let left_w = left
        .iter()
        .map(|s| s.content.chars().count())
        .sum::<usize>()
        .saturating_add(2)
        .min(inner.width.saturating_sub(24) as usize) as u16;
    let cols = Layout::horizontal([Constraint::Length(left_w), Constraint::Min(10)]).split(inner);
    f.render_widget(Paragraph::new(Line::from(left)), cols[0]);
    let bar = bars::decision_bar(a, p, b, cols[1].width.saturating_sub(12) as usize, t);
    let data = bars::sparkline_buckets(&d.flows, 12);
    let max = *data.iter().max().unwrap_or(&1).max(&1);
    let spark: String = data
        .iter()
        .map(|&v| {
            let idx = ((v * 7) / max) as usize;
            ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'][idx.min(7)]
        })
        .collect();
    let mut right = bar.spans;
    right.insert(
        0,
        Span::styled(format!("{spark} "), Style::new().fg(t.accent)),
    );
    f.render_widget(Paragraph::new(Line::from(right)), cols[1]);
}

fn render_flows(model: &Model, f: &mut Frame, area: Rect) {
    let t = &model.theme;
    let mut meta = crate::tui::ui::widgets::table::header_meta(&model.traffic.flows);
    let title = match &model.traffic.drill {
        Some(drill) => {
            let badge = "Esc clears";
            meta = if meta.is_empty() {
                badge.to_string()
            } else {
                format!("{badge} · {meta}")
            };
            format!("Flows ▸ {}", drill.title())
        }
        None => TrafficView::Flows.title().to_string(),
    };
    let block = panel(&title, &meta, t, true);
    let theme = t.clone();
    crate::tui::ui::widgets::table::render(f, area, &model.traffic.flows, t, block, &move |fl| {
        theme.decision_style(fl.policy_decision)
    });
}

/// Rows are styled by their dominant decision: blocked > potentially blocked > allowed.
fn render_groups(model: &Model, f: &mut Frame, area: Rect) {
    let t = &model.theme;
    let meta = crate::tui::ui::widgets::table::header_meta(&model.traffic.groups);
    let meta = format!(
        "keys {} · Enter drills{}{}",
        model.traffic.effective_keys().join(","),
        if meta.is_empty() { "" } else { " · " },
        meta
    );
    let block = panel(TrafficView::Aggregate.title(), &meta, t, true);
    let theme = t.clone();
    crate::tui::ui::widgets::table::render(f, area, &model.traffic.groups, t, block, &move |g| {
        theme.decision_style(g.dominant())
    });
}

/// Every row here is blocked or potentially blocked; both read as `bad`.
fn render_blocked(model: &Model, f: &mut Frame, area: Rect) {
    let t = &model.theme;
    let meta = crate::tui::ui::widgets::table::header_meta(&model.traffic.blocked);
    let meta = format!(
        "by dst {}{}{}",
        model.traffic.effective_keys().join(","),
        if meta.is_empty() { "" } else { " · " },
        meta
    );
    let block = panel(TrafficView::Blocked.title(), &meta, t, true);
    let bad = Style::new().fg(t.bad);
    crate::tui::ui::widgets::table::render(f, area, &model.traffic.blocked, t, block, &move |_| {
        bad
    });
}

/// `name  ip`, then the IP lists the endpoint belongs to.
fn endpoint_line(e: &ResolvedEndpoint) -> String {
    let mut s = format!("{}  {}", e.display_name(), e.ip.clone().unwrap_or_default());
    if !e.ip_lists.is_empty() {
        s.push_str(&format!("  ip lists: {}", e.ip_lists.join(", ")));
    }
    s
}

/// `app=shop env=prod role=web`, or `-` without labels.
fn labels_line(e: &ResolvedEndpoint) -> String {
    let s = e.label_text();
    if s.is_empty() { "-".into() } else { s }
}

/// `  bytes in 117.2K out 3.9K` when the PCE reported either count.
fn bytes_text(fl: &ResolvedFlow) -> String {
    if !fl.has_bytes() {
        return String::new();
    }
    let n = |b: Option<u64>| b.map_or_else(|| "-".to_string(), compact_count);
    format!("  bytes in {} out {}", n(fl.bytes_in), n(fl.bytes_out))
}

fn render_detail(model: &Model, f: &mut Frame, area: Rect) {
    let t = &model.theme;
    let block = panel(
        "Detail",
        if model.traffic.show_detail {
            "Enter hides"
        } else {
            "Enter shows"
        },
        t,
        false,
    );
    let Some(fl) = model
        .traffic
        .flows
        .selected_item()
        .filter(|_| model.traffic.show_detail)
    else {
        f.render_widget(
            Paragraph::new(Span::styled("select a flow and press Enter", t.dim_style()))
                .block(block),
            area,
        );
        return;
    };
    let rules = if fl.rules.is_empty() {
        "n/a".to_string()
    } else {
        fl.rules.join(", ")
    };
    let dash = || "-".to_string();
    let row = |head: &'static str, body: String| {
        Line::from(vec![
            Span::styled(format!("{head:<11}"), t.header_style()),
            Span::raw(body),
        ])
    };
    let lines = vec![
        row("src", endpoint_line(&fl.src)),
        row("src labels", labels_line(&fl.src)),
        row("dst", endpoint_line(&fl.dst)),
        row("dst labels", labels_line(&fl.dst)),
        row(
            "svc",
            format!(
                "{}  process {}  user {}  {}",
                fl.service_text(),
                fl.process_name.clone().unwrap_or_else(dash),
                fl.user_name.clone().unwrap_or_else(dash),
                fl.policy_decision.as_str()
            ),
        ),
        row(
            "seen",
            format!(
                "first {}  last {}{}  rules {}",
                fl.first_detected.clone().unwrap_or_default(),
                fl.last_detected.clone().unwrap_or_default(),
                bytes_text(fl),
                rules
            ),
        ),
    ];
    f.render_widget(
        Paragraph::new(lines).block(block).wrap(Wrap { trim: true }),
        area,
    );
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
    let decision = fl.decisions_text();
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
