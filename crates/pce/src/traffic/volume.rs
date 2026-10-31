//! The Volume view's tree (#89): one node per source group with its
//! destination pairs under it, each split by decision in connections and
//! in bytes, and the bar a node draws. Built from `group_by_labels`'
//! pairs; the bytes per decision (which `FlowGroup` does not carry) from
//! the same flows. Pure; no requests.
use super::aggregate::{DraftCounts, FlowGroup, group_key, keys_or_default};
use super::compare::{ChangeSet, RowChange, join_changes};
use super::resolved::ResolvedFlow;
use crate::models::PolicyDecision;
use std::collections::BTreeMap;

/// What a row and its bar weigh: connections or bytes (`b`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Weight {
    #[default]
    Connections,
    Bytes,
}

/// A weight split by decision.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Split {
    pub allowed: u64,
    pub potentially_blocked: u64,
    pub blocked: u64,
    /// Flows the PCE reported without a decision.
    pub unknown: u64,
}

impl Split {
    pub fn add(&mut self, decision: PolicyDecision, n: u64) {
        match decision {
            PolicyDecision::Allowed => self.allowed += n,
            PolicyDecision::PotentiallyBlocked => self.potentially_blocked += n,
            PolicyDecision::Blocked => self.blocked += n,
            PolicyDecision::Unknown => self.unknown += n,
        }
    }

    fn plus(self, o: Split) -> Split {
        Split {
            allowed: self.allowed + o.allowed,
            potentially_blocked: self.potentially_blocked + o.potentially_blocked,
            blocked: self.blocked + o.blocked,
            unknown: self.unknown + o.unknown,
        }
    }

    pub fn total(&self) -> u64 {
        self.allowed + self.potentially_blocked + self.blocked + self.unknown
    }

    /// The parts in the bar's order: allowed, potentially blocked,
    /// blocked, unknown.
    pub fn parts(&self) -> [u64; 4] {
        [
            self.allowed,
            self.potentially_blocked,
            self.blocked,
            self.unknown,
        ]
    }

    /// `61% blocked`, with `· 12% pb` when some is potentially blocked.
    /// A share that is neither none nor all never reads `0%` or `100%`.
    pub fn blocked_text(&self) -> String {
        let total = self.total();
        let mut s = format!("{}% blocked", percent(self.blocked, total));
        if self.potentially_blocked > 0 {
            s.push_str(&format!(
                " · {}% pb",
                percent(self.potentially_blocked, total)
            ));
        }
        s
    }
}

/// `part` of `total` in whole percent, rounded, kept within 1..=99 unless
/// it is none or all of it.
fn percent(part: u64, total: u64) -> u64 {
    if total == 0 || part == 0 {
        return 0;
    }
    if part >= total {
        return 100;
    }
    let p = (u128::from(part) * 100 + u128::from(total) / 2) / u128::from(total);
    (p as u64).clamp(1, 99)
}

/// A row of the tree: a destination pair (`group` as `group_by_labels`
/// built it, with its marks, compare change and draft counts), or a source
/// (`group.dst` empty: its pairs summed, its change joined from theirs),
/// with its bytes per decision.
#[derive(Debug, Clone, PartialEq)]
pub struct VolumeNode {
    pub group: FlowGroup,
    pub bytes: Split,
}

impl VolumeNode {
    /// The node split by decision in `w`.
    pub fn split(&self, w: Weight) -> Split {
        match w {
            Weight::Connections => {
                let c = &self.group.conns;
                Split {
                    allowed: c.allowed_conns,
                    potentially_blocked: c.pot_blocked_conns,
                    blocked: c.blocked_conns,
                    unknown: c.unknown_conns,
                }
            }
            Weight::Bytes => self.bytes,
        }
    }

    /// Connections or bytes.
    pub fn weight(&self, w: Weight) -> u64 {
        match w {
            Weight::Connections => self.group.connections,
            Weight::Bytes => self.group.bytes,
        }
    }
}

/// A source group and its destinations.
#[derive(Debug, Clone, PartialEq)]
pub struct VolumeSource {
    pub node: VolumeNode,
    pub dsts: Vec<VolumeNode>,
}

/// Each pair's bytes per decision under `keys`, by (source, destination)
/// group key: what `FlowGroup` sums without a split.
pub type ByteSplits = BTreeMap<(Vec<String>, Vec<String>), Split>;

/// The bytes per decision of every pair of `flows` under `keys`.
pub fn byte_splits(flows: &[ResolvedFlow], keys: &[String]) -> ByteSplits {
    let keys = keys_or_default(keys);
    let mut out = ByteSplits::new();
    for f in flows {
        out.entry((group_key(&f.src, &keys), group_key(&f.dst, &keys)))
            .or_default()
            .add(f.policy_decision, f.bytes());
    }
    out
}

/// The tree of `pairs` (`group_by_labels`' rows; compare mode's gone pairs
/// too), sources in order of first appearance: `sort` orders it. A pair's
/// bytes come from `bytes`, else `fallback` (a gone pair's, from the
/// baseline). A source sums its pairs (marks are left to the caller) and
/// joins their changes when they carry one.
pub fn tree(pairs: &[FlowGroup], bytes: &ByteSplits, fallback: &ByteSplits) -> Vec<VolumeSource> {
    let mut out: Vec<VolumeSource> = Vec::new();
    for g in pairs {
        let key = (g.src.clone(), g.dst.clone());
        let node = VolumeNode {
            group: g.clone(),
            bytes: bytes
                .get(&key)
                .or_else(|| fallback.get(&key))
                .copied()
                .unwrap_or_default(),
        };
        match out.iter_mut().find(|s| s.node.group.src == g.src) {
            Some(s) => s.dsts.push(node),
            // The source's own node is summed once its pairs are in.
            None => out.push(VolumeSource {
                node: VolumeNode {
                    group: g.clone(),
                    bytes: Split::default(),
                },
                dsts: vec![node],
            }),
        }
    }
    for s in &mut out {
        s.node = source_node(&s.node.group.src, &s.dsts);
    }
    out
}

/// A source's own row: its pairs summed.
fn source_node(src: &[String], dsts: &[VolumeNode]) -> VolumeNode {
    let mut g = FlowGroup {
        src: src.to_vec(),
        dst: Vec::new(),
        service: None,
        flows: 0,
        connections: 0,
        bytes: 0,
        allowed: 0,
        potentially_blocked: 0,
        blocked: 0,
        conns: Default::default(),
        draft: None,
        marks: None,
        change: None,
    };
    let mut bytes = Split::default();
    for d in dsts {
        let p = &d.group;
        g.flows += p.flows;
        g.connections += p.connections;
        g.bytes += p.bytes;
        g.allowed += p.allowed;
        g.potentially_blocked += p.potentially_blocked;
        g.blocked += p.blocked;
        g.conns.allowed_conns += p.conns.allowed_conns;
        g.conns.pot_blocked_conns += p.conns.pot_blocked_conns;
        g.conns.blocked_conns += p.conns.blocked_conns;
        g.conns.unknown_conns += p.conns.unknown_conns;
        if let Some(c) = p.draft {
            let s = g.draft.get_or_insert_with(DraftCounts::default);
            s.draft_allowed += c.draft_allowed;
            s.draft_blocked += c.draft_blocked;
        }
        bytes = bytes.plus(d.bytes);
    }
    let changes: Vec<RowChange> = dsts.iter().filter_map(|d| d.group.change).collect();
    if !changes.is_empty() {
        // The pairs' delta reads in services; a source's is its pairs.
        g.change = Some(RowChange {
            pair: None,
            ..join_changes(&changes)
        });
    }
    VolumeNode { group: g, bytes }
}

/// Sources by `w`, heaviest first, and each source's destinations the same
/// way; ties by name.
pub fn sort(sources: &mut [VolumeSource], w: Weight) {
    let by = |a: &VolumeNode, b: &VolumeNode| {
        b.weight(w)
            .cmp(&a.weight(w))
            .then_with(|| a.group.src.cmp(&b.group.src))
            .then_with(|| a.group.dst.cmp(&b.group.dst))
    };
    for s in sources.iter_mut() {
        s.dsts.sort_by(by);
    }
    sources.sort_by(|a, b| by(&a.node, &b.node));
}

/// Compare mode's `n`: the destinations whose change `show` keeps, and
/// the sources whose own change it keeps or that keep a destination. A
/// source kept only for its destinations stays as their context, its own
/// change cleared, so every row with a change is one `show` keeps.
pub fn keep_changes(sources: &mut Vec<VolumeSource>, show: ChangeSet) {
    for s in sources.iter_mut() {
        s.dsts.retain(|d| show.keeps(d.group.change.as_ref()));
    }
    sources.retain(|s| show.keeps(s.node.group.change.as_ref()) || !s.dsts.is_empty());
    for s in sources.iter_mut() {
        if !show.keeps(s.node.group.change.as_ref()) {
            s.node.group.change = None;
        }
    }
}

/// The cells of a bar for `split` on a scale where `scale` fills `width`:
/// its length proportional to the split's total (at least one cell for any
/// weight), shared among allowed, potentially blocked, blocked and unknown
/// by largest remainder, every part present getting a cell while the
/// length has room for it.
pub fn bar_cells(split: &Split, scale: u64, width: usize) -> [usize; 4] {
    let total = split.total();
    if total == 0 || scale == 0 || width == 0 {
        return [0; 4];
    }
    let len = ((u128::from(total) * width as u128 + u128::from(scale) / 2) / u128::from(scale))
        .clamp(1, width as u128) as usize;
    let parts = split.parts();
    let present = parts.iter().filter(|p| **p > 0).count();
    let mut cells = [0usize; 4];
    // A cell for every part present first, when the length has room.
    let floor = usize::from(len >= present);
    let rest = len - floor * present;
    let mut rems: Vec<(u128, usize)> = Vec::new();
    let mut used = 0;
    for (i, &p) in parts.iter().enumerate() {
        if p == 0 {
            continue;
        }
        let exact = u128::from(p) * rest as u128;
        let whole = (exact / u128::from(total)) as usize;
        cells[i] = floor + whole;
        used += floor + whole;
        rems.push((exact % u128::from(total), i));
    }
    rems.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    for (_, i) in rems.into_iter().take(len - used) {
        cells[i] += 1;
    }
    cells
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::traffic::aggregate::group_by_labels;
    use crate::traffic::compare::{Change, DecisionSet};
    use crate::traffic::resolved::tests::rich_flows;

    fn keys() -> Vec<String> {
        vec!["app".into(), "env".into()]
    }

    fn rich() -> Vec<VolumeSource> {
        let f = rich_flows();
        let mut t = tree(
            &group_by_labels(&f, &keys()),
            &byte_splits(&f, &keys()),
            &ByteSplits::new(),
        );
        sort(&mut t, Weight::Connections);
        t
    }

    fn text(g: &FlowGroup) -> String {
        if g.dst.is_empty() {
            g.src.join("/")
        } else {
            format!("{} → {}", g.src.join("/"), g.dst.join("/"))
        }
    }

    #[test]
    fn sources_sum_their_pairs_and_sort_heaviest_first() {
        let t = rich();
        assert_eq!(text(&t[0].node.group), "ipl:Corp/-");
        assert_eq!(t[0].node.weight(Weight::Connections), 1_301);
        let total: u64 = t.iter().map(|s| s.node.group.connections).sum();
        assert_eq!(total, 4_109, "every connection, nothing folded");
        for s in &t {
            assert!(s.node.group.dst.is_empty());
            let sum = |f: fn(&VolumeNode) -> u64| s.dsts.iter().map(f).sum::<u64>();
            assert_eq!(sum(|d| d.group.connections), s.node.group.connections);
            assert_eq!(sum(|d| d.group.bytes), s.node.group.bytes);
            assert_eq!(sum(|d| d.group.flows), s.node.group.flows);
            assert!(
                s.dsts
                    .windows(2)
                    .all(|p| p[0].group.connections >= p[1].group.connections)
            );
            for w in [Weight::Connections, Weight::Bytes] {
                assert_eq!(s.node.split(w).total(), s.node.weight(w), "{w:?}");
                for d in &s.dsts {
                    assert_eq!(d.split(w).total(), d.weight(w), "{w:?} {d:?}");
                }
            }
        }
        assert!(
            t.windows(2)
                .all(|p| p[0].node.group.connections >= p[1].node.group.connections)
        );
        let shop = t
            .iter()
            .find(|s| text(&s.node.group) == "shop/prod")
            .unwrap();
        let dsts: Vec<String> = shop.dsts.iter().map(|d| d.group.dst.join("/")).collect();
        assert_eq!(dsts, vec!["shop/prod", "crm/prod", "api.partner.example/-"]);
    }

    #[test]
    fn bytes_split_by_the_decision_of_each_flow() {
        let mut t = rich();
        sort(&mut t, Weight::Bytes);
        assert_eq!(t[0].node.weight(Weight::Bytes), 8_350_000);
        let f = rich_flows();
        let k = keys();
        for s in &t {
            for d in &s.dsts {
                let mut want = Split::default();
                for x in f.iter().filter(|x| d.group.contains(x, &k)) {
                    want.add(x.policy_decision, x.bytes());
                }
                assert_eq!(d.bytes, want, "{}", text(&d.group));
            }
        }
        // Bytes and connections can split differently: bytes follow bytes.
        assert!(t.iter().any(|s| {
            let (c, b) = (
                s.node.split(Weight::Connections),
                s.node.split(Weight::Bytes),
            );
            c.blocked * b.total() != b.blocked * c.total()
        }));
    }

    #[test]
    fn a_gone_pair_takes_its_bytes_from_the_fallback() {
        let f = rich_flows();
        let g = group_by_labels(&f, &keys());
        let t = tree(&g, &ByteSplits::new(), &byte_splits(&f, &keys()));
        assert!(
            t.iter()
                .all(|s| s.dsts.iter().all(|d| d.bytes.total() == d.group.bytes))
        );
    }

    #[test]
    fn the_blocked_text_never_rounds_a_part_to_none_or_all() {
        let s = |a, p, b| Split {
            allowed: a,
            potentially_blocked: p,
            blocked: b,
            unknown: 0,
        };
        assert_eq!(s(39, 0, 61).blocked_text(), "61% blocked");
        assert_eq!(s(10, 0, 0).blocked_text(), "0% blocked");
        assert_eq!(s(0, 0, 7).blocked_text(), "100% blocked");
        assert_eq!(s(1, 0, 999).blocked_text(), "99% blocked");
        assert_eq!(s(999, 0, 1).blocked_text(), "1% blocked");
        assert_eq!(s(50, 12, 38).blocked_text(), "38% blocked · 12% pb");
        assert_eq!(Split::default().blocked_text(), "0% blocked");
    }

    #[test]
    fn bars_scale_to_the_largest_and_split_by_decision() {
        let s = |a, p, b| Split {
            allowed: a,
            potentially_blocked: p,
            blocked: b,
            unknown: 0,
        };
        assert_eq!(bar_cells(&s(39, 0, 61), 100, 28), [11, 0, 17, 0]);
        assert_eq!(bar_cells(&s(50, 0, 0), 100, 20), [10, 0, 0, 0]);
        assert_eq!(
            bar_cells(&s(0, 0, 1), 1_000_000, 20),
            [0, 0, 1, 0],
            "never empty"
        );
        // A small blocked part still shows.
        assert_eq!(bar_cells(&s(990, 0, 10), 1000, 20), [19, 0, 1, 0]);
        assert_eq!(
            bar_cells(&s(98, 1, 1), 100, 2),
            [2, 0, 0, 0],
            "no room for all"
        );
        assert_eq!(bar_cells(&s(1, 1, 1), 3, 12), [4, 4, 4, 0]);
        assert_eq!(bar_cells(&Split::default(), 10, 10), [0; 4]);
        assert_eq!(bar_cells(&s(5, 0, 5), 10, 0), [0; 4]);
        for w in 1..40 {
            for (a, p, b) in [(3, 5, 7), (100, 1, 0), (1, 1, 98)] {
                let c = bar_cells(&s(a, p, b), 200, w);
                let len: usize = c.iter().sum();
                assert!(len <= w && len >= 1, "{w}: {c:?}");
            }
        }
    }

    fn change(c: Change) -> RowChange {
        RowChange {
            change: c,
            baseline_decision: (c != Change::New).then(|| DecisionSet::of(PolicyDecision::Allowed)),
            recent_decision: (c != Change::Gone).then(|| DecisionSet::of(PolicyDecision::Allowed)),
            pair: None,
        }
    }

    #[test]
    fn a_source_joins_its_pairs_changes_and_n_keeps_destinations() {
        let f = rich_flows();
        let mut g = group_by_labels(&f, &keys());
        let shop = |x: &FlowGroup| x.src == ["shop", "prod"];
        let mut first = true;
        for x in g.iter_mut() {
            x.change = Some(if shop(x) && first {
                first = false;
                change(Change::New)
            } else if shop(x) {
                change(Change::Same)
            } else {
                change(Change::Gone)
            });
        }
        let mut t = tree(&g, &ByteSplits::new(), &ByteSplits::new());
        let c = |t: &[VolumeSource], s: &str| {
            t.iter()
                .find(|x| text(&x.node.group) == s)
                .and_then(|x| x.node.group.change)
                .map(|c| c.change)
        };
        assert_eq!(c(&t, "shop/prod"), Some(Change::Changed));
        assert_eq!(c(&t, "ipl:Corp/-"), Some(Change::Gone));
        assert!(
            t.iter()
                .all(|s| s.node.group.change.unwrap().pair.is_none())
        );
        let all = t.clone();
        keep_changes(&mut t, ChangeSet::of(&[Change::New]));
        assert_eq!(t.len(), 1);
        assert_eq!(t[0].dsts.len(), 1);
        assert_eq!(
            c(&t, "shop/prod"),
            None,
            "kept as the context of its new pair"
        );
        let mut t = all.clone();
        keep_changes(&mut t, ChangeSet::of(&[Change::Changed]));
        assert_eq!(t.len(), 1, "the changed source, by its own change");
        assert!(t[0].dsts.is_empty());
        assert_eq!(c(&t, "shop/prod"), Some(Change::Changed));
        let mut t = all;
        keep_changes(&mut t, ChangeSet::of(&[Change::Gone]));
        assert!(!t.is_empty());
        assert!(
            t.iter()
                .all(|s| s.node.group.change.map(|c| c.change) == Some(Change::Gone))
        );
    }
}
