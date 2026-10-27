//! Objects screen: a tab strip of the four object kinds (labels, label
//! groups, services, IP lists), the current kind's table, and an optional
//! detail pane for the selected object. The IP lists tab is drawn by
//! `screens::iplists` below the strip.
use crate::tui::model::{LabelRow, Model, ObjectKind, ObjectsState, PceUsage};
use crate::tui::ui::widgets::bars::age_text;
use crate::tui::ui::widgets::panel::panel;
use crate::tui::ui::widgets::table::{header_meta, render};
use pce::models::{LabelGroup, Service};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Tabs};
use std::time::{Duration, Instant};

const DETAIL_HEIGHT: u16 = 6;

/// `(tab strip, everything below it)`: the IP lists tab lays out the rest
/// itself (`iplists::layout`).
pub fn strip_split(body: Rect) -> (Rect, Rect) {
    let rows = Layout::vertical([Constraint::Length(1), Constraint::Min(0)]).split(body);
    (rows[0], rows[1])
}

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
            Paragraph::new(Span::styled(note, t.dim_style())).block(panel("Objects", "", t, true)),
            body,
        );
        return;
    }
    if model.objects.kind == ObjectKind::IpLists {
        let (strip, rest) = strip_split(body);
        render_strip(model, f, strip);
        super::iplists::render_tab(model, f, rest);
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
        ObjectKind::Labels => (o.labels.len(), o.all_labels.len(), header_meta(&o.labels)),
        ObjectKind::LabelGroups => (
            o.groups.len(),
            o.groups.rows().len(),
            header_meta(&o.groups),
        ),
        ObjectKind::Services => (
            o.services.len(),
            o.services.rows().len(),
            header_meta(&o.services),
        ),
        // Drawn by `iplists::render_tab`; `render_screen` never gets here.
        ObjectKind::IpLists => return,
    };
    let freshness = if o.loading[kind.index()] {
        "loading…".to_string()
    } else {
        o.age_secs(kind)
            .map(|a| format!("cached {}", age_text(Duration::from_secs(a))))
            .unwrap_or_default()
    };
    let noun = if total == 1 { "row" } else { "rows" };
    let rows = if kind == ObjectKind::Labels && o.labels_filtered() {
        label_filter_meta(o, shown, total)
    } else if shown == total {
        format!("{total} {noun}")
    } else {
        format!("{shown}/{total} {noun}")
    };
    let mut meta = if freshness.is_empty() {
        rows
    } else {
        format!("{freshness} · {rows}")
    };
    meta.push_str(&o.auto.meta(Instant::now()));
    if !sort_meta.is_empty() {
        meta = format!("{sort_meta} · {meta}");
    }
    let block = panel(kind.title(), &meta, t, true);
    match kind {
        ObjectKind::Labels => {
            let dim = t.dim_style();
            render(f, area, &o.labels, t, block, &move |l: &LabelRow| {
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
        ObjectKind::Services => render(f, area, &o.services, t, block, &|_: &Service| {
            Style::default()
        }),
        ObjectKind::IpLists => {}
    }
}

/// `key app,env · unused · 212 of 4310 · 38 hidden`: the Labels tab's
/// filters, only the parts that apply.
fn label_filter_meta(o: &ObjectsState, shown: usize, total: usize) -> String {
    let mut parts = Vec::new();
    if !o.label_keys.is_empty() {
        parts.push(format!("key {}", o.label_keys.join(",")));
    }
    if o.unused_only {
        parts.push(match (&o.pce_usage, &o.label_usage, o.usage_truncated) {
            (PceUsage::Flags(_), ..) => "unused".to_string(),
            (PceUsage::Unknown, ..) | (PceUsage::Unsupported, None, _) => {
                "unused: loading".to_string()
            }
            (PceUsage::Unsupported, Some(_), true) => "unused (workloads truncated)".to_string(),
            (PceUsage::Unsupported, Some(_), false) => {
                format!("unused ({})", crate::cli::output::USAGE_FROM_COUNTS)
            }
        });
    }
    parts.push(format!("{shown} of {total}"));
    if o.ignored_hidden > 0 {
        parts.push(format!("{} hidden", o.ignored_hidden));
    }
    parts.join(" · ")
}

fn render_detail(model: &Model, f: &mut Frame, area: Rect) {
    let t = &model.theme;
    let o = &model.objects;
    let block = panel("Detail", "Enter hides", t, false);
    let capacity = block.inner(area).height as usize;
    let lines = match o.kind {
        ObjectKind::Labels => o.labels.selected_item().map(|l| label_lines(l)),
        ObjectKind::LabelGroups => o.groups.selected_item().map(|g| group_lines(model, g)),
        ObjectKind::Services => o.services.selected_item().map(|s| service_lines(s)),
        ObjectKind::IpLists => None,
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

/// A label's lines. Once the PCE's usage flags are in (`u`), `held by`
/// takes the second line and the two dates share the last, so all of it
/// fits the four-line pane.
fn label_lines(l: &LabelRow) -> Vec<DetailLine> {
    let dash = || "-".to_string();
    let label = (
        Some("label"),
        format!(
            "{}{}",
            l.display(),
            if l.is_deleted() { "  (deleted)" } else { "" }
        ),
    );
    let href = (Some("href"), l.href.clone());
    let created = l.created_at.clone().unwrap_or_else(dash);
    let updated = l.updated_at.clone().unwrap_or_else(dash);
    let Some(flags) = &l.held_by else {
        return vec![
            label,
            href,
            (Some("created"), created),
            (Some("updated"), updated),
        ];
    };
    let kinds = flags.kinds();
    let held = if kinds.is_empty() {
        "nothing: unused".to_string()
    } else {
        kinds.join(", ")
    };
    vec![
        label,
        (Some("held by"), held),
        href,
        (Some("created"), format!("{created} · updated {updated}")),
    ]
}

/// Members in the index's key order, then by value; unresolved hrefs last.
fn group_lines(model: &Model, g: &LabelGroup) -> Vec<DetailLine> {
    let idx = model.index();
    let unresolved = |href: &str| format!("{href} (unresolved)");
    let labels: Vec<String> = match idx {
        Some(i) => i
            .resolve_labels(&g.labels)
            .into_iter()
            .map(|l| {
                if l.key == "?" {
                    unresolved(&l.href)
                } else {
                    format!("{}={}", l.key, l.value)
                }
            })
            .collect(),
        None => g.labels.iter().map(|r| unresolved(&r.href)).collect(),
    };
    let group = |href: &str| match idx.and_then(|i| i.label_group_name(href)) {
        Some(n) => n.to_string(),
        None => format!("{href} (unresolved)"),
    };
    let mut out = vec![(Some("group"), format!("{}  key {}", g.name, g.key))];
    out.extend(items("labels", labels));
    if !g.sub_groups.is_empty() {
        out.extend(items(
            "groups",
            g.sub_groups.iter().map(|r| group(&r.href)).collect(),
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

/// The where-used overlay's box and its parts.
pub struct WhereUsedLayout {
    pub rect: Rect,
    pub head: Rect,
    pub table: Rect,
    pub foot: Rect,
}

/// `NOT_SEARCHED` where the box is too narrow for it (fits 74 columns).
const NOT_SEARCHED_SHORT: &str = "not searched: virtual services, boundaries, container profiles";

/// The where-used box: centred, at most 110 wide; three head lines (the
/// sections, what is not searched, then what is loading, failed or may be
/// missing), the table, the keys.
/// Shared by `view` and `update` (page size).
pub fn where_used_layout(area: Rect) -> WhereUsedLayout {
    report_layout(area, 3)
}

/// The `:hygiene` box: as where-used's, its head as many lines as
/// [`hygiene_head`] needs at the box's width.
pub fn hygiene_layout(area: Rect, v: &crate::tui::hygiene::HygieneView) -> WhereUsedLayout {
    let inner = usize::from(area.width.saturating_sub(4).min(110).saturating_sub(2));
    let n = hygiene_head(v, inner).len();
    report_layout(area, u16::try_from(n).unwrap_or(u16::MAX))
}

/// How a head line of `:hygiene` is drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HeadTone {
    Header,
    Dim,
    Warn,
}

/// The `:hygiene` head at `width`: unused objects per kind, workloads
/// missing each key, what is not searched (whole, wrapped), then every
/// warning (each wrapped; none: one blank line).
pub fn hygiene_head(v: &crate::tui::hygiene::HygieneView, width: usize) -> Vec<(String, HeadTone)> {
    use crate::tui::ui::widgets::generator::wrap_text;
    let unused = if v.unused.is_empty() && v.withheld.is_empty() {
        "Unused: listed once every list it reads is in".to_string()
    } else {
        v.unused_line()
    };
    let mut out = vec![
        (cut_to(&unused, width), HeadTone::Header),
        (cut_to(&v.missing_line(), width), HeadTone::Header),
    ];
    // Always shown, whole: an unused row is a candidate, not a verdict.
    for l in wrap_text(pce::hygiene::NOT_SEARCHED, width) {
        out.push((cut_to(&l, width), HeadTone::Dim));
    }
    let warnings = v.warnings();
    if warnings.is_empty() {
        out.push((String::new(), HeadTone::Dim));
    }
    for w in warnings {
        let tone = if w.starts_with("loading ") {
            HeadTone::Dim
        } else {
            HeadTone::Warn
        };
        for l in wrap_text(&w, width) {
            out.push((cut_to(&l, width), tone));
        }
    }
    out
}

/// A report box: centred, at most 110 wide; `head` lines, the table, the
/// keys.
fn report_layout(area: Rect, head: u16) -> WhereUsedLayout {
    let w = area.width.saturating_sub(4).min(110);
    let h = area.height.saturating_sub(4);
    let rect = crate::tui::ui::layout::centered(area, w, h);
    let inner = Rect::new(
        rect.x + 1,
        rect.y + 1,
        rect.width.saturating_sub(2),
        rect.height.saturating_sub(2),
    );
    let [head, table, foot] = Layout::vertical([
        Constraint::Length(head),
        Constraint::Min(1),
        Constraint::Length(1),
    ])
    .areas(inner);
    WhereUsedLayout {
        rect,
        head,
        table,
        foot,
    }
}

/// `text` cut with `…` to `width` characters.
fn cut_to(text: &str, width: usize) -> String {
    if text.chars().count() <= width {
        return text.to_string();
    }
    let mut out: String = text.chars().take(width.saturating_sub(1).max(1)).collect();
    out.push('…');
    out
}

/// `W` / `:used`: `where used: env=prod` with the total, the sections
/// with their counts, what is still loading (or failed), else what is not
/// searched, the references, the keys.
pub fn render_where_used(model: &Model, f: &mut Frame, area: Rect) {
    use ratatui::widgets::{Block, Clear};
    let Some(v) = model.objects.where_used.as_ref() else {
        return;
    };
    let t = &model.theme;
    let l = where_used_layout(area);
    f.render_widget(Clear, l.rect);
    let n = v.total();
    let mut meta = format!("{n} {}", if n == 1 { "reference" } else { "references" });
    if v.requests > 0 {
        meta.push_str(&format!(
            " · {} req{}",
            v.requests,
            if v.requests == 1 { "" } else { "s" }
        ));
    }
    let room = (l.rect.width as usize).saturating_sub(7 + meta.chars().count());
    // A label's `key=value` says what it is; other names get their kind.
    let what = match v.target.kind {
        pce::where_used::ObjectType::Label => v.name.clone(),
        k => format!("{} {}", k.word(), v.name),
    };
    let title = cut_to(&format!("where used: {what}"), room);
    f.render_widget(panel(&title, &meta, t, true), l.rect);
    let width = l.head.width as usize;
    let sections = v.sections();
    let first = if sections.is_empty() && v.waiting.is_empty() && v.failed.is_none() {
        "nothing in the draft policy or the label groups names it".to_string()
    } else {
        sections
    };
    // Always shown, whole: an empty result is not "unused everywhere".
    let not_searched = if pce::where_used::NOT_SEARCHED.chars().count() <= width {
        pce::where_used::NOT_SEARCHED
    } else {
        NOT_SEARCHED_SHORT
    };
    let (third, style) = if let Some(e) = &v.failed {
        (e.clone(), Style::new().fg(t.warn))
    } else if !v.waiting.is_empty() {
        (format!("loading {}…", v.waiting.join(", ")), t.dim_style())
    } else if v.deny_unread > 0 {
        (
            format!(
                "{} ruleset{} may hold deny rules not loaded (selecting one on Policy loads them)",
                v.deny_unread,
                if v.deny_unread == 1 { "" } else { "s" },
            ),
            Style::new().fg(t.warn),
        )
    } else {
        (String::new(), t.dim_style())
    };
    let head = vec![
        Line::from(Span::styled(cut_to(&first, width), t.header_style())),
        Line::from(Span::styled(cut_to(not_searched, width), t.dim_style())),
        Line::from(Span::styled(cut_to(&third, width), style)),
    ];
    f.render_widget(Paragraph::new(head).style(t.base()), l.head);
    let theme = t.clone();
    render(f, l.table, &v.table, t, Block::default(), &move |r| {
        if r.disabled {
            theme.dim_style()
        } else {
            Style::default()
        }
    });
    let foot = cut_to(
        // Fits the 74 columns of the box at 80.
        "Enter (rule, ruleset, group, profile, workloads) · / search · Esc close",
        l.foot.width as usize,
    );
    f.render_widget(
        Paragraph::new(Line::from(Span::styled(foot, t.dim_style()))).style(t.base()),
        l.foot,
    );
}

/// `:hygiene`: the candidates and the workloads missing labels in the
/// title; unused objects per kind, workloads missing each key, what is not
/// searched, what is loading or failed; the rows; the keys.
pub fn render_hygiene(model: &Model, f: &mut Frame, area: Rect) {
    use ratatui::widgets::{Block, Clear};
    let Some(v) = model.objects.hygiene.as_ref() else {
        return;
    };
    let t = &model.theme;
    let l = hygiene_layout(area, v);
    f.render_widget(Clear, l.rect);
    // Only what is counted: no `0 unused` before the lists are in.
    let (u, m) = (v.unused_total(), v.missing_total());
    let mut parts = Vec::new();
    if !v.unused.is_empty() {
        parts.push(format!("{u} unused"));
    }
    if v.workloads.is_some() {
        parts.push(format!(
            "{m} workload{} missing labels",
            if m == 1 { "" } else { "s" }
        ));
    }
    if parts.is_empty() && !v.waiting.is_empty() {
        parts.push("loading".to_string());
    }
    let mut meta = parts.join(" · ");
    if v.requests > 0 {
        let sep = if meta.is_empty() { "" } else { " · " };
        meta.push_str(&format!(
            "{sep}{} req{}",
            v.requests,
            if v.requests == 1 { "" } else { "s" }
        ));
    }
    let room = (l.rect.width as usize).saturating_sub(7 + meta.chars().count());
    let title = cut_to("hygiene", room);
    f.render_widget(panel(&title, &meta, t, true), l.rect);
    let head: Vec<Line> = hygiene_head(v, l.head.width as usize)
        .into_iter()
        .map(|(text, tone)| {
            let style = match tone {
                HeadTone::Header => t.header_style(),
                HeadTone::Dim => t.dim_style(),
                HeadTone::Warn => Style::new().fg(t.warn),
            };
            Line::from(Span::styled(text, style))
        })
        .collect();
    f.render_widget(Paragraph::new(head).style(t.base()), l.head);
    let warn = Style::new().fg(t.warn);
    // An uncertain row: the unread deny rules may name it.
    render(f, l.table, &v.table, t, Block::default(), &move |r| {
        if r.certain { Style::default() } else { warn }
    });
    let foot = cut_to(
        // Fits the 74 columns of the box at 80.
        "Enter (object on its tab, workload) · W where used · / search · Esc close",
        l.foot.width as usize,
    );
    f.render_widget(
        Paragraph::new(Line::from(Span::styled(foot, t.dim_style()))).style(t.base()),
        l.foot,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::model::Connection;
    use crate::tui::test_support::{fixture_index, model_fixture};
    use pce::{KeyOrder, NameIndex};
    use std::sync::Arc;

    #[test]
    fn group_members_follow_the_key_order_with_unresolved_last() {
        let mut m = model_fixture();
        let labels: Vec<pce::models::Label> =
            serde_json::from_str(include_str!("../../../../../../fixtures/labels.json")).unwrap();
        let ordered = NameIndex::new(&labels, &[], &[], &[])
            .with_key_order(KeyOrder::new(&["env".into()], "role"));
        let g: LabelGroup = serde_json::from_value(serde_json::json!({
            "href": "/orgs/5/sec_policy/draft/label_groups/11", "name": "mixed", "key": "role",
            "labels": [
                {"href": "/orgs/5/labels/77"}, {"href": "/orgs/5/labels/3"},
                {"href": "/orgs/5/labels/1"}, {"href": "/orgs/5/labels/2"},
                {"href": "/orgs/5/labels/4"}
            ]
        }))
        .unwrap();
        let members = |m: &Model| -> Vec<String> {
            group_lines(m, &g)
                .into_iter()
                .skip(1)
                .map(|(_, t)| t)
                .collect()
        };
        if let Connection::Connected { index, .. } = &mut m.connection {
            *index = Some(Arc::new(ordered));
        }
        assert_eq!(
            members(&m),
            vec![
                "env=prod",
                "role=db",
                "role=web",
                "app=shop",
                "/orgs/5/labels/77 (unresolved)"
            ]
        );
        if let Connection::Connected { index, .. } = &mut m.connection {
            *index = Some(fixture_index());
        }
        assert_eq!(
            members(&m)[..4],
            ["app=shop", "env=prod", "role=db", "role=web"],
            "alphabetical by default"
        );
    }
}
