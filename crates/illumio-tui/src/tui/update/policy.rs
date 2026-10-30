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
    let search = model.policy.search.is_some();
    crate::tui::ui::layout::chrome(area)
        .map(|c| {
            if search {
                crate::tui::ui::screens::policy::found_rows(c.body)
            } else {
                crate::tui::ui::screens::policy::table_rows(c.body, model.policy.pane)
            }
        })
        .unwrap_or(10)
        .max(1)
}

/// Shows the `scope` policy: what was loaded is forgotten, its rulesets
/// load (through the cache). `d`, and Explain's `Enter` from active.
pub(super) fn set_policy_scope(model: &mut Model, scope: PolicyScope) -> Vec<Cmd> {
    model.policy.scope = scope;
    model.policy.form_drafts_loading = false;
    model.policy.form_drafts_stale = false;
    model.policy.jump = None;
    model.policy.clear();
    model.objects.ip_lists.recount(&model.policy);
    load_rule_sets(model, CacheMode::Use)
}

pub(super) fn policy_key(model: &mut Model, k: KeyEvent) -> Vec<Cmd> {
    if model.policy.search.is_some()
        && let Some(cmds) = found_key(model, k)
    {
        return cmds;
    }
    let page = policy_page(model);
    if !model.policy.is_searching() {
        // Explain's pending `Enter` (`E`) is dropped by `Esc` and `d`.
        if matches!(k.code, KeyCode::Esc | KeyCode::Char('d')) {
            model.policy.jump = None;
        }
        match k.code {
            KeyCode::Char('f') => {
                open_rule_filter(model);
                return Vec::new();
            }
            KeyCode::Char('d') => {
                if model.policy.loading {
                    model.status = StatusLine::info("already loading");
                    return Vec::new();
                }
                let other = match model.policy.scope {
                    PolicyScope::Draft => PolicyScope::Active,
                    PolicyScope::Active => PolicyScope::Draft,
                };
                return set_policy_scope(model, other);
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
            KeyCode::Char('t') => return toggle_key(model),
            KeyCode::Char('I') if model.policy.pane == PolicyPane::Rules => {
                let href = model
                    .policy
                    .rules
                    .selected_item()
                    .map(|r| r.href.clone())
                    .unwrap_or_default();
                return open_matched(model, &href);
            }
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

/// `t`: the selected ruleset (rulesets pane) or rule (rules pane, allow or
/// deny) flipped between enabled and disabled. A draft change planned at
/// once (one `PUT` of `enabled` only); the confirm shows it, `y` sends it,
/// and it shows in Pending until provisioned. Draft only.
pub(super) fn toggle_key(model: &mut Model) -> Vec<Cmd> {
    plan_toggle(model, toggle_intent)
}

/// `t` (the panes or the search list): refused with writes off or on
/// active policy, said on the status; else the write `intent` makes of
/// the selected row planned (`no row selected` without one).
fn plan_toggle(model: &mut Model, intent: fn(&PolicyState) -> Option<WriteIntent>) -> Vec<Cmd> {
    if writes_refused(model) {
        return Vec::new();
    }
    if model.policy.scope == PolicyScope::Active {
        model.status = StatusLine::warn("switch to draft to edit policy");
        return Vec::new();
    }
    let Some(intent) = intent(&model.policy) else {
        model.status = StatusLine::warn("no row selected");
        return Vec::new();
    };
    begin_start(model, Start::Plan(intent))
}

/// The write `t` plans for the focused row: `enabled` flipped, nothing else.
pub(super) fn toggle_intent(p: &PolicyState) -> Option<WriteIntent> {
    match p.pane {
        PolicyPane::RuleSets => {
            let rs = p.selected_rule_set()?;
            Some(WriteIntent::UpdateRuleSet {
                href: rs.href.clone(),
                patch: pce::write::RuleSetPatch {
                    enabled: Some(!rs.enabled),
                    ..Default::default()
                },
            })
        }
        PolicyPane::Rules => {
            let v = p.rules.selected_item()?;
            Some(rule_toggle(&v.href, v.kind, v.enabled))
        }
    }
}

/// An allow or deny rule's `enabled` flipped.
pub(super) fn rule_toggle(href: &str, kind: RuleKind, enabled: bool) -> WriteIntent {
    let href = href.to_string();
    if kind == RuleKind::Allow {
        WriteIntent::UpdateRule {
            href,
            patch: pce::write::RulePatch {
                enabled: Some(!enabled),
                ..Default::default()
            },
        }
    } else {
        WriteIntent::UpdateDenyRule {
            href,
            patch: pce::write::DenyRulePatch {
                enabled: Some(!enabled),
                ..Default::default()
            },
        }
    }
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

/// `f` on Policy: the rule search editor, on the search in force (or the
/// last one applied).
pub(super) fn open_rule_filter(model: &mut Model) {
    let p = &model.policy;
    let text = p
        .search
        .as_ref()
        .map_or_else(|| p.last_search.clone(), ToString::to_string);
    let mut e = crate::tui::rule_search::RuleFilterEditor::new(&text);
    rule_filter_unknown(model, &mut e);
    model.overlay = Overlay::RuleFilter(e);
}

/// The editor's unknown names for its current text (none while it does
/// not parse): once per edit, never per frame.
fn rule_filter_unknown(model: &Model, e: &mut crate::tui::rule_search::RuleFilterEditor) {
    e.unknown = match (e.parse(), model.index()) {
        (Ok(f), Some(idx)) => f.unknown(idx),
        _ => Vec::new(),
    };
}

/// Keys of the rule search editor: typing, `Ctrl-N`/`Ctrl-P` completion,
/// `Ctrl-U` clears, `Esc` closes (first puts a completed token back),
/// `Enter` applies (an empty line clears the search). Never a request.
pub(super) fn rule_filter_key(
    model: &mut Model,
    mut e: crate::tui::rule_search::RuleFilterEditor,
    k: KeyEvent,
) -> Vec<Cmd> {
    let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
    if ctrl && matches!(k.code, KeyCode::Char('n' | 'p')) {
        complete_field(model, &mut e, k.code == KeyCode::Char('n'));
        rule_filter_unknown(model, &mut e);
        model.overlay = Overlay::RuleFilter(e);
        return Vec::new();
    }
    let esc = k.code == KeyCode::Esc;
    if e.end_completion(esc) && esc {
        model.overlay = Overlay::RuleFilter(e);
        return Vec::new();
    }
    match k.code {
        KeyCode::Esc => {
            model.overlay = Overlay::None;
            return Vec::new();
        }
        // The help, unless inside a quoted name (where `?` is text);
        // closing it comes back to the editor as it was.
        KeyCode::Char('?') if e.value.matches('"').count().is_multiple_of(2) => {
            model.overlay = Overlay::RuleFilter(e);
            open_help(model);
            return Vec::new();
        }
        KeyCode::Backspace => e.pop(),
        KeyCode::Char('u') if ctrl => e.clear(),
        KeyCode::Char(c) => e.push(c),
        KeyCode::Enter => match bare::apply(
            &mut e.value,
            Grammar::Rules,
            &loaded_label_pairs(model),
            pce::RuleFilter::parse,
        ) {
            Ok(f) => {
                model.overlay = Overlay::None;
                apply_rule_search(model, f);
                return Vec::new();
            }
            Err(err) => e.error = Some(err),
        },
        _ => {}
    }
    rule_filter_unknown(model, &mut e);
    model.overlay = Overlay::RuleFilter(e);
    Vec::new()
}

/// A search from the editor: the flat list of matching rules from the
/// loaded rulesets (an empty search clears it). Local, never a request.
pub(super) fn apply_rule_search(model: &mut Model, f: pce::RuleFilter) {
    if f.is_empty() {
        clear_rule_search(model);
        return;
    }
    // Names made the objects' own (`ipl:Any` → the `Any (0.0.0.0/0 and
    // ::/0)` list); those naming nothing or several are kept and said.
    let (f, mut unknown) = match model.index() {
        Some(i) => f.resolve(i),
        None => (f, Vec::new()),
    };
    if f.names_workloads() && model.workloads.impact_source().is_none() {
        unknown.push("wl: names need the workloads (i loads them)".into());
    }
    model.policy.last_search = f.to_string();
    model.policy.search = Some(f);
    model.policy.found.search_clear();
    model.rebuild_rules();
    model.policy.found.home();
    let p = &model.policy;
    let mut text = format!(
        "{} · {}",
        crate::tui::ui::screens::policy::matched_text(p.found.len(), p.found_loaded),
        p.last_search
    );
    if p.found_deny_unloaded > 0 {
        text.push_str(&format!(" · {}", deny_unloaded_text(p.found_deny_unloaded)));
    }
    model.status = if unknown.is_empty() {
        StatusLine::info(text)
    } else {
        StatusLine::warn(format!("{text} · {}", unknown.join(", ")))
    };
}

/// `c` on the search list: back to rulesets | rules; `f` offers the
/// search again.
pub(super) fn clear_rule_search(model: &mut Model) {
    model.policy.put_search_aside();
    model.status = StatusLine::info("rule search cleared");
}

/// Keys while the search list shows: `None` for those the Policy screen
/// handles as always (`d` `r` `p` `a` `i`). `Enter` opens the rule in its
/// ruleset, `c` clears, `f` edits the search, `t` toggles the rule; the
/// pane and edit keys say how to get to the panes.
fn found_key(model: &mut Model, k: KeyEvent) -> Option<Vec<Cmd>> {
    let page = policy_page(model);
    if model.policy.found.is_searching() {
        drive_table(&mut model.policy.found, k, page);
        return Some(Vec::new());
    }
    match k.code {
        KeyCode::Char('d' | 'r' | 'p' | 'a' | 'i') => return None,
        KeyCode::Char('f') => open_rule_filter(model),
        KeyCode::Char('c') => clear_rule_search(model),
        KeyCode::Enter => return Some(open_found(model)),
        KeyCode::Char('t') => {
            return Some(plan_toggle(model, |p| {
                let r = p.found.selected_item()?;
                Some(rule_toggle(&r.href, r.kind, r.enabled))
            }));
        }
        KeyCode::Char('I') => {
            let href = model
                .policy
                .found
                .selected_item()
                .map(|r| r.href.clone())
                .unwrap_or_default();
            return Some(open_matched(model, &href));
        }
        KeyCode::Tab | KeyCode::BackTab | KeyCode::Char('n' | 'N' | 'e' | 'x') => {
            model.status =
                StatusLine::info("Enter opens the rule in its ruleset · c clears the search");
        }
        _ => drive_table(&mut model.policy.found, k, page),
    }
    Some(Vec::new())
}

/// `Enter` on the search list: the rule's ruleset selected, its rules
/// shown with the rule selected and focused; the search is put aside (`f`
/// offers it again). Asks for the ruleset's deny rules when not loaded,
/// as selecting it does.
fn open_found(model: &mut Model) -> Vec<Cmd> {
    let Some(row) = model.policy.found.selected_item().cloned() else {
        return Vec::new();
    };
    let p = &mut model.policy;
    p.put_search_aside();
    p.rulesets.select_where(|r| r.href == row.ruleset_href);
    if p.selected_href().as_deref() != Some(row.ruleset_href.as_str()) {
        // A table search hid it.
        p.rulesets.search_clear();
        p.rulesets.select_where(|r| r.href == row.ruleset_href);
    }
    model.rebuild_rules();
    let p = &mut model.policy;
    p.rules.search_clear();
    p.rules.select_where(|r| r.href == row.href);
    p.pane = PolicyPane::RuleSets;
    let page = policy_page(model);
    model.policy.rulesets.ensure_visible(page);
    model.policy.pane = PolicyPane::Rules;
    let page = policy_page(model);
    model.policy.rules.ensure_visible(page);
    model.status = StatusLine::info(format!(
        "{} · f then Enter: back to the search",
        row.ruleset
    ));
    deny_fallback(model, CacheMode::Use)
}

/// `2 rulesets may hold deny rules not loaded`: the search's caveat on a
/// PCE that may keep deny rules apart (`PolicyState::deny_maybe_apart`).
pub fn deny_unloaded_text(n: usize) -> String {
    format!("{} may hold deny rules not loaded", counted_rulesets(n))
}

/// `1 ruleset`, `2 rulesets`.
fn counted_rulesets(n: usize) -> String {
    if n == 1 {
        "1 ruleset".into()
    } else {
        format!("{n} rulesets")
    }
}

/// `I` on the rules pane or in a rule's detail: the workloads its impact
/// counted, providers first (`Tab` the consumers). From the cached
/// workloads only: without them it says to press `i`, and asks nothing.
pub(super) fn open_matched(model: &mut Model, href: &str) -> Vec<Cmd> {
    use pce::impact::Side;
    if href.is_empty() {
        model.status = StatusLine::warn("no rule selected");
        return Vec::new();
    }
    match crate::tui::impact_list::build(model, href, Side::Provider) {
        Ok(mut v) => {
            // Opened from a rule's detail: `Esc` goes back to it.
            if let Overlay::RuleDetail(r) = &model.overlay {
                v.back = Some(r.clone());
            }
            model.policy.matched = Some(v);
            model.overlay = Overlay::Matched;
        }
        Err(e) => model.status = StatusLine::warn(e),
    }
    Vec::new()
}

/// Rows of the matched list's table on screen.
fn matched_page(model: &Model) -> usize {
    let area = ratatui::layout::Rect::new(0, 0, model.size.0, model.size.1);
    crate::tui::ui::screens::policy::matched_layout(area)
        .table
        .height
        .saturating_sub(1)
        .max(1) as usize
}

/// Keys of the matched list: `Tab` the other side, `/` search, `Enter`
/// the workload on Workloads, `:export <path>` (typed after `:`) the list
/// as CSV, `Esc`/`q` close; the table keys move. Never a request.
pub(super) fn matched_key(model: &mut Model, k: KeyEvent) -> Vec<Cmd> {
    use crate::tui::impact_list::other;
    let page = matched_page(model);
    let Some(v) = model.policy.matched.as_mut() else {
        model.overlay = Overlay::None;
        return Vec::new();
    };
    if let Some(input) = v.prompt.as_mut() {
        match k.code {
            KeyCode::Esc => v.prompt = None,
            KeyCode::Backspace => {
                input.pop();
            }
            KeyCode::Char('u') if k.modifiers.contains(KeyModifiers::CONTROL) => input.clear(),
            KeyCode::Char(c) => input.push(c),
            KeyCode::Enter => {
                let input = v.prompt.take().unwrap_or_default();
                return match commands::parse(&input) {
                    Ok(Command::Export { path, force }) => {
                        let (headers, rows) = v.table.export();
                        vec![Cmd::Export {
                            path,
                            force,
                            headers,
                            rows,
                        }]
                    }
                    Ok(_) => {
                        model.status = StatusLine::warn("only :export <path> works in this list");
                        Vec::new()
                    }
                    Err(e) => {
                        model.status = StatusLine::error(e);
                        Vec::new()
                    }
                };
            }
            _ => {}
        }
        return Vec::new();
    }
    if v.table.is_searching() {
        drive_table(&mut v.table, k, page);
        return Vec::new();
    }
    match k.code {
        KeyCode::Esc | KeyCode::Char('q') => {
            let back = model.policy.matched.take().and_then(|v| v.back);
            model.overlay = back.map_or(Overlay::None, Overlay::RuleDetail);
        }
        KeyCode::Char('?') => open_help(model),
        KeyCode::Tab | KeyCode::BackTab => {
            let side = other(v.side);
            v.show(side);
        }
        KeyCode::Char(':') => v.prompt = Some(String::new()),
        KeyCode::Enter => {
            let Some(row) = v.table.selected_item().cloned() else {
                return Vec::new();
            };
            model.policy.matched = None;
            model.overlay = Overlay::None;
            let cmds = switch(model, Screen::Workloads);
            if model.workloads.select_href(&row.href) {
                let page = workloads_page(model);
                model.workloads.ensure_visible(page);
                model.status = StatusLine::info(row.hostname);
            } else {
                model.status = StatusLine::warn(format!(
                    "{} is hidden by the Workloads filters or search (c, Esc clear them)",
                    row.hostname
                ));
            }
            return cmds;
        }
        _ => drive_table(&mut v.table, k, page),
    }
    Vec::new()
}
