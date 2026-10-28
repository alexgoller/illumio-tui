//! What a draft re-evaluation moves (#86): the flow records per reported
//! decision and draft decision (`transitions`), and the group pairs whose
//! flows the draft would newly block or newly allow (`draft_pairs`). Pure
//! folds over flows that carry a draft decision; no request.
use super::aggregate::{group_key, keys_or_default};
use super::resolved::ResolvedFlow;
use crate::models::PolicyDecision;
use serde::Serialize;
use std::collections::BTreeMap;

/// The flow records that move from one reported decision to one draft
/// decision (by class: `allowed_across_boundary` is allowed).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TransitionCell {
    pub reported: PolicyDecision,
    pub draft: PolicyDecision,
    pub flows: u64,
    /// The records' lifetime connection counts, summed.
    pub connections: u64,
}

impl TransitionCell {
    /// `allowed → blocked`, the decisions as the Flows view shortens them.
    pub fn arrow(&self) -> String {
        format!(
            "{} → {}",
            super::short_decision(self.reported.as_str()),
            super::short_decision(self.draft.as_str())
        )
    }
}

/// Every evaluated flow record by reported → draft decision: each cell
/// where the draft changes the verdict (`ResolvedFlow::draft_change`), the
/// rest as one `unchanged` count.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Transitions {
    /// Flow records that carry a draft decision.
    pub evaluated: u64,
    /// Flow records without one (the draft re-evaluation did not reach them).
    pub not_evaluated: u64,
    /// Evaluated records whose verdict the draft keeps.
    pub unchanged: u64,
    /// The non-zero cells where it changes, in decision order (allowed,
    /// potentially blocked, blocked, unknown; reported first).
    pub changed: Vec<TransitionCell>,
    /// Allowed now, blocked or potentially blocked under the draft
    /// (`ResolvedFlow::blocked_by_draft`).
    pub newly_blocked: u64,
    /// Blocked or potentially blocked now, allowed under the draft
    /// (`ResolvedFlow::fixed_by_draft`).
    pub newly_allowed: u64,
}

/// The order cells are listed in.
fn rank(d: PolicyDecision) -> u8 {
    match d {
        PolicyDecision::Allowed => 0,
        PolicyDecision::PotentiallyBlocked => 1,
        PolicyDecision::Blocked => 2,
        PolicyDecision::Unknown => 3,
    }
}

/// Adds `f` to the cell of its decisions in `cells`.
fn add_cell(
    cells: &mut BTreeMap<(u8, u8), TransitionCell>,
    f: &ResolvedFlow,
    draft: PolicyDecision,
) {
    let c = cells
        .entry((rank(f.policy_decision), rank(draft)))
        .or_insert_with(|| TransitionCell {
            reported: f.policy_decision,
            draft,
            flows: 0,
            connections: 0,
        });
    c.flows += 1;
    c.connections += f.num_connections;
}

/// The reported → draft table of `flows`.
pub fn transitions<'a>(flows: impl IntoIterator<Item = &'a ResolvedFlow>) -> Transitions {
    let mut t = Transitions::default();
    let mut cells = BTreeMap::new();
    for f in flows {
        let Some(d) = &f.draft_policy_decision else {
            t.not_evaluated += 1;
            continue;
        };
        t.evaluated += 1;
        t.newly_blocked += u64::from(f.blocked_by_draft());
        t.newly_allowed += u64::from(f.fixed_by_draft());
        if f.draft_change().is_none() {
            t.unchanged += 1;
        } else {
            add_cell(&mut cells, f, d.class());
        }
    }
    t.changed = cells.into_values().collect();
    t
}

/// One source group → destination group whose flows the draft newly
/// blocks (or newly allows): the services of those flows and their cells.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DraftPair {
    /// One value per group key (`group_key`: an IP list or a host name in
    /// the first slot for an unlabelled endpoint).
    pub src: Vec<String>,
    pub dst: Vec<String>,
    /// `443/tcp`, `icmp`: the services of the pair's moved flows, the
    /// busiest first.
    pub services: Vec<String>,
    /// Moved flow records.
    pub flows: u64,
    pub connections: u64,
    /// The moved records by reported → draft decision.
    pub transitions: Vec<TransitionCell>,
}

/// The pairs the draft moves, each list busiest first (flow records, then
/// connections, then the groups).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct DraftPairs {
    /// Pairs with flows allowed now that the draft blocks or potentially
    /// blocks.
    pub newly_blocked: Vec<DraftPair>,
    /// Pairs with flows blocked or potentially blocked now that the draft
    /// allows.
    pub newly_allowed: Vec<DraftPair>,
}

#[derive(Default)]
struct PairAcc {
    services: BTreeMap<String, u64>,
    flows: u64,
    connections: u64,
    cells: BTreeMap<(u8, u8), TransitionCell>,
}

fn pairs_of<'a>(
    flows: impl IntoIterator<Item = &'a ResolvedFlow>,
    keys: &[String],
) -> Vec<DraftPair> {
    let mut map: BTreeMap<(Vec<String>, Vec<String>), PairAcc> = BTreeMap::new();
    for f in flows {
        let Some(d) = &f.draft_policy_decision else {
            continue;
        };
        let acc = map
            .entry((group_key(&f.src, keys), group_key(&f.dst, keys)))
            .or_default();
        *acc.services.entry(f.service_text()).or_default() += 1;
        acc.flows += 1;
        acc.connections += f.num_connections;
        add_cell(&mut acc.cells, f, d.class());
    }
    let mut out: Vec<DraftPair> = map
        .into_iter()
        .map(|((src, dst), acc)| {
            let mut services: Vec<(String, u64)> = acc.services.into_iter().collect();
            services.sort_by(|(a, x), (b, y)| y.cmp(x).then_with(|| a.cmp(b)));
            DraftPair {
                src,
                dst,
                services: services.into_iter().map(|(s, _)| s).collect(),
                flows: acc.flows,
                connections: acc.connections,
                transitions: acc.cells.into_values().collect(),
            }
        })
        .collect();
    out.sort_by(|a, b| {
        b.flows
            .cmp(&a.flows)
            .then_with(|| b.connections.cmp(&a.connections))
            .then_with(|| (&a.src, &a.dst).cmp(&(&b.src, &b.dst)))
    });
    out
}

/// The group pairs (by `keys`, `app,env` when empty) of the flows the
/// draft newly blocks and of those it newly allows.
pub fn draft_pairs(flows: &[ResolvedFlow], keys: &[String]) -> DraftPairs {
    let keys = keys_or_default(keys);
    DraftPairs {
        newly_blocked: pairs_of(flows.iter().filter(|f| f.blocked_by_draft()), &keys),
        newly_allowed: pairs_of(flows.iter().filter(|f| f.fixed_by_draft()), &keys),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::DraftDecision as D;
    use crate::traffic::resolved::tests::{flows, rich_flows_draft};

    fn keys() -> Vec<String> {
        vec!["app".into(), "env".into()]
    }

    #[test]
    fn transitions_count_every_changed_cell_and_the_rest_as_unchanged() {
        let f = rich_flows_draft();
        let t = transitions(&f);
        assert_eq!(t.evaluated, f.len() as u64);
        assert_eq!(t.not_evaluated, 0);
        let moved: u64 = t.changed.iter().map(|c| c.flows).sum();
        assert_eq!(t.unchanged + moved, t.evaluated);
        assert!(
            t.changed
                .iter()
                .all(|c| c.flows > 0 && c.reported != c.draft)
        );
        let s = crate::traffic::DraftSummary::of(&f);
        assert_eq!(
            (t.newly_blocked, t.newly_allowed),
            (s.newly_blocked, s.fixed)
        );
        // Reported first, in decision order.
        let order: Vec<(u8, u8)> = t
            .changed
            .iter()
            .map(|c| (rank(c.reported), rank(c.draft)))
            .collect();
        let mut sorted = order.clone();
        sorted.sort();
        assert_eq!(order, sorted);
        assert_eq!(t.changed[0].arrow(), "allowed → blocked");
    }

    #[test]
    fn a_flow_without_a_draft_decision_is_not_evaluated() {
        let mut f = flows();
        for x in &mut f {
            x.draft_policy_decision = None;
        }
        f[0].draft_policy_decision = Some(D::Blocked);
        f[0].policy_decision = PolicyDecision::Allowed;
        f[1].draft_policy_decision = Some(D::Other("allowed_across_boundary".into()));
        f[1].policy_decision = PolicyDecision::Allowed;
        let t = transitions(&f);
        assert_eq!(t.evaluated, 2);
        assert_eq!(t.not_evaluated, f.len() as u64 - 2);
        assert_eq!(t.unchanged, 1, "a boundary suffix keeps the class");
        assert_eq!(
            t.changed,
            vec![TransitionCell {
                reported: PolicyDecision::Allowed,
                draft: PolicyDecision::Blocked,
                flows: 1,
                connections: f[0].num_connections,
            }]
        );
        assert_eq!(t.newly_blocked, 1);
    }

    #[test]
    fn pairs_split_newly_blocked_from_newly_allowed_busiest_first() {
        let f = rich_flows_draft();
        let p = draft_pairs(&f, &keys());
        let blocked: u64 = p.newly_blocked.iter().map(|x| x.flows).sum();
        let allowed: u64 = p.newly_allowed.iter().map(|x| x.flows).sum();
        let t = transitions(&f);
        assert_eq!((blocked, allowed), (t.newly_blocked, t.newly_allowed));
        assert!(blocked > 0 && allowed > 0);
        for list in [&p.newly_blocked, &p.newly_allowed] {
            for w in list.windows(2) {
                assert!(
                    (w[0].flows, w[0].connections) >= (w[1].flows, w[1].connections),
                    "{w:?}"
                );
            }
            for x in list {
                assert_eq!(x.src.len(), 2);
                assert!(!x.services.is_empty());
                assert_eq!(x.transitions.iter().map(|c| c.flows).sum::<u64>(), x.flows);
            }
        }
        assert!(
            p.newly_blocked
                .iter()
                .flat_map(|x| &x.transitions)
                .all(|c| c.reported == PolicyDecision::Allowed && c.draft != c.reported)
        );
        assert!(
            p.newly_allowed
                .iter()
                .flat_map(|x| &x.transitions)
                .all(|c| c.draft == PolicyDecision::Allowed)
        );
    }

    #[test]
    fn a_pair_lists_its_services_busiest_first() {
        let mut f = flows();
        for (i, x) in f.iter_mut().enumerate() {
            x.policy_decision = PolicyDecision::Allowed;
            x.draft_policy_decision = Some(D::Blocked);
            x.src = f_src();
            x.dst = f_src();
            x.port = Some(if i == 0 { 22 } else { 443 });
            x.proto = "tcp".into();
        }
        let p = draft_pairs(&f, &keys());
        assert_eq!(p.newly_blocked.len(), 1);
        assert!(p.newly_allowed.is_empty());
        assert!(f.len() >= 3);
        assert_eq!(
            p.newly_blocked[0].services,
            ["443/tcp".to_string(), "22/tcp".to_string()]
        );
    }

    fn f_src() -> crate::traffic::ResolvedEndpoint {
        crate::traffic::ResolvedEndpoint {
            ip: Some("10.0.0.1".into()),
            ..Default::default()
        }
    }
}
