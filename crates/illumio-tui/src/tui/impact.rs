//! Rule impact as the TUI shows it: the rules pane's `P/C` cells and the
//! rule detail's impact rows. Computed from the loaded rulesets, the name
//! index and the cached workloads when the rules pane is built (once per
//! row, never per draw); never a request (`i` asks for the workloads, in
//! `update.rs`).
use crate::tui::model::{Model, PolicyState, WorkloadsState};
use pce::NameIndex;
use pce::impact::{Impact, ImpactInputs, RuleActors, rule_impact_with};
use pce::models::{Actor, Workload};
use std::collections::HashSet;

/// What the impact says while the workloads are not cached.
pub const PRESS_I: &str = "press i to load workloads (1 request)";
const LOADING: &str = "loading workloads…";
/// IP list rows shown before `+N more`.
const MAX_IP_LIST_ROWS: usize = 3;

/// Where the workload list behind the impact stands.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Source<'a> {
    Cold,
    Loading,
    Failed(&'a str),
    Ready {
        workloads: &'a [Workload],
        truncated: bool,
    },
}

pub fn source(w: &WorkloadsState) -> Source<'_> {
    if let Some((workloads, truncated)) = w.impact_source() {
        Source::Ready {
            workloads,
            truncated,
        }
    } else if w.loading {
        Source::Loading
    } else if let Some(e) = w.error.as_deref() {
        Source::Failed(e)
    } else {
        Source::Cold
    }
}

/// The impact of a rule with these scopes and actors over `workloads`.
/// `labels` is `pce::workload_labels(workloads)`, built once by a caller
/// that evaluates many rules over the same list.
pub fn compute_with(
    scopes: &[Vec<Actor>],
    actors: &RuleActors,
    idx: &NameIndex,
    workloads: &[Workload],
    truncated: bool,
    labels: &[HashSet<&str>],
) -> Impact {
    let inputs = ImpactInputs {
        workloads,
        index: idx,
        truncated,
    };
    rule_impact_with(
        scopes,
        &actors.providers,
        &actors.consumers,
        actors.unscoped_consumers,
        &inputs,
        labels,
    )
}

/// The same ruleset in either scope (an active href folds to draft).
fn same_policy_href(a: &str, b: &str) -> bool {
    let fold = |h: &str| h.replacen("/sec_policy/active/", "/sec_policy/draft/", 1);
    fold(a) == fold(b)
}

/// A loaded ruleset's scopes by href.
pub(crate) fn scopes_of<'a>(p: &'a PolicyState, rs_href: &str) -> Option<&'a [Vec<Actor>]> {
    p.raw
        .iter()
        .find(|rs| same_policy_href(&rs.href, rs_href))
        .map(|rs| rs.scopes.as_slice())
}

/// A loaded rule's ruleset scopes and actors (allow, deny or fetched deny).
pub fn loaded_rule<'a>(p: &'a PolicyState, href: &str) -> Option<(&'a [Vec<Actor>], RuleActors)> {
    let (actors, rs_href) = match p.find_rule(href) {
        Some((r, rs)) => (RuleActors::of_rule(&r), rs),
        None => {
            let (d, rs) = p.find_deny_rule(href)?;
            (RuleActors::of_deny(&d), rs)
        }
    };
    Some((scopes_of(p, &rs_href)?, actors))
}

/// One line per IP list actor (`Corp: 65.5K addresses`), at most three,
/// then `+N more`.
pub fn ip_list_lines(i: &Impact) -> Vec<String> {
    let mut out: Vec<String> = i
        .ip_lists
        .iter()
        .take(MAX_IP_LIST_ROWS)
        .map(|l| l.line())
        .collect();
    if i.ip_lists.len() > MAX_IP_LIST_ROWS {
        out.push(format!("+{} more", i.ip_lists.len() - MAX_IP_LIST_ROWS));
    }
    out
}

/// What the impact says without an answer yet: press `i`, loading, or the
/// failure. (`Ready` has an answer; callers match it first.)
pub(crate) fn waiting_text(s: Source<'_>) -> String {
    match s {
        Source::Cold | Source::Ready { .. } => PRESS_I.to_string(),
        Source::Loading => LOADING.to_string(),
        Source::Failed(e) => format!("unavailable ({e})"),
    }
}

/// The rule detail's impact rows, `(label, text)`: the counts and one row
/// per IP list, or what it takes to get them. Reads the impact stored on the
/// rules-pane row (rebuilt when the workloads or rulesets load): no
/// evaluation here, so drawing the overlay stays cheap.
pub fn rule_detail_rows(model: &Model, href: &str) -> Vec<(&'static str, String)> {
    if !matches!(source(&model.workloads), Source::Ready { .. }) {
        return vec![("impact", waiting_text(source(&model.workloads)))];
    }
    let Some(impact) = model
        .policy
        .rules
        .rows()
        .iter()
        .find(|r| r.view.href == href)
        .and_then(|r| r.impact.as_ref())
    else {
        return vec![("impact", "rule not loaded".to_string())];
    };
    let mut rows = vec![("impact", impact.summary())];
    rows.extend(
        ip_list_lines(impact)
            .into_iter()
            .enumerate()
            .map(|(i, l)| (if i == 0 { "ip lists" } else { "" }, l)),
    );
    rows
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::test_support::{model_with_policy, with_workloads};

    const RULE_2: &str = "/orgs/5/sec_policy/draft/rule_sets/100/sec_rules/2";
    const DENY_8: &str = "/orgs/5/sec_policy/draft/rule_sets/100/deny_rules/8";

    fn row(label: &'static str, text: &str) -> (&'static str, String) {
        (label, text.to_string())
    }

    #[test]
    fn the_rule_detail_rows_follow_the_workload_cache() {
        let mut m = model_with_policy();
        assert_eq!(rule_detail_rows(&m, RULE_2), vec![row("impact", PRESS_I)]);
        m.workloads.loading = true;
        assert_eq!(
            rule_detail_rows(&m, RULE_2),
            vec![row("impact", "loading workloads…")]
        );
        m.workloads.loading = false;
        m.workloads.error = Some("timeout".into());
        assert_eq!(
            rule_detail_rows(&m, RULE_2),
            vec![row("impact", "unavailable (timeout)")]
        );
        let m = with_workloads(model_with_policy());
        assert_eq!(
            rule_detail_rows(&m, RULE_2),
            vec![
                row("impact", "1 provider · 0 consumers"),
                row("ip lists", "Corp: 65.5K addresses"),
            ]
        );
        assert_eq!(
            rule_detail_rows(&m, DENY_8),
            vec![row("impact", "1 provider · 2 consumers")]
        );
        assert_eq!(
            rule_detail_rows(&m, "/orgs/5/sec_policy/draft/rule_sets/100/sec_rules/99"),
            vec![row("impact", "rule not loaded")]
        );
    }

    #[test]
    fn the_detail_reads_the_stored_impact_until_the_rules_are_rebuilt() {
        let mut m = with_workloads(model_with_policy());
        let warm = rule_detail_rows(&m, RULE_2);
        assert_eq!(warm[0].1, "1 provider · 0 consumers");
        m.workloads.raw.clear();
        assert_eq!(rule_detail_rows(&m, RULE_2), warm, "no evaluation on draw");
        m.rebuild_rules();
        assert_eq!(
            rule_detail_rows(&m, RULE_2),
            vec![
                row("impact", "0 providers · 0 consumers"),
                row("ip lists", "Corp: 65.5K addresses")
            ]
        );
    }

    #[test]
    fn the_p_c_column_sorts_numerically_with_dashes_last() {
        let m = with_workloads(model_with_policy());
        let rows = m.policy.rules.rows();
        let key = |i: usize| rows[i].pc_key();
        // 1/2, 1/1, 1/0, 2/0
        assert!(key(2) < key(1) && key(1) < key(0) && key(0) < key(3));
        let cold = model_with_policy();
        assert!(cold.policy.rules.rows()[0].pc_key() > key(3));
    }

    #[test]
    fn ip_list_rows_stop_at_three() {
        let one = pce::impact::IpListRef {
            name: "L".into(),
            href: "/orgs/5/sec_policy/draft/ip_lists/1".into(),
            size: None,
            side: pce::impact::Side::Consumer,
        };
        let impact = Impact {
            providers: 0,
            consumers: 0,
            per_scope: vec![],
            ip_lists: vec![one; 5],
            unsupported: vec![],
            truncated: false,
        };
        assert_eq!(
            ip_list_lines(&impact),
            vec![
                "L: ? addresses",
                "L: ? addresses",
                "L: ? addresses",
                "+2 more"
            ]
        );
    }

    #[test]
    fn the_rules_pane_shows_p_c_once_the_workloads_are_cached() {
        let m = model_with_policy();
        assert!(m.policy.rules.rows().iter().all(|r| r.pc == "-"));
        let m = with_workloads(model_with_policy());
        let pcs: Vec<&str> = m
            .policy
            .rules
            .rows()
            .iter()
            .map(|r| r.pc.as_str())
            .collect();
        assert_eq!(
            pcs,
            vec!["1/2", "1/1", "1/0", "2/0"],
            "override deny, allow, allow, deny"
        );
        let titles: Vec<&str> = m.policy.rules.columns.iter().map(|c| c.title).collect();
        assert_eq!(
            titles,
            vec![
                "kind",
                "on",
                "consumers",
                "providers",
                "services",
                "P/C",
                "pending"
            ]
        );
    }
}
