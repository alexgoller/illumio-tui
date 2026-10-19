//! Policy screen: rulesets on the left, the selected ruleset's rules on the right.
//! Below 120 columns the panes stack (rulesets above rules) so both stay readable.
use crate::tui::edit::scopes_text;
use crate::tui::model::{Model, PolicyPane};
use crate::tui::ui::widgets::bars::age_text;
use crate::tui::ui::widgets::panel::panel;
use crate::tui::ui::widgets::table::{header_meta, render};
use pce::models::RuleSet;
use pce::{NameIndex, RuleKind};
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
    meta.push_str(&p.auto.meta(Instant::now()));
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
    let theme = t.clone();
    let mut rules_meta = header_meta(&p.rules);
    if p.deny_not_loaded() {
        let note = "deny rules not loaded · ↑/↓ loads";
        rules_meta = if rules_meta.is_empty() {
            note.into()
        } else {
            format!("{note} · {rules_meta}")
        };
    }
    // Room left of the meta: two corners and a space each side of both titles.
    let meta_room = if rules_meta.is_empty() {
        0
    } else {
        rules_meta.chars().count() + 3
    };
    let room = (right.width as usize).saturating_sub(4 + meta_room);
    let empty = NameIndex::default();
    let idx = model.index().map_or(&empty, |i| i.as_ref());
    let title = rules_title(p.selected_rule_set(), idx, room);
    render(
        f,
        right,
        &p.rules,
        t,
        panel(&title, &rules_meta, t, p.pane == PolicyPane::Rules),
        &move |r| match r.kind {
            // A disabled rule is dimmed, as a disabled ruleset is.
            _ if !r.enabled => theme.dim_style(),
            RuleKind::Allow => Style::default(),
            RuleKind::Deny => Style::new().fg(theme.warn),
            RuleKind::OverrideDeny => Style::new().fg(theme.bad),
        },
    );
}

/// `Rules · app=shop, env=prod | app=shop, env=staging`: the selected
/// ruleset's scopes as its form shows them, `Scopeless` when it has none, cut
/// with `…` to `width` characters. Plain `Rules` with no ruleset selected.
pub fn rules_title(rs: Option<&RuleSet>, idx: &NameIndex, width: usize) -> String {
    let Some(rs) = rs else {
        return "Rules".into();
    };
    let scopes = scopes_text(&rs.scopes, idx);
    let scopes = if scopes.is_empty() {
        "Scopeless"
    } else {
        &scopes
    };
    let title = format!("Rules · {scopes}");
    if title.chars().count() <= width {
        return title;
    }
    let mut cut: String = title.chars().take(width.saturating_sub(1)).collect();
    cut.push('…');
    cut
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::test_support::fixture_index;

    fn shop() -> RuleSet {
        let sets: Vec<RuleSet> =
            serde_json::from_str(include_str!("../../../../../../fixtures/rule_sets.json"))
                .unwrap();
        sets[0].clone()
    }

    #[test]
    fn the_rules_title_names_the_selected_rulesets_scopes() {
        let idx = fixture_index();
        let mut rs = shop();
        assert_eq!(
            rules_title(Some(&rs), &idx, 80),
            "Rules · app=shop, env=prod"
        );
        rs.scopes.push(
            serde_json::from_value(serde_json::json!([
                {"label": {"href": "/orgs/5/labels/3"}},
                {"label": {"href": "/orgs/5/labels/5"}}
            ]))
            .unwrap(),
        );
        assert_eq!(
            rules_title(Some(&rs), &idx, 80),
            "Rules · app=shop, env=prod | app=shop, loc=eu"
        );
        let cut = rules_title(Some(&rs), &idx, 20);
        assert_eq!(cut, "Rules · app=shop, e…");
        assert_eq!(cut.chars().count(), 20);
        rs.scopes = vec![vec![]];
        assert_eq!(rules_title(Some(&rs), &idx, 80), "Rules · Scopeless");
        rs.scopes.clear();
        assert_eq!(rules_title(Some(&rs), &idx, 80), "Rules · Scopeless");
        assert_eq!(rules_title(None, &idx, 80), "Rules");
    }
}
