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
use ratatui::widgets::{Clear, Paragraph, Wrap};

pub const HELP_LINES: &[(&str, &str)] = &[
    ("1 2 3 4", "Traffic · Policy · Labels & objects · Workloads"),
    (":", "command prompt: :traffic :policy :objects :workloads"),
    ("", ":filter <expr>  :window <dur>  :profile <name>"),
    ("", ":draft  :active  :help  :q"),
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
    ("Policy   Enter", "rule detail"),
    ("Objects  Tab", "next object type"),
    (
        "Workloads t / T",
        "jump to Traffic with workload as src / dst",
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
        Screen::Objects => "Tab type  r refresh  / search",
        Screen::Workloads => "t/T to traffic  / search  r refresh",
    }
}
