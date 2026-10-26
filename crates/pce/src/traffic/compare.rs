//! What changed between a baseline window and a recent one: the rows of a
//! view matched on the view's key, without the policy decision and without
//! `first_detected` (a long-lived flow comes back as a fresh record every
//! day, so `first_detected` never says "new"). Each recent row is `new`
//! (only recent), `changed` (in both, another set of decisions) or `same`;
//! a key only in the baseline is one `gone` row, with the baseline's
//! values. Pure; the two results are fetched by the caller.
use super::aggregate::FlowGroup;
use super::mesh::MeshRow;
use super::resolved::{ResolvedEndpoint, ResolvedFlow};
use super::sections::{AppSection, Direction};
use crate::models::PolicyDecision;
use serde::{Serialize, Serializer};
use std::collections::BTreeMap;

/// A row's status against the baseline.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Change {
    /// Only in the recent window.
    New,
    /// In both, with another set of decisions.
    Changed,
    /// Only in the baseline: shown with the baseline's values.
    Gone,
    /// In both, the same decisions.
    Same,
}

impl Change {
    pub const ALL: [Change; 4] = [Change::New, Change::Changed, Change::Gone, Change::Same];

    pub fn as_str(self) -> &'static str {
        match self {
            Change::New => "new",
            Change::Changed => "changed",
            Change::Gone => "gone",
            Change::Same => "same",
        }
    }

    /// The `Δ` cell: `NEW`, `CHG`, `GONE`, blank for the same.
    pub fn tag(self) -> &'static str {
        match self {
            Change::New => "NEW",
            Change::Changed => "CHG",
            Change::Gone => "GONE",
            Change::Same => "",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Change::ALL
            .into_iter()
            .find(|c| c.as_str() == s.trim().to_ascii_lowercase())
    }
}

/// The decisions a row's flows were seen with, in a fixed order.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct DecisionSet(u8);

/// The order a set is written in.
const ORDER: [PolicyDecision; 4] = [
    PolicyDecision::Allowed,
    PolicyDecision::PotentiallyBlocked,
    PolicyDecision::Blocked,
    PolicyDecision::Unknown,
];

fn bit(d: PolicyDecision) -> u8 {
    1 << ORDER.iter().position(|o| *o == d).unwrap_or(3)
}

impl DecisionSet {
    pub fn of(d: PolicyDecision) -> Self {
        Self(bit(d))
    }

    /// The decisions with a non-zero count; `unknown` when some flow
    /// records carried none of the three (`flows` above their sum).
    pub fn from_counts(allowed: u64, potentially_blocked: u64, blocked: u64, flows: u64) -> Self {
        let mut s = Self::default();
        for (n, d) in [
            (allowed, PolicyDecision::Allowed),
            (potentially_blocked, PolicyDecision::PotentiallyBlocked),
            (blocked, PolicyDecision::Blocked),
        ] {
            if n > 0 {
                s.insert(d);
            }
        }
        if flows > allowed + potentially_blocked + blocked {
            s.insert(PolicyDecision::Unknown);
        }
        s
    }

    pub fn insert(&mut self, d: PolicyDecision) {
        self.0 |= bit(d);
    }

    pub fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    pub fn is_empty(self) -> bool {
        self.0 == 0
    }

    pub fn decisions(self) -> impl Iterator<Item = PolicyDecision> {
        ORDER.into_iter().filter(move |d| self.0 & bit(*d) != 0)
    }

    /// `allowed`, `allowed+blocked`.
    pub fn text(self) -> String {
        self.decisions()
            .map(PolicyDecision::as_str)
            .collect::<Vec<_>>()
            .join("+")
    }
}

impl Serialize for DecisionSet {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.text())
    }
}

/// A row's change, carried by the row while compare mode is on. In JSON
/// (flattened into the row) `change` and `baseline_decision`: the
/// decisions the baseline saw for the row's key, `null` for a new row.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct RowChange {
    pub change: Change,
    pub baseline_decision: Option<DecisionSet>,
    /// The recent window's decisions for the key; none for a gone row.
    #[serde(skip)]
    pub recent_decision: Option<DecisionSet>,
}

impl RowChange {
    /// `allowed→potentially_blocked` for a changed row; else the change.
    pub fn text(&self) -> String {
        match (self.change, self.baseline_decision, self.recent_decision) {
            (Change::Changed, Some(b), Some(r)) => format!("{}→{}", b.text(), r.text()),
            (c, _, _) => c.as_str().to_string(),
        }
    }

    /// `text` with `pot.blocked` for `potentially_blocked`: the TUI's cell.
    pub fn short_text(&self) -> String {
        self.text().replace("potentially_blocked", "pot.blocked")
    }

    /// The baseline's decisions as a cell: empty for a new row.
    pub fn baseline_text(&self) -> String {
        self.baseline_decision
            .map(DecisionSet::text)
            .unwrap_or_default()
    }
}

/// Rows per status: the header's `12 new · 3 changed · 5 gone`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct CompareCounts {
    pub new: u64,
    pub changed: u64,
    pub gone: u64,
    pub same: u64,
}

impl CompareCounts {
    pub fn add(&mut self, c: Change) {
        match c {
            Change::New => self.new += 1,
            Change::Changed => self.changed += 1,
            Change::Gone => self.gone += 1,
            Change::Same => self.same += 1,
        }
    }

    pub fn of(&self, c: Change) -> u64 {
        match c {
            Change::New => self.new,
            Change::Changed => self.changed,
            Change::Gone => self.gone,
            Change::Same => self.same,
        }
    }

    /// `12 new · 3 changed · 5 gone`.
    pub fn text(&self) -> String {
        format!(
            "{} new · {} changed · {} gone",
            self.new, self.changed, self.gone
        )
    }
}

/// Which statuses are shown: `--changes` (default new, changed, gone) and
/// the TUI's `n`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ChangeSet(u8);

impl Default for ChangeSet {
    /// New, changed and gone: what is different.
    fn default() -> Self {
        Self::of(&[Change::New, Change::Changed, Change::Gone])
    }
}

impl ChangeSet {
    pub const ALL: ChangeSet = ChangeSet(0b1111);

    pub fn of(changes: &[Change]) -> Self {
        Self(changes.iter().fold(0, |b, c| b | (1 << *c as u8)))
    }

    pub fn contains(self, c: Change) -> bool {
        self.0 & (1 << c as u8) != 0
    }

    /// Whether a row with `change` is shown (a row without one always is).
    pub fn keeps(self, change: Option<&RowChange>) -> bool {
        change.is_none_or(|c| self.contains(c.change))
    }

    /// `new,changed,gone,same` (any subset, comma-separated).
    pub fn parse(s: &str) -> Result<Self, String> {
        let mut out = Vec::new();
        for t in s.split(',').map(str::trim).filter(|t| !t.is_empty()) {
            out.push(
                Change::parse(t)
                    .ok_or_else(|| format!("unknown change {t:?}: use new, changed, gone, same"))?,
            );
        }
        if out.is_empty() {
            return Err("no change given: use new, changed, gone, same".into());
        }
        Ok(Self::of(&out))
    }

    /// `new,changed,gone`.
    pub fn text(self) -> String {
        Change::ALL
            .into_iter()
            .filter(|c| self.contains(*c))
            .map(Change::as_str)
            .collect::<Vec<_>>()
            .join(",")
    }
}

/// The recent rows' changes, in their order, and the gone rows: the first
/// baseline row of each key the recent rows lack.
#[derive(Debug, Clone, PartialEq)]
pub struct Compared<T> {
    pub recent: Vec<RowChange>,
    pub gone: Vec<(T, RowChange)>,
    pub counts: CompareCounts,
}

/// Compares `recent` with `baseline` on `key`: each key's decisions are
/// those of all its rows on that side (`decisions` per row, joined).
pub fn compare_by<'a, T: Clone + 'a, K: Ord + Clone>(
    recent: impl IntoIterator<Item = &'a T>,
    baseline: impl IntoIterator<Item = &'a T>,
    key: impl Fn(&T) -> K,
    decisions: impl Fn(&T) -> DecisionSet,
) -> Compared<T> {
    let recent: Vec<(&T, K)> = recent.into_iter().map(|r| (r, key(r))).collect();
    let mut seen: BTreeMap<&K, DecisionSet> = BTreeMap::new();
    for (r, k) in &recent {
        let d = seen.entry(k).or_default();
        *d = d.union(decisions(r));
    }
    // The baseline per key: its decisions, and its first row.
    let mut base: BTreeMap<K, (DecisionSet, Option<&T>)> = BTreeMap::new();
    let mut order: Vec<K> = Vec::new();
    for b in baseline {
        let e = base.entry(key(b)).or_insert_with_key(|k| {
            order.push(k.clone());
            (DecisionSet::default(), Some(b))
        });
        e.0 = e.0.union(decisions(b));
    }
    let mut counts = CompareCounts::default();
    let changes = recent
        .iter()
        .map(|(_, k)| {
            let now = seen[k];
            let rc = match base.get(k) {
                None => RowChange {
                    change: Change::New,
                    baseline_decision: None,
                    recent_decision: Some(now),
                },
                Some((then, _)) => RowChange {
                    change: if *then == now {
                        Change::Same
                    } else {
                        Change::Changed
                    },
                    baseline_decision: Some(*then),
                    recent_decision: Some(now),
                },
            };
            counts.add(rc.change);
            rc
        })
        .collect();
    let mut gone = Vec::new();
    for k in order {
        if seen.contains_key(&k) {
            continue;
        }
        if let Some((then, Some(row))) = base.remove(&k) {
            counts.add(Change::Gone);
            gone.push((
                row.clone(),
                RowChange {
                    change: Change::Gone,
                    baseline_decision: Some(then),
                    recent_decision: None,
                },
            ));
        }
    }
    Compared {
        recent: changes,
        gone,
        counts,
    }
}

/// One side of a flow: its workload, else its IP.
fn endpoint(e: &ResolvedEndpoint) -> Option<String> {
    e.workload_href.clone().or_else(|| e.ip.clone())
}

/// A flow's compare key: both endpoints (workload href, else IP), port,
/// proto, process, user and direction. Unlike `record_key`, no decision
/// and no `first_detected`.
pub type FlowKey = (
    Option<String>,
    Option<String>,
    Option<u16>,
    String,
    Option<String>,
    Option<String>,
    Option<String>,
);

pub fn flow_key(f: &ResolvedFlow) -> FlowKey {
    (
        endpoint(&f.src),
        endpoint(&f.dst),
        f.port,
        f.proto.clone(),
        f.process_name.clone(),
        f.user_name.clone(),
        f.flow_direction.clone(),
    )
}

/// Flow records compared on `flow_key`. A gone key is shown once: its
/// latest baseline record (by `last_detected`), not one per day.
pub fn compare_flows(recent: &[ResolvedFlow], baseline: &[ResolvedFlow]) -> Compared<ResolvedFlow> {
    let mut latest: Vec<&ResolvedFlow> = baseline.iter().collect();
    latest.sort_by(|a, b| b.last_detected.cmp(&a.last_detected));
    compare_by(recent, latest, flow_key, |f| {
        DecisionSet::of(f.policy_decision)
    })
}

/// `rows` with each recent row's change set (`set`) and the gone rows
/// appended after them; the counts.
fn annotate<T: Clone, K: Ord + Clone>(
    rows: &mut Vec<T>,
    baseline: &[T],
    key: impl Fn(&T) -> K,
    decisions: impl Fn(&T) -> DecisionSet,
    set: impl Fn(&mut T, RowChange),
) -> CompareCounts {
    let c = compare_by(rows.iter(), baseline, key, decisions);
    for (r, rc) in rows.iter_mut().zip(c.recent) {
        set(r, rc);
    }
    for (mut g, rc) in c.gone {
        set(&mut g, rc);
        rows.push(g);
    }
    c.counts
}

/// The decisions of a group's flow records.
pub fn group_decisions(g: &FlowGroup) -> DecisionSet {
    DecisionSet::from_counts(g.allowed, g.potentially_blocked, g.blocked, g.flows)
}

/// Pairs (or pairs by service) compared on their source group,
/// destination group and service: `rows` (built from the recent flows)
/// annotated, the baseline's gone pairs appended.
pub fn compare_groups(rows: &mut Vec<FlowGroup>, baseline: &[FlowGroup]) -> CompareCounts {
    annotate(
        rows,
        baseline,
        |g| (g.src.clone(), g.dst.clone(), g.service.clone()),
        group_decisions,
        |g, c| g.change = Some(c),
    )
}

/// Mesh rows compared on `MeshKey` (the columns shown take part, as in
/// the grouping).
pub fn compare_mesh(rows: &mut Vec<MeshRow>, baseline: &[MeshRow]) -> CompareCounts {
    annotate(
        rows,
        baseline,
        MeshRow::key,
        |r| DecisionSet::from_counts(r.allowed, r.potentially_blocked, r.blocked, r.flows),
        |r, c| r.change = Some(c),
    )
}

/// App sections compared on their peer rows: app, direction, peer and
/// service, i.e. the source group, destination group and service. A gone
/// peer row joins its app's section in that direction (the header keeps
/// the recent totals), or a new section after that app's others (the
/// app's sections stay adjacent), with empty totals.
pub fn compare_sections(sections: &mut Vec<AppSection>, baseline: &[AppSection]) -> CompareCounts {
    type Key = (Vec<String>, Direction, Vec<String>, String);
    let flat = |s: &[AppSection]| -> Vec<(Key, super::sections::PeerRow)> {
        s.iter()
            .flat_map(|sec| {
                sec.rows.iter().map(|r| {
                    (
                        (
                            sec.app.clone(),
                            sec.direction,
                            r.peer.clone(),
                            r.service.clone(),
                        ),
                        r.clone(),
                    )
                })
            })
            .collect()
    };
    let recent = flat(sections);
    let base = flat(baseline);
    let peer_decisions = |(_, r): &(Key, super::sections::PeerRow)| {
        DecisionSet::from_counts(r.allowed, r.potentially_blocked, r.blocked, r.flows)
    };
    let c = compare_by(
        recent.iter(),
        base.iter(),
        |(k, _)| k.clone(),
        peer_decisions,
    );
    let mut changes = c.recent.into_iter();
    for sec in sections.iter_mut() {
        for r in &mut sec.rows {
            r.change = changes.next();
        }
    }
    for ((app, direction, _, _), mut row) in c.gone.into_iter().map(|((k, r), rc)| {
        let mut r = r;
        r.change = Some(rc);
        (k, r)
    }) {
        row.marks = None;
        match sections
            .iter_mut()
            .find(|s| s.app == app && s.direction == direction)
        {
            Some(s) => s.rows.push(row),
            None => {
                let at = sections
                    .iter()
                    .rposition(|s| s.app == app)
                    .map_or(sections.len(), |i| i + 1);
                sections.insert(
                    at,
                    AppSection {
                        app,
                        direction,
                        rows: vec![row],
                        totals: Default::default(),
                    },
                );
            }
        }
    }
    c.counts
}

/// Keeps the rows `show` keeps: flows by their change, `None` rows always.
pub fn keep_changes<T>(
    rows: &mut Vec<T>,
    show: ChangeSet,
    change: impl Fn(&T) -> Option<&RowChange>,
) {
    rows.retain(|r| show.keeps(change(r)));
}

/// `keep_changes` on the peer rows of `sections`; a section left without
/// rows goes too.
pub fn keep_section_changes(sections: &mut Vec<AppSection>, show: ChangeSet) {
    for s in sections.iter_mut() {
        s.rows.retain(|r| show.keeps(r.change.as_ref()));
    }
    sections.retain(|s| !s.rows.is_empty());
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::traffic::aggregate::{group_by_labels, group_by_service};
    use crate::traffic::mesh::{MeshColumns, mesh};
    use crate::traffic::sections::{Decisions, app_sections, app_sections_both};

    fn ep(href: Option<&str>, ip: &str, app: &str) -> ResolvedEndpoint {
        ResolvedEndpoint {
            ip: Some(ip.into()),
            workload_href: href.map(str::to_string),
            labels: vec![crate::resolve::ResolvedLabel {
                href: format!("/orgs/1/labels/{app}"),
                key: "app".into(),
                value: app.into(),
            }],
            ..Default::default()
        }
    }

    /// A flow from `/w/<src>` (app `src`) to `/w/<dst>` (app `dst`) on
    /// `port`/tcp with `decision`.
    fn flow(src: &str, dst: &str, port: u16, decision: PolicyDecision) -> ResolvedFlow {
        ResolvedFlow {
            src: ep(Some(&format!("/w/{src}")), "10.0.0.1", src),
            dst: ep(Some(&format!("/w/{dst}")), "10.0.0.2", dst),
            port: Some(port),
            proto: "tcp".into(),
            process_name: Some("nginx".into()),
            user_name: None,
            num_connections: 3,
            policy_decision: decision,
            draft_policy_decision: None,
            flow_direction: Some("outbound".into()),
            first_detected: Some("2026-10-25T00:00:00Z".into()),
            last_detected: Some("2026-10-25T11:00:00Z".into()),
            rules: Vec::new(),
            side_rules: Vec::new(),
            bytes_in: None,
            bytes_out: None,
        }
    }

    use PolicyDecision::{Allowed, Blocked, PotentiallyBlocked as Pot};

    fn changes(c: &Compared<ResolvedFlow>) -> Vec<Change> {
        c.recent.iter().map(|r| r.change).collect()
    }

    #[test]
    fn flows_are_new_changed_same_or_gone_on_their_key() {
        let recent = vec![
            flow("a", "b", 443, Allowed),
            flow("a", "b", 80, Pot),
            flow("a", "c", 22, Allowed),
        ];
        let baseline = vec![
            flow("a", "b", 443, Allowed),
            flow("a", "b", 80, Allowed),
            flow("x", "y", 53, Blocked),
        ];
        let c = compare_flows(&recent, &baseline);
        assert_eq!(changes(&c), [Change::Same, Change::Changed, Change::New]);
        assert_eq!(c.recent[1].text(), "allowed→potentially_blocked");
        assert_eq!(c.recent[1].short_text(), "allowed→pot.blocked");
        assert_eq!(c.recent[1].baseline_text(), "allowed");
        assert_eq!(c.recent[2].baseline_decision, None, "new: no baseline");
        assert_eq!(c.recent[2].baseline_text(), "");
        assert_eq!(c.gone.len(), 1);
        assert_eq!(c.gone[0].0.port, Some(53), "the baseline's values");
        assert_eq!(c.gone[0].1.change, Change::Gone);
        assert_eq!(c.gone[0].1.baseline_text(), "blocked");
        assert_eq!(
            c.counts,
            CompareCounts {
                new: 1,
                changed: 1,
                gone: 1,
                same: 1
            }
        );
        assert_eq!(c.counts.text(), "1 new · 1 changed · 1 gone");
    }

    #[test]
    fn first_detected_never_makes_a_flow_new_nor_does_the_count() {
        let mut today = flow("a", "b", 443, Allowed);
        today.first_detected = Some("2026-10-26T00:00:00Z".into());
        today.num_connections = 1;
        let mut monday = flow("a", "b", 443, Allowed);
        monday.first_detected = Some("2026-10-19T00:00:00Z".into());
        monday.last_detected = Some("2026-10-19T23:59:00Z".into());
        let c = compare_flows(&[today], &[monday]);
        assert_eq!(changes(&c), [Change::Same]);
        assert!(c.gone.is_empty());
    }

    #[test]
    fn every_key_part_but_the_decision_and_first_detected_tells_flows_apart() {
        let base = flow("a", "b", 443, Allowed);
        let mut variants = Vec::new();
        let mut v = base.clone();
        v.port = Some(80);
        variants.push(v);
        let mut v = base.clone();
        v.proto = "udp".into();
        variants.push(v);
        let mut v = base.clone();
        v.src.workload_href = Some("/w/other".into());
        variants.push(v);
        let mut v = base.clone();
        v.dst.workload_href = None;
        v.dst.ip = Some("10.9.9.9".into());
        variants.push(v);
        let mut v = base.clone();
        v.process_name = None;
        variants.push(v);
        let mut v = base.clone();
        v.user_name = Some("root".into());
        variants.push(v);
        let mut v = base.clone();
        v.flow_direction = Some("inbound".into());
        variants.push(v);
        for v in &variants {
            let c = compare_flows(std::slice::from_ref(v), std::slice::from_ref(&base));
            assert_eq!(changes(&c), [Change::New], "{v:?}");
            assert_eq!(c.gone.len(), 1);
        }
        // A workload is matched by its href, whatever its IP that day.
        let mut moved = base.clone();
        moved.src.ip = Some("10.7.7.7".into());
        let c = compare_flows(&[moved], std::slice::from_ref(&base));
        assert_eq!(changes(&c), [Change::Same]);
    }

    #[test]
    fn decision_sets_join_every_record_of_a_key() {
        // Recent: allowed and blocked records of one flow; the baseline saw
        // only allowed: changed, for both rows.
        let recent = vec![flow("a", "b", 443, Allowed), flow("a", "b", 443, Blocked)];
        let c = compare_flows(&recent, &[flow("a", "b", 443, Allowed)]);
        assert_eq!(changes(&c), [Change::Changed, Change::Changed]);
        assert_eq!(c.recent[0].text(), "allowed→allowed+blocked");
        // The baseline saw both too: the same.
        let both = [flow("a", "b", 443, Blocked), flow("a", "b", 443, Allowed)];
        let c = compare_flows(&recent, &both);
        assert_eq!(changes(&c), [Change::Same, Change::Same]);
    }

    #[test]
    fn a_gone_flow_is_one_row_its_latest_record() {
        let mut d1 = flow("x", "y", 53, Allowed);
        d1.last_detected = Some("2026-10-18T23:00:00Z".into());
        let mut d2 = flow("x", "y", 53, Blocked);
        d2.last_detected = Some("2026-10-19T23:00:00Z".into());
        let c = compare_flows(&[], &[d1, d2]);
        assert_eq!(c.gone.len(), 1);
        assert_eq!(c.gone[0].0.policy_decision, Blocked, "the latest");
        assert_eq!(c.gone[0].1.baseline_text(), "allowed+blocked");
        assert_eq!(c.counts.gone, 1);
    }

    fn app_keys() -> Vec<String> {
        vec!["app".into()]
    }

    #[test]
    fn pairs_by_service_compare_on_groups_and_service_with_decision_sets() {
        let k = app_keys();
        let recent = vec![
            flow("a", "b", 443, Allowed),
            // Another host of app a: the same pair and service.
            flow("a", "b", 443, Blocked),
            flow("a", "c", 22, Allowed),
        ];
        let baseline = vec![flow("a", "b", 443, Allowed), flow("x", "b", 443, Allowed)];
        let mut rows = group_by_service(&recent, &k);
        let base = group_by_service(&baseline, &k);
        let counts = compare_groups(&mut rows, &base);
        let by = |src: &str, dst: &str| {
            rows.iter()
                .find(|g| g.src == [src] && g.dst == [dst])
                .unwrap()
                .change
                .unwrap()
        };
        assert_eq!(by("a", "b").change, Change::Changed);
        assert_eq!(by("a", "b").text(), "allowed→allowed+blocked");
        assert_eq!(by("a", "c").change, Change::New);
        let gone = by("x", "b");
        assert_eq!(gone.change, Change::Gone);
        assert_eq!(rows.last().unwrap().src, ["x"], "gone rows come last");
        assert_eq!((counts.new, counts.changed, counts.gone), (1, 1, 1));
        // Another service on the same pair is another row.
        let mut rows = group_by_service(&[flow("a", "b", 80, Allowed)], &k);
        compare_groups(&mut rows, &group_by_service(&baseline, &k));
        assert_eq!(rows[0].change.unwrap().change, Change::New);
        // Plain pairs ignore the service.
        let mut rows = group_by_labels(&[flow("a", "b", 80, Allowed)], &k);
        compare_groups(&mut rows, &group_by_labels(&baseline, &k));
        assert_eq!(rows[0].change.unwrap().change, Change::Same);
    }

    #[test]
    fn mesh_rows_compare_on_the_mesh_key() {
        let k = app_keys();
        let cols = MeshColumns {
            user: true,
            process: true,
        };
        let mut other = flow("a", "b", 443, Allowed);
        other.process_name = Some("java".into());
        let mut rows = mesh(&[flow("a", "b", 443, Allowed), other], &k, cols);
        let counts = compare_mesh(&mut rows, &mesh(&[flow("a", "b", 443, Pot)], &k, cols));
        let by = |p: &str| {
            rows.iter()
                .find(|r| r.process.as_deref() == Some(p))
                .unwrap()
                .change
                .unwrap()
        };
        assert_eq!(by("nginx").text(), "potentially_blocked→allowed");
        assert_eq!(by("java").change, Change::New);
        assert_eq!((counts.new, counts.changed, counts.gone), (1, 1, 0));
    }

    #[test]
    fn sections_compare_peer_rows_and_put_gone_ones_under_their_app() {
        let k = app_keys();
        let recent = vec![flow("a", "b", 443, Blocked), flow("c", "b", 22, Blocked)];
        let baseline = vec![
            flow("a", "b", 443, Blocked),
            // Gone, under b (in): b's section exists.
            flow("d", "b", 8080, Pot),
            // Gone, under e: no section yet.
            flow("a", "e", 25, Blocked),
        ];
        let agg = |f: &[ResolvedFlow]| app_sections(f, &k, Direction::In, Decisions::Blocked);
        let mut sections = agg(&recent);
        let totals = sections[0].totals.clone();
        let counts = compare_sections(&mut sections, &agg(&baseline));
        assert_eq!((counts.new, counts.same, counts.gone), (1, 1, 2));
        let b = sections.iter().find(|s| s.app == ["b"]).unwrap();
        assert_eq!(b.totals, totals, "the header keeps the recent totals");
        let tags: Vec<(String, Change)> = b
            .rows
            .iter()
            .map(|r| (r.peer.join("/"), r.change.unwrap().change))
            .collect();
        assert!(tags.contains(&("a".into(), Change::Same)), "{tags:?}");
        assert!(tags.contains(&("c".into(), Change::New)), "{tags:?}");
        assert_eq!(tags.last(), Some(&("d".into(), Change::Gone)));
        let e = sections.iter().find(|s| s.app == ["e"]).unwrap();
        assert_eq!(e.totals, Default::default());
        assert_eq!(e.rows[0].change.unwrap().baseline_text(), "blocked");
        keep_section_changes(&mut sections, ChangeSet::of(&[Change::Gone]));
        let apps: Vec<String> = sections.iter().map(|s| s.app_text()).collect();
        assert_eq!(apps, ["b", "e"]);
        assert!(
            sections
                .iter()
                .flat_map(|s| &s.rows)
                .all(|r| r.change.unwrap().change == Change::Gone)
        );
    }

    #[test]
    fn a_new_section_for_an_app_stays_beside_its_other_direction() {
        let k = app_keys();
        let recent = vec![flow("a", "b", 443, Allowed), flow("c", "d", 22, Allowed)];
        // Gone: a as a destination (a's `in` section, which recent lacks).
        let baseline = vec![flow("z", "a", 80, Allowed)];
        let mut sections = app_sections_both(&recent, &k, Decisions::All);
        compare_sections(
            &mut sections,
            &app_sections_both(&baseline, &k, Decisions::All),
        );
        let order: Vec<(String, Direction)> = sections
            .iter()
            .map(|s| (s.app_text(), s.direction))
            .collect();
        let a: Vec<usize> = order
            .iter()
            .enumerate()
            .filter(|(_, (app, _))| app == "a")
            .map(|(i, _)| i)
            .collect();
        assert_eq!(a.len(), 2, "{order:?}");
        assert_eq!(a[1], a[0] + 1, "adjacent: {order:?}");
    }

    #[test]
    fn squelched_rows_left_out_of_both_sides_never_count_as_gone() {
        // The caller drops squelched flows from both sides before
        // comparing: a squelched flow is neither new nor gone.
        let squelched = |f: &ResolvedFlow| f.port == Some(53);
        let recent = vec![flow("a", "b", 443, Allowed), flow("x", "y", 53, Allowed)];
        let baseline = vec![flow("a", "b", 443, Allowed), flow("x", "y", 53, Allowed)];
        let keep = |v: &[ResolvedFlow]| -> Vec<ResolvedFlow> {
            v.iter().filter(|f| !squelched(f)).cloned().collect()
        };
        let c = compare_flows(&keep(&recent), &keep(&baseline));
        assert_eq!(c.counts.gone + c.counts.new, 0);
        // Hidden on one side only, it would read as gone.
        let c = compare_flows(&keep(&recent), &baseline);
        assert_eq!(c.counts.gone, 1);
    }

    #[test]
    fn change_sets_parse_filter_and_print() {
        assert_eq!(ChangeSet::default().text(), "new,changed,gone");
        assert_eq!(ChangeSet::ALL.text(), "new,changed,gone,same");
        let s = ChangeSet::parse("gone, NEW").unwrap();
        assert_eq!(s.text(), "new,gone");
        assert!(ChangeSet::parse("old").unwrap_err().contains("\"old\""));
        assert!(ChangeSet::parse(" , ").is_err());
        let rc = |c| RowChange {
            change: c,
            baseline_decision: None,
            recent_decision: None,
        };
        assert!(s.keeps(Some(&rc(Change::New))));
        assert!(!s.keeps(Some(&rc(Change::Same))));
        assert!(s.keeps(None), "a row without a change stays");
        let mut rows = vec![Some(rc(Change::Same)), Some(rc(Change::Gone)), None];
        keep_changes(&mut rows, s, |r| r.as_ref());
        assert_eq!(rows.len(), 2);
        assert_eq!(Change::Changed.tag(), "CHG");
        assert_eq!(Change::Same.tag(), "");
    }

    #[test]
    fn decision_sets_from_counts_and_json() {
        let s = DecisionSet::from_counts(1, 0, 2, 4);
        assert_eq!(s.text(), "allowed+blocked+unknown");
        assert_eq!(
            DecisionSet::from_counts(0, 1, 0, 1).text(),
            "potentially_blocked"
        );
        let rc = RowChange {
            change: Change::Changed,
            baseline_decision: Some(DecisionSet::of(Allowed)),
            recent_decision: Some(DecisionSet::of(Blocked)),
        };
        assert_eq!(
            serde_json::to_value(rc).unwrap(),
            serde_json::json!({"change": "changed", "baseline_decision": "allowed"})
        );
        let new = RowChange {
            change: Change::New,
            baseline_decision: None,
            recent_decision: None,
        };
        assert_eq!(
            serde_json::to_value(new).unwrap(),
            serde_json::json!({"change": "new", "baseline_decision": null})
        );
    }
}
