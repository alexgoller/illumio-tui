//! Modal overlays drawn on top of the current screen: help and the `:` prompt.
use crate::tui::filter::{FIELD_NAMES, FilterEditor};
use crate::tui::model::{Model, Screen, rule_kind_name};
use crate::tui::ui::layout::centered;
use crate::tui::ui::widgets::panel::panel;
use pce::RuleView;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Clear, Paragraph, Wrap};

pub const HELP_LINES: &[(&str, &str)] = &[
    (
        "1 2 3 4 5",
        "Traffic · Policy · Objects · Workloads · Events",
    ),
    (":", "command prompt: :traffic :policy :objects :workloads"),
    ("", ":events  :filter <expr>  :window <dur>"),
    ("", ":profile <name>  :draft  :active  :help  :q"),
    ("?", "this help"),
    ("q / Ctrl-C", "quit"),
    ("↑ ↓ PgUp PgDn Home End", "move in the current table"),
    ("s", "cycle sort column and direction"),
    ("/", "search in the current table (Enter keep, Esc clear)"),
    ("r", "refresh the current screen (bypasses the cache)"),
    ("a", "toggle auto-refresh for the current screen"),
    ("Traffic  v", "cycle Flows · App-to-app · Blocked"),
    (
        "Traffic  f",
        "edit filter: window src dst service decisions",
    ),
    ("Traffic  g", "change the aggregate label keys"),
    (
        "Traffic  Enter",
        "detail (flows) · drill into a pair (aggregate)",
    ),
    ("Policy   d", "toggle draft / active"),
    ("Policy   p", "pending changes"),
    (
        "Policy   Tab / Enter",
        "pane · rule detail · on a ruleset: load deny rules",
    ),
    ("Objects  Tab / Enter", "next object type · detail"),
    (
        "Workloads t / T",
        "jump to Traffic with workload as src / dst",
    ),
    (
        "Events   f / Enter / S",
        "filter · event as JSON · sort by severity",
    ),
    ("Esc", "close overlay / cancel"),
];

pub fn render_help(model: &Model, f: &mut Frame, area: Rect) {
    let t = &model.theme;
    let r = centered(area, area.width.min(84), (HELP_LINES.len() as u16) + 2);
    f.render_widget(Clear, r);
    let block = panel("Keys", "Esc closes", t, true);
    let lines: Vec<Line> = HELP_LINES
        .iter()
        .map(|(k, d)| {
            Line::from(vec![
                Span::styled(format!("{k:<24}"), t.header_style()),
                Span::styled(*d, t.base()),
            ])
        })
        .collect();
    f.render_widget(Paragraph::new(lines).block(block).style(t.base()), r);
}

pub fn render_prompt(model: &Model, input: &str, f: &mut Frame, area: Rect) {
    let t = &model.theme;
    let r = Rect::new(
        area.x,
        area.y + area.height.saturating_sub(1),
        area.width,
        1,
    );
    f.render_widget(Clear, r);
    let line = Line::from(vec![
        Span::styled(":", t.header_style()),
        Span::styled(input.to_string(), t.base()),
        Span::styled("▏", t.dim_style()),
    ]);
    f.render_widget(Paragraph::new(line).style(t.base()), r);
}

pub fn render_group_prompt(model: &Model, input: &str, f: &mut Frame, area: Rect) {
    let t = &model.theme;
    let r = Rect::new(
        area.x,
        area.y + area.height.saturating_sub(1),
        area.width,
        1,
    );
    f.render_widget(Clear, r);
    let line = Line::from(vec![
        Span::styled("group keys: ", t.header_style()),
        Span::styled(input.to_string(), t.base()),
        Span::styled("▏", t.dim_style()),
    ]);
    f.render_widget(Paragraph::new(line).style(t.base()), r);
}

pub fn render_filter_editor(model: &Model, e: &FilterEditor, f: &mut Frame, area: Rect) {
    let t = &model.theme;
    let r = centered(area, 70, 12);
    f.render_widget(Clear, r);
    let block = panel("Edit filter", "Tab next · Enter run · Esc cancel", t, true);
    let mut lines: Vec<Line> = FIELD_NAMES
        .iter()
        .enumerate()
        .map(|(i, name)| {
            let active = i == e.active;
            let cursor = if active { "▏" } else { "" };
            Line::from(vec![
                Span::styled(if active { "▸ " } else { "  " }, Style::new().fg(t.accent)),
                Span::styled(format!("{name:<12}"), t.header_style()),
                Span::styled(
                    format!("{}{cursor}", e.values[i]),
                    if active {
                        Style::new().fg(t.fg).bg(t.selected_bg)
                    } else {
                        t.base()
                    },
                ),
            ])
        })
        .collect();
    lines.push(Line::from(""));
    match &e.error {
        Some(err) => lines.push(Line::from(Span::styled(
            err.clone(),
            Style::new().fg(t.bad),
        ))),
        None => {
            let preview = e.to_filter().map(|f| f.cost_hint()).unwrap_or_default();
            lines.push(Line::from(Span::styled(preview, t.dim_style())));
        }
    }
    f.render_widget(Paragraph::new(lines).block(block).style(t.base()), r);
}

/// Field names of the events editor, in the order of `Overlay::EventsEditor::values`.
const EVENTS_FIELDS: [&str; 3] = ["severity", "type", "window"];

pub fn render_events_editor(
    model: &Model,
    values: &[String; 3],
    active: usize,
    error: &Option<String>,
    f: &mut Frame,
    area: Rect,
) {
    let t = &model.theme;
    let r = centered(area, 70, 9);
    f.render_widget(Clear, r);
    let block = panel(
        "Edit events",
        "Tab next · Enter apply · Esc cancel",
        t,
        true,
    );
    let mut lines: Vec<Line> = EVENTS_FIELDS
        .iter()
        .enumerate()
        .map(|(i, name)| {
            let on = i == active;
            let cursor = if on { "▏" } else { "" };
            Line::from(vec![
                Span::styled(if on { "▸ " } else { "  " }, Style::new().fg(t.accent)),
                Span::styled(format!("{name:<12}"), t.header_style()),
                Span::styled(
                    format!("{}{cursor}", values[i]),
                    if on {
                        Style::new().fg(t.fg).bg(t.selected_bg)
                    } else {
                        t.base()
                    },
                ),
            ])
        })
        .collect();
    lines.push(Line::from(""));
    lines.push(match error {
        Some(err) => Line::from(Span::styled(err.clone(), Style::new().fg(t.bad))),
        None => Line::from(Span::styled(
            "severity empty = any · type matches locally · window e.g. 24h",
            t.dim_style(),
        )),
    });
    f.render_widget(Paragraph::new(lines).block(block).style(t.base()), r);
}

/// Lines of JSON shown for one event; the rest is cut.
const EVENT_DETAIL_LINES: usize = 30;

pub fn render_event_detail(model: &Model, event: &pce::models::Event, f: &mut Frame, area: Rect) {
    let t = &model.theme;
    let r = centered(area, 100, EVENT_DETAIL_LINES as u16 + 2);
    f.render_widget(Clear, r);
    let json = serde_json::to_string_pretty(event).unwrap_or_default();
    let total = json.lines().count();
    let lines: Vec<Line> = json
        .lines()
        .take(EVENT_DETAIL_LINES)
        .map(|l| Line::from(crate::tui::ui::screens::workloads::plain(l)))
        .collect();
    let meta = if total > EVENT_DETAIL_LINES {
        format!("first {EVENT_DETAIL_LINES} of {total} lines · Esc closes")
    } else {
        "Esc closes".to_string()
    };
    let title = event.event_type.as_deref().unwrap_or("event");
    f.render_widget(
        Paragraph::new(lines)
            .block(panel(title, &meta, t, true))
            .style(t.base()),
        r,
    );
}

pub fn render_confirm(model: &Model, reason: &str, f: &mut Frame, area: Rect) {
    let t = &model.theme;
    let r = centered(area, 64, 5);
    f.render_widget(Clear, r);
    let block = panel("Confirm", "y / n", t, true);
    f.render_widget(
        Paragraph::new(Line::from(Span::styled(
            reason.to_string(),
            Style::new().fg(t.warn),
        )))
        .block(block)
        .style(t.base())
        .wrap(Wrap { trim: true }),
        r,
    );
}

/// Longest message drawn; the panel holds five wrapped lines.
const ERROR_MAX_CHARS: usize = 300;

pub fn render_error(model: &Model, text: &str, f: &mut Frame, area: Rect) {
    let t = &model.theme;
    let r = centered(area, 70, 7);
    f.render_widget(Clear, r);
    let shown: String = if text.chars().count() > ERROR_MAX_CHARS {
        let cut: String = text.chars().take(ERROR_MAX_CHARS - 1).collect();
        format!("{cut}…")
    } else {
        text.to_string()
    };
    let bad = Style::new().fg(t.bad);
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(bad)
        .title(Line::from(Span::styled(
            " Error ",
            bad.add_modifier(ratatui::style::Modifier::BOLD),
        )))
        .title_top(Line::from(Span::styled(" Esc closes ", t.dim_style())).right_aligned());
    f.render_widget(
        Paragraph::new(shown)
            .block(block)
            .style(t.base())
            .wrap(Wrap { trim: true }),
        r,
    );
}

fn labelled<'a>(model: &Model, label: &str, value: String) -> Line<'a> {
    let t = &model.theme;
    Line::from(vec![
        Span::styled(format!("{label:<12}"), t.header_style()),
        Span::styled(value, t.base()),
    ])
}

pub fn render_rule_detail(model: &Model, rule: &RuleView, f: &mut Frame, area: Rect) {
    let t = &model.theme;
    let r = centered(area, 76, 14);
    f.render_widget(Clear, r);
    let names = |v: &[pce::ResolvedActor]| {
        v.iter()
            .map(|a| a.name.clone())
            .collect::<Vec<_>>()
            .join(", ")
    };
    let services = rule
        .services
        .iter()
        .map(|s| s.name.clone())
        .collect::<Vec<_>>()
        .join(", ");
    let flags = format!(
        "{} · {}",
        if rule.enabled { "enabled" } else { "disabled" },
        if rule.unscoped_consumers {
            "unscoped consumers"
        } else {
            "scoped consumers"
        }
    );
    let lines = vec![
        labelled(
            model,
            "description",
            rule.description.clone().unwrap_or_else(|| "(none)".into()),
        ),
        labelled(model, "consumers", names(&rule.consumers)),
        labelled(model, "providers", names(&rule.providers)),
        labelled(model, "services", services),
        labelled(model, "flags", flags),
        labelled(
            model,
            "pending",
            rule.update_type.clone().unwrap_or_else(|| "none".into()),
        ),
        labelled(model, "href", rule.href.clone()),
    ];
    let block = panel(rule_kind_name(rule.kind), "Esc closes", t, true);
    f.render_widget(
        Paragraph::new(lines)
            .block(block)
            .style(t.base())
            .wrap(Wrap { trim: true }),
        r,
    );
}

pub fn render_pending(model: &Model, f: &mut Frame, area: Rect) {
    let t = &model.theme;
    let r = centered(area, 70, 16);
    f.render_widget(Clear, r);
    let pending = model.policy.pending.as_deref().unwrap_or(&[]);
    let mut lines: Vec<Line> = if pending.is_empty() {
        vec![Line::from(Span::styled("none", t.dim_style()))]
    } else {
        pending
            .iter()
            .map(|c| {
                Line::from(vec![
                    Span::styled(format!("{:<16}", c.object_type), t.header_style()),
                    Span::styled(format!("{:<8}", c.change_type), Style::new().fg(t.warn)),
                    Span::styled(c.name.clone().unwrap_or_else(|| c.href.clone()), t.base()),
                ])
            })
            .collect()
    };
    let rows = r.height.saturating_sub(2) as usize;
    if lines.len() > rows {
        let hidden = lines.len() - (rows - 1);
        lines.truncate(rows - 1);
        lines.push(Line::from(Span::styled(
            format!("… {hidden} more"),
            t.dim_style(),
        )));
    }
    let meta = format!("{} · Esc closes", pending.len());
    f.render_widget(
        Paragraph::new(lines)
            .block(panel("Pending changes", &meta, t, true))
            .style(t.base()),
        r,
    );
}

pub fn screen_hint(screen: Screen) -> &'static str {
    match screen {
        Screen::Traffic => "f filter  r run  a auto  v view  s sort  / search",
        Screen::Policy => "d scope  p pending  Enter detail  Tab pane  a auto",
        Screen::Objects => "Tab type  Enter detail  r refresh  a auto  / search",
        Screen::Workloads => "t/T to traffic  / search  r refresh",
        Screen::Events => "f filter  r refresh  a auto  S severity  Enter detail",
    }
}
