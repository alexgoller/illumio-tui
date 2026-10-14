//! The Traffic screen: filter bar, result stats, flow table and detail pane.
use crate::tui::model::{By, Model, QueryState, SectionRow, TrafficView, TreemapState};
use crate::tui::table::TableView;
use crate::tui::ui::theme::Theme;
use crate::tui::ui::widgets::bars;
use crate::tui::ui::widgets::panel::{panel, panel_spans};
use pce::KeyOrder;
use pce::models::PolicyDecision;
use pce::traffic::treemap::{Rect as TreemapRect, Tile, Weight};
use pce::traffic::{Decisions, ResolvedEndpoint, ResolvedFlow, compact_count};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Paragraph, Wrap};
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

/// The cells the treemap fills on a terminal of `size`: the table and
/// detail area of `layout` minus the panel border, exactly where
/// `render_treemap` draws.
pub fn treemap_area(size: (u16, u16)) -> TreemapRect {
    let Some(c) = crate::tui::ui::layout::chrome(Rect::new(0, 0, size.0, size.1)) else {
        return TreemapRect::default();
    };
    let l = layout(c.body);
    let r = l.table.union(l.detail);
    TreemapRect {
        x: r.x + 1,
        y: r.y + 1,
        w: r.width.saturating_sub(2),
        h: r.height.saturating_sub(2),
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
        TrafficView::Mesh => render_mesh(model, f, l.table.union(l.detail)),
        TrafficView::Treemap => render_treemap(model, f, l.table.union(l.detail)),
        TrafficView::Blocked => render_blocked(model, f, l.table.union(l.detail)),
    }
}

/// The Result header: the focus badge (`focus shop/prod · in`, or `focus is
/// partial: r re-queries shop/prod · in`) in front of the query's meta.
fn query_meta(model: &Model) -> String {
    let meta = result_meta(model);
    match model.traffic.focus_meta() {
        Some(focus) => format!("{focus} · {meta}"),
        None => meta,
    }
}

fn result_meta(model: &Model) -> String {
    match &model.traffic.query {
        QueryState::Running {
            started, status, ..
        } => format!("running {:.1}s · {status}", started.elapsed().as_secs_f64()),
        QueryState::Failed(_) => "failed".into(),
        QueryState::Idle => match &model.traffic.data {
            Some(d) => {
                let auto = &model.traffic.auto;
                format!(
                    "{} · {}{}{}{}",
                    d.count_text(),
                    if d.incomplete { "incomplete · " } else { "" },
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
    let shown = model.traffic.view_flows();
    let (mut a, mut p, mut b) = (0u64, 0u64, 0u64);
    for fl in shown.iter() {
        match fl.policy_decision {
            PolicyDecision::Allowed => a += 1,
            PolicyDecision::PotentiallyBlocked => p += 1,
            PolicyDecision::Blocked => b += 1,
            PolicyDecision::Unknown => {}
        }
    }
    let count = if model.traffic.focus.is_some() {
        format!("{} of {} flows ", shown.len(), d.flows.len())
    } else {
        format!("{} flows ", d.flows.len())
    };
    let mut left = vec![
        Span::styled(count, t.header_style()),
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
    // No time sparkline: the flows are lifetime aggregates, so their
    // last-detected times say nothing honest about traffic over time.
    let bar = bars::decision_bar(a, p, b, cols[1].width as usize, t);
    f.render_widget(Paragraph::new(bar), cols[1]);
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

/// The panel meta of App-to-app or Blocked: `in · blocked · keys app,env ·
/// Enter drills` (`pairs · by service · blocked · ...`), then sort and
/// search as every table shows them.
fn view_meta<T>(model: &Model, by: &str, view: &TableView<T>) -> String {
    let st = model.traffic.section_decisions();
    let mut parts = vec![by.to_string()];
    if st != Decisions::All {
        parts.push(st.label().to_string());
    }
    parts.push(format!("keys {}", model.traffic.effective_keys().join(",")));
    parts.push("Enter drills".into());
    let meta = crate::tui::ui::widgets::table::header_meta(view);
    if !meta.is_empty() {
        parts.push(meta);
    }
    parts.join(" · ")
}

/// A sectioned table: header rows in the accent colour across the table,
/// peer rows by `peer_style`.
fn render_sections(
    model: &Model,
    f: &mut Frame,
    area: Rect,
    title: &str,
    meta: &str,
    view: &TableView<SectionRow>,
    peer_style: &dyn Fn(&SectionRow) -> Style,
) {
    let t = &model.theme;
    let block = panel(title, meta, t, true);
    let accent = Style::new().fg(t.accent).add_modifier(Modifier::BOLD);
    crate::tui::ui::widgets::table::render_spanning(f, area, view, t, block, peer_style, &|r| {
        r.header_line().map(|l| (l, accent))
    });
}

/// Rows are styled by their dominant decision: blocked > potentially blocked > allowed.
fn render_groups(model: &Model, f: &mut Frame, area: Rect) {
    let t = &model.theme;
    let by = model.traffic.apps_state.by;
    if by != By::Pairs {
        let meta = view_meta(model, by.label(), &model.traffic.apps);
        let theme = t.clone();
        render_sections(
            model,
            f,
            area,
            TrafficView::Aggregate.title(),
            &meta,
            &model.traffic.apps,
            &move |r| {
                r.peer()
                    .map_or_else(Style::new, |(_, p)| theme.decision_style(p.dominant()))
            },
        );
        return;
    }
    let table = model.traffic.pairs_table();
    let by = if model.traffic.apps_state.by_service {
        "pairs · by service"
    } else {
        "pairs"
    };
    let meta = view_meta(model, by, table);
    let block = panel(TrafficView::Aggregate.title(), &meta, t, true);
    let theme = t.clone();
    crate::tui::ui::widgets::table::render(f, area, table, t, block, &move |g| {
        theme.decision_style(g.dominant())
    });
}

/// Rows are styled by their dominant decision, like App-to-app.
fn render_mesh(model: &Model, f: &mut Frame, area: Rect) {
    let t = &model.theme;
    let meta = crate::tui::ui::widgets::table::header_meta(&model.traffic.mesh);
    let meta = format!(
        "keys {} · u user · p process · Enter drills{}{}",
        model.traffic.effective_keys().join(","),
        if meta.is_empty() { "" } else { " · " },
        meta
    );
    let block = panel(TrafficView::Mesh.title(), &meta, t, true);
    let theme = t.clone();
    crate::tui::ui::widgets::table::render(f, area, &model.traffic.mesh, t, block, &move |r| {
        theme.decision_style(r.dominant())
    });
}

/// The tiles the model holds: each source a plain-bordered block titled
/// with its group, its destinations filled with `▒` in their decision's
/// colour, label and weight written over the fill where they fit; the
/// cursor tile reversed.
fn render_treemap(model: &Model, f: &mut Frame, area: Rect) {
    let t = &model.theme;
    let tm = &model.traffic.treemap;
    let title = match &tm.zoom {
        Some(z) => format!("{} ▸ {}", TrafficView::Treemap.title(), z.join("/")),
        None => TrafficView::Treemap.title().to_string(),
    };
    let block = panel_spans(&title, treemap_meta(tm, t), t, true);
    let inner = block.inner(area);
    f.render_widget(block, area);
    if tm.tiles.is_empty() {
        f.render_widget(
            Paragraph::new(Line::from(Span::styled("no rows", t.dim_style()))),
            inner,
        );
        return;
    }
    let cur = tm.current();
    let is_cur = |outer: usize, inner_i: Option<usize>| {
        cur.is_some_and(|c| c.outer == outer && c.inner == inner_i)
    };
    for (i, tile) in tm.tiles.iter().enumerate() {
        if tile.children.is_empty() {
            draw_fill(f, inner, tile, t, is_cur(i, None));
            continue;
        }
        let r = cells(tile.rect).intersection(inner);
        if r.is_empty() {
            continue;
        }
        let mut style = t.dim_style();
        if is_cur(i, None) {
            style = style.add_modifier(Modifier::REVERSED);
        }
        let label = clip_text(&tile.label, r.width.saturating_sub(2) as usize);
        f.render_widget(
            Block::bordered()
                .border_style(style)
                .title(Span::styled(label, style)),
            r,
        );
        for (j, c) in tile.children.iter().enumerate() {
            draw_fill(f, inner, c, t, is_cur(i, Some(j)));
        }
    }
}

/// `by conns · b bytes · Enter zoom · Esc out` (or `by bytes · b conns ·
/// …`), dim like every panel meta; the `b` hint fainter still when no flow
/// in the result carried bytes, since `b` then refuses.
fn treemap_meta(tm: &TreemapState, t: &Theme) -> Vec<Span<'static>> {
    let (by, toggle) = match tm.weight {
        Weight::Connections => ("by conns · ", "b bytes"),
        Weight::Bytes => ("by bytes · ", "b conns"),
    };
    let dim = t.dim_style();
    let toggle_style = if tm.has_bytes {
        dim
    } else {
        dim.add_modifier(Modifier::DIM)
    };
    vec![
        Span::styled(by, dim),
        Span::styled(toggle, toggle_style),
        Span::styled(" · Enter zoom · Esc out", dim),
    ]
}

fn cells(r: TreemapRect) -> Rect {
    Rect::new(r.x, r.y, r.w, r.h)
}

fn clip_text(s: &str, n: usize) -> String {
    s.chars().take(n).collect()
}

/// A filled tile: `▒` in the decision colour, the label on the first row
/// when it fits with a cell either side, the weight on the second.
fn draw_fill(f: &mut Frame, clip: Rect, tile: &Tile, t: &Theme, cursor: bool) {
    let r = cells(tile.rect).intersection(clip);
    if r.is_empty() {
        return;
    }
    let mut fill = t.decision_style(tile.decision);
    let mut text = Style::new().fg(t.fg).add_modifier(Modifier::BOLD);
    if cursor {
        fill = fill.add_modifier(Modifier::REVERSED);
        text = text.add_modifier(Modifier::REVERSED);
    }
    let w = r.width as usize;
    let row = |label: Option<String>| -> Line<'static> {
        match label {
            Some(l) if l.chars().count() + 2 <= w => {
                let n = l.chars().count();
                Line::from(vec![
                    Span::styled("▒", fill),
                    Span::styled(l, text),
                    Span::styled("▒".repeat(w - 1 - n), fill),
                ])
            }
            _ => Line::from(Span::styled("▒".repeat(w), fill)),
        }
    };
    let mut lines = vec![row(Some(tile.label.clone()))];
    if r.height >= 2 {
        lines.push(row(Some(compact_count(tile.weight))));
    }
    while lines.len() < r.height as usize {
        lines.push(row(None));
    }
    f.render_widget(Paragraph::new(lines), r);
}

/// App-to-app `in` with `blocked`: every peer row is blocked or
/// potentially blocked; both read as `bad`.
fn render_blocked(model: &Model, f: &mut Frame, area: Rect) {
    let meta = view_meta(model, "in", &model.traffic.blocked);
    let bad = Style::new().fg(model.theme.bad);
    render_sections(
        model,
        f,
        area,
        TrafficView::Blocked.title(),
        &meta,
        &model.traffic.blocked,
        &move |_| bad,
    );
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
fn labels_line(e: &ResolvedEndpoint, order: &KeyOrder) -> String {
    let s = e.label_text(order);
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
    let order = model.limits.key_order();
    let row = |head: &'static str, body: String| {
        Line::from(vec![
            Span::styled(format!("{head:<11}"), t.header_style()),
            Span::raw(body),
        ])
    };
    let lines = vec![
        row("src", endpoint_line(&fl.src)),
        row("src labels", labels_line(&fl.src, &order)),
        row("dst", endpoint_line(&fl.dst)),
        row("dst labels", labels_line(&fl.dst, &order)),
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
    // Shown only when narrowed (`:unicast`, the editor): the default is every kind.
    let line = if fl.has_transmission() {
        let mut spans = line.spans;
        spans.push(Span::styled("  tx ", t.dim_style()));
        spans.push(Span::styled(
            fl.transmission_text(),
            Style::new().fg(t.accent),
        ));
        Line::from(spans)
    } else {
        line
    };
    f.render_widget(Paragraph::new(line).block(block).style(t.base()), area);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn meta_text(spans: &[Span]) -> String {
        spans.iter().map(|s| s.content.as_ref()).collect()
    }

    #[test]
    fn the_treemap_meta_dims_b_bytes_when_no_flow_has_bytes() {
        let t = Theme::dark();
        let mut tm = TreemapState::new(TreemapRect::default());
        tm.has_bytes = true;
        let with = treemap_meta(&tm, &t);
        assert_eq!(
            meta_text(&with),
            "by conns · b bytes · Enter zoom · Esc out"
        );
        assert_eq!(with[1].content, "b bytes");
        assert_eq!(with[1].style, t.dim_style());
        tm.has_bytes = false;
        let without = treemap_meta(&tm, &t);
        assert_eq!(meta_text(&without), meta_text(&with));
        assert_eq!(without[1].style, t.dim_style().add_modifier(Modifier::DIM));
        assert_eq!(without[0].style, t.dim_style(), "only the b hint greys");
        tm.has_bytes = true;
        tm.weight = Weight::Bytes;
        assert_eq!(
            meta_text(&treemap_meta(&tm, &t)),
            "by bytes · b conns · Enter zoom · Esc out"
        );
    }
}
