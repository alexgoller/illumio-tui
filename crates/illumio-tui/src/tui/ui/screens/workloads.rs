//! Workloads screen: one table of workloads and an optional detail pane for
//! the selected one (href, OS, VEN status, interfaces, labels).
use crate::tui::model::Model;
use crate::tui::ui::screens::objects::{DetailLine, clip, items};
use crate::tui::ui::widgets::bars::age_text;
use crate::tui::ui::widgets::panel::panel;
use crate::tui::ui::widgets::table::{header_meta, render};
use pce::models::Workload;
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use std::time::{Duration, Instant};

/// Border plus six rows of detail.
const DETAIL_HEIGHT: u16 = 8;

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

/// Rows of data visible in the table (minus border and header).
pub fn table_rows(body: Rect, show_detail: bool) -> usize {
    layout(body, show_detail).0.height.saturating_sub(3) as usize
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
    let (table, detail) = layout(body, model.workloads.show_detail);
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
    if w.auto.enabled {
        meta.push_str(&format!(
            " · auto in {}s",
            w.auto.remaining(Instant::now()).as_secs()
        ));
    }
    let sort_meta = header_meta(&w.table);
    if !sort_meta.is_empty() {
        meta = format!("{sort_meta} · {meta}");
    }
    let theme = t.clone();
    render(
        f,
        area,
        &w.table,
        t,
        panel("Workloads", &meta, t, true),
        &move |wl: &Workload| {
            if wl.enforcement_mode.as_deref() == Some("idle") {
                Style::new().fg(theme.bad)
            } else if wl.online == Some(false) {
                theme.dim_style()
            } else {
                Style::default()
            }
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

fn detail_lines(w: &Workload) -> Vec<DetailLine> {
    let mut out = vec![
        (Some("href"), w.href.clone()),
        (Some("os"), w.os_id.clone().unwrap_or_else(|| "-".into())),
        (Some("ven"), ven_text(w)),
        (
            Some("labels"),
            Some(w.label_text())
                .filter(|l| !l.is_empty())
                .unwrap_or_else(|| "-".into()),
        ),
    ];
    out.extend(items(
        "iface",
        w.interfaces.iter().map(interface_text).collect(),
    ));
    out
}
