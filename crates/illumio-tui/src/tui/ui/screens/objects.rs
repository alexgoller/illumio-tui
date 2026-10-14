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
        parts.push(
            match (&o.pce_usage, &o.label_usage, o.usage_truncated) {
                (PceUsage::Flags(_), ..) => "unused",
                (PceUsage::Unknown, ..) | (PceUsage::Unsupported, None, _) => "unused: loading",
                (PceUsage::Unsupported, Some(_), true) => "unused (workloads truncated)",
                (PceUsage::Unsupported, Some(_), false) => {
                    "unused (workloads, rules and scopes only)"
                }
            }
            .to_string(),
        );
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
