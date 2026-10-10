//! Policy screen: rulesets on the left, the selected ruleset's rules on the right.
//! Below 120 columns the panes stack (rulesets above rules) so both stay readable.
use crate::tui::model::{Model, PolicyPane};
use crate::tui::ui::widgets::bars::age_text;
use crate::tui::ui::widgets::panel::panel;
use crate::tui::ui::widgets::table::{header_meta, render};
use pce::RuleKind;
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Style;
use std::time::{Duration, Instant};

const STACK_BELOW: u16 = 120;

/// `(rulesets, rules)` areas. Shared by `view` and `update` (page sizes).
pub fn layout(body: Rect) -> (Rect, Rect) {
    let areas = if body.width < STACK_BELOW {
        Layout::vertical([
            Constraint::Length(7.min(body.height / 3)),
            Constraint::Min(4),
        ])
        .split(body)
    } else {
        Layout::horizontal([Constraint::Percentage(45), Constraint::Percentage(55)]).split(body)
    };
    (areas[0], areas[1])
}

/// Rows of data visible in the focused pane's table (minus border and header).
pub fn table_rows(body: Rect, pane: PolicyPane) -> usize {
    let (rulesets, rules) = layout(body);
    let area = match pane {
        PolicyPane::RuleSets => rulesets,
        PolicyPane::Rules => rules,
    };
    area.height.saturating_sub(3) as usize
}

pub fn render_screen(model: &Model, f: &mut Frame, body: Rect) {
    let t = &model.theme;
    let p = &model.policy;
    let (left, right) = layout(body);
    let freshness = if p.loading {
        "loading…".to_string()
    } else {
        p.loaded_at
            .map(|l| age_text(Duration::from_secs(p.age_secs) + l.elapsed()))
            .unwrap_or_default()
    };
    let mut meta = format!("{} · {freshness}", p.scope.segment());
    if p.auto.enabled {
        meta.push_str(&format!(
            " · auto in {}s",
            p.auto.remaining(Instant::now()).as_secs()
        ));
    }
    let sort_meta = header_meta(&p.rulesets);
    if !sort_meta.is_empty() {
        meta = format!("{sort_meta} · {meta}");
    }
    let theme = t.clone();
    render(
        f,
        left,
        &p.rulesets,
        t,
        panel("Rulesets", &meta, t, p.pane == PolicyPane::RuleSets),
        &move |r| {
            if r.enabled {
                Style::default()
            } else {
                theme.dim_style()
            }
        },
    );
    let title = p
        .rulesets
        .selected_item()
        .map(|r| format!("Rules · {}", r.name))
        .unwrap_or_else(|| "Rules".into());
    let theme = t.clone();
    render(
        f,
        right,
        &p.rules,
        t,
        panel(
            &title,
            &header_meta(&p.rules),
            t,
            p.pane == PolicyPane::Rules,
        ),
        &move |r| match r.kind {
            RuleKind::Allow => Style::default(),
            RuleKind::Deny => Style::new().fg(theme.warn),
            RuleKind::OverrideDeny => Style::new().fg(theme.bad),
        },
    );
}
