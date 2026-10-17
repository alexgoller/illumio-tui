//! The bulk workload overlays: the enforcement picker (`M`) and the
//! confirm of a bulk change (typed count, loaded traffic, draft check,
//! every change). The labels form (`L`) is the generic form; the report is
//! the batch report.
use crate::tui::bulk::{BulkConfirm, DraftCheck, EnforcePicker};
use crate::tui::form::wrap_rows;
use crate::tui::model::Model;
use crate::tui::ui::layout::centered;
use crate::tui::ui::screens::workloads::plain;
use crate::tui::ui::widgets::overlay::{scroll_meta, wrap_chars};
use crate::tui::ui::widgets::panel::panel;
use pce::write::EnforcementMode;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph};

const PICKER_WIDTH: u16 = 64;
const CONFIRM_WIDTH: u16 = 88;

/// `M`: the modes, the visibility levels (the focused column's cursor
/// `▸`, the choice `●`), then the change it makes.
pub fn render_picker(model: &Model, p: &EnforcePicker, f: &mut Frame, area: Rect) {
    let t = &model.theme;
    let width = usize::from(PICKER_WIDTH.min(area.width).saturating_sub(2));
    let mut lines: Vec<Line> = Vec::new();
    let option = |focused: bool, chosen: bool, name: &str| {
        let cursor = if focused && chosen { "▸ " } else { "  " };
        let dot = if chosen { "● " } else { "○ " };
        let style = if chosen {
            Style::new().fg(t.accent).add_modifier(Modifier::BOLD)
        } else {
            t.base()
        };
        Line::from(vec![
            Span::styled(cursor.to_string(), Style::new().fg(t.accent)),
            Span::styled(format!("{dot}{name}"), style),
        ])
    };
    lines.push(Line::from(Span::styled(
        "Enforcement mode",
        t.header_style(),
    )));
    for (i, m) in EnforcementMode::ALL.iter().enumerate() {
        lines.push(option(p.column == 0, i == p.mode, m.as_str()));
    }
    lines.push(Line::from(Span::styled("Visibility", t.header_style())));
    for (i, name) in EnforcePicker::visibility_names().iter().enumerate() {
        lines.push(option(p.column == 1, i == p.visibility, name));
    }
    lines.push(Line::from(""));
    for l in wrap_rows(&plain(&p.summary), width, 6) {
        lines.push(Line::from(Span::styled(l, Style::new().fg(t.warn))));
    }
    if p.change().enforces() {
        for l in wrap_rows(
            "full and selective block what the policy does not allow; on several workloads the confirm asks for their count",
            width,
            4,
        ) {
            lines.push(Line::from(Span::styled(l, t.dim_style())));
        }
    }
    let height = u16::try_from(lines.len() + 2).unwrap_or(u16::MAX);
    let r = centered(area, PICKER_WIDTH, height);
    f.render_widget(Clear, r);
    let title = format!("Enforcement · {}", p.what);
    let fits = |meta: &str| {
        title.chars().count() + meta.chars().count() + 6 <= usize::from(r.width.saturating_sub(2))
    };
    let meta = [
        "j/k choose · Tab column · Enter review · Esc cancel",
        "Tab column · Enter review · Esc",
        "Enter · Esc",
    ]
    .into_iter()
    .find(|m| fits(m))
    .unwrap_or("");
    f.render_widget(
        Paragraph::new(lines)
            .block(panel(&title, meta, t, true))
            .style(t.base()),
        r,
    );
}

/// The confirm as drawn: the pinned lines (summary, loaded traffic, the
/// draft check, the typed count) and the scrolled ones (the check's top
/// flows, every change, every request), each wrapped to the box.
struct ConfirmLayout {
    rect: Rect,
    pinned: Vec<(String, Style)>,
    content: Vec<String>,
    rows: usize,
}

fn confirm_layout(model: &Model, c: &BulkConfirm, area: Rect) -> ConfirmLayout {
    let t = &model.theme;
    let width = usize::from(CONFIRM_WIDTH.min(area.width).saturating_sub(2));
    let warn = Style::new().fg(t.warn);
    let mut pinned: Vec<(String, Style)> = Vec::new();
    let mut pin = |text: &str, style: Style| {
        for l in wrap_rows(&plain(text), width, 6) {
            pinned.push((l, style));
        }
    };
    let plan = &c.plan;
    // The typed count first: on a short terminal it is what must show.
    if let Some(why) = plan.count_reason() {
        let n = plan.items.len();
        let text = if c.count_ok() {
            format!("{why}: count typed, y sends")
        } else {
            format!("{why}: type {n} to confirm: {}▏", c.typed)
        };
        pin(&text, Style::new().fg(t.bad).add_modifier(Modifier::BOLD));
    }
    pin(&plan.summary(), warn);
    if c.checkable() {
        pin(
            &match c.loaded {
                Some(l) => format!("loaded traffic of these workloads: {}", l.text()),
                None => "no traffic loaded: d asks the draft policy".to_string(),
            },
            t.base(),
        );
        let check = match &c.check {
            DraftCheck::Idle => {
                "d: draft check, what their recent traffic in and out would block once enforced (1 Explorer query, several requests)".to_string()
            }
            DraftCheck::Running { .. } => "draft check running … Esc cancels it".to_string(),
            DraftCheck::Done(r) => format!("draft check: {}", r.text()),
            DraftCheck::Failed(e) => format!("draft check failed: {e}"),
        };
        let style = match &c.check {
            DraftCheck::Done(r) if !r.blocked.is_empty() => Style::new().fg(t.bad),
            DraftCheck::Failed(_) => Style::new().fg(t.bad),
            _ => t.dim_style(),
        };
        pin(&check, style);
    }
    let mut content: Vec<String> = Vec::new();
    if let DraftCheck::Done(r) = &c.check
        && !r.blocked.is_empty()
    {
        content.push(format!(
            "would be blocked (top {}):",
            r.blocked.len().min(5)
        ));
        content.extend(r.top(5).into_iter().map(|l| format!("  {l}")));
        content.push(String::new());
    }
    content.extend(c.lines.iter().cloned());
    let content: Vec<String> = content
        .iter()
        .flat_map(|l| wrap_chars(&plain(l), width))
        .collect();
    let chrome = u16::try_from(pinned.len())
        .unwrap_or(u16::MAX)
        .saturating_add(2);
    let height = u16::try_from(content.len())
        .unwrap_or(u16::MAX)
        .saturating_add(chrome);
    let rect = centered(area, CONFIRM_WIDTH, height);
    let rows = usize::from(rect.height.saturating_sub(chrome));
    ConfirmLayout {
        rect,
        pinned,
        content,
        rows,
    }
}

/// The first line shown when the confirm is scrolled to the end.
pub fn confirm_last_scroll(model: &Model, c: &BulkConfirm, area: Rect) -> usize {
    let l = confirm_layout(model, c, area);
    l.content.len().saturating_sub(l.rows)
}

pub fn render_confirm(model: &Model, c: &BulkConfirm, f: &mut Frame, area: Rect) {
    let t = &model.theme;
    let ConfirmLayout {
        rect: r,
        pinned,
        content,
        rows,
    } = confirm_layout(model, c, area);
    f.render_widget(Clear, r);
    let first = usize::from(c.scroll).min(content.len().saturating_sub(rows));
    let below = content.len().saturating_sub(first + rows);
    let n = c.plan.items.len();
    let keys = match (c.count_ok(), c.checkable()) {
        (false, _) => format!("type {n}, then y · Esc cancel"),
        (true, true) => "y send · d draft check · Esc cancel".to_string(),
        (true, false) => "y send · Esc cancel".to_string(),
    };
    let meta = scroll_meta(below, &keys);
    let mut lines: Vec<Line> = pinned
        .into_iter()
        .map(|(s, style)| Line::from(Span::styled(s, style)))
        .collect();
    lines.extend(content.into_iter().skip(first).take(rows).map(|s| {
        let head = s.split_once(' ').is_some_and(|(n, _)| {
            n.contains('/') && n.chars().next().is_some_and(|c| c.is_ascii_digit())
        });
        let style = if head || s.starts_with("skipped (") || s.starts_with("would be blocked") {
            t.header_style()
        } else {
            t.base()
        };
        Line::from(Span::styled(s, style))
    }));
    let title = format!(
        "Send this change to {n} workload{}?",
        if n == 1 { "" } else { "s" }
    );
    f.render_widget(
        Paragraph::new(lines)
            .block(panel(&title, &meta, t, true))
            .style(t.base()),
        r,
    );
}
