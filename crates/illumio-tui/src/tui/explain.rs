//! `E`: the Explain overlay's state and content. Which rule allows or
//! blocks a flow, from the loaded result, draft policy and workloads; no
//! request. Rendering is in `ui::widgets::explain`, keys in
//! `update::explain`.
use pce::explain::{Explanation, MatchKind, MatchedRule, enforcement_note};
use pce::models::PolicyDecision;
use pce::traffic::ResolvedFlow;

/// What the destination's enforcement is, as far as the loaded workloads know.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DstEnforcement {
    /// No workload: an IP or FQDN outside the PCE's workloads.
    NotWorkload,
    /// A workload the loaded list does not hold (or no list is loaded).
    Unknown,
    /// The effective enforcement (`full`, `visibility_only`, `unmanaged`, …).
    Mode(String),
}

/// The policy the rules were resolved through.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PolicySource {
    /// The draft rulesets: what the PCE evaluated.
    Draft,
    /// Only the active rulesets were loaded: names as provisioned.
    Active,
    /// No rulesets loaded: no rule resolves.
    NotLoaded,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ExplainView {
    pub flow: ResolvedFlow,
    pub explanation: Explanation,
    pub policy: PolicySource,
    pub dst: DstEnforcement,
    /// The selected rule (`j`/`k`), an index into `explanation.rules`.
    pub selected: usize,
    /// Opened from the app overlay: `Esc` goes back there.
    pub from_app: bool,
    /// The flow is in the result on screen, the one `D` evaluates.
    pub on_screen: bool,
}

/// One row of the overlay's content.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Row {
    /// `label  value`.
    Field(&'static str, String),
    /// A rule: its index, its head line and its detail line.
    Rule {
        index: usize,
        head: String,
        detail: String,
    },
    /// A hint: what a key does from here.
    Hint(String),
}

impl ExplainView {
    /// Whether some rule allows the flow (an allow or an essential rule).
    pub fn allowed_by_rule(&self) -> bool {
        self.explanation
            .rules
            .iter()
            .any(|r| matches!(r.kind, MatchKind::Allow | MatchKind::Essential))
    }

    pub fn selected_rule(&self) -> Option<&MatchedRule> {
        self.explanation.rules.get(self.selected)
    }

    pub fn move_cursor(&mut self, delta: isize) {
        let n = self.explanation.rules.len();
        if n == 0 {
            return;
        }
        self.selected = self.selected.saturating_add_signed(delta).min(n - 1);
    }

    /// Whether `w` (an allow rule from this flow) applies: the reported
    /// decision is blocked or potentially blocked.
    /// An override deny beats any allow rule, so none is offered there.
    pub fn offers_rule(&self) -> bool {
        matches!(
            self.flow.policy_decision,
            PolicyDecision::Blocked | PolicyDecision::PotentiallyBlocked
        ) && !self
            .explanation
            .rules
            .iter()
            .any(|r| r.kind == MatchKind::OverrideDeny)
    }

    /// The overlay's content, top to bottom.
    pub fn rows(&self) -> Vec<Row> {
        let f = &self.flow;
        let e = &self.explanation;
        let mut rows = vec![Row::Field(
            "flow",
            format!(
                "{} → {}  {}{}",
                f.src.display_name(),
                f.dst.display_name(),
                f.service_text(),
                f.process_name
                    .as_deref()
                    .map(|p| format!(" ({p})"))
                    .unwrap_or_default()
            ),
        )];
        let draft = match &e.draft {
            Some(d) => format!("draft {}", d.as_str()),
            None => "draft not evaluated".to_string(),
        };
        rows.push(Row::Field(
            "decision",
            format!("reported {} · {draft}", e.reported.as_str()),
        ));
        rows.push(Row::Field("verdict", e.verdict.clone()));
        for (index, r) in e.rules.iter().enumerate() {
            rows.push(self.rule_row(index, r));
        }
        if !self.allowed_by_rule() {
            rows.push(Row::Field("dst", self.dst_text()));
        }
        if !e.evaluated {
            rows.push(Row::Hint(if self.on_screen {
                "D: evaluate the result against the draft (asks first)".into()
            } else {
                "not in the result on screen: D on Traffic evaluates that one".into()
            }));
        }
        if e.evaluated && !self.allowed_by_rule() && self.offers_rule() {
            rows.push(Row::Hint(
                "w: write an allow rule from this flow (writes)".into(),
            ));
        }
        rows
    }

    fn rule_row(&self, index: usize, r: &MatchedRule) -> Row {
        let kind = r.kind.as_str();
        if r.kind == MatchKind::Essential {
            return Row::Rule {
                index,
                head: format!("{kind} {}", r.href.trim_start_matches("essential:")),
                detail: "built into the PCE: no ruleset".into(),
            };
        }
        if !r.resolved {
            let why = match self.policy {
                PolicySource::NotLoaded => {
                    "the draft policy is not loaded: Enter opens Policy (draft)"
                }
                PolicySource::Active => "not in the loaded active policy: Enter opens the draft",
                PolicySource::Draft => "not in the loaded draft policy: Enter, then r reloads",
            };
            return Row::Rule {
                index,
                head: format!("{kind} {}", r.href),
                detail: match &r.ruleset {
                    Some(rs) => format!("ruleset {rs} · {why}"),
                    None => why.to_string(),
                },
            };
        }
        let on = match r.enabled {
            Some(false) => "disabled",
            _ => "enabled",
        };
        Row::Rule {
            index,
            head: format!("{kind} {}", r.summary()),
            detail: format!(
                "ruleset {}{} · scope {} · {on}",
                r.ruleset.as_deref().unwrap_or("?"),
                if self.policy == PolicySource::Active {
                    " (active)"
                } else {
                    ""
                },
                if r.scopes.is_empty() {
                    "All".to_string()
                } else {
                    r.scopes.join(" | ")
                }
            ),
        }
    }

    fn dst_text(&self) -> String {
        let name = self.flow.dst.display_name();
        match &self.dst {
            DstEnforcement::NotWorkload => format!("{name}: not a workload, no VEN enforces there"),
            DstEnforcement::Unknown => {
                format!("{name}: enforcement unknown (Workloads, 4, loads the list)")
            }
            DstEnforcement::Mode(m) => format!("{name}: {m}: {}", enforcement_note(m)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::test_support::{fixture_flows, fixture_index};
    use pce::models::{DraftDecision, RuleSet};
    use std::collections::HashMap;

    fn sets() -> Vec<RuleSet> {
        serde_json::from_str(include_str!("../../../../fixtures/rule_sets.json")).unwrap()
    }

    fn view(
        i: usize,
        rules: &[&str],
        draft: Option<DraftDecision>,
        dst: DstEnforcement,
    ) -> ExplainView {
        let mut flow = fixture_flows().remove(i);
        flow.rules = rules.iter().map(|s| s.to_string()).collect();
        flow.draft_policy_decision = draft;
        let explanation =
            pce::explain::explain(&flow, &sets(), &HashMap::new(), &fixture_index(), None);
        ExplainView {
            flow,
            explanation,
            policy: PolicySource::Draft,
            dst,
            selected: 0,
            from_app: false,
            on_screen: true,
        }
    }

    fn texts(v: &ExplainView) -> Vec<String> {
        v.rows()
            .into_iter()
            .map(|r| match r {
                Row::Field(l, t) => format!("{l}: {t}"),
                Row::Rule { head, detail, .. } => format!("{head} | {detail}"),
                Row::Hint(h) => format!("hint: {h}"),
            })
            .collect()
    }

    #[test]
    fn an_allowed_flow_names_its_rule_and_ruleset() {
        let v = view(
            3,
            &["/orgs/5/sec_policy/draft/rule_sets/100/sec_rules/2"],
            Some(DraftDecision::Allowed),
            DstEnforcement::NotWorkload,
        );
        assert_eq!(
            texts(&v),
            vec![
                "flow: host-1.example → fqdn-1.example  443/tcp (curl)",
                "decision: reported allowed · draft allowed",
                "verdict: allowed by 1 rule (draft)",
                "allow ipl:Corp → role=web · 443/tcp, 8000-8099/tcp | ruleset Shop-Prod · scope app=shop & env=prod · enabled",
            ]
        );
    }

    #[test]
    fn not_evaluated_offers_the_draft_evaluation() {
        let v = view(1, &[], None, DstEnforcement::Mode("full".into()));
        let t = texts(&v);
        assert!(t[2].starts_with("verdict: not evaluated"), "{t:?}");
        assert!(
            t.iter()
                .any(|l| l.starts_with("dst: host-1.example: full: enforced"))
        );
        assert!(t.iter().any(|l| l.starts_with("hint: D: evaluate")));
        assert!(
            !t.iter().any(|l| l.starts_with("hint: w:")),
            "w after the evaluation"
        );
    }

    #[test]
    fn no_rule_says_so_with_the_enforcement_and_offers_w() {
        let v = view(
            1,
            &[],
            Some(DraftDecision::PotentiallyBlocked),
            DstEnforcement::Mode("visibility_only".into()),
        );
        let t = texts(&v);
        assert_eq!(t[2], "verdict: no rule allows this flow");
        assert_eq!(
            t[3],
            "dst: host-1.example: visibility_only: not enforced: reported as potentially blocked, still passes"
        );
        assert_eq!(t[4], "hint: w: write an allow rule from this flow (writes)");
        assert_eq!(t.len(), 5);
    }

    #[test]
    fn deny_and_unknown_rules_show_their_hrefs_and_how_to_reload() {
        let mut v = view(
            2,
            &[
                "/orgs/5/sec_policy/draft/rule_sets/100/deny_rules/8",
                "/orgs/5/sec_policy/draft/rule_sets/999/deny_rules/1",
            ],
            Some(DraftDecision::Blocked),
            DstEnforcement::Unknown,
        );
        let t = texts(&v);
        assert_eq!(t[2], "verdict: blocked by 1 override deny rule (draft)");
        assert!(
            t[3].starts_with("override_deny All workloads → role=db · 22/tcp | ruleset Shop-Prod"),
            "{t:?}"
        );
        assert_eq!(
            t[4],
            "deny /orgs/5/sec_policy/draft/rule_sets/999/deny_rules/1 | not in the loaded draft policy: Enter, then r reloads"
        );
        assert!(t[5].contains("enforcement unknown"));
        assert_eq!(t.len(), 6, "no w: an override deny beats an allow rule");
        v.policy = PolicySource::NotLoaded;
        assert!(
            texts(&v)[4].ends_with("the draft policy is not loaded: Enter opens Policy (draft)")
        );
        v.move_cursor(5);
        assert_eq!(v.selected, 1);
        v.move_cursor(-3);
        assert_eq!(v.selected, 0);
    }
}
