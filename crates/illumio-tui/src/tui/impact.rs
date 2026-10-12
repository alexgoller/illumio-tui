//! Rule impact as the TUI shows it: the rules pane's `P/C` cells, the
//! rule detail's impact rows and the confirm overlay's impact line. Computed from the loaded rulesets, the name
//! index and the cached workloads when the rules pane is built (once per
//! row, never per draw); never a request (`i` asks for the workloads, in
//! `update.rs`).
use crate::tui::model::{Model, PolicyState, WorkloadsState};
use pce::NameIndex;
use pce::WriteMethod;
use pce::impact::{
    Impact, ImpactInputs, RuleActors, is_rule_href, rule_impact_with, rule_set_href_of,
};
use pce::models::{Actor, Workload};
use pce::write::{Plan, WriteRequest};
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
    rule_impact_with(scopes, actors, &inputs, labels)
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
    rows.extend(note(impact).map(|n| ("note", n)));
    rows
}

/// The first `unsupported` note, with `(+N more)`; `None` without notes.
pub fn note(i: &Impact) -> Option<String> {
    let first = i.unsupported.first()?;
    Some(match i.unsupported.len() - 1 {
        0 => first.clone(),
        more => format!("{first} (+{more} more)"),
    })
}

/// A rule or deny-rule create or update: the request the confirm overlay
/// adds an impact line to.
fn rule_request(plan: &Plan) -> Option<&WriteRequest> {
    match plan {
        Plan::Send {
            request,
            kind: pce::write::ObjectKind::Rule | pce::write::ObjectKind::DenyRule,
        } if request.method != WriteMethod::Delete => Some(request),
        _ => None,
    }
}

/// Rows the confirm overlay adds for `plan`: one for a rule or deny-rule
/// create or update, none otherwise.
pub fn confirm_rows(plan: &Plan) -> u16 {
    u16::from(rule_request(plan).is_some())
}

/// The confirm overlay's impact line for a rule plan: the planned body's
/// providers and consumers (an update's untouched sides from the loaded
/// rule) in the target ruleset's scopes over the cached workloads; or
/// what it takes to get there. `None` for any other plan. An evaluation:
/// the caller stores it (`Model::refresh_confirm_impact`), the overlay
/// never calls this per draw.
pub fn confirm_line(model: &Model, plan: &Plan) -> Option<String> {
    let request = rule_request(plan)?;
    let (workloads, truncated) = match source(&model.workloads) {
        Source::Ready {
            workloads,
            truncated,
        } => (workloads, truncated),
        waiting => return Some(format!("impact: {}", waiting_text(waiting))),
    };
    let p = &model.policy;
    let scopes = rule_set_href_of(&request.path).and_then(|rs| scopes_of(p, rs));
    let (Some(idx), Some(scopes)) = (model.index(), scopes) else {
        return Some("impact: ruleset not loaded; open Policy (2) first".to_string());
    };
    let base = if is_rule_href(&request.path) {
        loaded_rule(p, &request.path).map(|(_, a)| a)
    } else {
        None
    };
    let actors = RuleActors::planned(request.body.as_ref(), base.as_ref());
    let labels = pce::workload_labels(workloads);
    let impact = compute_with(scopes, &actors, idx, workloads, truncated, &labels);
    Some(match note(&impact) {
        Some(n) => format!("impact: {} · note: {n}", impact.summary()),
        None => format!("impact: {}", impact.summary()),
    })
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
    fn the_p_c_cell_is_compact_and_its_widest_form_fits_the_column() {
        let mut m = with_workloads(model_with_policy());
        for r in m.policy.rules.rows() {
            assert_eq!(r.pc, r.impact.as_ref().unwrap().pc_compact());
        }
        let big = |providers, consumers, truncated| Impact {
            providers,
            consumers,
            per_scope: vec![],
            ip_lists: vec![],
            unsupported: vec![],
            truncated,
        };
        let width = crate::tui::model::rule_columns()
            .iter()
            .find(|c| c.title == "P/C")
            .unwrap()
            .width as usize;
        for i in [
            big(1234, 5678, false),
            big(12_345, 67_890, false),
            big(10_000, 10_000, true),
            big(123_456, 456_789, true),
        ] {
            let cell = i.pc_compact();
            assert!(cell.chars().count() <= width, "{cell} in {width}");
        }
        // Many workloads through the real path: 1234 copies of the fixture.
        let one = m.workloads.raw.clone();
        m.workloads.raw = (0..1234).flat_map(|_| one.clone()).collect();
        m.rebuild_rules();
        let pcs: Vec<&str> = m
            .policy
            .rules
            .rows()
            .iter()
            .map(|r| r.pc.as_str())
            .collect();
        assert_eq!(pcs, vec!["1.2K/2.5K", "1.2K/1.2K", "1.2K/0", "2.5K/0"]);
    }

    #[test]
    fn an_ip_list_reload_updates_the_sizes_in_the_rule_detail() {
        use crate::tui::msg::Msg;
        use crate::tui::update::update;
        let mut m = with_workloads(model_with_policy());
        assert_eq!(
            rule_detail_rows(&m, RULE_2)[1],
            row("ip lists", "Corp: 65.5K addresses")
        );
        // Corp edited on screen 6 down to a /24; the reload brings it back.
        let mut lists: Vec<pce::models::IpList> =
            serde_json::from_str(include_str!("../../../../fixtures/ip_lists.json")).unwrap();
        lists[1].ip_ranges =
            serde_json::from_value(serde_json::json!([{"from_ip": "10.1.0.0/24"}])).unwrap();
        update(&mut m, Msg::IpListsLoaded(Ok((lists, 0))));
        assert_eq!(
            rule_detail_rows(&m, RULE_2)[1],
            row("ip lists", "Corp: 256 addresses")
        );
    }

    #[test]
    fn a_labels_reload_rebuilds_the_rules_pane() {
        use crate::tui::model::ObjectKind;
        use crate::tui::msg::{Msg, ObjectsPayload};
        use crate::tui::update::update;
        let mut m = with_workloads(model_with_policy());
        let consumers = |m: &Model| m.policy.rules.rows()[1].consumers[0].name.clone();
        assert_eq!(consumers(&m), "role=web");
        let mut labels: Vec<pce::models::Label> =
            serde_json::from_str(include_str!("../../../../fixtures/labels.json")).unwrap();
        labels[0].value = "frontend".into();
        update(
            &mut m,
            Msg::ObjectsLoaded {
                kind: ObjectKind::Labels,
                result: Ok(ObjectsPayload::Labels(labels, 0)),
            },
        );
        assert_eq!(consumers(&m), "role=frontend");
    }

    #[test]
    fn unsupported_notes_show_in_the_rule_detail_and_the_confirm_line() {
        use pce::write::{Plan, WriteRequest};
        let mut m = with_workloads(model_with_policy());
        let rule = m.policy.raw[0]
            .rules
            .iter_mut()
            .find(|r| r.href == RULE_2)
            .unwrap();
        rule.resolve_labels_as = Some(pce::models::ResolveLabelsAs {
            providers: vec!["virtual_services".into()],
            consumers: vec!["workloads".into()],
        });
        m.rebuild_rules();
        assert_eq!(
            rule_detail_rows(&m, RULE_2),
            vec![
                row("impact", "0 providers · 0 consumers"),
                row("ip lists", "Corp: 65.5K addresses"),
                row("note", "providers resolve as virtual_services only"),
            ]
        );
        let rule = m.policy.raw[0]
            .rules
            .iter_mut()
            .find(|r| r.href == RULE_2)
            .unwrap();
        rule.consumers.push(
            serde_json::from_value(serde_json::json!(
                {"virtual_service": {"href": "/orgs/5/sec_policy/draft/virtual_services/v1"}}
            ))
            .unwrap(),
        );
        m.rebuild_rules();
        assert_eq!(
            rule_detail_rows(&m, RULE_2).last().unwrap(),
            &row(
                "note",
                "providers resolve as virtual_services only (+1 more)"
            )
        );
        let update = Plan::Send {
            request: WriteRequest {
                method: pce::WriteMethod::Put,
                path: RULE_2.into(),
                body: Some(serde_json::json!({"resolve_labels_as":
                    {"providers": ["workloads"], "consumers": ["virtual_services"]}})),
                summary: "update rule".into(),
                objects: Vec::new(),
            },
            kind: pce::write::ObjectKind::Rule,
        };
        assert_eq!(
            confirm_line(&m, &update).as_deref(),
            Some(
                "impact: 1 provider · 0 consumers · note: consumers resolve as \
                 virtual_services only (+1 more)"
            )
        );
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

    #[test]
    fn the_confirm_line_reads_the_planned_body() {
        use crate::tui::test_support::{label_post_plan, model_fixture, rule_post_plan};
        use pce::WriteMethod;
        use pce::write::{Plan, WriteRequest};
        let m = model_with_policy();
        assert_eq!(confirm_line(&m, &label_post_plan()), None);
        assert_eq!(confirm_rows(&label_post_plan()), 0);
        assert_eq!(confirm_rows(&rule_post_plan()), 1);
        assert_eq!(
            confirm_line(&m, &rule_post_plan()).as_deref(),
            Some("impact: press i to load workloads (1 request)")
        );
        let m = with_workloads(model_with_policy());
        assert_eq!(
            confirm_line(&m, &rule_post_plan()).as_deref(),
            Some("impact: 1 provider · 0 consumers")
        );
        // An update sends only what changes; the rest is the loaded rule's.
        let update = Plan::Send {
            request: WriteRequest {
                method: WriteMethod::Put,
                path: RULE_2.into(),
                body: Some(serde_json::json!({"consumers": [
                    {"ip_list": {"href": "/orgs/5/sec_policy/draft/ip_lists/21"}},
                    {"actors": "ams"}
                ]})),
                summary: "update rule".into(),
                objects: Vec::new(),
            },
            kind: pce::write::ObjectKind::Rule,
        };
        assert_eq!(confirm_rows(&update), 1);
        assert_eq!(
            confirm_line(&m, &update).as_deref(),
            Some("impact: 1 provider · 4 consumers"),
            "unscoped, as loaded"
        );
        let delete = Plan::Send {
            request: WriteRequest {
                method: WriteMethod::Delete,
                path: RULE_2.into(),
                body: None,
                summary: "delete rule".into(),
                objects: Vec::new(),
            },
            kind: pce::write::ObjectKind::Rule,
        };
        assert_eq!(confirm_rows(&delete), 0);
        assert_eq!(confirm_line(&m, &delete), None);
        let lone = with_workloads(model_fixture());
        assert_eq!(
            confirm_line(&lone, &rule_post_plan()).as_deref(),
            Some("impact: ruleset not loaded; open Policy (2) first")
        );
    }

    #[test]
    fn the_stored_confirm_impact_follows_the_open_confirm() {
        use crate::tui::model::Overlay;
        use crate::tui::test_support::rule_post_plan;
        let mut m = model_with_policy();
        m.refresh_confirm_impact();
        assert_eq!(m.policy.confirm_impact, None, "no confirm open");
        m.overlay = Overlay::ConfirmWrite {
            plan: rule_post_plan(),
            kind: pce::write::ObjectKind::Rule,
            scroll: 0,
        };
        m.refresh_confirm_impact();
        assert_eq!(
            m.policy.confirm_impact.as_deref(),
            Some("impact: press i to load workloads (1 request)")
        );
        m.overlay = Overlay::None;
        m.refresh_confirm_impact();
        assert_eq!(m.policy.confirm_impact, None);
    }
}
