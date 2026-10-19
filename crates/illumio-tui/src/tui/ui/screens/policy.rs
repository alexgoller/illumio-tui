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

/// Rows of data visible in the search list (minus border and header).
pub fn found_rows(body: Rect) -> usize {
    body.height.saturating_sub(3) as usize
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
    if let Some(search) = &p.search {
        render_found(model, search, &meta, f, body);
        return;
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

/// The search's flat rule list over the whole body: `Search · <terms>`,
/// the count and what was not searched in the meta. A disabled rule, or a
/// rule of a disabled ruleset, is dimmed; deny rules keep their colours.
fn render_found(model: &Model, search: &pce::RuleFilter, meta: &str, f: &mut Frame, body: Rect) {
    let t = &model.theme;
    let p = &model.policy;
    let n = p.found.len();
    let mut meta = format!("{n} {} · {meta}", if n == 1 { "rule" } else { "rules" });
    // Narrower, the title (the search) keeps the room; the status said it.
    if p.found_deny_unloaded > 0 && body.width >= STACK_BELOW {
        meta = format!(
            "deny rules of {} not loaded · {meta}",
            crate::tui::update::counted_rulesets(p.found_deny_unloaded)
        );
    }
    let sort_meta = header_meta(&p.found);
    if !sort_meta.is_empty() {
        meta = format!("{sort_meta} · {meta}");
    }
    let room = (body.width as usize).saturating_sub(7 + meta.chars().count());
    let title = cut(&format!("Search · {search}"), room);
    let theme = t.clone();
    render(
        f,
        body,
        &p.found,
        t,
        panel(&title, &meta, t, true),
        &move |r| match r.kind {
            _ if !r.enabled || !r.ruleset_enabled => theme.dim_style(),
            RuleKind::Allow => Style::default(),
            RuleKind::Deny => Style::new().fg(theme.warn),
            RuleKind::OverrideDeny => Style::new().fg(theme.bad),
        },
    );
}

/// `text` cut with `…` to `width` characters (at least the first one).
fn cut(text: &str, width: usize) -> String {
    if text.chars().count() <= width {
        return text.to_string();
    }
    let mut out: String = text.chars().take(width.saturating_sub(1).max(1)).collect();
    out.push('…');
    out
}

/// `f` on Policy: the rule search on one line, the completion strip under
/// it while completing, then the parse error, the unknown names (dim:
/// they match nothing) or the syntax.
pub fn render_rule_filter(
    model: &Model,
    e: &crate::tui::rule_search::RuleFilterEditor,
    f: &mut Frame,
    area: Rect,
) {
    use crate::tui::ui::layout::centered;
    use crate::tui::ui::screens::workloads::plain;
    use crate::tui::ui::widgets::overlay::completion_line;
    use ratatui::text::{Line, Span};
    use ratatui::widgets::{Clear, Paragraph};
    let t = &model.theme;
    let strip = e.completion_strip();
    let r = centered(area, 76, 7 + u16::from(strip.is_some()));
    f.render_widget(Clear, r);
    let block = panel(
        "Rule search",
        "Ctrl-N complete · Enter apply · Esc cancel",
        t,
        true,
    );
    let inner = (r.width as usize).saturating_sub(2);
    let label = "rules ";
    let room = inner.saturating_sub(2 + label.len() + 1);
    let shown: String = {
        let chars: Vec<char> = e.value.chars().collect();
        let skip = chars.len().saturating_sub(room);
        chars[skip..].iter().collect()
    };
    let mut lines = vec![Line::from(vec![
        Span::styled("▸ ", Style::new().fg(t.accent)),
        Span::styled(label, t.header_style()),
        Span::styled(format!("{shown}▏"), Style::new().fg(t.fg).bg(t.selected_bg)),
    ])];
    if let Some(strip) = strip.as_ref() {
        let width = inner.saturating_sub(2 + label.len());
        lines.push(completion_line(strip, 2 + label.len(), width, t));
    }
    lines.push(Line::from(""));
    let note = match (&e.error, e.parse()) {
        (Some(err), _) => Line::from(Span::styled(plain(err), Style::new().fg(t.bad))),
        (None, Err(err)) => Line::from(Span::styled(plain(&err), t.dim_style())),
        (None, Ok(_)) if !e.unknown.is_empty() => Line::from(Span::styled(
            format!("no match: {}", e.unknown.join(", ")),
            t.dim_style(),
        )),
        (None, Ok(_)) => Line::from(Span::styled(
            "app=shop,crm ipl:NAME svc:NAME|443/tcp wl:NAME href:TEXT disabled deny",
            t.dim_style(),
        )),
    };
    lines.push(note);
    lines.push(Line::from(Span::styled(
        "comma: any of · space: all of · labels count in scope and via groups",
        t.dim_style(),
    )));
    lines.push(Line::from(Span::styled(
        "local: no request · Enter on a rule opens it · c clears",
        t.dim_style(),
    )));
    f.render_widget(Paragraph::new(lines).block(block).style(t.base()), r);
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
