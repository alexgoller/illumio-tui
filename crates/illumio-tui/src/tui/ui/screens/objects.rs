//! Labels & objects screen: a tab strip of the four object kinds, the current
//! kind's table, and an optional detail pane for the selected object.
use crate::tui::model::{Model, ObjectKind};
use crate::tui::ui::widgets::bars::age_text;
use crate::tui::ui::widgets::panel::panel;
use crate::tui::ui::widgets::table::{header_meta, render};
use pce::models::{IpList, Label, LabelGroup, Service};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Tabs};
use std::time::{Duration, Instant};

const DETAIL_HEIGHT: u16 = 6;

/// `(tab strip, table, detail)` areas. Shared by `view` and `update` (page sizes).
/// The detail pane is dropped when the terminal is too short to keep a usable table.
pub fn layout(body: Rect, show_detail: bool) -> (Rect, Rect, Rect) {
    let detail_h = if show_detail && body.height >= 14 {
        DETAIL_HEIGHT
    } else {
        0
    };
    let rows = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(4),
        Constraint::Length(detail_h),
    ])
    .split(body);
    (rows[0], rows[1], rows[2])
}

/// Rows of data visible in the table (minus border and header).
pub fn table_rows(body: Rect, show_detail: bool) -> usize {
    layout(body, show_detail).1.height.saturating_sub(3) as usize
}

pub fn render_screen(model: &Model, f: &mut Frame, body: Rect) {
    if model.index().is_none() {
        let t = &model.theme;
        let note = if model.objects.index_failed {
            "no objects loaded · r retries"
        } else {
            "loading objects…"
        };
        f.render_widget(
            Paragraph::new(Span::styled(note, t.dim_style())).block(panel(
                "Labels & objects",
                "",
                t,
                true,
            )),
            body,
        );
        return;
    }
    let (strip, table, detail) = layout(body, model.objects.show_detail);
    render_strip(model, f, strip);
    render_table(model, f, table);
    if detail.height > 0 {
        render_detail(model, f, detail);
    }
}

fn render_strip(model: &Model, f: &mut Frame, area: Rect) {
    let t = &model.theme;
    let titles: Vec<Line> = ObjectKind::ALL
        .iter()
        .map(|k| Line::from(format!(" {} ", k.title())))
        .collect();
    let tabs = Tabs::new(titles)
        .select(model.objects.kind.index())
        .style(t.dim_style())
        .highlight_style(Style::new().fg(t.accent).add_modifier(Modifier::BOLD))
        .divider(Span::raw("│"));
    f.render_widget(tabs, area);
}

fn render_table(model: &Model, f: &mut Frame, area: Rect) {
    let t = &model.theme;
    let o = &model.objects;
    let kind = o.kind;
    let (shown, total, sort_meta) = match kind {
        ObjectKind::Labels => (
            o.labels.len(),
            o.labels.rows().len(),
            header_meta(&o.labels),
        ),
        ObjectKind::LabelGroups => (
            o.groups.len(),
            o.groups.rows().len(),
            header_meta(&o.groups),
        ),
        ObjectKind::IpLists => (
            o.ip_lists.len(),
            o.ip_lists.rows().len(),
            header_meta(&o.ip_lists),
        ),
        ObjectKind::Services => (
            o.services.len(),
            o.services.rows().len(),
            header_meta(&o.services),
        ),
    };
    let freshness = if o.loading[kind.index()] {
        "loading…".to_string()
    } else {
        o.age_secs(kind)
            .map(|a| format!("cached {}", age_text(Duration::from_secs(a))))
            .unwrap_or_default()
    };
    let noun = if total == 1 { "row" } else { "rows" };
    let rows = if shown == total {
        format!("{total} {noun}")
    } else {
        format!("{shown}/{total} {noun}")
    };
    let mut meta = if freshness.is_empty() {
        rows
    } else {
        format!("{freshness} · {rows}")
    };
    if o.auto.enabled {
        meta.push_str(&format!(
            " · auto in {}s",
            o.auto.remaining(Instant::now()).as_secs()
        ));
    }
    if !sort_meta.is_empty() {
        meta = format!("{sort_meta} · {meta}");
    }
    let block = panel(kind.title(), &meta, t, true);
    match kind {
        ObjectKind::Labels => {
            let dim = t.dim_style();
            render(f, area, &o.labels, t, block, &move |l: &Label| {
                if l.is_deleted() {
                    dim
                } else {
                    Style::default()
                }
            });
        }
        ObjectKind::LabelGroups => render(f, area, &o.groups, t, block, &|_: &LabelGroup| {
            Style::default()
        }),
        ObjectKind::IpLists => render(f, area, &o.ip_lists, t, block, &|_: &IpList| {
            Style::default()
        }),
        ObjectKind::Services => render(f, area, &o.services, t, block, &|_: &Service| {
            Style::default()
        }),
    }
}

fn render_detail(model: &Model, f: &mut Frame, area: Rect) {
    let t = &model.theme;
    let o = &model.objects;
    let block = panel("Detail", "Enter hides", t, false);
    let capacity = block.inner(area).height as usize;
    let lines = match o.kind {
        ObjectKind::Labels => o.labels.selected_item().map(|l| label_lines(l)),
        ObjectKind::LabelGroups => o.groups.selected_item().map(|g| group_lines(model, g)),
        ObjectKind::IpLists => o.ip_lists.selected_item().map(|l| ip_list_lines(l)),
        ObjectKind::Services => o.services.selected_item().map(|s| service_lines(s)),
    };
    let Some(lines) = lines else {
        f.render_widget(
            Paragraph::new(Span::styled("no object selected", t.dim_style())).block(block),
            area,
        );
        return;
    };
    let lines = clip(lines, capacity);
    let text: Vec<Line> = lines
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

/// `(heading, text)`; a missing heading continues the previous one.
pub(super) type DetailLine = (Option<&'static str>, String);

/// Fits `lines` into `capacity` rows; the last row says how many were cut.
pub(super) fn clip(mut lines: Vec<DetailLine>, capacity: usize) -> Vec<DetailLine> {
    if capacity > 0 && lines.len() > capacity {
        let hidden = lines.len() - (capacity - 1);
        lines.truncate(capacity - 1);
        lines.push((None, format!("… {hidden} more not shown")));
    }
    lines
}

/// First item carries the heading, the rest continue it. Nothing: a dash.
pub(super) fn items(head: &'static str, values: Vec<String>) -> Vec<DetailLine> {
    if values.is_empty() {
        return vec![(Some(head), "-".into())];
    }
    values
        .into_iter()
        .enumerate()
        .map(|(i, v)| ((i == 0).then_some(head), v))
        .collect()
}

fn label_lines(l: &Label) -> Vec<DetailLine> {
    let dash = || "-".to_string();
    vec![
        (
            Some("label"),
            format!(
                "{}{}",
                l.display(),
                if l.is_deleted() { "  (deleted)" } else { "" }
            ),
        ),
        (Some("href"), l.href.clone()),
        (Some("created"), l.created_at.clone().unwrap_or_else(dash)),
        (Some("updated"), l.updated_at.clone().unwrap_or_else(dash)),
    ]
}

fn group_lines(model: &Model, g: &LabelGroup) -> Vec<DetailLine> {
    let idx = model.index();
    let label = |href: &str| match idx.and_then(|i| i.label(href)) {
        Some(l) => l.display(),
        None => format!("{href} (unresolved)"),
    };
    let group = |href: &str| match idx.and_then(|i| i.label_group_name(href)) {
        Some(n) => n.to_string(),
        None => format!("{href} (unresolved)"),
    };
    let mut out = vec![(Some("group"), format!("{}  key {}", g.name, g.key))];
    out.extend(items(
        "labels",
        g.labels.iter().map(|r| label(&r.href)).collect(),
    ));
    if !g.sub_groups.is_empty() {
        out.extend(items(
            "groups",
            g.sub_groups.iter().map(|r| group(&r.href)).collect(),
        ));
    }
    out
}

fn ip_list_lines(l: &IpList) -> Vec<DetailLine> {
    let mut out = vec![(
        Some("list"),
        match &l.description {
            Some(d) if !d.is_empty() => format!("{}  {d}", l.name),
            _ => l.name.clone(),
        },
    )];
    let ranges: Vec<String> = l
        .ip_ranges
        .iter()
        .map(|r| {
            let mut s = String::new();
            if r.exclusion.unwrap_or(false) {
                s.push_str("exclude ");
            }
            s.push_str(&r.from_ip);
            if let Some(to) = &r.to_ip {
                s.push_str(&format!(" - {to}"));
            }
            if let Some(d) = r.description.as_deref().filter(|d| !d.is_empty()) {
                s.push_str(&format!("  {d}"));
            }
            s
        })
        .collect();
    if !ranges.is_empty() || l.fqdns.is_empty() {
        out.extend(items("ranges", ranges));
    }
    if !l.fqdns.is_empty() {
        out.extend(items(
            "fqdns",
            l.fqdns.iter().map(|f| f.fqdn.clone()).collect(),
        ));
    }
    out
}

fn service_lines(s: &Service) -> Vec<DetailLine> {
    let mut out = vec![(
        Some("service"),
        match &s.description {
            Some(d) if !d.is_empty() => format!("{}  {d}", s.name),
            _ => s.name.clone(),
        },
    )];
    out.extend(items(
        "ports",
        s.service_ports.iter().map(|p| p.display()).collect(),
    ));
    out
}
