//! Workloads screen: one table of workloads and an optional detail pane for
//! the selected one (href, OS, VEN status, interfaces, labels).
use crate::tui::model::{Model, WorkloadRowView};
use crate::tui::ui::screens::objects::{DetailLine, clip, items};
use crate::tui::ui::theme::Theme;
use crate::tui::ui::widgets::bars::age_text;
use crate::tui::ui::widgets::panel::panel;
use crate::tui::ui::widgets::table::{header_meta, render};
use pce::models::{VenHealth, Workload};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
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

/// Rows of data visible in the table (minus the summary line, border and header).
pub fn table_rows(body: Rect, show_detail: bool) -> usize {
    layout(body, show_detail).0.height.saturating_sub(4) as usize
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
    let [summary, table] =
        Layout::vertical([Constraint::Length(1), Constraint::Min(3)]).areas(area);
    f.render_widget(Paragraph::new(summary_line(model, summary.width)), summary);
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
    let rows = if shown == total {
        format!("{total}")
    } else {
        format!("{shown}/{total}")
    };
    let mut meta = if freshness.is_empty() {
        rows
    } else {
        format!("{rows} · {freshness}")
    };
    meta.push_str(&w.auto.meta(Instant::now()));
    if w.unhealthy_only {
        meta = format!("unhealthy only · {meta}");
    }
    let sort_meta = header_meta(&w.table);
    if !sort_meta.is_empty() {
        meta = format!("{sort_meta} · {meta}");
    }
    if w.unhealthy_only && w.table.is_empty() && w.table.search_text().is_empty() {
        f.render_widget(
            Paragraph::new(Span::styled(
                "no unhealthy VENs · h shows all workloads",
                t.dim_style(),
            ))
            .block(panel("Workloads", &meta, t, true)),
            area,
        );
        return;
    }
    let theme = t.clone();
    render(
        f,
        area,
        &w.table,
        t,
        panel("Workloads", &meta, t, true),
        &move |r: &WorkloadRowView| match r.health {
            VenHealth::Error | VenHealth::Offline => Style::new().fg(theme.bad),
            VenHealth::Warning => Style::new().fg(theme.warn),
            VenHealth::Unmanaged => theme.dim_style(),
            VenHealth::Healthy if r.workload.enforcement_mode.as_deref() == Some("idle") => {
                Style::new().fg(theme.bad)
            }
            VenHealth::Healthy => Style::default(),
        },
    );
}

fn render_detail(model: &Model, f: &mut Frame, area: Rect) {
    let t = &model.theme;
    let block = panel("Detail", "Enter hides", t, false);
    let capacity = block.inner(area).height as usize;
    let Some(wl) = model.workloads.table.selected_item() else {
        f.render_widget(
            Paragraph::new(Span::styled("no workload selected", t.dim_style())).block(block),
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
    let mut out = vec![
        (Some("href"), w.href.clone()),
        (Some("os"), w.os_id.clone().unwrap_or_else(|| "-".into())),
        (Some("ven"), ven_detail(w, a.as_ref())),
        (Some("health"), r.health.as_str().to_string()),
    ];
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
        Some(w.label_text())
            .filter(|l| !l.is_empty())
            .unwrap_or_else(|| "-".into()),
    ));
    out.extend(items(
        "iface",
        w.interfaces.iter().map(interface_text).collect(),
    ));
    out
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

/// `healthy 1 · warning 0 · error 0 · offline 1 · unmanaged 1` plus a stacked bar that fills what is left.
fn summary_line(model: &Model, width: u16) -> Line<'static> {
    let t = &model.theme;
    // summary() is in rank order: healthy, unmanaged, warning, offline, error.
    let [healthy, unmanaged, warning, offline, error] = model.workloads.summary();
    let text = format!(
        "healthy {healthy} · warning {warning} · error {error} · offline {offline} · unmanaged {unmanaged}"
    );
    let bar_w = (width as usize)
        .saturating_sub(text.chars().count() + 2)
        .min(24);
    let mut spans = vec![Span::raw(text)];
    if bar_w >= 4 {
        spans.push(Span::raw("  "));
        spans.extend(health_bar(
            [healthy, warning, error, offline, unmanaged],
            bar_w,
            t,
        ));
    }
    Line::from(spans)
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
