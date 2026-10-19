//! The Policy screen's transitions: the rulesets and rules panes, the
//! deny fallback, the pending list, provision and revert, and the edit
//! keys on rulesets and rules.
use super::*;

pub(super) fn load_rule_sets(model: &mut Model, mode: CacheMode) -> Vec<Cmd> {
    model.policy.loading = true;
    model.policy.auto.last_run = Some(Instant::now());
    model.policy.auto.from_tick = false;
    vec![Cmd::LoadRuleSets {
        scope: model.policy.scope,
        mode,
    }]
}

/// Deny-rule endpoint request for the selected ruleset when its inline list is empty.
pub(super) fn deny_fallback(model: &mut Model, mode: CacheMode) -> Vec<Cmd> {
    model
        .policy
        .take_deny_fallback()
        .map(|href| vec![Cmd::LoadDenyRules { href, mode }])
        .unwrap_or_default()
}

pub(super) fn policy_page(model: &Model) -> usize {
    let area = ratatui::layout::Rect::new(0, 0, model.size.0, model.size.1);
    crate::tui::ui::layout::chrome(area)
        .map(|c| crate::tui::ui::screens::policy::table_rows(c.body, model.policy.pane))
        .unwrap_or(10)
        .max(1)
}

pub(super) fn policy_key(model: &mut Model, k: KeyEvent) -> Vec<Cmd> {
    let page = policy_page(model);
    if !model.policy.is_searching() {
        match k.code {
            KeyCode::Char('d') => {
                if model.policy.loading {
                    model.status = StatusLine::info("already loading");
                    return Vec::new();
                }
                model.policy.scope = match model.policy.scope {
                    PolicyScope::Draft => PolicyScope::Active,
                    PolicyScope::Active => PolicyScope::Draft,
                };
                model.policy.form_drafts_loading = false;
                model.policy.form_drafts_stale = false;
                model.policy.clear();
                model.objects.ip_lists.recount(&model.policy);
                return load_rule_sets(model, CacheMode::Use);
            }
            KeyCode::Char('r') => {
                if model.policy.loading {
                    // A tick result would consume the deny bypass meant for this one.
                    model.status = StatusLine::info("already loading");
                    return Vec::new();
                }
                model.policy.bypass_deny = true;
                model.policy.auto.rearm();
                return load_rule_sets(model, CacheMode::Bypass);
            }
            KeyCode::Char('p') => {
                if model.policy.pending_loading {
                    model.status = StatusLine::info("pending changes already loading");
                    return Vec::new();
                }
                model.policy.pending_loading = true;
                model.status = StatusLine::info("loading pending changes");
                return vec![Cmd::LoadPending(CacheMode::Use)];
            }
            KeyCode::Char('a') => {
                model.status = model.policy.auto.toggle();
                return Vec::new();
            }
            KeyCode::Char('i') => return impact_key(model),
            KeyCode::Char(c @ ('n' | 'e' | 'x')) => return policy_edit_key(model, c),
            // `N` (a deny rule) is bound on the rules pane only.
            KeyCode::Char('N') if model.policy.pane == PolicyPane::Rules => {
                return policy_edit_key(model, 'N');
            }
            KeyCode::Tab | KeyCode::BackTab => {
                model.policy.pane = match model.policy.pane {
                    PolicyPane::RuleSets => PolicyPane::Rules,
                    PolicyPane::Rules => PolicyPane::RuleSets,
                };
                return Vec::new();
            }
            KeyCode::Enter if model.policy.pane == PolicyPane::RuleSets => {
                return deny_fallback(model, CacheMode::Use);
            }
            KeyCode::Enter if model.policy.pane == PolicyPane::Rules => {
                if let Some(r) = model.policy.rules.selected_item() {
                    model.overlay = Overlay::RuleDetail(r.view.clone());
                }
                return Vec::new();
            }
            _ => {}
        }
    }
    let before = model.policy.selected_href();
    match model.policy.pane {
        PolicyPane::RuleSets => {
            let t = &mut model.policy.rulesets;
            if t.is_searching() {
                search_key(t, k);
            } else {
                table_key(t, k, page);
            }
            t.ensure_visible(page);
        }
        PolicyPane::Rules => {
            let t = &mut model.policy.rules;
            if t.is_searching() {
                search_key(t, k);
            } else {
                table_key(t, k, page);
            }
            t.ensure_visible(page);
        }
    }
    if model.policy.selected_href() != before
        && let Some(idx) = model.index().cloned()
    {
        model
            .policy
            .rebuild_rules(&idx, model.workloads.impact_source());
        return deny_fallback(model, CacheMode::Use);
    }
    Vec::new()
}

/// `Enter` on the pending overlay: one diff request for an IP list, a
/// service or a ruleset; anything else says why there is none.
pub(super) fn pending_diff(model: &mut Model) -> Vec<Cmd> {
    let Some(change) = model
        .policy
        .pending
        .as_ref()
        .and_then(|p| p.get(model.policy.pending_selected))
        .cloned()
    else {
        return Vec::new();
    };
    let Some(kind) = crate::tui::diff::diff_object_kind(&change.object_type) else {
        model.status = StatusLine::info(match change.object_type.as_str() {
            // A rule is provisioned, and diffed, through its ruleset.
            "sec_rules" | "deny_rules" => "rule changes show in their ruleset's diff".to_string(),
            other => format!("no diff for {other}"),
        });
        return Vec::new();
    };
    if model.policy.diff_loading {
        model.status = StatusLine::info("diff already loading");
        return Vec::new();
    }
    model.policy.diff_loading = true;
    model.status = StatusLine::info(format!(
        "loading the active version of {}",
        change.name.as_deref().unwrap_or(&change.href)
    ));
    // The deny rules the Policy screen fetched for a ruleset whose list
    // payload had none: the diff reads them from the model, not the PCE.
    let fallback_deny = (kind == pce::write::ObjectKind::RuleSet)
        .then(|| model.policy.fallback_deny.get(&change.href).cloned())
        .flatten();
    vec![Cmd::LoadDiff {
        change,
        kind,
        fallback_deny,
    }]
}

pub(super) const PROVISION_DESCRIPTION: &str = "illumio-tui provision";

/// `P` in the pending overlay: provision the marked entries, or everything
/// pending when none is marked. The plan reads the pending list once more,
/// past the cache, and the confirm overlay shows the exact body.
pub(super) fn provision_key(model: &mut Model) -> Vec<Cmd> {
    if writes_refused(model) {
        return Vec::new();
    }
    if model.policy.pending.as_ref().is_none_or(Vec::is_empty) {
        model.status = StatusLine::info("nothing pending");
        return Vec::new();
    }
    let marked = &model.policy.pending_marked;
    let subset = if marked.is_empty() {
        ProvisionSubset::All
    } else {
        ProvisionSubset::Hrefs(marked.iter().cloned().collect())
    };
    model.overlay = Overlay::None;
    model.status = StatusLine::info("planning…");
    model.writes.in_flight = true;
    vec![Cmd::PlanWrite(WriteIntent::Provision(ProvisionRequest {
        description: Some(PROVISION_DESCRIPTION.into()),
        subset,
    }))]
}

/// `R` in the pending overlay: revert the marked entries to their active
/// version (a draft-only object is deleted). Never everything: with nothing
/// marked it only says how. The plan reads the pending list once more, past
/// the cache, and the confirm names each object and its fate.
pub(super) fn revert_key(model: &mut Model) -> Vec<Cmd> {
    if writes_refused(model) {
        return Vec::new();
    }
    if model.policy.pending_marked.is_empty() {
        model.status = StatusLine::info("mark changes with Space first");
        return Vec::new();
    }
    let targets = model.policy.pending_marked.iter().cloned().collect();
    model.overlay = Overlay::None;
    model.status = StatusLine::info("planning…");
    model.writes.in_flight = true;
    vec![Cmd::PlanWrite(WriteIntent::Revert(RevertRequest {
        all: false,
        targets,
    }))]
}

/// A revert's object lines name a ruleset pending only through its rules
/// by href (the pending list has no name for it): the loaded rulesets
/// (either scope) give the name when they hold it. Display only: the lines
/// are neither sent nor audited.
pub(super) fn name_rulesets(plan: &mut Plan, sets: &[pce::models::RuleSet]) {
    let Plan::Send { request, .. } = plan else {
        return;
    };
    for line in &mut request.objects {
        let Some(rest) = line.strip_prefix("rule_set /") else {
            continue;
        };
        let href = format!("/{}", rest.split(' ').next().unwrap_or_default());
        let named = sets.iter().find(|rs| {
            rs.href
                .replacen("/sec_policy/active/", "/sec_policy/draft/", 1)
                == href
        });
        if let Some(rs) = named {
            *line = line.replacen(&href, &format!("{:?}", rs.name), 1);
        }
    }
}

/// The word typed into a revert confirm before `Enter` sends it.
pub const REVERT_WORD: &str = "revert";

/// Keys of the revert confirm: letters type the word, `Enter` sends only
/// when it reads `revert`, `Esc` cancels, arrows scroll. `y` is a letter.
pub(super) fn revert_confirm_key(
    model: &mut Model,
    plan: Plan,
    scroll: u16,
    k: KeyEvent,
) -> Vec<Cmd> {
    let kind = pce::write::ObjectKind::Revert;
    let area = ratatui::layout::Rect::new(0, 0, model.size.0, model.size.1);
    let last = confirm_write_last_scroll(&plan, kind, area);
    let last = u16::try_from(last).unwrap_or(u16::MAX);
    let typed = &mut model.policy.revert_typed;
    match k.code {
        KeyCode::Enter if typed == REVERT_WORD => {
            typed.clear();
            model.policy.provision_counts =
                Some(counts_text(plan.request().and_then(|r| r.body.as_ref())));
            model.overlay = Overlay::None;
            model.status = StatusLine::info("sending …");
            model.writes.in_flight = true;
            return vec![Cmd::ExecuteWrite { plan, kind }];
        }
        KeyCode::Enter => {
            model.status = StatusLine::warn("type revert, then Enter, to send it");
        }
        KeyCode::Esc => {
            typed.clear();
            model.overlay = Overlay::None;
            model.status = StatusLine::info("cancelled, nothing sent");
        }
        KeyCode::Backspace => {
            typed.pop();
        }
        KeyCode::Char('u') if k.modifiers.contains(KeyModifiers::CONTROL) => typed.clear(),
        KeyCode::Char(c) if typed.chars().count() < REVERT_WORD.chars().count() + 8 => {
            typed.push(c)
        }
        KeyCode::Down => {
            model.overlay = Overlay::ConfirmWrite {
                plan,
                kind,
                scroll: (scroll + 1).min(last),
            };
        }
        KeyCode::Up => {
            model.overlay = Overlay::ConfirmWrite {
                plan,
                kind,
                scroll: scroll.saturating_sub(1),
            };
        }
        _ => {}
    }
    Vec::new()
}

/// `ip_lists=1, rule_sets=2`: what a provision body lists, per type, as the
/// CLI prints it.
pub(super) fn counts_text(body: Option<&serde_json::Value>) -> String {
    body.map(provision_counts)
        .unwrap_or_default()
        .iter()
        .map(|(k, n)| format!("{k}={n}"))
        .collect::<Vec<_>>()
        .join(", ")
}

/// `n` / `e` / `x` on the rulesets pane: draft only.
pub(super) fn policy_edit_key(model: &mut Model, c: char) -> Vec<Cmd> {
    if writes_refused(model) {
        return Vec::new();
    }
    if model.policy.scope == PolicyScope::Active {
        model.status = StatusLine::warn("switch to draft to edit policy");
        return Vec::new();
    }
    let intent = match model.policy.pane {
        PolicyPane::RuleSets => ruleset_intent(&model.policy, c),
        PolicyPane::Rules => rule_intent(&model.policy, c),
    };
    let Some(intent) = intent else {
        model.status = StatusLine::warn("no row selected");
        return Vec::new();
    };
    begin_edit(model, intent)
}

/// `n` a new ruleset; `e` / `x` the selected one.
pub(super) fn ruleset_intent(p: &PolicyState, c: char) -> Option<EditIntent> {
    if c == 'n' {
        return Some(EditIntent::CreateRuleSet);
    }
    let rs = p.selected_rule_set()?.clone();
    Some(if c == 'e' {
        EditIntent::UpdateRuleSet(rs)
    } else {
        EditIntent::DeleteRuleSet(rs)
    })
}

/// `n` / `N` a new allow / deny rule in the selected ruleset; `e` / `x` the
/// selected rule, allow or deny by its kind, found by href in what was loaded.
pub(super) fn rule_intent(p: &PolicyState, c: char) -> Option<EditIntent> {
    let ruleset_href = p.selected_href()?;
    match c {
        'n' => return Some(EditIntent::CreateRule { ruleset_href }),
        'N' => return Some(EditIntent::CreateDenyRule { ruleset_href }),
        _ => {}
    }
    let view = p.rules.selected_item()?;
    let href = view.href.clone();
    match (c, view.kind == RuleKind::Allow) {
        ('x', true) => Some(EditIntent::DeleteRule {
            href,
            name: rule_line(view),
        }),
        ('x', false) => Some(EditIntent::DeleteDenyRule {
            href,
            name: rule_line(view),
        }),
        (_, true) => p
            .find_rule(&href)
            .map(|(r, rs)| EditIntent::UpdateRule(r, rs)),
        (_, false) => p
            .find_deny_rule(&href)
            .map(|(d, rs)| EditIntent::UpdateDenyRule(d, rs)),
    }
}

/// `:draft` / `:active`: like `d` on the Policy screen, but explicit. The scope
/// already shown is not reloaded (like `2`); a load in flight is not doubled.
pub(super) fn show_policy(model: &mut Model, scope: PolicyScope) -> Vec<Cmd> {
    if !model.is_connected() {
        model.status = StatusLine::error("not connected");
        return Vec::new();
    }
    if model.policy.scope != scope {
        if model.policy.loading {
            model.screen = Screen::Policy;
            model.status = StatusLine::info("already loading");
            return Vec::new();
        }
        model.policy.scope = scope;
        model.policy.form_drafts_loading = false;
        model.policy.form_drafts_stale = false;
        model.policy.clear();
    }
    switch(model, Screen::Policy)
}
