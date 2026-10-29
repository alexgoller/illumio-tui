//! The Traffic screen: filter bar, result stats, flow table and detail pane.
use crate::tui::model::{
    By, Model, QueryState, SectionRow, TrafficView, TreemapState, is_gone as gone,
};
use crate::tui::table::TableView;
use crate::tui::ui::theme::Theme;
use crate::tui::ui::widgets::bars;
use crate::tui::ui::widgets::panel::{panel, panel_spans};
use pce::KeyOrder;
use pce::models::PolicyDecision;
use pce::traffic::treemap::{Rect as TreemapRect, Tile, Weight};
use pce::traffic::{Decisions, ResolvedEndpoint, ResolvedFlow, SectionOrder, compact_count};
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
    layout_with(body, DetailLines::default())
}

/// What the detail pane holds, for its height: whether the Flows detail
/// is open and whether it has a `mark` line.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DetailLines {
    pub open: bool,
    pub mark: bool,
}

/// The detail pane's inner width from which an open Flows detail is drawn
/// in source | destination | service columns.
pub const COLUMNS_MIN_WIDTH: u16 = 90;
/// The body height from which the columns get their taller pane (the
/// table keeps 9 rows there); below it the detail stays stacked.
pub const COLUMNS_MIN_HEIGHT: u16 = 28;
/// The columns' pane: border 2, headings 1, 6 rows, footer 1.
const COLUMNS_DETAIL_H: u16 = 10;
/// Rows of each column in the columns' pane.
const COLUMN_ROWS: usize = (COLUMNS_DETAIL_H - 4) as usize;

/// Whether an open Flows detail in `body` is drawn in columns: wide and
/// tall enough. Shared by the layout and the drawing.
pub fn detail_columns(body: Rect, d: DetailLines) -> bool {
    d.open && body.width.saturating_sub(2) >= COLUMNS_MIN_WIDTH && body.height >= COLUMNS_MIN_HEIGHT
}

/// `layout` with the detail pane sized for `d`: the columns' taller pane,
/// and a `mark` line while marks are active; the table gives the lines
/// up, so the treemap's area (the two together) is the same.
pub fn layout_with(body: Rect, d: DetailLines) -> TrafficLayout {
    let detail_h = if body.height < 20 {
        0
    } else if detail_columns(body, d) {
        COLUMNS_DETAIL_H + u16::from(d.mark)
    } else {
        8 + u16::from(d.mark)
    };
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

/// What the detail pane holds: open while the Flows detail is shown, a
/// `mark` line while marks are active too (a closed one, "press Enter",
/// needs neither). Shared by `render` and `table_rows`, so drawing and
/// scrolling agree.
pub fn detail_extra(t: &crate::tui::model::TrafficState) -> DetailLines {
    let open = t.show_detail && t.view == TrafficView::Flows;
    let marks_on = t.marked.as_ref().is_some_and(|m| m.active > 0);
    DetailLines {
        open,
        mark: marks_on && open,
    }
}

/// Rows of data visible in the flow table (minus border and header), with
/// the detail pane sized for `d` (`detail_extra`).
pub fn table_rows(body: Rect, d: DetailLines) -> usize {
    layout_with(body, d).table.height.saturating_sub(3) as usize
}

pub fn render(model: &Model, f: &mut Frame, body: Rect) {
    let d = detail_extra(&model.traffic);
    let l = layout_with(body, d);
    render_filter_bar(model, f, l.filter);
    render_stats(model, f, l.stats);
    match model.traffic.view {
        TrafficView::Flows => {
            render_flows(model, f, l.table);
            if l.detail.height > 0 {
                render_detail(model, f, l.detail, detail_columns(body, d));
            }
        }
        // The detail pane belongs to the Flows view; the others use its space.
        TrafficView::Aggregate => render_groups(model, f, l.table.union(l.detail)),
        TrafficView::Mesh => render_mesh(model, f, l.table.union(l.detail)),
        TrafficView::Treemap => render_treemap(model, f, l.table.union(l.detail)),
        TrafficView::Blocked => render_blocked(model, f, l.table.union(l.detail)),
        TrafficView::Exposure => render_exposure(model, f, l.table.union(l.detail)),
    }
}

/// The Result header: the focus badge (`focus shop/prod · in`, or `focus is
/// partial: r re-queries shop/prod · in`) in front of the query's meta.
fn query_meta(model: &Model) -> String {
    let meta = result_meta(model);
    let meta = match model.traffic.focus_meta() {
        Some(focus) => format!("{focus} · {meta}"),
        None => meta,
    };
    // Exposure mode (#82): the result is the exposure query's.
    if model.traffic.exposure.is_some() {
        format!("exposure · {meta}")
    } else {
        meta
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
                let draft = d
                    .draft
                    .map(|s| format!("draft ✓ {} fixed · ", s.fixed))
                    .unwrap_or_default();
                format!(
                    "{} · {draft}{}{}{}{}",
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
    // Compare mode leads the header; a truncated side in the warning
    // colour, in place of the result's own meta (the line below counts).
    // The longest that fits beside the title: the baseline's end time and
    // the result's meta go first.
    let block = match model.traffic.compare_meta() {
        Some(cmp) => {
            let room = area.width.saturating_sub(12) as usize;
            let short = model.traffic.compare_meta_with(false).unwrap_or_default();
            let fits = |s: &str| Span::raw(s).width() <= room;
            let spans = match model.traffic.compare_warning() {
                Some(w) => {
                    let head = [cmp, short]
                        .into_iter()
                        .find(|h| fits(&format!("{h} · {w}")))
                        .unwrap_or_else(|| "vs baseline".into());
                    vec![
                        Span::styled(format!("{head} · "), t.dim_style()),
                        Span::styled(w, Style::new().fg(t.warn)),
                    ]
                }
                None => {
                    let full = format!("{cmp} · {meta}");
                    let text = [full, cmp, short.clone()]
                        .into_iter()
                        .find(|s| fits(s))
                        .unwrap_or(short);
                    vec![Span::styled(text, t.dim_style())]
                }
            };
            panel_spans("Result", spans, t, false)
        }
        None => panel("Result", &meta, t, false),
    };
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
    // The rows the tables aggregate (compare mode: without the keys
    // squelched on either side).
    let shown = model.traffic.shown_flows();
    let (mut a, mut p, mut b) = (0u64, 0u64, 0u64);
    for fl in shown.iter() {
        match fl.policy_decision {
            PolicyDecision::Allowed => a += 1,
            PolicyDecision::PotentiallyBlocked => p += 1,
            PolicyDecision::Blocked => b += 1,
            PolicyDecision::Unknown => {}
        }
    }
    // The count says the same flows the breakdown counts: the visible ones
    // of the loaded, the squelched ones left out named.
    let (total, squelched) = (d.flows.len(), model.traffic.squelched_hidden());
    let mut count = if model.traffic.focus.is_some() || squelched > 0 {
        format!("{} of {total} flows ", shown.len())
    } else {
        format!("{total} flows ")
    };
    if squelched > 0 {
        count.push_str(&format!("({squelched} squelched) "));
    }
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
        // The bar keeps 24 cells; 10 when the squelched count needs room.
        .min(
            inner
                .width
                .saturating_sub(if squelched > 0 { 10 } else { 24 }) as usize,
        ) as u16;
    let cols = Layout::horizontal([Constraint::Length(left_w), Constraint::Min(10)]).split(inner);
    f.render_widget(Paragraph::new(Line::from(left)), cols[0]);
    // No time sparkline: the flows are lifetime aggregates, so their
    // last-detected times say nothing honest about traffic over time.
    let bar = bars::decision_bar(a, p, b, cols[1].width as usize, t);
    f.render_widget(Paragraph::new(bar), cols[1]);
}

fn render_flows(model: &Model, f: &mut Frame, area: Rect) {
    let t = &model.theme;
    let tr = &model.traffic;
    let mut meta = if tr.flows_grouped() {
        // `by src app · by app A→Z` (the order when not the default), the
        // flat table's sort and search the flows keep (`flows /web`), then
        // the sections' search.
        let mut parts = vec![tr.flows_state.by.label().to_string()];
        if tr.flows_state.order != SectionOrder::default() {
            parts.push(tr.flows_state.order.label().to_string());
        }
        if let Some((c, desc)) = tr.flows.sort() {
            let arrow = if desc { "↓" } else { "↑" };
            parts.push(format!("sort {} {arrow}", tr.flows.columns[c].title));
        }
        if !tr.flows.search_text().is_empty() {
            parts.push(format!("flows /{}", tr.flows.search_text()));
        }
        let m = crate::tui::ui::widgets::table::header_meta(&tr.flows_grouped);
        if !m.is_empty() {
            parts.push(m);
        }
        parts.join(" · ")
    } else {
        crate::tui::ui::widgets::table::header_meta(&tr.flows)
    };
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
    if let Some(m) = model.traffic.marks_meta(TrafficView::Flows) {
        meta = if meta.is_empty() {
            m
        } else {
            format!("{m} · {meta}")
        };
    }
    let theme = t.clone();
    let dim = t.dim_style();
    // A flow whose verdict the draft changes is drawn in its draft colour,
    // bold; a gone one (compare mode) dim.
    let style = move |fl: &crate::tui::model::FlowRow| match fl.draft_change() {
        _ if gone(fl.change.as_ref()) => dim,
        Some(d) => theme
            .decision_style(d.class())
            .add_modifier(ratatui::style::Modifier::BOLD),
        None => theme.decision_style(fl.policy_decision),
    };
    if tr.flows_grouped() {
        render_sections(
            model,
            f,
            area,
            &title,
            &meta,
            &tr.flows_grouped,
            &move |r| r.flow().map_or_else(Style::new, &style),
        );
        return;
    }
    let title = unit_title(&title, &meta, area.width);
    let block = panel(&title, &meta, t, true);
    crate::tui::ui::widgets::table::render(f, area, &tr.flows, t, block, &style);
}

/// The count unit beside a traffic table's title, where it fits.
pub const UNIT_HINT: &str = "conns: lifetime";

/// `title · conns: lifetime` when the title, the hint and `meta` fit
/// `width` (borders and padding included), else `title` alone: the unit
/// never pushes the meta out.
pub fn unit_title(title: &str, meta: &str, width: u16) -> String {
    let w = |s: &str| Span::raw(s).width();
    let with = format!("{title} · {UNIT_HINT}");
    // Two corners, a space either side of each title, one gap between.
    if w(&with) + w(meta) + 7 <= width as usize {
        with
    } else {
        title.to_string()
    }
}

/// The panel meta of App-to-app or Blocked: `in · blocked · keys app,env ·
/// Enter drills` (`pairs · by service · blocked · ...`), then sort and
/// search as every table shows them.
fn view_meta<T>(model: &Model, by: &str, view: &TableView<T>) -> String {
    let st = model.traffic.section_decisions();
    let mut parts = vec![by.to_string()];
    // App-to-app's sections in another order than by connections.
    let order = model.traffic.apps_state.order;
    if model.traffic.view == TrafficView::Aggregate
        && model.traffic.apps_state.by != By::Pairs
        && order != SectionOrder::default()
    {
        parts.push(order.label().to_string());
    }
    if st != Decisions::All {
        parts.push(st.label().to_string());
    }
    // The marks' counts take the place of the keys and the Enter hint, so
    // the title keeps its room.
    match model.traffic.marks_meta(model.traffic.view) {
        Some(m) => parts.push(m),
        None => {
            parts.push(format!("keys {}", model.traffic.effective_keys().join(",")));
            parts.push("Enter drills".into());
        }
    }
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
    let title = unit_title(title, meta, area.width);
    let block = panel(&title, meta, t, true);
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
            &move |r| match r.peer() {
                Some((_, p)) if gone(p.change.as_ref()) => theme.dim_style(),
                Some((_, p)) => theme.decision_style(p.dominant()),
                None => Style::new(),
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
    let title = unit_title(TrafficView::Aggregate.title(), &meta, area.width);
    let block = panel(&title, &meta, t, true);
    let theme = t.clone();
    crate::tui::ui::widgets::table::render(f, area, table, t, block, &move |g| {
        if gone(g.change.as_ref()) {
            theme.dim_style()
        } else {
            theme.decision_style(g.dominant())
        }
    });
}

/// Rows are styled by their dominant decision, like App-to-app.
fn render_mesh(model: &Model, f: &mut Frame, area: Rect) {
    let t = &model.theme;
    let meta = crate::tui::ui::widgets::table::header_meta(&model.traffic.mesh);
    // The marks' counts take the place of the key hints.
    let hints = model
        .traffic
        .marks_meta(TrafficView::Mesh)
        .unwrap_or_else(|| "u user · p process · Enter drills".into());
    let meta = format!(
        "keys {} · {hints}{}{}",
        model.traffic.effective_keys().join(","),
        if meta.is_empty() { "" } else { " · " },
        meta
    );
    let title = unit_title(TrafficView::Mesh.title(), &meta, area.width);
    let block = panel(&title, &meta, t, true);
    let theme = t.clone();
    crate::tui::ui::widgets::table::render(f, area, &model.traffic.mesh, t, block, &move |r| {
        if gone(r.change.as_ref()) {
            theme.dim_style()
        } else {
            theme.decision_style(r.dominant())
        }
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
    let mut meta = treemap_meta(tm, t);
    if let Some(m) = model.traffic.marks_meta(TrafficView::Treemap) {
        meta.push(Span::styled(format!(" · {m}"), t.dim_style()));
    }
    let block = panel_spans(&title, meta, t, true);
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
    // A tile whose pair (or a source with a pair) has a highlight shows
    // its glyph before the label. Zoomed, the tiles are the destinations.
    let zoom = tm.zoom.as_ref().map(|z| z.join("/"));
    for (i, tile) in tm.tiles.iter().enumerate() {
        if tile.children.is_empty() {
            let mark = match &zoom {
                Some(z) => tm.highlight(z, Some(&tile.label)),
                None => tm.highlight(&tile.label, None),
            };
            draw_fill(f, inner, tile, tm.weight, t, is_cur(i, None), mark);
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
        let label = match tm.highlight(&tile.label, None) {
            Some(m) => format!("{} {}", m.glyph(), tile.label),
            None => tile.label.clone(),
        };
        let label = clip_text(&label, r.width.saturating_sub(2) as usize);
        f.render_widget(
            Block::bordered()
                .border_style(style)
                .title(Span::styled(label, style)),
            r,
        );
        for (j, c) in tile.children.iter().enumerate() {
            let mark = tm.highlight(&tile.label, Some(&c.label));
            draw_fill(f, inner, c, tm.weight, t, is_cur(i, Some(j)), mark);
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
/// when it fits with a cell either side (after the glyph of its
/// highlight), the weight on the second.
fn draw_fill(
    f: &mut Frame,
    clip: Rect,
    tile: &Tile,
    weight: Weight,
    t: &Theme,
    cursor: bool,
    mark: Option<pce::marks::Marker>,
) {
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
    let label = match mark {
        Some(m) => format!("{} {}", m.glyph(), tile.label),
        None => tile.label.clone(),
    };
    let mut lines = vec![row(Some(label))];
    if r.height >= 2 {
        lines.push(row(Some(match weight {
            Weight::Bytes => pce::traffic::human_bytes(tile.weight),
            _ => compact_count(tile.weight),
        })));
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
    let dim = model.theme.dim_style();
    render_sections(
        model,
        f,
        area,
        TrafficView::Blocked.title(),
        &meta,
        &model.traffic.blocked,
        &move |r| if gone(r.change()) { dim } else { bad },
    );
}

/// Exposure (#82): one row per destination group and risky service;
/// exposed rows in the bad colour, potentially blocked ones in the
/// warning one, blocked ones in the good one, intra ones dim. The meta is
/// the selected row's service summary (`rdp: 12 destinations · 4 exposed
/// …`), then the keys.
fn render_exposure(model: &Model, f: &mut Frame, area: Rect) {
    use pce::traffic::ExposureStatus as S;
    let t = &model.theme;
    let x = &model.traffic;
    let table = &x.exposure_rows;
    // The selected row's service summary, long or short; the marks' counts
    // or the keys; sort and search: the longest that fits.
    let (long, short) = match table.selected_item() {
        Some(r) => {
            let services = x.exposure.as_ref().map(|e| e.services.as_slice());
            let sum = pce::traffic::summarize(table.rows(), services.unwrap_or_default());
            match sum.into_iter().find(|s| s.service == r.service) {
                Some(s) => (Some(s.text()), Some(s.short())),
                None => (None, None),
            }
        }
        None if x.exposure.is_none() => (Some("off: 9 or :exposure".into()), None),
        None => (None, None),
    };
    let keys = x
        .marks_meta(TrafficView::Exposure)
        .unwrap_or_else(|| "Enter flows · E explain · Esc leaves".into());
    // Squelch never hides an exposure quietly: said in every form.
    let sq = (x.exposure_squelched > 0)
        .then(|| format!("{} exposed squelched (M)", x.exposure_squelched));
    let sq = sq.as_deref();
    let sort = crate::tui::ui::widgets::table::header_meta(table);
    let join = |parts: &[Option<&str>]| {
        parts
            .iter()
            .flatten()
            .filter(|p| !p.is_empty())
            .copied()
            .collect::<Vec<_>>()
            .join(" · ")
    };
    let (long, short) = (long.as_deref(), short.as_deref());
    let room = area.width.saturating_sub(16) as usize;
    let meta = [
        join(&[sq, long, Some(&keys), Some(&sort)]),
        join(&[sq, long, Some(&sort)]),
        join(&[sq, short, Some(&keys), Some(&sort)]),
        join(&[sq, short, Some(&sort)]),
    ]
    .into_iter()
    .find(|m| Span::raw(m.as_str()).width() <= room)
    .unwrap_or_else(|| join(&[sq, short]));
    let title = unit_title(TrafficView::Exposure.title(), &meta, area.width);
    let block = panel(&title, &meta, t, true);
    let theme = t.clone();
    crate::tui::ui::widgets::table::render(f, area, table, t, block, &move |r| match r.status {
        S::Exposed => Style::new().fg(theme.bad),
        S::PotentiallyBlocked => Style::new().fg(theme.warn),
        S::Blocked => Style::new().fg(theme.ok),
        S::Intra => theme.dim_style(),
    });
}

/// The selected flow's mark: `squelch m-7k2q9x · Nagios NRPE, known ·
/// expires 2027-01-15 (UTC)`, then `also matches: m-… (reviewed)` for the
/// weaker marks that match too. `None` without a hit.
fn mark_line(model: &Model, fl: &crate::tui::model::FlowRow) -> Option<String> {
    let hit = fl.mark?;
    let set = model.traffic.marks.set()?;
    let m = set.marks.get(hit.mark)?;
    let comment = if m.comment.trim().is_empty() {
        "(no comment)".to_string()
    } else {
        m.comment.clone()
    };
    let mut s = format!(
        "{} {} · {comment} · {}",
        hit.action.as_str(),
        m.id,
        pce::marks::expiry_text(m.expires)
    );
    let others: Vec<String> = model
        .traffic
        .flow_marks(&fl.flow)
        .into_iter()
        .filter(|i| *i != hit.mark)
        .filter_map(|i| set.marks.get(i))
        .map(|o| format!("{} ({})", o.id, o.action.as_str()))
        .collect();
    if !others.is_empty() {
        s.push_str(&format!(" · also matches: {}", others.join(", ")));
    }
    Some(s)
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

/// The Flows detail: a section header's two lines, the hint while closed,
/// else the selected flow, in columns (`columns`, see `detail_columns`)
/// or stacked rows.
fn render_detail(model: &Model, f: &mut Frame, area: Rect, columns: bool) {
    let t = &model.theme;
    let toggle = if model.traffic.show_detail {
        "Enter hides"
    } else {
        "Enter shows"
    };
    // Compare mode: the selected flow's change (`allowed→pot.blocked`).
    let meta = match model.traffic.selected_flow().and_then(|r| r.change) {
        Some(c) => format!("vs baseline: {} · {toggle}", c.short_text()),
        None => toggle.to_string(),
    };
    let block = panel("Detail", &meta, t, false);
    // A Flows section's header: what it holds and what the keys do.
    if let Some(line) = model
        .traffic
        .flows_grouped
        .selected_item()
        .filter(|_| model.traffic.flows_grouped() && model.traffic.show_detail)
        .and_then(|r| r.header_line())
    {
        let lines = vec![
            Line::from(Span::raw(
                line.trim_start_matches(['▾', '▹', ' ']).to_string(),
            )),
            Line::from(Span::styled(
                "Enter drills to the app's flows · Space folds · z folds all · s orders",
                t.dim_style(),
            )),
        ];
        f.render_widget(
            Paragraph::new(lines).block(block).wrap(Wrap { trim: true }),
            area,
        );
        return;
    }
    let Some(fl) = model
        .traffic
        .selected_flow()
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
    if columns {
        let lines = detail_column_lines(model, fl, &order, &rules, block.inner(area).width);
        f.render_widget(Paragraph::new(lines).block(block), area);
        return;
    }
    // One line per row, cut with `…` where it does not fit (no wrapping:
    // a wrapped row pushes the ones under it, the mark, out of the pane).
    let body_w = (block.inner(area).width as usize).saturating_sub(11);
    let row = |head: &'static str, body: String| {
        Line::from(vec![
            Span::styled(format!("{head:<11}"), t.header_style()),
            Span::raw(clip_ellipsis(&body, body_w)),
        ])
    };
    let mut lines = vec![
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
        row("seen", seen_text(fl, &rules)),
    ];
    if let Some(text) = mark_line(model, fl) {
        lines.push(row("mark", text));
    }
    f.render_widget(Paragraph::new(lines).block(block), area);
}

/// How a detail cell is drawn; theme-free, so the builders are testable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CellStyle {
    Plain,
    Dim,
    /// In the decision's colour, bold when `true` (a draft change).
    Decision(PolicyDecision, bool),
}

/// One line of a detail column: `key` (aligned, dim) before `text`, or
/// `text` alone when `key` is empty.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Cell {
    key: String,
    text: String,
    style: CellStyle,
}

impl Cell {
    fn new(text: impl Into<String>, style: CellStyle) -> Self {
        Self {
            key: String::new(),
            text: text.into(),
            style,
        }
    }

    fn label(key: &str, value: &str) -> Self {
        Self {
            key: key.to_string(),
            text: value.to_string(),
            style: CellStyle::Plain,
        }
    }
}

/// An endpoint's column in at most `rows` lines: its name, its IP (unless
/// that is the name), its labels one per line in `order` (minus the keys
/// `order` hides; `unlabeled` without any), then its IP lists. What does
/// not fit ends in `+N more`.
fn endpoint_cells(e: &ResolvedEndpoint, order: &KeyOrder, rows: usize) -> Vec<Cell> {
    let name = e.display_name();
    let mut head = vec![Cell::new(name.clone(), CellStyle::Plain)];
    if let Some(ip) = e.ip.as_deref().filter(|ip| !ip.is_empty() && *ip != name) {
        head.push(Cell::new(ip, CellStyle::Plain));
    }
    let mut labels: Vec<_> = e
        .labels
        .iter()
        .filter(|l| order.shows(&l.key))
        .cloned()
        .collect();
    order.sort_labels(&mut labels);
    // `(cell, counts)`: whether `+N more` counts it (the `ip lists` heading not).
    let mut body: Vec<(Cell, bool)> = labels
        .iter()
        .map(|l| (Cell::label(&l.key, &l.value), true))
        .collect();
    if !e.ip_lists.is_empty() {
        body.push((Cell::new("ip lists", CellStyle::Dim), false));
        body.extend(
            e.ip_lists
                .iter()
                .map(|n| (Cell::new(n.clone(), CellStyle::Plain), true)),
        );
    } else if labels.is_empty() {
        body.push((Cell::new("unlabeled", CellStyle::Dim), false));
    }
    let rows = rows.max(1);
    let fits = head.len() + body.len() <= rows;
    if !fits {
        head.truncate(rows.saturating_sub(1).max(1));
    }
    let keep = if fits {
        body.len()
    } else {
        rows.saturating_sub(1).saturating_sub(head.len())
    };
    let hidden = body[keep..].iter().filter(|(_, n)| *n).count();
    body.truncate(keep);
    // No heading (`ip lists`) left without the names under it.
    if !fits && body.last().is_some_and(|(_, n)| !n) {
        body.pop();
    }
    // The keys shown, aligned.
    let width = body
        .iter()
        .map(|(c, _)| text_width(&c.key))
        .max()
        .unwrap_or(0);
    head.extend(body.into_iter().map(|(mut c, _)| {
        if !c.key.is_empty() {
            c.key = pad_to(&c.key, width);
        }
        c
    }));
    if hidden > 0 && head.len() < rows {
        head.push(Cell::new(format!("+{hidden} more"), CellStyle::Dim));
    }
    head
}

/// The service column: `5432/tcp`, `process  postgres` and `user  app`
/// (each when known), the reported decision, then the draft's (`draft blocked →
/// allowed`, as the Flows table's draft column) when the flow has one.
fn service_cells(fl: &ResolvedFlow) -> Vec<Cell> {
    let mut cells = vec![Cell::new(fl.service_text(), CellStyle::Plain)];
    let known = |v: &Option<String>| {
        v.as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty() && !s.chars().all(|c| c == '-'))
            .map(str::to_string)
    };
    let who: Vec<(&str, String)> = [("process", &fl.process_name), ("user", &fl.user_name)]
        .into_iter()
        .filter_map(|(k, v)| known(v).map(|v| (k, v)))
        .collect();
    let width = who.iter().map(|(k, _)| text_width(k)).max().unwrap_or(0);
    cells.extend(who.iter().map(|(k, v)| Cell::label(&pad_to(k, width), v)));
    cells.push(Cell::new(
        fl.policy_decision.as_str(),
        CellStyle::Decision(fl.policy_decision, false),
    ));
    if let Some(d) = &fl.draft_policy_decision {
        let changed = fl.draft_change().is_some();
        cells.push(Cell::new(
            format!("draft {}", fl.draft_text()),
            CellStyle::Decision(d.class(), changed),
        ));
    }
    cells
}

/// `2026-10-16T08:12:33Z` -> `2026-10-16 08:12` and `true`; anything
/// else as given and `false`.
fn seen_time(s: &str) -> (String, bool) {
    match (s.get(..10), s.get(10..11), s.get(11..16)) {
        (Some(day), Some("T"), Some(hm)) if day.as_bytes().get(4) == Some(&b'-') => {
            (format!("{day} {hm}"), true)
        }
        _ => (s.to_string(), false),
    }
}

/// The detail's `seen` row, both layouts: `2026-10-16 08:12 → 10-17 09:40
/// · bytes in 117.2K out 3.9K · rules …` (the last time without the year
/// it shares with the first; `-` for a time the PCE did not report).
fn seen_text(fl: &ResolvedFlow, rules: &str) -> String {
    let time = |t: &Option<String>| match t.as_deref().filter(|t| !t.is_empty()) {
        Some(t) => seen_time(t),
        None => ("-".to_string(), false),
    };
    let (first, first_iso) = time(&fl.first_detected);
    let (mut last, last_iso) = time(&fl.last_detected);
    let missing = |t: &Option<String>| t.as_deref().is_none_or(str::is_empty);
    let mut s = if missing(&fl.first_detected) && missing(&fl.last_detected) {
        "-".to_string()
    } else {
        if first_iso && last_iso && first.get(..5) == last.get(..5) {
            last = last[5..].to_string();
        }
        format!("{first} → {last}")
    };
    if fl.has_bytes() {
        let n = |b: Option<u64>| b.map_or_else(|| "-".to_string(), pce::traffic::human_bytes);
        s.push_str(&format!(
            " · bytes in {} out {}",
            n(fl.bytes_in),
            n(fl.bytes_out)
        ));
    }
    s.push_str(&format!(" · rules {rules}"));
    s
}

/// `s`'s display width in cells (ratatui's measure).
fn text_width(s: &str) -> usize {
    Span::raw(s).width()
}

/// `s` padded with spaces to `w` cells of display width.
fn pad_to(s: &str, w: usize) -> String {
    format!("{s}{}", " ".repeat(w.saturating_sub(text_width(s))))
}

/// `s` in at most `w` cells, its end cut to `…` when it does not fit.
fn clip_ellipsis(s: &str, w: usize) -> String {
    use crate::tui::ui::widgets::table::clip_width;
    if Span::raw(s).width() <= w {
        return s.to_string();
    }
    if w == 0 {
        return String::new();
    }
    format!("{}…", clip_width(s, w - 1))
}

/// `cell` as spans of exactly `w` cells: clipped with `…` one short of
/// `w` (a gap before the next column), then padded.
fn cell_spans(cell: Option<&Cell>, w: usize, t: &Theme) -> Vec<Span<'static>> {
    let Some(c) = cell else {
        return vec![Span::raw(" ".repeat(w))];
    };
    let room = w.saturating_sub(1);
    let style = match c.style {
        CellStyle::Plain => Style::new(),
        CellStyle::Dim => t.dim_style(),
        CellStyle::Decision(d, bold) if bold => t.decision_style(d).add_modifier(Modifier::BOLD),
        CellStyle::Decision(d, _) => t.decision_style(d),
    };
    let mut spans = Vec::new();
    let mut used = 0;
    if !c.key.is_empty() {
        let key = clip_ellipsis(&format!("{}  ", c.key), room);
        used += Span::raw(key.as_str()).width();
        spans.push(Span::styled(key, t.dim_style()));
    }
    let text = clip_ellipsis(&c.text, room.saturating_sub(used));
    used += Span::raw(text.as_str()).width();
    spans.push(Span::styled(text, style));
    spans.push(Span::raw(" ".repeat(w.saturating_sub(used))));
    spans
}

/// The source, destination and service widths in `width` inner cells,
/// less the two `│ ` separators: the service column about a quarter,
/// the endpoints equal.
fn column_widths(width: u16) -> [usize; 3] {
    let avail = (width as usize).saturating_sub(4);
    let ep = (avail - avail * 26 / 100) / 2;
    [ep, ep, avail - 2 * ep]
}

/// The Flows detail in columns, in `width` inner cells: the headings,
/// `COLUMN_ROWS` rows of source | destination | service, the `seen`
/// footer, then the `mark` line when the flow has one.
fn detail_column_lines(
    model: &Model,
    fl: &crate::tui::model::FlowRow,
    order: &KeyOrder,
    rules: &str,
    width: u16,
) -> Vec<Line<'static>> {
    let t = &model.theme;
    let w = column_widths(width);
    let sep = || Span::styled("│ ", t.dim_style());
    let heading = |s: &str, w: usize| Span::styled(fit_cell(s, w), t.header_style());
    let mut lines = vec![Line::from(vec![
        heading("SOURCE", w[0]),
        sep(),
        heading("DESTINATION", w[1]),
        sep(),
        heading("SERVICE", w[2]),
    ])];
    let src = endpoint_cells(&fl.flow.src, order, COLUMN_ROWS);
    let dst = endpoint_cells(&fl.flow.dst, order, COLUMN_ROWS);
    let svc = service_cells(&fl.flow);
    for i in 0..COLUMN_ROWS {
        let mut spans = cell_spans(src.get(i), w[0], t);
        spans.push(sep());
        spans.extend(cell_spans(dst.get(i), w[1], t));
        spans.push(sep());
        spans.extend(cell_spans(svc.get(i), w[2], t));
        lines.push(Line::from(spans));
    }
    let width = width as usize;
    lines.push(Line::from(Span::raw(clip_ellipsis(
        &format!("seen {}", seen_text(&fl.flow, rules)),
        width,
    ))));
    if let Some(text) = mark_line(model, fl) {
        lines.push(Line::from(vec![
            Span::styled("mark ", t.header_style()),
            Span::raw(clip_ellipsis(&text, width.saturating_sub(5))),
        ]));
    }
    lines
}

/// `s` clipped (with `…`) and padded to exactly `w` cells. Not
/// `widgets::table::fit`: that one cuts without `…` and leaves no gap.
fn fit_cell(s: &str, w: usize) -> String {
    let s = clip_ellipsis(s, w.saturating_sub(1));
    let pad = w.saturating_sub(Span::raw(s.as_str()).width());
    format!("{s}{}", " ".repeat(pad))
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
    let mut spans = Vec::new();
    // The saved filter applied, while it is the one in force.
    if let Some(name) = crate::tui::saved::tag(model, crate::tui::model::Screen::Traffic) {
        spans.push(Span::styled(
            format!("[{name}]  "),
            Style::new().fg(t.accent),
        ));
    }
    spans.extend([
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
    f.render_widget(
        Paragraph::new(Line::from(spans))
            .block(block)
            .style(t.base()),
        area,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn meta_text(spans: &[Span]) -> String {
        spans.iter().map(|s| s.content.as_ref()).collect()
    }

    #[test]
    fn the_unit_joins_the_title_only_where_it_fits() {
        assert_eq!(unit_title("Flows", "", 80), "Flows · conns: lifetime");
        let meta = "pairs · by service · keys app,env · Enter drills";
        assert_eq!(unit_title("App-to-app", meta, 80), "App-to-app");
        assert_eq!(
            unit_title("App-to-app", meta, 120),
            "App-to-app · conns: lifetime"
        );
        // Exactly full: 5 + 3 + 15 + 4 + 7 = 34.
        assert_eq!(unit_title("Flows", "abcd", 34), "Flows · conns: lifetime");
        assert_eq!(unit_title("Flows", "abcd", 33), "Flows");
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

    fn ep(labels: &[(&str, &str)]) -> ResolvedEndpoint {
        ResolvedEndpoint {
            ip: Some("10.0.0.1".into()),
            hostname: Some("web-01".into()),
            workload_href: Some("/orgs/1/workloads/w1".into()),
            labels: labels
                .iter()
                .map(|(k, v)| pce::resolve::ResolvedLabel {
                    href: format!("/orgs/1/labels/{k}"),
                    key: k.to_string(),
                    value: v.to_string(),
                })
                .collect(),
            ..Default::default()
        }
    }

    fn texts(cells: &[Cell]) -> Vec<String> {
        cells
            .iter()
            .map(|c| {
                if c.key.is_empty() {
                    c.text.clone()
                } else {
                    format!("{}  {}", c.key, c.text)
                }
            })
            .collect()
    }

    fn keys(k: &[&str]) -> Vec<String> {
        k.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn endpoint_cells_list_labels_in_key_order_with_aligned_keys() {
        let order = KeyOrder::new(&keys(&["app", "env"]), "role");
        let e = ep(&[
            ("loc", "fra"),
            ("role", "web"),
            ("env", "prod"),
            ("app", "shop"),
        ]);
        assert_eq!(
            texts(&endpoint_cells(&e, &order, 6)),
            [
                "web-01",
                "10.0.0.1",
                "app   shop",
                "env   prod",
                "role  web",
                "loc   fra"
            ]
        );
    }

    #[test]
    fn endpoint_cells_hide_ignored_keys_and_say_unlabeled() {
        let order = KeyOrder::new(&keys(&["app"]), "role").with_ignored(&keys(&["bu"]));
        let e = ep(&[("bu", "retail"), ("app", "shop")]);
        assert_eq!(
            texts(&endpoint_cells(&e, &order, 6)),
            ["web-01", "10.0.0.1", "app  shop"]
        );
        let only_ignored = ep(&[("bu", "retail")]);
        let cells = endpoint_cells(&only_ignored, &order, 6);
        assert_eq!(texts(&cells), ["web-01", "10.0.0.1", "unlabeled"]);
        assert_eq!(cells[2].style, CellStyle::Dim);
    }

    #[test]
    fn endpoint_cells_omit_an_ip_that_is_the_name() {
        let order = KeyOrder::new(&[], "");
        let e = ResolvedEndpoint {
            ip: Some("10.1.4.9".into()),
            ..Default::default()
        };
        assert_eq!(
            texts(&endpoint_cells(&e, &order, 6)),
            ["10.1.4.9", "unlabeled"]
        );
        let none = ResolvedEndpoint {
            hostname: Some("db".into()),
            ..Default::default()
        };
        assert_eq!(
            texts(&endpoint_cells(&none, &order, 6)),
            ["db", "unlabeled"]
        );
    }

    #[test]
    fn endpoint_cells_show_ip_lists_where_labels_would_be() {
        let order = KeyOrder::new(&[], "");
        let e = ResolvedEndpoint {
            ip: Some("10.1.4.9".into()),
            ip_lists: keys(&["Corp", "VPN"]),
            ..Default::default()
        };
        let cells = endpoint_cells(&e, &order, 6);
        assert_eq!(texts(&cells), ["10.1.4.9", "ip lists", "Corp", "VPN"]);
        assert_eq!(cells[1].style, CellStyle::Dim);
        // Cut: the heading never stands alone before `+N more`.
        assert_eq!(
            texts(&endpoint_cells(&e, &order, 3)),
            ["10.1.4.9", "+2 more"]
        );
        let three = ResolvedEndpoint {
            ip_lists: keys(&["Corp", "VPN", "Lab"]),
            ..e.clone()
        };
        assert_eq!(
            texts(&endpoint_cells(&three, &order, 4)),
            ["10.1.4.9", "ip lists", "Corp", "+2 more"]
        );
    }

    #[test]
    fn label_keys_align_by_display_width() {
        let order = KeyOrder::new(&[], "");
        let e = ep(&[("ort", "x"), ("区域", "y")]);
        let cells = endpoint_cells(&e, &order, 6);
        assert_eq!(cells[2].key, "ort ");
        assert_eq!(cells[3].key, "区域");
    }

    #[test]
    fn endpoint_cells_end_in_n_more_when_they_do_not_fit() {
        let order = KeyOrder::new(&keys(&["app", "env"]), "role");
        let e = ep(&[
            ("app", "shop"),
            ("env", "prod"),
            ("role", "web"),
            ("datacenter", "fra1"),
            ("os", "linux"),
            ("zone", "a"),
        ]);
        // The keys shown are aligned among themselves (not to `datacenter`).
        let cells = endpoint_cells(&e, &order, 6);
        assert_eq!(
            texts(&cells),
            [
                "web-01",
                "10.0.0.1",
                "app   shop",
                "env   prod",
                "role  web",
                "+3 more"
            ]
        );
        assert_eq!(cells[5].style, CellStyle::Dim);
        // Exactly full: no `+N more`, and the keys align to `datacenter`.
        let all = texts(&endpoint_cells(&e, &order, 8));
        assert_eq!(all.len(), 8);
        assert_eq!(all[2], "app         shop");
        assert_eq!(all[5], "datacenter  fra1");
        assert_eq!(texts(&endpoint_cells(&e, &order, 1)), ["web-01"]);
        assert_eq!(texts(&endpoint_cells(&e, &order, 2)), ["web-01", "+6 more"]);
    }

    fn flow() -> ResolvedFlow {
        ResolvedFlow {
            src: ep(&[]),
            dst: ep(&[]),
            port: Some(5432),
            proto: "tcp".into(),
            process_name: Some("postgres".into()),
            user_name: Some("-".into()),
            num_connections: 1,
            policy_decision: PolicyDecision::Blocked,
            draft_policy_decision: None,
            flow_direction: None,
            first_detected: Some("2026-10-16T08:12:33Z".into()),
            last_detected: Some("2026-10-17T09:40:00Z".into()),
            rules: vec![],
            side_rules: Vec::new(),
            bytes_in: Some(117_200),
            bytes_out: None,
        }
    }

    #[test]
    fn service_cells_show_process_user_decision_and_draft() {
        let mut fl = flow();
        let cells = service_cells(&fl);
        // The dashes-only user is left out.
        assert_eq!(texts(&cells), ["5432/tcp", "process  postgres", "blocked"]);
        assert_eq!(
            cells[2].style,
            CellStyle::Decision(PolicyDecision::Blocked, false)
        );
        fl.process_name = None;
        fl.user_name = None;
        assert_eq!(texts(&service_cells(&fl)), ["5432/tcp", "blocked"]);
        fl.user_name = Some("svc".into());
        assert_eq!(
            texts(&service_cells(&fl)),
            ["5432/tcp", "user  svc", "blocked"]
        );
        fl.process_name = Some("java".into());
        fl.draft_policy_decision = Some(pce::models::DraftDecision::Allowed);
        let cells = service_cells(&fl);
        assert_eq!(
            texts(&cells),
            [
                "5432/tcp",
                "process  java",
                "user     svc",
                "blocked",
                "draft blocked → allowed"
            ]
        );
        assert_eq!(
            cells[4].style,
            CellStyle::Decision(PolicyDecision::Allowed, true)
        );
        fl.draft_policy_decision = Some(pce::models::DraftDecision::Blocked);
        let cells = service_cells(&fl);
        assert_eq!(cells[4].text, "draft blocked");
        assert_eq!(
            cells[4].style,
            CellStyle::Decision(PolicyDecision::Blocked, false)
        );
    }

    #[test]
    fn the_seen_text_shortens_the_times_and_drops_a_shared_year() {
        let mut fl = flow();
        assert_eq!(
            seen_text(&fl, "n/a"),
            "2026-10-16 08:12 → 10-17 09:40 · bytes in 114K out - · rules n/a"
        );
        fl.last_detected = Some("2027-01-02T00:00:00Z".into());
        fl.bytes_in = None;
        assert_eq!(
            seen_text(&fl, "r1"),
            "2026-10-16 08:12 → 2027-01-02 00:00 · rules r1"
        );
        // Not ISO: as given, no year dropped.
        fl.first_detected = Some("2026-10-16 garbage!".into());
        fl.last_detected = Some("2026-10-17T09:40:00Z".into());
        assert_eq!(
            seen_text(&fl, "r1"),
            "2026-10-16 garbage! → 2026-10-17 09:40 · rules r1"
        );
        fl.first_detected = None;
        assert_eq!(seen_text(&fl, "r1"), "- → 2026-10-17 09:40 · rules r1");
        fl.last_detected = Some(String::new());
        assert_eq!(seen_text(&fl, "r1"), "- · rules r1");
    }

    #[test]
    fn columns_need_an_open_detail_a_wide_and_a_tall_body() {
        let open = DetailLines {
            open: true,
            mark: false,
        };
        let body = |w, h| Rect::new(0, 0, w, h);
        assert!(detail_columns(body(92, 28), open));
        assert!(!detail_columns(body(91, 28), open));
        assert!(!detail_columns(body(120, 27), open));
        assert!(!detail_columns(body(120, 40), DetailLines::default()));
        assert_eq!(column_widths(118), [42, 42, 30]);
        assert_eq!(column_widths(90), [32, 32, 22]);
    }

    /// The table's rows and the pane drawn are cut from the same layout:
    /// every size, open or closed, with or without a mark line.
    #[test]
    fn table_rows_and_the_detail_height_agree() {
        for w in [60, 91, 92, 120, 200] {
            for h in 5..=60 {
                let body = Rect::new(0, 0, w, h);
                for (open, mark) in [(false, false), (true, false), (true, true)] {
                    let d = DetailLines { open, mark };
                    let l = layout_with(body, d);
                    assert_eq!(
                        table_rows(body, d),
                        l.table.height.saturating_sub(3) as usize
                    );
                    let want = match (h >= 20, detail_columns(body, d)) {
                        (false, _) => 0,
                        (true, true) => 10 + u16::from(mark),
                        (true, false) => 8 + u16::from(mark),
                    };
                    assert_eq!(l.detail.height, want, "{w}x{h} {d:?}");
                    assert_eq!(l.table.height + l.detail.height + 6, h.max(10), "{w}x{h}");
                }
            }
        }
    }
}
