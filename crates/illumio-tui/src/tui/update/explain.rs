//! `E`: the Explain overlay. Opening it on a flow (Flows, an App-to-app
//! pair holding one flow, an app overlay peer row), its keys, and `Enter`'s
//! jump to the rule in Policy (draft). Explain itself sends nothing: `D`
//! asks first (the existing draft evaluation), `Enter` lets the Policy
//! screen load what it lacks, as switching to it always does.
use super::*;
use crate::tui::explain::{DstEnforcement, ExplainView, PolicySource};
use pce::explain::{MatchKind, canon_href};
use pce::traffic::ResolvedFlow;

/// The Explain view of `flow` from what is loaded: the draft rulesets (and
/// the deny rules fetched for them), else the active ones the Policy screen
/// shows, the name index, the workloads.
pub(super) fn explain_view(model: &Model, flow: ResolvedFlow, from_app: bool) -> ExplainView {
    let p = &model.policy;
    let (sets, policy) = match model.draft_rulesets() {
        Some(s) => (s, PolicySource::Draft),
        None if p.loaded_at.is_some() => (p.raw.as_slice(), PolicySource::Active),
        None => (&[][..], PolicySource::NotLoaded),
    };
    // The fetched deny rules are those of the scope the Policy screen shows.
    let none = std::collections::HashMap::new();
    let same_scope = match policy {
        PolicySource::Draft => p.scope == PolicyScope::Draft,
        PolicySource::Active => true,
        PolicySource::NotLoaded => false,
    };
    let extra = if same_scope { &p.fallback_deny } else { &none };
    let dst = match &flow.dst.workload_href {
        None => DstEnforcement::NotWorkload,
        Some(href) => model
            .workloads
            .loaded_at
            .and_then(|_| model.workloads.raw.iter().find(|w| &w.href == href))
            .and_then(|w| w.enforcement())
            .map_or(DstEnforcement::Unknown, |m| {
                DstEnforcement::Mode(m.to_string())
            }),
    };
    let mode = match &dst {
        DstEnforcement::Mode(m) => Some(m.as_str()),
        _ => None,
    };
    let empty = pce::NameIndex::default();
    let idx = model.index().map_or(&empty, |i| i.as_ref());
    let explanation = pce::explain::explain(&flow, sets, extra, idx, mode);
    // `D` evaluates the result on screen: a flow of another one (the app
    // overlay's unfocused result) is not in it.
    let on_screen = model.traffic.loaded_flows().contains(&flow);
    ExplainView {
        flow,
        explanation,
        policy,
        dst,
        selected: 0,
        from_app,
        on_screen,
    }
}

fn open(model: &mut Model, flow: ResolvedFlow, from_app: bool) -> Vec<Cmd> {
    let v = explain_view(model, flow, from_app);
    model.status = StatusLine::info(format!("explain: {} (no request)", v.explanation.verdict));
    model.overlay = Overlay::Explain(Box::new(v));
    Vec::new()
}

/// `E` on the Traffic screen: the selected flow (Flows), or the one flow
/// behind an App-to-app pair row.
pub(super) fn explain_key(model: &mut Model) -> Vec<Cmd> {
    let t = &model.traffic;
    let flow = match t.view {
        TrafficView::Flows => match t.selected_flow() {
            Some(r) => (*r.flow).clone(),
            None if t.flows_grouped() && t.flows_grouped.selected_item().is_some() => {
                model.status = StatusLine::warn("select a flow row (a header holds a whole app)");
                return Vec::new();
            }
            None => {
                model.status = StatusLine::warn("no row selected");
                return Vec::new();
            }
        },
        TrafficView::Aggregate if t.apps_state.by == By::Pairs => {
            let Some(g) = t.pairs_table().selected_item() else {
                model.status = StatusLine::warn("no row selected");
                return Vec::new();
            };
            let keys = t.effective_keys();
            let decisions = t.apps_state.decisions;
            let flows: Vec<&ResolvedFlow> = t
                .loaded_flows()
                .iter()
                .filter(|f| decisions.keeps(f) && g.contains(f, &keys))
                .collect();
            match flows.as_slice() {
                [f] => (*f).clone(),
                [] => {
                    model.status = StatusLine::warn("no loaded flow behind this row");
                    return Vec::new();
                }
                many => {
                    model.status = StatusLine::warn(format!(
                        "{} flows behind this pair: Enter, then E on one",
                        many.len()
                    ));
                    return Vec::new();
                }
            }
        }
        _ => {
            model.status = StatusLine::warn("E explains a flow: select one in the Flows view");
            return Vec::new();
        }
    };
    open(model, flow, false)
}

/// `E` on an app overlay peer row: the first loaded flow behind it that no
/// mark squelches (unless `M` shows them; the status says how many there
/// are).
pub(super) fn explain_app_key(model: &mut Model) -> Vec<Cmd> {
    let Some(v) = model.app_view.as_ref() else {
        return Vec::new();
    };
    let Some((side, p)) = v.selected_peer() else {
        model.status = StatusLine::warn("select a peer row (Inbound or Outbound)");
        return Vec::new();
    };
    let focus = &v.focus;
    let t = &model.traffic;
    let marks = t.active_marks().filter(|_| !t.show_squelched);
    let squelched = |f: &ResolvedFlow| {
        marks
            .as_ref()
            .and_then(|a| a.classify(f))
            .is_some_and(|h| h.action == pce::marks::Action::Squelch)
    };
    let flows: Vec<&ResolvedFlow> = t
        .flows_for(focus)
        .iter()
        .filter(|f| {
            let (app_ep, peer_ep) = match side {
                Direction::In => (&f.dst, &f.src),
                Direction::Out => (&f.src, &f.dst),
            };
            f.policy_decision == p.decision
                && f.service_text() == p.service
                && pce::traffic::group_key(app_ep, &focus.keys) == focus.app
                && pce::traffic::group_key(peer_ep, &focus.keys) == p.peer
        })
        .collect();
    let shown: Vec<&ResolvedFlow> = flows.iter().copied().filter(|f| !squelched(f)).collect();
    let flows = if shown.is_empty() { flows } else { shown };
    let Some(first) = flows.first().map(|f| (*f).clone()) else {
        model.status = StatusLine::warn("no loaded flow behind this row");
        return Vec::new();
    };
    let n = flows.len();
    let cmds = open(model, first, true);
    if n > 1 {
        model.status = StatusLine::info(format!(
            "explain: the first of {n} flows behind this row (Enter on the row lists them)"
        ));
    }
    cmds
}

/// The overlay's keys: `j`/`k` rules, `Enter` the rule in Policy (draft),
/// `D` the draft evaluation (asks first), `w` an allow rule from the flow
/// (writes), `Esc`/`q` back.
pub(super) fn explain_overlay_key(
    model: &mut Model,
    mut v: Box<ExplainView>,
    k: KeyEvent,
) -> Vec<Cmd> {
    match k.code {
        KeyCode::Esc | KeyCode::Char('q') => {
            model.overlay = if v.from_app && model.app_view.is_some() {
                Overlay::App
            } else {
                Overlay::None
            };
            Vec::new()
        }
        KeyCode::Down | KeyCode::Char('j') => {
            v.move_cursor(1);
            model.overlay = Overlay::Explain(v);
            Vec::new()
        }
        KeyCode::Up | KeyCode::Char('k') => {
            v.move_cursor(-1);
            model.overlay = Overlay::Explain(v);
            Vec::new()
        }
        KeyCode::Enter => jump_to_rule(model, v),
        KeyCode::Char('D') => explain_draft(model, &v),
        KeyCode::Char('w') => explain_rule_form(model, &v),
        _ => Vec::new(),
    }
}

/// `D` in the overlay: the existing draft evaluation of the result on
/// screen, always behind one confirm (its cost in the question).
fn explain_draft(model: &mut Model, v: &ExplainView) -> Vec<Cmd> {
    let t = &model.traffic;
    if v.explanation.evaluated {
        model.status =
            StatusLine::info("already evaluated against the draft (D on Traffic clears)");
        return Vec::new();
    }
    if !v.on_screen {
        model.status = StatusLine::warn(
            "this flow is not in the result on screen (the app's own query): D there cannot evaluate it",
        );
        return Vec::new();
    }
    if t.is_draft_running() {
        model.status = StatusLine::info("draft re-evaluation running: E again once it is in");
        return Vec::new();
    }
    if t.is_running() {
        model.status = StatusLine::info("query running: D once it is done");
        return Vec::new();
    }
    let Some(target) = draft_target(t) else {
        model.status = StatusLine::info("no result to evaluate: r runs the query first");
        return Vec::new();
    };
    let q = target.queries;
    let (reason, hrefs) = match draft_ask(&target.filter, &target.hrefs, q) {
        Some(DraftAsk::Rerun) => (
            format!(
                "No query of this session to re-evaluate (a cached or dropped result). Run it again with the draft: {}? (y/n)",
                draft_cost(q, true)
            ),
            Vec::new(),
        ),
        Some(DraftAsk::Costly(why)) => (
            format!(
                "{why}. Re-evaluate against the draft policy anyway: {}? (y/n)",
                draft_cost(q, false)
            ),
            target.hrefs,
        ),
        None => (
            format!(
                "Evaluate the result against the draft policy, so the PCE names each flow's rules: {}? (y/n)",
                draft_cost(q, false)
            ),
            target.hrefs,
        ),
    };
    model.explain_back = Some(Box::new(v.clone()));
    model.overlay = Overlay::Confirm {
        reason,
        action: PendingAction::RunDraft {
            filter: target.filter,
            focus: target.focus,
            hrefs,
        },
    };
    Vec::new()
}

/// `w` in the overlay: the `w` form for this flow (writes armed), as `w`
/// on its row; from the app overlay, the form goes back there.
fn explain_rule_form(model: &mut Model, v: &ExplainView) -> Vec<Cmd> {
    if !v.offers_rule() {
        model.status = StatusLine::warn(
            if v.explanation
                .rules
                .iter()
                .any(|r| r.kind == MatchKind::OverrideDeny)
            {
                "an override deny rule blocks this flow: an allow rule would not change it"
            } else {
                "this flow is allowed; nothing to add"
            },
        );
        return Vec::new();
    }
    if writes_refused(model) {
        return Vec::new();
    }
    let cmds = begin_edit(model, EditIntent::RuleFromFlow(Box::new(v.flow.clone())));
    if v.from_app {
        model.form_from_app = matches!(model.overlay, Overlay::Form(_));
    }
    cmds
}

/// `Enter` in the overlay: the selected rule on the Policy screen, in the
/// draft, its ruleset and rule selected. Shows the draft first when the
/// screen shows the active policy (as `d` there: one request, counted);
/// a policy still on its way is selected in when it lands.
fn jump_to_rule(model: &mut Model, v: Box<ExplainView>) -> Vec<Cmd> {
    let Some(r) = v.selected_rule() else {
        model.status = StatusLine::info("no rule to open: none names this flow");
        return Vec::new();
    };
    let Some(rs) = r.ruleset_href.clone() else {
        model.status = StatusLine::info(match r.kind {
            MatchKind::Essential => "an essential service rule: built into the PCE, no ruleset",
            _ => "this rule's href names no ruleset",
        });
        return Vec::new();
    };
    let rule = r.href.clone();
    if model.policy.scope != PolicyScope::Draft {
        if model.policy.loading {
            model.status = StatusLine::info("rulesets loading: Enter again once they are in");
            return Vec::new();
        }
        model.overlay = Overlay::None;
        model.screen = Screen::Policy;
        // As `d` on the Policy screen: the draft rulesets (cached: none sent).
        let cmds = set_policy_scope(model, PolicyScope::Draft);
        model.policy.jump = Some((rs, rule));
        return cmds;
    }
    model.overlay = Overlay::None;
    if model.policy.loaded_at.is_some() {
        // Selected first, so the deny-rules fallback `switch` asks is for
        // this ruleset.
        if select_rule(model, &rs, &rule) == Jump::WaitDeny {
            model.policy.jump = Some((rs.clone(), rule.clone()));
        }
    }
    if model.policy.loading || model.policy.loaded_at.is_none() {
        model.policy.jump = Some((rs, rule));
    }
    switch(model, Screen::Policy)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Jump {
    /// The rule is selected.
    Done,
    /// The ruleset is not loaded: nothing to wait for.
    NoRuleset,
    /// A deny rule of a ruleset whose deny rules come on demand: selected
    /// once they land.
    WaitDeny,
    /// The ruleset is selected, the rule is not in it.
    NoRule,
}

/// Selects the ruleset `rs` and its rule `rule` on the Policy screen (the
/// rules pane focused), hrefs compared as drafts. No request.
pub(super) fn select_rule(model: &mut Model, rs: &str, rule: &str) -> Jump {
    let (rs, rule) = (canon_href(rs), canon_href(rule));
    let p = &mut model.policy;
    p.pane = PolicyPane::RuleSets;
    p.rulesets.select_where(|r| canon_href(&r.href) == rs);
    let Some(name) = p
        .rulesets
        .selected_item()
        .filter(|r| canon_href(&r.href) == rs)
        .map(|r| r.name.clone())
    else {
        model.status = StatusLine::warn(format!(
            "{rs} is not in the loaded draft policy (r reloads it)"
        ));
        return Jump::NoRuleset;
    };
    model.rebuild_rules();
    let p = &mut model.policy;
    p.rules.select_where(|r| canon_href(&r.href) == rule);
    if let Some(r) = p
        .rules
        .selected_item()
        .filter(|r| canon_href(&r.href) == rule)
    {
        model.status = StatusLine::info(format!("{name}: {}", rule_line(r)));
        p.pane = PolicyPane::Rules;
        return Jump::Done;
    }
    let deny_on_demand = MatchKind::of_href(&rule).is_deny()
        && p.raw
            .iter()
            .find(|s| canon_href(&s.href) == rs)
            .is_some_and(|s| s.deny_rules.is_empty())
        && !p.fallback_deny.keys().any(|h| canon_href(h) == rs);
    if deny_on_demand {
        model.status = StatusLine::info(format!("{name}: loading its deny rules"));
        return Jump::WaitDeny;
    }
    model.status = StatusLine::warn(format!(
        "{name}: the rule is not in it any more (r reloads)"
    ));
    Jump::NoRule
}

/// Explain's pending `Enter`, once the draft rulesets (`deny: None`) or
/// the deny rules of ruleset `deny` land: selects the rule, once. Deny
/// rules of another ruleset leave it waiting.
pub(super) fn apply_policy_jump(model: &mut Model, deny: Option<&str>) {
    if model.policy.scope != PolicyScope::Draft {
        model.policy.jump = None;
        return;
    }
    let Some((rs, rule)) = model.policy.jump.take() else {
        return;
    };
    if deny.is_some_and(|h| canon_href(h) != canon_href(&rs)) {
        model.policy.jump = Some((rs, rule));
        return;
    }
    if select_rule(model, &rs, &rule) == Jump::WaitDeny && deny.is_none() {
        model.policy.jump = Some((rs, rule));
    }
}

/// A failed deny-rule load of the ruleset a pending jump waits for: the
/// jump is dropped.
pub(super) fn drop_policy_jump(model: &mut Model, ruleset: &str) {
    if model
        .policy
        .jump
        .as_ref()
        .is_some_and(|(rs, _)| canon_href(rs) == canon_href(ruleset))
    {
        model.policy.jump = None;
    }
}
