//! Workloads screen: one table of workloads and an optional detail pane for
//! the selected one (href, OS, VEN status, enforcement, interfaces, labels).
use crate::tui::model::{Model, WlRow, WorkloadRowView, WorkloadsState};
use crate::tui::ui::screens::objects::{DetailLine, clip, items};
use crate::tui::ui::theme::Theme;
use crate::tui::ui::widgets::bars::age_text;
use crate::tui::ui::widgets::panel::panel;
use crate::tui::ui::widgets::table::{header_meta, render, render_spanning_spans};
use crate::tui::wl_groups::{
    WlGroup, WlGroupRow, bar_counts, changes_text, enforcement_text, health_text,
};
use pce::models::{VenHealth, Workload};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Border plus ten rows of detail.
const DETAIL_HEIGHT: u16 = 12;

/// `(table, detail)` areas. Shared by `view` and `update` (page sizes).
/// The detail pane is dropped when the terminal is too short to keep a usable table.
pub fn layout(body: Rect, show_detail: bool) -> (Rect, Rect) {
    let detail_h = if show_detail && body.height >= 16 {
        DETAIL_HEIGHT
    } else {
        0
    };
    let rows = Layout::vertical([Constraint::Min(4), Constraint::Length(detail_h)]).split(body);
    (rows[0], rows[1])
}

/// Rows of data visible in the table (minus the summary line, the filter
/// line while a filter is on, border and header).
pub fn table_rows(body: Rect, w: &WorkloadsState) -> usize {
    let lines = 4 + u16::from(w.filtered());
    layout(body, w.show_detail).0.height.saturating_sub(lines) as usize
}

pub fn render_screen(model: &Model, f: &mut Frame, body: Rect) {
    let t = &model.theme;
    if model.index().is_none() {
        let note = if model.workloads.index_failed {
            "no workloads loaded · r retries"
        } else {
            "loading workloads…"
        };
        f.render_widget(
            Paragraph::new(Span::styled(note, t.dim_style())).block(panel(
                "Workloads",
                "",
                t,
                true,
            )),
            body,
        );
        return;
    }
    let (area, detail) = layout(body, model.workloads.show_detail);
    let filter = model.workloads.filter_line();
    let [summary, filter_area, table] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(u16::from(filter.is_some())),
        Constraint::Min(3),
    ])
    .areas(area);
    f.render_widget(Paragraph::new(summary_line(model, summary.width)), summary);
    if let Some(text) = filter {
        let mut spans = vec![Span::styled("filter ", t.header_style())];
        if let Some(name) = crate::tui::saved::tag(model, crate::tui::model::Screen::Workloads) {
            spans.push(Span::styled(
                format!("[{name}] "),
                Style::new().fg(t.accent),
            ));
        }
        spans.push(Span::raw(text));
        // Unknown keys and values match nothing: said, dimmed, not refused.
        if !model.workloads.label_warnings.is_empty() {
            spans.push(Span::styled(
                format!(" · no match: {}", model.workloads.label_warnings.join(", ")),
                t.dim_style(),
            ));
        }
        f.render_widget(Paragraph::new(Line::from(spans)), filter_area);
    }
    render_table(model, f, table);
    if detail.height > 0 {
        render_detail(model, f, detail);
    }
}

fn render_table(model: &Model, f: &mut Frame, area: Rect) {
    let t = &model.theme;
    let w = &model.workloads;
    let freshness = if w.loading {
        "loading…".to_string()
    } else {
        w.loaded_at
            .map(|l| {
                format!(
                    "cached {}",
                    age_text(Duration::from_secs(w.age_secs) + l.elapsed())
                )
            })
            .unwrap_or_default()
    };
    let (shown, total) = (w.table.len(), w.table.rows().len());
    let mut rows = if shown == total {
        format!("{total}")
    } else {
        format!("{shown}/{total}")
    };
    if w.grouped {
        let n = w.groups.len();
        rows = format!(
            "{} · {n} group{} · {rows}",
            w.group_order.label(),
            if n == 1 { "" } else { "s" }
        );
    }
    let mut meta = if freshness.is_empty() {
        rows
    } else {
        format!("{rows} · {freshness}")
    };
    meta.push_str(&w.auto.meta(Instant::now()));
    let sort_meta = header_meta(&w.table);
    if !sort_meta.is_empty() {
        meta = format!("{sort_meta} · {meta}");
    }
    // Compare mode first: what the rows are against; narrow, with the
    // row count only.
    if let Some(c) = w.compare_meta() {
        meta = if area.width < 100 {
            format!("{c} · {shown}")
        } else {
            format!("{c} · {meta}")
        };
    }
    if w.filtered() && w.table.is_empty() && w.table.search_text().is_empty() {
        let note = if w.filters.is_empty() {
            "no unhealthy VENs · h shows all workloads"
        } else {
            "no workloads match the filters · c clears them"
        };
        f.render_widget(
            Paragraph::new(Span::styled(note, t.dim_style())).block(panel(
                "Workloads",
                &meta,
                t,
                true,
            )),
            area,
        );
        return;
    }
    let theme = t.clone();
    let title = if w.grouped {
        format!("Workloads by {}", w.group_keys.join(" | "))
    } else {
        "Workloads".to_string()
    };
    let block = panel(&title, &meta, t, true);
    if w.grouped {
        let accent = Style::new().fg(t.accent).add_modifier(Modifier::BOLD);
        let theme2 = t.clone();
        let cols = HeaderCols::of(&w.groups);
        render_spanning_spans(
            f,
            area,
            &w.sections,
            t,
            block,
            &move |r: &WlGroupRow| r.member().map_or_else(Style::new, |w| row_style(w, &theme)),
            &|r, width| match r {
                WlGroupRow::Header { group, folded } => {
                    Some((header_spans(group, *folded, cols, width, &theme2), accent))
                }
                WlGroupRow::Member { .. } => None,
            },
        );
        return;
    }
    render(f, area, &w.table, t, block, &move |r: &WlRow| {
        row_style(r, &theme)
    });
}

/// A workload row's colour: its VEN health (idle enforcement as bad); a
/// gone one (compare mode) dim.
fn row_style(r: &WorkloadRowView, theme: &Theme) -> Style {
    if crate::tui::model::wl_gone(r) {
        return theme.dim_style();
    }
    match r.health {
        VenHealth::Error | VenHealth::Offline => Style::new().fg(theme.bad),
        VenHealth::Warning => Style::new().fg(theme.warn),
        VenHealth::Unmanaged => theme.dim_style(),
        VenHealth::Healthy if r.workload.enforcement() == Some("idle") => {
            Style::new().fg(theme.bad)
        }
        VenHealth::Healthy => Style::default(),
    }
}

/// Health bar cells in a section header.
const HEADER_BAR: usize = 10;

/// The widest name, count, health and enforcement text over the section
/// headers: each header pads its parts to them and keeps the parts that
/// fit at those widths, so every header shows the same columns, lined up
/// down the list, folded or not. A name over [`NAME_MAX`] cells keeps its
/// own width.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct HeaderCols {
    name: usize,
    count: usize,
    health: usize,
    enforcement: usize,
}

/// The widest name the header columns pad to.
const NAME_MAX: usize = 40;

impl HeaderCols {
    pub fn of(groups: &[Arc<WlGroup>]) -> Self {
        groups.iter().fold(Self::default(), |c, g| Self {
            name: c.name.max(cells(&name_text(g, false)).min(NAME_MAX)),
            count: c.count.max(cells(&count_text(g))),
            health: c.health.max(cells(&health_text(g))),
            enforcement: c.enforcement.max(cells(&enforcement_text(g))),
        })
    }
}

fn cells(s: &str) -> usize {
    Span::raw(s.to_string()).width()
}

/// `s` with spaces up to `n` cells.
fn pad(s: String, n: usize) -> String {
    let fill = n.saturating_sub(cells(&s));
    s + &" ".repeat(fill)
}

fn name_text(g: &WlGroup, folded: bool) -> String {
    format!("{} {}", if folded { "▹" } else { "▾" }, g.title)
}

/// `12 wl`, and in compare mode its changes and the workloads gone since:
/// `12 wl · 2 new 1 chg · 1 gone`.
fn count_text(g: &WlGroup) -> String {
    let mut s = format!("{} wl", g.count());
    let changes = changes_text(g);
    if !changes.is_empty() {
        s.push_str(&format!(" · {changes}"));
    }
    if g.changes.gone > 0 {
        s.push_str(&format!(" · {} gone", g.changes.gone));
    }
    s
}

/// A section header in `width` cells: `▾ shop | prod   12 wl  ███▒  10 ok
/// · 1 offline   full 8 vis 3 idle 1`; `▹` when folded. The name, count
/// and health pad to `cols`, and the parts kept are those that fit at
/// `cols`, so the parts line up across headers.
/// Narrow, the parts go right to left: enforcement, health, bar, count;
/// the name stays (cut at the end).
pub fn header_spans(
    g: &WlGroup,
    folded: bool,
    cols: HeaderCols,
    width: usize,
    t: &Theme,
) -> Vec<Span<'static>> {
    let name = pad(name_text(g, folded), cols.name);
    let count = pad(count_text(g), cols.count);
    let health = health_text(g);
    let enforcement = enforcement_text(g);
    let w = |s: &str| cells(s);
    // name, then each optional part with the gap before it.
    let parts: [(usize, usize); 4] = [
        (3, w(&count)),
        (2, HEADER_BAR),
        (2, w(&health).max(cols.health)),
        (3, w(&enforcement).max(cols.enforcement)),
    ];
    let mut keep = parts.len();
    while keep > 0 {
        let total = w(&name) + parts[..keep].iter().map(|(g, n)| g + n).sum::<usize>();
        let empty = match keep {
            3 => health.is_empty(),
            4 => enforcement.is_empty(),
            _ => false,
        };
        if total <= width && !empty {
            break;
        }
        keep -= 1;
    }
    let mut spans = vec![Span::raw(name)];
    if keep >= 1 {
        spans.push(Span::styled(format!("   {count}"), Style::new().fg(t.fg)));
    }
    if keep >= 2 {
        spans.push(Span::raw("  "));
        spans.extend(health_bar(bar_counts(g), HEADER_BAR, t));
    }
    if keep >= 3 {
        // Padded only when the enforcement follows: no trailing blanks.
        let health = if keep >= 4 {
            pad(health, cols.health)
        } else {
            health
        };
        spans.push(Span::styled(format!("  {health}"), Style::new().fg(t.fg)));
    }
    if keep >= 4 {
        spans.push(Span::styled(format!("   {enforcement}"), t.dim_style()));
    }
    spans
}

fn render_detail(model: &Model, f: &mut Frame, area: Rect) {
    let t = &model.theme;
    let block = panel("Detail", "Enter hides", t, false);
    let capacity = block.inner(area).height as usize;
    let Some(wl) = model.workloads.selected_row() else {
        let note = match model.workloads.selected_group() {
            Some(g) => format!(
                "{} · {} workloads · Enter narrows the list to it",
                g.title,
                g.count()
            ),
            None => "no workload selected".to_string(),
        };
        f.render_widget(
            Paragraph::new(Span::styled(note, t.dim_style())).block(block),
            area,
        );
        return;
    };
    let text: Vec<Line> = clip(detail_lines(wl), capacity)
        .into_iter()
        .map(|(head, body)| match head {
            Some(h) => Line::from(vec![
                Span::styled(format!("{h:<8}"), t.header_style()),
                Span::raw(body),
            ]),
            None => Line::from(vec![Span::raw("        "), Span::raw(body)]),
        })
        .collect();
    f.render_widget(Paragraph::new(text).block(block), area);
}

fn ven_text(w: &Workload) -> String {
    match (w.agent_status(), w.managed) {
        (Some(s), _) => s,
        (None, Some(false)) => "unmanaged".into(),
        (None, _) => "-".into(),
    }
}

fn interface_text(i: &pce::models::Interface) -> String {
    let mut s = format!("{} {}", i.name, i.address);
    if let Some(c) = i.cidr_block {
        s.push_str(&format!("/{c}"));
    }
    if let Some(l) = &i.link_state {
        s.push_str(&format!(" {l}"));
    }
    s
}

/// VEN-supplied text is shown verbatim, minus control characters that could
/// move the cursor or inject terminal escapes.
pub(crate) fn plain(s: &str) -> String {
    s.chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect()
}

fn detail_lines(r: &WorkloadRowView) -> Vec<DetailLine> {
    let w = &r.workload;
    let a = w.agent_status_view();
    let mut out = Vec::new();
    // Compare mode: the change in full (`was→now` is cut in the table).
    if let Some(c) = &r.change {
        let text = match c.change {
            pce::traffic::Change::Changed => format!("changed · {}", c.was_text()),
            pce::traffic::Change::Gone => "gone · only in the snapshot: its values".into(),
            other => other.as_str().to_string(),
        };
        out.push((Some("change"), text));
    }
    out.extend([
        (Some("href"), w.href.clone()),
        (Some("os"), w.os_id.clone().unwrap_or_else(|| "-".into())),
        (Some("ven"), ven_detail(w, a.as_ref())),
        (Some("health"), health_detail(r)),
    ]);
    if let Some(a) = &a {
        let sync = a.sync_state.as_deref().unwrap_or("-");
        out.push((
            Some("sync"),
            match &a.applied_at {
                Some(t) => format!("{sync} · applied {t}"),
                None => sync.to_string(),
            },
        ));
        let beat = match (&a.last_heartbeat, r.heartbeat.is_empty()) {
            (Some(t), false) => format!("{t} ({})", r.heartbeat),
            (Some(t), true) => t.clone(),
            (None, _) => "-".into(),
        };
        out.push((Some("beat"), beat));
        if !a.errors.is_empty() {
            out.extend(items("errors", a.errors.iter().map(|e| plain(e)).collect()));
        }
        if !a.warnings.is_empty() {
            out.extend(items("warn", a.warnings.iter().map(|e| plain(e)).collect()));
        }
    }
    out.push((
        Some("labels"),
        Some(r.labels.clone())
            .filter(|l| !l.is_empty())
            .unwrap_or_else(|| "-".into()),
    ));
    out.extend(items(
        "iface",
        w.interfaces.iter().map(interface_text).collect(),
    ));
    out
}

/// VEN health and the effective enforcement (`healthy · full`); without a
/// VEN, the mode the PCE reports for it (`unmanaged · PCE reports
/// visibility_only`).
fn health_detail(r: &WorkloadRowView) -> String {
    let w = &r.workload;
    let health = r.health.as_str();
    match (w.is_unmanaged(), w.enforcement_mode.as_deref()) {
        (true, Some(raw)) => format!("{health} · PCE reports {raw}"),
        (true, None) => health.to_string(),
        (false, mode) => format!("{health} · {}", mode.unwrap_or("-")),
    }
}

/// `active · 23.2.31-3113 · up 16m`, or the `ven_text` fallback for unmanaged workloads.
fn ven_detail(w: &Workload, a: Option<&pce::models::AgentStatus>) -> String {
    let mut s = ven_text(w);
    if let Some(a) = a {
        if let Some(v) = &a.version {
            s.push_str(&format!(" · {v}"));
        }
        if let Some(u) = a.uptime_seconds {
            s.push_str(&format!(" · up {}", uptime_text(u)));
        }
    }
    s
}

fn uptime_text(secs: u64) -> String {
    match secs {
        s if s < 60 => format!("{s}s"),
        s if s < 3600 => format!("{}m", s / 60),
        s if s < 86_400 => format!("{}h", s / 3600),
        s => format!("{}d", s / 86_400),
    }
}

/// The VEN strip: `VEN 3 · healthy 1 · warning 1 · error 0 · offline 1 ·
/// unmanaged 1`, each state in its colour (dim at 0), `· of 500 loaded
/// (capped)` on a capped list, then a stacked bar in what is left. Narrow,
/// the words shorten (`ok 1 · warn 1 …`), then the zero counts go.
fn summary_line(model: &Model, width: u16) -> Line<'static> {
    let t = &model.theme;
    let s = &model.workloads.ven;
    // Marks for a bulk change first: they decide what L and M change.
    let marked = model.workloads.marked.len();
    let hidden = model.workloads.marked_hidden();
    let mark = match (marked, hidden) {
        (0, _) => String::new(),
        (m, 0) => format!("{m} marked · "),
        (m, h) => format!("{m} marked ({h} hidden by the filter) · "),
    };
    let room = (width as usize).saturating_sub(cells(&mark));
    let variants = [(true, true), (false, true), (false, false)];
    let mut parts = Vec::new();
    for (i, (long, zeros)) in variants.into_iter().enumerate() {
        parts = strip_parts(s, long, zeros, t);
        let len: usize = parts.iter().map(|p: &Span| p.width()).sum();
        if len <= room || i == variants.len() - 1 {
            break;
        }
    }
    let used: usize = cells(&mark) + parts.iter().map(Span::width).sum::<usize>();
    let bar_w = (width as usize).saturating_sub(used + 2).min(24);
    let mut spans = Vec::new();
    if marked > 0 {
        spans.push(Span::styled(
            mark,
            Style::new().fg(t.accent).add_modifier(Modifier::BOLD),
        ));
    }
    spans.extend(parts);
    if bar_w >= 4 {
        spans.push(Span::raw("  "));
        spans.extend(health_bar(VenHealth::ALL.map(|h| s.count(h)), bar_w, t));
    }
    Line::from(spans)
}

/// The strip's parts: `VEN n`, then each state (`long` words or short
/// ones; `zeros` keeps the states at 0), then the cap note.
fn strip_parts(s: &pce::ven::VenSummary, long: bool, zeros: bool, t: &Theme) -> Vec<Span<'static>> {
    let mut out = vec![Span::raw(format!("VEN {}", s.vens))];
    for h in VenHealth::ALL {
        let n = s.count(h);
        if n == 0 && !zeros {
            continue;
        }
        let word = match (h, long) {
            (_, true) => h.as_str(),
            (VenHealth::Healthy, false) => "ok",
            (VenHealth::Warning, false) => "warn",
            (VenHealth::Error, false) => "err",
            (VenHealth::Offline, false) => "off",
            (VenHealth::Unmanaged, false) => "unm",
        };
        let style = if n == 0 {
            t.dim_style()
        } else {
            crate::tui::ui::widgets::overlay::health_style(h, t)
        };
        out.push(Span::raw(" · "));
        out.push(Span::styled(format!("{word} {n}"), style));
    }
    if s.truncated {
        let note = if long {
            format!(" · of {} loaded (capped)", s.workloads)
        } else {
            " · capped".to_string()
        };
        out.push(Span::styled(note, Style::new().fg(t.warn)));
    }
    out
}

/// Segments in the order healthy, warning, error, offline, unmanaged; rounding leftovers go to the largest.
fn health_bar(counts: [usize; 5], width: usize, t: &Theme) -> Vec<Span<'static>> {
    let total: usize = counts.iter().sum();
    if total == 0 {
        return vec![Span::styled("░".repeat(width), t.dim_style())];
    }
    let mut cells = counts.map(|c| c * width / total);
    let rest = width - cells.iter().sum::<usize>();
    if let Some(i) = (0..5).max_by_key(|&i| counts[i]) {
        cells[i] += rest;
    }
    let colours = [t.ok, t.warn, t.bad, t.dim, t.dim];
    cells
        .iter()
        .zip(colours)
        .map(|(&n, c)| Span::styled("█".repeat(n), Style::new().fg(c)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use pce::KeyOrder;

    fn row(i: usize, raw: &str) -> WorkloadRowView {
        let mut w: Vec<Workload> =
            serde_json::from_str(include_str!("../../../../../../fixtures/workloads.json"))
                .unwrap();
        w[i].enforcement_mode = Some(raw.into());
        let now: jiff::Timestamp = "2026-10-10T08:00:00Z".parse().unwrap();
        WorkloadRowView::new(w.swap_remove(i), now, &KeyOrder::default())
    }

    fn health(r: &WorkloadRowView) -> String {
        let lines = detail_lines(r);
        assert!(lines.iter().all(|(h, _)| *h != Some("enforce")), "one line");
        lines
            .into_iter()
            .find(|(h, _)| *h == Some("health"))
            .map(|(_, b)| b)
            .unwrap()
    }

    #[test]
    fn detail_health_line_carries_the_enforcement_and_what_the_pce_reports() {
        assert_eq!(
            health(&row(2, "visibility_only")),
            "unmanaged · PCE reports visibility_only"
        );
        assert_eq!(health(&row(0, "full")), "healthy · full");
        let mut none = row(2, "x");
        none.workload.enforcement_mode = None;
        assert_eq!(health(&none), "unmanaged");
    }

    fn group(title: &str, members: usize, health: [usize; 5]) -> Arc<WlGroup> {
        Arc::new(WlGroup {
            key: vec![Some(title.into())],
            keys: vec!["app".into()],
            title: title.into(),
            health,
            enforcement: [members, 0, 0, 0],
            changes: Default::default(),
            members: (0..members).map(|_| Arc::new(row(0, "full"))).collect(),
        })
    }

    /// Where each part starts: the text before `needle` in cells.
    fn at(spans: &[Span], needle: &str) -> usize {
        let line: String = spans.iter().map(|s| s.content.as_ref()).collect();
        cells(&line[..line.find(needle).unwrap()])
    }

    #[test]
    fn section_headers_line_their_parts_up_folded_or_not() {
        let t = Theme::dark();
        let groups = [
            group("crm", 3, [3, 0, 0, 0, 0]),
            group("payment-gateway | Production", 12, [10, 0, 1, 1, 0]),
        ];
        let cols = HeaderCols::of(&groups);
        let a = header_spans(&groups[0], true, cols, 200, &t);
        let b = header_spans(&groups[1], false, cols, 200, &t);
        assert_eq!(at(&a, "3 wl"), at(&b, "12 wl"), "count");
        assert_eq!(at(&a, "█"), at(&b, "█"), "bar");
        assert_eq!(at(&a, "3 ok"), at(&b, "10 ok"), "health");
        assert_eq!(at(&a, "full 3"), at(&b, "full 12"), "enforcement");
        // Narrow, the parts still drop right to left, without trailing blanks.
        let narrow = header_spans(&groups[0], false, cols, 50, &t);
        let line: String = narrow.iter().map(|s| s.content.as_ref()).collect();
        assert!(!line.contains("full"), "{line}");
        assert_eq!(line.trim_end(), line, "{line}");
    }
}
