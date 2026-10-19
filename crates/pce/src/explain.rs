//! Which rule allows or blocks a flow (`E` in the TUI, `traffic explain`):
//! the rule hrefs the PCE puts on a flow once the query was evaluated
//! against the draft policy (`update_rules`), resolved through the loaded
//! draft rulesets. Pure: no request.
use crate::models::{DenyRule, DraftDecision, PolicyDecision, RuleSet};
use crate::resolve::{NameIndex, RuleKind, RuleSetView, RuleView};
use crate::traffic::ResolvedFlow;
use serde::Serialize;
use std::collections::HashMap;

/// What kind of rule a flow's rule entry is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MatchKind {
    Allow,
    Deny,
    OverrideDeny,
    /// An essential service rule (`essential:<name>`): built into the PCE.
    Essential,
    /// An href of no rule path this knows.
    Unknown,
}

impl MatchKind {
    pub fn as_str(self) -> &'static str {
        match self {
            MatchKind::Allow => "allow",
            MatchKind::Deny => "deny",
            MatchKind::OverrideDeny => "override_deny",
            MatchKind::Essential => "essential",
            MatchKind::Unknown => "unknown",
        }
    }

    /// By the href alone: `…/sec_rules/N` allows, `…/deny_rules/N` and
    /// `…/enforcement_boundaries/N` deny (an override deny is known only
    /// once the rule is found), `essential:<name>` is built in.
    pub fn of_href(href: &str) -> Self {
        if href.starts_with("essential:") {
            return MatchKind::Essential;
        }
        let mut parts = href.trim_end_matches('/').rsplit('/');
        let _id = parts.next();
        match parts.next() {
            Some("sec_rules") => MatchKind::Allow,
            Some("deny_rules" | "enforcement_boundaries") => MatchKind::Deny,
            _ => MatchKind::Unknown,
        }
    }

    fn of_rule(kind: RuleKind) -> Self {
        match kind {
            RuleKind::Allow => MatchKind::Allow,
            RuleKind::Deny => MatchKind::Deny,
            RuleKind::OverrideDeny => MatchKind::OverrideDeny,
        }
    }

    pub fn is_deny(self) -> bool {
        matches!(self, MatchKind::Deny | MatchKind::OverrideDeny)
    }
}

/// One rule behind a flow: its href as the flow names it and, when the
/// loaded policy holds it (`resolved`), its ruleset and rule in names.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct MatchedRule {
    pub href: String,
    pub kind: MatchKind,
    /// Found in the loaded policy: the fields below are filled.
    pub resolved: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ruleset: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ruleset_href: Option<String>,
    pub scopes: Vec<String>,
    pub consumers: Vec<String>,
    pub providers: Vec<String>,
    pub services: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

impl MatchedRule {
    /// `role=web → role=db · PostgreSQL`: consumers → providers, services.
    pub fn summary(&self) -> String {
        let join = |v: &[String]| {
            if v.is_empty() {
                "-".to_string()
            } else {
                v.join(", ")
            }
        };
        format!(
            "{} → {} · {}",
            join(&self.consumers),
            join(&self.providers),
            join(&self.services)
        )
    }
}

/// An href with `/api/v2` dropped and the active policy read as the draft:
/// the form rule hrefs are compared in.
pub fn canon_href(href: &str) -> String {
    href.trim_start_matches("/api/v2")
        .trim_end_matches('/')
        .replace("/sec_policy/active/", "/sec_policy/draft/")
}

/// The ruleset an href of one of its rules names (`…/rule_sets/100`).
pub fn ruleset_of(href: &str) -> Option<String> {
    crate::impact::rule_set_href_of(&canon_href(href)).map(str::to_string)
}

/// Resolves a flow's rule entries through `sets` (the draft rulesets) and
/// `extra_deny` (deny rules fetched per ruleset, by ruleset href, for
/// rulesets that carry none inline). An entry the policy does not hold
/// keeps its href, its kind read from the href.
pub fn resolve_rules(
    rules: &[String],
    sets: &[RuleSet],
    extra_deny: &HashMap<String, Vec<DenyRule>>,
    idx: &NameIndex,
) -> Vec<MatchedRule> {
    rules
        .iter()
        .map(|href| resolve_one(href, sets, extra_deny, idx))
        .collect()
}

fn resolve_one(
    href: &str,
    sets: &[RuleSet],
    extra_deny: &HashMap<String, Vec<DenyRule>>,
    idx: &NameIndex,
) -> MatchedRule {
    let unresolved = MatchedRule {
        href: href.to_string(),
        kind: MatchKind::of_href(href),
        resolved: false,
        ruleset: None,
        ruleset_href: ruleset_of(href),
        scopes: Vec::new(),
        consumers: Vec::new(),
        providers: Vec::new(),
        services: Vec::new(),
        enabled: None,
        description: None,
    };
    let key = canon_href(href);
    let Some(rs_href) = unresolved.ruleset_href.as_deref() else {
        return unresolved;
    };
    let Some(rs) = sets.iter().find(|s| canon_href(&s.href) == rs_href) else {
        return unresolved;
    };
    let view = rs
        .rules
        .iter()
        .find(|r| canon_href(&r.href) == key)
        .map(|r| RuleView::from_rule(r, idx))
        .or_else(|| {
            rs.deny_rules
                .iter()
                .chain(extra_deny.get(&rs.href).into_iter().flatten())
                .find(|d| canon_href(&d.href) == key)
                .map(|d| RuleView::from_deny(d, idx))
        });
    let Some(view) = view else {
        return MatchedRule {
            ruleset: Some(rs.name.clone()),
            scopes: RuleSetView::from_rule_set(rs, idx).scopes,
            ..unresolved
        };
    };
    MatchedRule {
        href: href.to_string(),
        kind: MatchKind::of_rule(view.kind),
        resolved: true,
        ruleset: Some(rs.name.clone()),
        ruleset_href: Some(rs.href.clone()),
        scopes: RuleSetView::from_rule_set(rs, idx).scopes,
        consumers: view.consumers.iter().map(|a| a.name.clone()).collect(),
        providers: view.providers.iter().map(|a| a.name.clone()).collect(),
        services: view.services.iter().map(|s| s.name.clone()).collect(),
        enabled: Some(view.enabled && rs.enabled),
        description: view.description.clone(),
    }
}

/// The answer to "which rule allows or blocks this flow".
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Explanation {
    pub reported: PolicyDecision,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub draft: Option<DraftDecision>,
    /// The flow was evaluated against the draft (`update_rules`): only
    /// then does the PCE name the rules behind it.
    pub evaluated: bool,
    pub rules: Vec<MatchedRule>,
    /// One sentence: what decides the flow.
    pub verdict: String,
}

/// Explains `flow` through the loaded draft rulesets. No request.
pub fn explain(
    flow: &ResolvedFlow,
    sets: &[RuleSet],
    extra_deny: &HashMap<String, Vec<DenyRule>>,
    idx: &NameIndex,
) -> Explanation {
    let evaluated = flow.draft_policy_decision.is_some();
    let rules = resolve_rules(&flow.rules, sets, extra_deny, idx);
    let verdict = verdict(evaluated, &rules);
    Explanation {
        reported: flow.policy_decision,
        draft: flow.draft_policy_decision.clone(),
        evaluated,
        rules,
        verdict,
    }
}

fn plural(n: usize, one: &str) -> String {
    if n == 1 {
        format!("1 {one}")
    } else {
        format!("{n} {one}s")
    }
}

/// The verdict line: an override deny beats an allow, which beats a deny.
pub fn verdict(evaluated: bool, rules: &[MatchedRule]) -> String {
    if !evaluated && rules.is_empty() {
        return "not evaluated against the draft yet: the PCE names the rules only after it"
            .to_string();
    }
    let count = |k: MatchKind| rules.iter().filter(|r| r.kind == k).count();
    let (allow, deny, over, essential) = (
        count(MatchKind::Allow),
        count(MatchKind::Deny),
        count(MatchKind::OverrideDeny),
        count(MatchKind::Essential),
    );
    if over > 0 {
        format!("blocked by {} (draft)", plural(over, "override deny rule"))
    } else if allow > 0 && deny > 0 {
        format!(
            "allowed by {} (draft); an allow rule beats the deny rule",
            plural(allow, "rule")
        )
    } else if allow > 0 {
        format!("allowed by {} (draft)", plural(allow, "rule"))
    } else if deny > 0 {
        format!("blocked by {} (draft)", plural(deny, "deny rule"))
    } else if essential > 0 {
        "allowed by an essential service rule (built into the PCE)".to_string()
    } else if rules.is_empty() {
        "no rule allows this flow".to_string()
    } else {
        format!(
            "{} the PCE names, none of a kind this knows",
            plural(rules.len(), "rule")
        )
    }
}

/// What the destination's enforcement means for a flow no rule allows:
/// `full` blocks it; the others only report it (potentially blocked).
pub fn enforcement_note(mode: &str) -> &'static str {
    match mode {
        "full" => "enforced: without an allow rule the flow is blocked",
        "selective" => "selective: blocked only where a deny rule applies",
        "visibility_only" | "idle" => "not enforced: reported as potentially blocked, still passes",
        "unmanaged" => "unmanaged: no VEN enforces anything there",
        _ => "enforcement unknown to this tool",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::traffic::resolve_flows;
    use crate::traffic::resolved::tests::index;

    fn sets() -> Vec<RuleSet> {
        serde_json::from_str(include_str!("../../../fixtures/rule_sets.json")).unwrap()
    }

    fn flow(rules: &[&str], draft: Option<DraftDecision>) -> ResolvedFlow {
        let raw: Vec<crate::models::Flow> =
            serde_json::from_str(include_str!("../../../fixtures/traffic_flows.json")).unwrap();
        let mut f = resolve_flows(&raw, &index()).remove(0);
        f.rules = rules.iter().map(|s| s.to_string()).collect();
        f.draft_policy_decision = draft;
        f
    }

    const ALLOW: &str = "/orgs/5/sec_policy/draft/rule_sets/100/sec_rules/1";
    const DENY: &str = "/orgs/5/sec_policy/draft/rule_sets/100/deny_rules/7";
    const OVERRIDE: &str = "/orgs/5/sec_policy/draft/rule_sets/100/deny_rules/8";

    #[test]
    fn kinds_by_href() {
        assert_eq!(MatchKind::of_href(ALLOW), MatchKind::Allow);
        assert_eq!(MatchKind::of_href(DENY), MatchKind::Deny);
        assert_eq!(
            MatchKind::of_href("/orgs/5/sec_policy/draft/deny_rules/3"),
            MatchKind::Deny,
            "an org-level deny rule"
        );
        assert_eq!(
            MatchKind::of_href("/orgs/5/sec_policy/draft/enforcement_boundaries/4"),
            MatchKind::Deny
        );
        assert_eq!(MatchKind::of_href("essential:dhcp"), MatchKind::Essential);
        assert_eq!(MatchKind::of_href("/orgs/5/foo/1"), MatchKind::Unknown);
    }

    #[test]
    fn an_allow_rule_resolves_with_its_ruleset() {
        let r = resolve_rules(&[ALLOW.into()], &sets(), &HashMap::new(), &index());
        let r = &r[0];
        assert!(r.resolved);
        assert_eq!(r.kind, MatchKind::Allow);
        assert_eq!(r.ruleset.as_deref(), Some("Shop-Prod"));
        assert_eq!(
            r.ruleset_href.as_deref(),
            Some("/orgs/5/sec_policy/draft/rule_sets/100")
        );
        assert_eq!(r.scopes.len(), 1);
        assert_eq!(r.consumers, vec!["role=web"]);
        assert_eq!(r.providers, vec!["role=db"]);
        assert_eq!(r.services, vec!["PostgreSQL"]);
        assert_eq!(r.enabled, Some(true));
        assert_eq!(r.summary(), "role=web → role=db · PostgreSQL");
    }

    #[test]
    fn deny_and_override_deny_resolve_and_active_hrefs_match_the_draft() {
        let active = DENY.replace("/draft/", "/active/");
        let r = resolve_rules(
            &[active.clone(), OVERRIDE.into()],
            &sets(),
            &HashMap::new(),
            &index(),
        );
        assert_eq!(r[0].kind, MatchKind::Deny);
        assert!(r[0].resolved);
        assert_eq!(r[0].href, active, "the flow's href is kept");
        assert_eq!(r[1].kind, MatchKind::OverrideDeny);
    }

    #[test]
    fn deny_rules_fetched_per_ruleset_resolve_too() {
        let mut s = sets();
        let fetched = std::mem::take(&mut s[0].deny_rules);
        let r = resolve_rules(&[DENY.into()], &s, &HashMap::new(), &index());
        assert!(!r[0].resolved, "not inline, not fetched");
        assert_eq!(
            r[0].ruleset.as_deref(),
            Some("Shop-Prod"),
            "its ruleset is known"
        );
        let extra = HashMap::from([(s[0].href.clone(), fetched)]);
        let r = resolve_rules(&[DENY.into()], &s, &extra, &index());
        assert!(r[0].resolved);
    }

    #[test]
    fn unknown_hrefs_stay_hrefs() {
        let gone = "/orgs/5/sec_policy/draft/rule_sets/999/sec_rules/9";
        let r = resolve_rules(&[gone.into()], &sets(), &HashMap::new(), &index());
        assert!(!r[0].resolved);
        assert_eq!(r[0].kind, MatchKind::Allow);
        assert_eq!(r[0].ruleset, None);
        assert_eq!(
            r[0].ruleset_href.as_deref(),
            Some("/orgs/5/sec_policy/draft/rule_sets/999")
        );
        let e = resolve_rules(
            &["essential:dhcp".into()],
            &sets(),
            &HashMap::new(),
            &index(),
        );
        assert_eq!(e[0].kind, MatchKind::Essential);
        assert_eq!(e[0].ruleset_href, None);
    }

    #[test]
    fn a_disabled_ruleset_disables_its_rules() {
        let r = resolve_rules(
            &["/orgs/5/sec_policy/draft/rule_sets/101/sec_rules/3".into()],
            &sets(),
            &HashMap::new(),
            &index(),
        );
        assert_eq!(r[0].enabled, Some(false));
    }

    #[test]
    fn verdicts() {
        let e = |rules: &[&str], d| explain(&flow(rules, d), &sets(), &HashMap::new(), &index());
        assert!(e(&[], None).verdict.starts_with("not evaluated"));
        assert!(!e(&[], None).evaluated);
        let blocked = Some(DraftDecision::Blocked);
        assert_eq!(e(&[], blocked.clone()).verdict, "no rule allows this flow");
        assert_eq!(
            e(&[ALLOW], Some(DraftDecision::Allowed)).verdict,
            "allowed by 1 rule (draft)"
        );
        assert_eq!(
            e(&[DENY], blocked.clone()).verdict,
            "blocked by 1 deny rule (draft)"
        );
        assert_eq!(
            e(&[ALLOW, OVERRIDE], blocked.clone()).verdict,
            "blocked by 1 override deny rule (draft)"
        );
        assert!(
            e(&[ALLOW, DENY], Some(DraftDecision::Allowed))
                .verdict
                .contains("allow rule beats")
        );
        assert!(
            e(&["essential:dhcp"], Some(DraftDecision::Allowed))
                .verdict
                .contains("essential")
        );
    }
}
