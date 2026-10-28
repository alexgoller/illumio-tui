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
    /// The draft decision is stricter than the reported one (allowed <
    /// potentially blocked < blocked): allowed → potentially blocked,
    /// allowed → blocked, potentially blocked → blocked ([`stricter`]).
    pub newly_blocked: u64,
    /// The draft decision is looser: blocked → potentially blocked,
    /// blocked → allowed, potentially blocked → allowed ([`looser`]).
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

/// How strict a decision is: allowed < potentially blocked < blocked;
/// `None` for unknown, which is neither stricter nor looser than any.
fn strictness(d: PolicyDecision) -> Option<u8> {
    match d {
        PolicyDecision::Allowed => Some(0),
        PolicyDecision::PotentiallyBlocked => Some(1),
        PolicyDecision::Blocked => Some(2),
        PolicyDecision::Unknown => None,
    }
}

/// `f`'s draft decision (by class) against its reported one: `Some(true)`
/// stricter, `Some(false)` looser, `None` the same, unknown or no draft.
fn direction(f: &ResolvedFlow) -> Option<bool> {
    let d = f.draft_policy_decision.as_ref()?;
    let (r, d) = (strictness(f.policy_decision)?, strictness(d.class())?);
    (r != d).then_some(d > r)
}

/// The draft would block `f` more than today: allowed → potentially
/// blocked or blocked, potentially blocked → blocked.
pub fn stricter(f: &ResolvedFlow) -> bool {
    direction(f) == Some(true)
}

/// The draft would block `f` less than today: blocked → potentially
/// blocked or allowed, potentially blocked → allowed.
pub fn looser(f: &ResolvedFlow) -> bool {
    direction(f) == Some(false)
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
        t.newly_blocked += u64::from(stricter(f));
        t.newly_allowed += u64::from(looser(f));
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
    /// `22/tcp`, `443/tcp`, `icmp`: the services of the pair's moved
    /// flows, by port then protocol (no port last).
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
    /// By (port, protocol): no port (icmp) after every port.
    services: BTreeMap<(u32, String), String>,
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
        acc.services
            .entry((f.port.map_or(u32::MAX, u32::from), f.proto.clone()))
            .or_insert_with(|| f.service_text());
        acc.flows += 1;
        acc.connections += f.num_connections;
        add_cell(&mut acc.cells, f, d.class());
    }
    let mut out: Vec<DraftPair> = map
        .into_iter()
        .map(|((src, dst), acc)| DraftPair {
            src,
            dst,
            services: acc.services.into_values().collect(),
            flows: acc.flows,
            connections: acc.connections,
            transitions: acc.cells.into_values().collect(),
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
        newly_blocked: pairs_of(flows.iter().filter(|f| stricter(f)), &keys),
        newly_allowed: pairs_of(flows.iter().filter(|f| looser(f)), &keys),
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
        let stricter_cells: u64 = t
            .changed
            .iter()
            .filter(|c| c.draft != PolicyDecision::Unknown && rank(c.draft) > rank(c.reported))
            .map(|c| c.flows)
            .sum();
        assert_eq!(t.newly_blocked, stricter_cells);
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
                .all(|c| rank(c.draft) > rank(c.reported))
        );
        assert!(
            p.newly_allowed
                .iter()
                .flat_map(|x| &x.transitions)
                .all(|c| rank(c.draft) < rank(c.reported))
        );
    }

    #[test]
    fn stricter_is_by_rank_so_potentially_blocked_to_blocked_counts() {
        use PolicyDecision as P;
        let base = flows().remove(0);
        let cases = [
            (P::Allowed, D::PotentiallyBlocked, Some(true)),
            (P::Allowed, D::Blocked, Some(true)),
            (P::PotentiallyBlocked, D::Blocked, Some(true)),
            (P::Blocked, D::PotentiallyBlocked, Some(false)),
            (P::Blocked, D::Allowed, Some(false)),
            (P::PotentiallyBlocked, D::Allowed, Some(false)),
            (P::Blocked, D::Blocked, None),
            (P::Unknown, D::Blocked, None),
            (
                P::Allowed,
                D::Other("blocked_by_boundary".into()),
                Some(true),
            ),
        ];
        for (r, d, want) in cases {
            let mut x = base.clone();
            x.policy_decision = r;
            x.draft_policy_decision = Some(d.clone());
            assert_eq!(
                (stricter(&x), looser(&x)),
                (want == Some(true), want == Some(false)),
                "{r:?} → {d:?}"
            );
        }
        let mut x = base;
        x.policy_decision = P::PotentiallyBlocked;
        x.draft_policy_decision = Some(D::Blocked);
        let t = transitions(std::slice::from_ref(&x));
        assert_eq!((t.newly_blocked, t.newly_allowed), (1, 0));
        let p = draft_pairs(std::slice::from_ref(&x), &keys());
        assert_eq!(p.newly_blocked.len(), 1, "pb → blocked is a blocked pair");
    }

    #[test]
    fn a_pair_lists_its_services_by_port_then_protocol() {
        let mut f = flows();
        for (i, x) in f.iter_mut().enumerate() {
            x.policy_decision = PolicyDecision::Allowed;
            x.draft_policy_decision = Some(D::Blocked);
            x.src = f_src();
            x.dst = f_src();
            (x.port, x.proto) = match i {
                0 => (Some(443), "tcp".into()),
                1 => (None, "icmp".into()),
                2 => (Some(25), "udp".into()),
                _ => (Some(25), "tcp".into()),
            };
        }
        let p = draft_pairs(&f, &keys());
        assert_eq!(p.newly_blocked.len(), 1);
        assert!(p.newly_allowed.is_empty());
        assert!(f.len() >= 4);
        assert_eq!(
            p.newly_blocked[0].services,
            ["25/tcp", "25/udp", "443/tcp", "icmp"]
        );
    }

    fn f_src() -> crate::traffic::ResolvedEndpoint {
        crate::traffic::ResolvedEndpoint {
            ip: Some("10.0.0.1".into()),
            ..Default::default()
        }
    }
}
