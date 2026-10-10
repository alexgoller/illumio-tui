//! Modal overlays drawn on top of the current screen: help and the `:` prompt.
use crate::tui::model::{Model, Screen};
use crate::tui::ui::layout::centered;
use crate::tui::ui::widgets::panel::panel;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph};

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

pub fn screen_hint(screen: Screen) -> &'static str {
    match screen {
        Screen::Traffic => "f filter  r run  a auto  v view  s sort  / search",
        Screen::Policy => "d draft/active  p pending  Enter detail  r refresh",
        Screen::Objects => "Tab type  r refresh  / search",
        Screen::Workloads => "t/T to traffic  / search  r refresh",
    }
}
