//! The Objects screen's IP lists tab: every draft IP list with its size,
//! entry counts and how many loaded rules use it, and a detail pane for the
//! selected one (size, the rules by name, the first ranges and FQDNs). No
//! request beyond the list itself.
use crate::tui::model::{IpListRow, Model, rule_kind_name};
use crate::tui::ui::screens::objects::{DetailLine, clip, items};
use crate::tui::ui::widgets::bars::age_text;
use crate::tui::ui::widgets::panel::panel;
use crate::tui::ui::widgets::table::{header_meta, render};
use pce::models::Actor;
use pce::{IpListSize, NameIndex, describe_actor};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use std::time::{Duration, Instant};

/// Rows of the detail pane, borders included.
const DETAIL_HEIGHT: u16 = 10;

/// `(table, detail)` areas below the Objects tab strip. Shared by the view
/// and `update` (page sizes); the detail is dropped when the terminal is too
/// short to keep a usable table.
pub fn layout(body: Rect, show_detail: bool) -> (Rect, Rect) {
    let detail_h = if show_detail && body.height >= 18 {
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

/// The tab's table and detail in `body` (the area below the strip); the
/// Objects screen has already handled a missing name index.
pub fn render_tab(model: &Model, f: &mut Frame, body: Rect) {
    let t = &model.theme;
    let s = &model.objects.ip_lists;
    let (table, detail) = layout(body, s.show_detail);
    let freshness = if s.loading {
        "loading…".to_string()
    } else {
        s.loaded_at
            .map(|l| {
                format!(
                    "cached {}",
                    age_text(Duration::from_secs(s.age_secs) + l.elapsed())
                )
            })
            .unwrap_or_default()
    };
    let (shown, total) = (s.table.len(), s.table.rows().len());
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
    meta.push_str(&s.auto.meta(Instant::now()));
    let sort_meta = header_meta(&s.table);
    if !sort_meta.is_empty() {
        meta = format!("{sort_meta} · {meta}");
    }
    render(
        f,
        table,
        &s.table,
        t,
        panel("IP lists", &meta, t, true),
        &|_: &IpListRow| Style::default(),
    );
    if detail.height > 0 {
        render_detail(model, f, detail);
    }
}

fn render_detail(model: &Model, f: &mut Frame, area: Rect) {
    let t = &model.theme;
    let block = panel("Detail", "Enter hides", t, false);
    let capacity = block.inner(area).height as usize;
    let Some(row) = model.objects.ip_lists.table.selected_item() else {
        f.render_widget(
            Paragraph::new(Span::styled("no IP list selected", t.dim_style())).block(block),
            area,
        );
        return;
    };
    let text: Vec<Line> = clip(detail_lines(model, row), capacity)
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

/// Range and FQDN lines shown before `+N more` (the CLI prints them all).
const MAX_ENTRIES: usize = 3;

/// `items` with at most `MAX_ENTRIES` values, then `+N more <noun>`.
fn capped(head: &'static str, mut values: Vec<String>, noun: &str) -> Vec<DetailLine> {
    let hidden = values.len().saturating_sub(MAX_ENTRIES);
    values.truncate(MAX_ENTRIES);
    if hidden > 0 {
        values.push(format!("+{hidden} more {noun}"));
    }
    items(head, values)
}

/// Name and description, size, the loaded rules that name the list, then
/// the first ranges (`!` before an exclusion, its description after) and
/// FQDNs: what a long list cannot push out of the pane comes first.
fn detail_lines(model: &Model, row: &IpListRow) -> Vec<DetailLine> {
    let l = &row.list;
    let mut out = vec![
        (
            Some("list"),
            match &l.description {
                Some(d) if !d.is_empty() => format!("{}  {d}", l.name),
                _ => l.name.clone(),
            },
        ),
        (Some("size"), size_text(&row.size)),
    ];
    out.extend(used_by_lines(model, &l.href));
    let ranges: Vec<String> = l
        .ip_ranges
        .iter()
        .map(|r| {
            let mut s = String::new();
            if r.exclusion.unwrap_or(false) {
                s.push('!');
            }
            s.push_str(&r.from_ip);
            if let Some(to) = &r.to_ip {
                s.push('-');
                s.push_str(to);
            }
            if let Some(d) = r.description.as_deref().filter(|d| !d.is_empty()) {
                s.push_str("  ");
                s.push_str(d);
            }
            s
        })
        .collect();
    if !ranges.is_empty() || l.fqdns.is_empty() {
        out.extend(capped("ranges", ranges, "ranges"));
    }
    if !l.fqdns.is_empty() {
        out.extend(capped(
            "fqdns",
            l.fqdns.iter().map(|f| f.fqdn.clone()).collect(),
            "FQDNs",
        ));
    }
    out
}

/// `<ruleset> · <kind> · <providers> ← <consumers>` per rule, or where to
/// load the rules from.
fn used_by_lines(model: &Model, href: &str) -> Vec<DetailLine> {
    let Some(uses) = model.policy.ip_list_uses(href) else {
        return vec![(Some("used by"), "load Policy (2) to see rules".into())];
    };
    let empty = NameIndex::default();
    let idx = model.index().map_or(&empty, |i| i.as_ref());
    let names = |v: &[Actor]| {
        v.iter()
            .map(|a| describe_actor(a, idx))
            .collect::<Vec<_>>()
            .join(", ")
    };
    items(
        "used by",
        uses.iter()
            .map(|u| {
                format!(
                    "{} · {} · {} ← {}",
                    u.ruleset,
                    rule_kind_name(u.kind),
                    names(&u.providers),
                    names(&u.consumers)
                )
            })
            .collect(),
    )
}

/// `65.5K addresses (65536 IPv4, 0 IPv6)`, or the size alone for `any` and
/// FQDN-only lists.
fn size_text(s: &IpListSize) -> String {
    if s.unbounded.is_some() || (s.ranges == 0 && s.fqdns > 0) {
        return s.display();
    }
    format!("{} addresses ({} IPv4, {} IPv6)", s.display(), s.v4, s.v6)
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::test_support::{model_fixture, model_with_ip_lists};

    fn text(lines: &[DetailLine]) -> Vec<String> {
        lines
            .iter()
            .map(|(h, b)| format!("{:<8}{b}", h.unwrap_or("")))
            .collect()
    }

    #[test]
    fn the_detail_names_size_ranges_fqdns_and_the_rules_using_the_list() {
        let m = model_with_ip_lists();
        let corp = &m.objects.ip_lists.table.rows()[1];
        assert_eq!(
            text(&detail_lines(&m, corp)),
            vec![
                "list    Corp  Office ranges",
                "size    65.5K addresses (65536 IPv4, 0 IPv6)",
                "used by Shop-Prod · allow · role=web ← ipl:Corp",
                "ranges  10.1.0.0-10.1.255.255",
                "fqdns   *.corp.example",
            ]
        );
        let any = text(&detail_lines(&m, &m.objects.ip_lists.table.rows()[0]));
        assert_eq!(any[1], "size    any (0.0.0.0/0)");
        assert_eq!(
            any[2],
            "used by Shop-Prod · deny · All workloads ← ipl:Any (0.0.0.0/0 and ::/0)"
        );
        assert_eq!(any[3], "ranges  0.0.0.0/0");
        assert_eq!(any[4], "        ::/0");
    }

    #[test]
    fn size_and_used_by_come_first_and_ranges_and_fqdns_stop_at_three() {
        let mut m = model_with_ip_lists();
        let ranges: Vec<_> = (0..10)
            .map(|i| serde_json::json!({"from_ip": format!("10.0.{i}.0/24")}))
            .collect();
        let fqdns: Vec<_> = (0..5)
            .map(|i| serde_json::json!({"fqdn": format!("h{i}.example")}))
            .collect();
        let many: pce::models::IpList = serde_json::from_value(serde_json::json!({
            "href": "/orgs/5/sec_policy/draft/ip_lists/21", "name": "Corp",
            "ip_ranges": ranges, "fqdns": fqdns
        }))
        .unwrap();
        m.objects.ip_lists.set_lists(vec![many], &m.policy);
        assert_eq!(
            text(&detail_lines(&m, &m.objects.ip_lists.table.rows()[0])),
            vec![
                "list    Corp",
                "size    2.6K addresses (2560 IPv4, 0 IPv6)",
                "used by Shop-Prod · allow · role=web ← ipl:Corp",
                "ranges  10.0.0.0/24",
                "        10.0.1.0/24",
                "        10.0.2.0/24",
                "        +7 more ranges",
                "fqdns   h0.example",
                "        h1.example",
                "        h2.example",
                "        +2 more FQDNs",
            ]
        );
    }

    #[test]
    fn without_loaded_rulesets_the_detail_says_where_to_load_them() {
        let mut m = model_fixture();
        let lists =
            serde_json::from_str(include_str!("../../../../../../fixtures/ip_lists.json")).unwrap();
        m.objects.ip_lists.set_lists(lists, &m.policy);
        let lines = text(&detail_lines(&m, &m.objects.ip_lists.table.rows()[1]));
        assert_eq!(lines[2], "used by load Policy (2) to see rules");
        let excluded: pce::models::IpList = serde_json::from_value(serde_json::json!({
            "href": "/orgs/5/sec_policy/draft/ip_lists/30", "name": "Lab",
            "ip_ranges": [
                {"from_ip": "10.0.0.0/24", "description": "lab net"},
                {"from_ip": "10.0.0.128/25", "exclusion": true}
            ]
        }))
        .unwrap();
        m.objects.ip_lists.set_lists(vec![excluded], &m.policy);
        let lines = text(&detail_lines(&m, &m.objects.ip_lists.table.rows()[0]));
        assert_eq!(lines[1], "size    128 addresses (128 IPv4, 0 IPv6)");
        assert_eq!(lines[2], "used by load Policy (2) to see rules");
        assert_eq!(lines[3], "ranges  10.0.0.0/24  lab net");
        assert_eq!(lines[4], "        !10.0.0.128/25");
    }
}
