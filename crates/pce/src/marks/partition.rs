//! Marks applied to a result: classify every flow, keep the unsquelched
//! ones for the views' own aggregators, and join each mark's subset,
//! aggregated by the same aggregator, to the view's rows by the row's key.
//! No key logic is copied: the aggregators keep their signatures.
use super::{Action, ActiveMarks, Hit};
use crate::app::AppPeer;
use crate::traffic::{AppSection, FlowGroup, MeshRow, ResolvedFlow};
use serde::Serialize;
use serde::ser::SerializeMap;
use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet};

/// The flows one mark wins, in input order.
#[derive(Debug, Clone, PartialEq)]
pub struct MarkedFlows {
    /// Index into `MarkSet::marks`.
    pub mark: usize,
    pub action: Action,
    pub flows: Vec<ResolvedFlow>,
}

/// The marked flows of a result, by winning mark: what the annotations
/// join to a view's rows. Owned, so the TUI keeps it between rebuilds.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Marked {
    /// One entry per mark that wins a flow, by mark index; together they
    /// partition the marked flows.
    pub subsets: Vec<MarkedFlows>,
    /// Squelched flows are in the views' input (`M`, `--show-squelched`).
    pub keep_squelched: bool,
}

/// A result split by the marks.
#[derive(Debug, Clone)]
pub struct Partition<'a> {
    /// What the views aggregate: every flow with squelched ones shown,
    /// else the unsquelched ones. Borrowed unless a flow is left out.
    pub kept: Cow<'a, [ResolvedFlow]>,
    /// The hit of each flow of `kept`, in its order.
    pub kept_hits: Vec<Option<Hit>>,
    /// Flows with a hit.
    pub hits: usize,
    pub marked: Marked,
}

/// Classifies `flows` against `m`. Copies only what matches: the marked
/// flows into their subsets, and `kept` only when a flow is left out.
pub fn partition<'a>(
    flows: &'a [ResolvedFlow],
    m: &ActiveMarks,
    keep_squelched: bool,
) -> Partition<'a> {
    if m.is_empty() {
        return Partition {
            kept: Cow::Borrowed(flows),
            kept_hits: vec![None; flows.len()],
            hits: 0,
            marked: Marked {
                subsets: Vec::new(),
                keep_squelched,
            },
        };
    }
    let all: Vec<Option<Hit>> = flows.iter().map(|f| m.classify(f)).collect();
    let mut by_mark: BTreeMap<usize, MarkedFlows> = BTreeMap::new();
    for (f, h) in flows.iter().zip(&all) {
        if let Some(h) = h {
            by_mark
                .entry(h.mark)
                .or_insert_with(|| MarkedFlows {
                    mark: h.mark,
                    action: h.action,
                    flows: Vec::new(),
                })
                .flows
                .push(f.clone());
        }
    }
    let squelched = |h: &Option<Hit>| h.is_some_and(|h| h.action == Action::Squelch);
    let hide = !keep_squelched && all.iter().any(squelched);
    let (kept, kept_hits) = if hide {
        let (kept, hits): (Vec<ResolvedFlow>, Vec<Option<Hit>>) = flows
            .iter()
            .zip(&all)
            .filter(|(_, h)| !squelched(h))
            .map(|(f, h)| (f.clone(), *h))
            .unzip();
        (Cow::Owned(kept), hits)
    } else {
        (Cow::Borrowed(flows), all.clone())
    };
    Partition {
        kept,
        kept_hits,
        hits: all.iter().filter(|h| h.is_some()).count(),
        marked: Marked {
            subsets: by_mark.into_values().collect(),
            keep_squelched,
        },
    }
}

impl Marked {
    /// The squelched flows, shown or not.
    pub fn squelched(&self) -> impl Iterator<Item = &ResolvedFlow> {
        self.subsets
            .iter()
            .filter(|s| s.action == Action::Squelch)
            .flat_map(|s| &s.flows)
    }

    /// The squelched flows `pred` keeps (a view's decision filter, a
    /// drill): a view's `squelched.flows`, each record once.
    pub fn squelched_where(&self, pred: impl Fn(&ResolvedFlow) -> bool) -> u64 {
        self.squelched().filter(|f| pred(f)).count() as u64
    }

    /// Flow records whose hit is `action`.
    pub fn count(&self, action: Action) -> u64 {
        self.subsets
            .iter()
            .filter(|s| s.action == action)
            .map(|s| s.flows.len() as u64)
            .sum()
    }
}

/// The strongest mark of one action among a row's flows.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TopMark {
    pub mark: usize,
    pub id: String,
}

/// What the marks say about one aggregated row. Flattened into the row's
/// JSON as `mark` (the marker's action, its strongest mark and its flow
/// records), `marks` (records per action) and `squelched` (hidden records).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RowMarks {
    /// The row's flow records (its `flows`).
    pub flows: u64,
    /// The row's records per winning action, by `Action::index`.
    pub by_action: [u64; 6],
    /// Per action, the strongest mark among the row's records.
    pub top: [Option<TopMark>; 6],
    /// Squelched records under the row's key left out of its counts.
    pub hidden: u64,
}

/// What a row's gutter shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Marker {
    Critical,
    Important,
    Revisit,
    /// Every flow accepted as blocked (or squelched, shown).
    Accepted,
    /// Every flow reviewed or accepted (or squelched, shown).
    Reviewed,
    /// Every flow squelched (shown).
    Squelched,
    /// Some records of the row are squelched and hidden: how many.
    PartlySquelched(u64),
}

impl Marker {
    /// The action `mark.action` reports; none for a partly squelched row.
    pub fn action(self) -> Option<Action> {
        match self {
            Marker::Critical => Some(Action::Critical),
            Marker::Important => Some(Action::Important),
            Marker::Revisit => Some(Action::Revisit),
            Marker::Accepted => Some(Action::AcceptBlocked),
            Marker::Reviewed => Some(Action::Reviewed),
            Marker::Squelched => Some(Action::Squelch),
            Marker::PartlySquelched(_) => None,
        }
    }

    /// The gutter glyph: `●` critical and important, `◆` revisit, `≈`
    /// accepted, `✓` reviewed, `~` squelched.
    pub fn glyph(self) -> &'static str {
        match self {
            Marker::Critical | Marker::Important => "●",
            Marker::Revisit => "◆",
            Marker::Accepted => "≈",
            Marker::Reviewed => "✓",
            Marker::Squelched | Marker::PartlySquelched(_) => "~",
        }
    }

    /// A flow's marker from its hit.
    pub fn of(action: Action) -> Marker {
        match action {
            Action::Critical => Marker::Critical,
            Action::Important => Marker::Important,
            Action::Revisit => Marker::Revisit,
            Action::AcceptBlocked => Marker::Accepted,
            Action::Reviewed => Marker::Reviewed,
            Action::Squelch => Marker::Squelched,
        }
    }
}

impl RowMarks {
    pub fn count(&self, a: Action) -> u64 {
        self.by_action[a.index()]
    }

    pub fn top_id(&self, a: Action) -> Option<&str> {
        self.top[a.index()].as_ref().map(|t| t.id.as_str())
    }

    /// Highlights: any (the strongest a flow has). Quiet: all, and no
    /// highlight. A partly squelched row says how many records it hides.
    pub fn marker(&self) -> Option<Marker> {
        for (a, m) in [
            (Action::Critical, Marker::Critical),
            (Action::Important, Marker::Important),
            (Action::Revisit, Marker::Revisit),
        ] {
            if self.count(a) > 0 {
                return Some(m);
            }
        }
        let (acc, rev, sq) = (
            self.count(Action::AcceptBlocked),
            self.count(Action::Reviewed),
            self.count(Action::Squelch),
        );
        if self.flows > 0 && acc + rev + sq == self.flows {
            return Some(if sq == self.flows {
                Marker::Squelched
            } else if rev == 0 {
                Marker::Accepted
            } else {
                Marker::Reviewed
            });
        }
        (self.hidden > 0).then_some(Marker::PartlySquelched(self.hidden))
    }
}

/// `{"critical": 3, "reviewed": 38}`: the non-zero counts, strongest first.
struct ActionCounts<'a>(&'a [u64; 6]);

impl Serialize for ActionCounts<'_> {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let mut map = s.serialize_map(None)?;
        for a in Action::ALL {
            let n = self.0[a.index()];
            if n > 0 {
                map.serialize_entry(a.as_str(), &n)?;
            }
        }
        map.end()
    }
}

#[derive(Serialize)]
struct RowMark<'a> {
    action: Action,
    id: Option<&'a str>,
    flows: u64,
}

impl Serialize for RowMarks {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let mut map = s.serialize_map(None)?;
        if let Some(a) = self.marker().and_then(Marker::action) {
            map.serialize_entry(
                "mark",
                &RowMark {
                    action: a,
                    id: self.top_id(a),
                    flows: self.count(a),
                },
            )?;
        }
        if self.by_action.iter().any(|n| *n > 0) {
            map.serialize_entry("marks", &ActionCounts(&self.by_action))?;
        }
        if self.hidden > 0 {
            map.serialize_entry("squelched", &self.hidden)?;
        }
        map.end()
    }
}

/// Joins each subset's aggregation (`subset_rows`: its row keys and flow
/// records) to the view's rows by key. Returns the rows squelch removed:
/// keys of squelched records the view no longer has, or, with squelched
/// flows shown, rows all of whose records are squelched.
fn join<K: Ord>(
    rows: Vec<(K, u64, &mut Option<RowMarks>)>,
    p: &Marked,
    m: &ActiveMarks,
    subset_rows: impl Fn(&[ResolvedFlow]) -> Vec<(K, u64)>,
) -> u64 {
    let mut index: BTreeMap<K, (u64, &mut Option<RowMarks>)> = rows
        .into_iter()
        .map(|(k, n, slot)| (k, (n, slot)))
        .collect();
    let mut gone: BTreeSet<K> = BTreeSet::new();
    for s in &p.subsets {
        let a = s.action.index();
        let hide = s.action == Action::Squelch && !p.keep_squelched;
        for (k, n) in subset_rows(&s.flows) {
            let Some((flows, slot)) = index.get_mut(&k) else {
                if hide {
                    gone.insert(k);
                }
                continue;
            };
            let rm = slot.get_or_insert_with(|| RowMarks {
                flows: *flows,
                ..RowMarks::default()
            });
            if hide {
                rm.hidden += n;
                continue;
            }
            rm.by_action[a] += n;
            if rm.top[a]
                .as_ref()
                .is_none_or(|t| m.rank(s.mark) < m.rank(t.mark))
            {
                rm.top[a] = Some(TopMark {
                    mark: s.mark,
                    id: m.mark(s.mark).id.clone(),
                });
            }
        }
    }
    if p.keep_squelched {
        index
            .values()
            .filter(|(_, slot)| {
                slot.as_ref()
                    .is_some_and(|r| r.flows > 0 && r.count(Action::Squelch) == r.flows)
            })
            .count() as u64
    } else {
        gone.len() as u64
    }
}

type GroupKey = (Vec<String>, Vec<String>, Option<String>);

fn group_key(g: &FlowGroup) -> GroupKey {
    (g.src.clone(), g.dst.clone(), g.service.clone())
}

/// Pairs (plain or by service; the Volume view's groups): `agg` is the view's
/// own aggregator, applied to the flows `rows` came from.
pub fn annotate_groups(
    rows: &mut [FlowGroup],
    p: &Marked,
    m: &ActiveMarks,
    agg: impl Fn(&[ResolvedFlow]) -> Vec<FlowGroup>,
) -> u64 {
    let slots = rows
        .iter_mut()
        .map(|g| (group_key(g), g.flows, &mut g.marks))
        .collect();
    join(slots, p, m, |f| {
        agg(f).iter().map(|g| (group_key(g), g.flows)).collect()
    })
}

/// Mesh rows, joined by `MeshRow::key`.
pub fn annotate_mesh(
    rows: &mut [MeshRow],
    p: &Marked,
    m: &ActiveMarks,
    agg: impl Fn(&[ResolvedFlow]) -> Vec<MeshRow>,
) -> u64 {
    let slots = rows
        .iter_mut()
        .map(|r| (r.key(), r.flows, &mut r.marks))
        .collect();
    join(slots, p, m, |f| {
        agg(f).iter().map(|r| (r.key(), r.flows)).collect()
    })
}

type PeerKey = (Vec<String>, crate::traffic::Direction, Vec<String>, String);

fn peer_keys(sections: &[AppSection]) -> Vec<(PeerKey, u64)> {
    sections
        .iter()
        .flat_map(|s| {
            s.rows.iter().map(move |r| {
                (
                    (
                        s.app.clone(),
                        s.direction,
                        r.peer.clone(),
                        r.service.clone(),
                    ),
                    r.flows,
                )
            })
        })
        .collect()
}

/// App sections' peer rows, joined by (section app, direction, peer,
/// service). A section header is not a row: one that loses every row is
/// gone and not counted.
pub fn annotate_sections(
    sections: &mut [AppSection],
    p: &Marked,
    m: &ActiveMarks,
    agg: impl Fn(&[ResolvedFlow]) -> Vec<AppSection>,
) -> u64 {
    let slots = sections
        .iter_mut()
        .flat_map(|s| {
            let (app, dir) = (s.app.clone(), s.direction);
            s.rows.iter_mut().map(move |r| {
                (
                    (app.clone(), dir, r.peer.clone(), r.service.clone()),
                    r.flows,
                    &mut r.marks,
                )
            })
        })
        .collect();
    join(slots, p, m, |f| peer_keys(&agg(f)))
}

/// The app overlay's peers, joined by (peer, service, decision).
pub fn annotate_peers(
    rows: &mut [AppPeer],
    p: &Marked,
    m: &ActiveMarks,
    agg: impl Fn(&[ResolvedFlow]) -> Vec<AppPeer>,
) -> u64 {
    let key = |r: &AppPeer| (r.peer.clone(), r.service.clone(), r.decision.as_str());
    let slots = rows
        .iter_mut()
        .map(|r| (key(r), r.flows, &mut r.marks))
        .collect();
    join(slots, p, m, |f| {
        agg(f).iter().map(|r| (key(r), r.flows)).collect()
    })
}

#[cfg(test)]
mod tests {
    use super::super::tests::mark;
    use super::super::{MarkSet, ServiceSpec, parse_decision};
    use super::*;
    use crate::app::app_peers;
    use crate::models::PolicyDecision;
    use crate::traffic::resolved::tests::rich_flows;
    use crate::traffic::{
        Decisions, Direction, Focus, MeshColumns, app_sections, app_sections_both, group_by_labels,
        group_by_service, mesh,
    };
    use jiff::Timestamp;

    fn now() -> Timestamp {
        "2026-10-17T12:00:00Z".parse().unwrap()
    }

    fn keys() -> Vec<String> {
        vec!["app".into(), "env".into()]
    }

    /// Two squelches (the Corp list; shop's eu web tier to its databases
    /// on 5432), a critical on blocked flows into shop/prod's databases
    /// (it takes two flows from the squelches), erp/prod reviewed inside
    /// itself, erp/dev accepted as blocked, hr to revisit.
    fn set() -> MarkSet {
        let mut sq = mark("m-sq2", Action::Squelch, "role=web loc=eu", "role=db");
        sq.services = vec![ServiceSpec::parse("5432/tcp").unwrap()];
        let mut crit = mark(
            "m-crit",
            Action::Critical,
            "any",
            "app=shop env=prod role=db",
        );
        crit.decisions = vec![PolicyDecision::Blocked];
        let mut acc = mark("m-acc", Action::AcceptBlocked, "app=erp env=dev", "any");
        acc.decisions = vec![
            parse_decision("blocked").unwrap(),
            parse_decision("potentially_blocked").unwrap(),
        ];
        MarkSet {
            marks: vec![
                mark("m-sq1", Action::Squelch, "ipl:Corp", "any"),
                sq,
                crit,
                mark(
                    "m-rev",
                    Action::Reviewed,
                    "app=erp env=prod",
                    "app=erp env=prod",
                ),
                acc,
                mark("m-hr", Action::Revisit, "app=hr", "any"),
            ],
        }
    }

    fn find<'a>(rows: &'a [FlowGroup], src: &str, dst: &str) -> Option<&'a FlowGroup> {
        rows.iter()
            .find(|g| g.src.join("/") == src && g.dst.join("/") == dst)
    }

    #[test]
    fn partition_copies_only_what_matches() {
        let flows = rich_flows();
        let empty = MarkSet::default();
        let p = partition(&flows, &empty.active(None, now(), false), false);
        assert!(matches!(p.kept, Cow::Borrowed(_)));
        assert!(p.marked.subsets.is_empty() && p.hits == 0);
        let s = set();
        let a = s.active(None, now(), false);
        let hidden = partition(&flows, &a, false);
        assert!(matches!(hidden.kept, Cow::Owned(_)));
        assert_eq!(hidden.marked.count(Action::Squelch), 9);
        assert_eq!(hidden.kept.len(), 30 - 9);
        assert_eq!(hidden.kept_hits.len(), hidden.kept.len());
        assert_eq!(hidden.hits, 9 + 2 + 2 + 2 + 4);
        let shown = partition(&flows, &a, true);
        assert!(matches!(shown.kept, Cow::Borrowed(_)));
        assert_eq!(shown.kept.len(), 30);
        assert_eq!(shown.marked.squelched().count(), 9);
        for (f, h) in shown.kept.iter().zip(&shown.kept_hits) {
            assert_eq!(*h, a.classify(f));
        }
    }

    #[test]
    fn a_row_is_hidden_iff_all_its_flows_are_squelched_and_partial_rows_keep_visible_counts() {
        let flows = rich_flows();
        let s = set();
        let a = s.active(None, now(), false);
        let p = partition(&flows, &a, false);
        let agg = |f: &[ResolvedFlow]| group_by_labels(f, &keys());
        let mut rows = agg(&p.kept);
        let gone = annotate_groups(&mut rows, &p.marked, &a, agg);
        // Corp to crm, hr and erp, and shop/prod inside itself.
        assert_eq!(gone, 4);
        for (src, dst) in [
            ("ipl:Corp/-", "crm/prod"),
            ("ipl:Corp/-", "hr/prod"),
            ("ipl:Corp/-", "erp/prod"),
            ("shop/prod", "shop/prod"),
        ] {
            assert!(find(&rows, src, dst).is_none(), "{src} → {dst}");
        }
        // Corp to shop/prod keeps its critical flow (21) and hides 17.
        let corp = find(&rows, "ipl:Corp/-", "shop/prod").unwrap();
        assert_eq!(corp.flows, 1);
        let rm = corp.marks.as_ref().unwrap();
        assert_eq!((rm.flows, rm.hidden), (1, 1));
        assert_eq!(rm.marker(), Some(Marker::Critical));
        assert_eq!(rm.top_id(Action::Critical), Some("m-crit"));
        // shop/dev inside itself: 5432 squelched and hidden, 22 shown.
        let dev = find(&rows, "shop/dev", "shop/dev").unwrap();
        assert_eq!(dev.flows, 1);
        assert_eq!(
            dev.marks.as_ref().unwrap().marker(),
            Some(Marker::PartlySquelched(1))
        );
        // With squelched rows shown, nothing disappears and the counts are
        // the full ones; the squelched-only rows say so.
        let shown = partition(&flows, &a, true);
        let mut all = agg(&shown.kept);
        let gone = annotate_groups(&mut all, &shown.marked, &a, agg);
        assert_eq!(gone, 4);
        assert_eq!(all.len(), rows.len() + 4);
        let full = find(&all, "shop/prod", "shop/prod").unwrap();
        assert_eq!(
            full.marks.as_ref().unwrap().marker(),
            Some(Marker::Squelched)
        );
        assert_eq!(full.marks.as_ref().unwrap().count(Action::Squelch), 4);
    }

    #[test]
    fn highlights_show_if_any_flow_has_one_and_quiet_rows_need_every_flow() {
        let flows = rich_flows();
        let s = set();
        let a = s.active(None, now(), false);
        let p = partition(&flows, &a, false);
        let agg = |f: &[ResolvedFlow]| group_by_labels(f, &keys());
        let mut rows = agg(&p.kept);
        annotate_groups(&mut rows, &p.marked, &a, agg);
        let marker = |src: &str, dst: &str| {
            find(&rows, src, dst)
                .and_then(|g| g.marks.as_ref())
                .and_then(RowMarks::marker)
        };
        assert_eq!(marker("erp/prod", "erp/prod"), Some(Marker::Reviewed));
        assert_eq!(marker("erp/dev", "erp/prod"), Some(Marker::Accepted));
        assert_eq!(marker("hr/prod", "shop/prod"), Some(Marker::Revisit));
        assert_eq!(marker("shop/dev", "shop/prod"), Some(Marker::Critical));
        assert_eq!(marker("crm/prod", "crm/prod"), None);
        // Quiet needs all: erp/prod's flows by service, plus one unmarked.
        let rm = |flows: u64, by: &[(Action, u64)]| {
            let mut r = RowMarks {
                flows,
                ..RowMarks::default()
            };
            for (act, n) in by {
                r.by_action[act.index()] = *n;
            }
            r
        };
        assert_eq!(rm(3, &[(Action::Reviewed, 2)]).marker(), None);
        assert_eq!(
            rm(3, &[(Action::Reviewed, 2), (Action::AcceptBlocked, 1)]).marker(),
            Some(Marker::Reviewed)
        );
        assert_eq!(
            rm(2, &[(Action::AcceptBlocked, 1), (Action::Squelch, 1)]).marker(),
            Some(Marker::Accepted)
        );
        assert_eq!(
            rm(5, &[(Action::Important, 1), (Action::Revisit, 1)]).marker(),
            Some(Marker::Important)
        );
        let mut partly = rm(4, &[]);
        partly.hidden = 12;
        assert_eq!(partly.marker(), Some(Marker::PartlySquelched(12)));
    }

    /// Every view: the shown rows' records plus the squelched records of
    /// the view's input are its input, and the rows reported gone are
    /// exactly the rows the unmarked aggregation has and the marked one
    /// lacks.
    #[test]
    fn visible_plus_squelched_is_the_input_in_every_view() {
        let flows = rich_flows();
        let s = set();
        let a = s.active(None, now(), false);
        let p = partition(&flows, &a, false);
        let k = keys();
        let groups = |rows: &[FlowGroup]| -> u64 { rows.iter().map(|g| g.flows).sum() };
        for decisions in [Decisions::All, Decisions::Blocked] {
            let input = flows.iter().filter(|f| decisions.keeps(f)).count() as u64;
            let sq = p.marked.squelched_where(|f| decisions.keeps(f));
            type Agg<'k> = Box<dyn Fn(&[ResolvedFlow]) -> Vec<FlowGroup> + 'k>;
            let aggs: [Agg; 2] = [
                Box::new(|f: &[ResolvedFlow]| group_by_labels(&decisions.kept(f), &k)),
                Box::new(|f: &[ResolvedFlow]| group_by_service(&decisions.kept(f), &k)),
            ];
            for agg in aggs {
                let mut rows = agg(&p.kept);
                let gone = annotate_groups(&mut rows, &p.marked, &a, &agg);
                assert_eq!(groups(&rows) + sq, input);
                assert_eq!(gone as usize, agg(&flows).len() - rows.len());
            }
            for d in [Direction::In, Direction::Out] {
                let agg = |f: &[ResolvedFlow]| app_sections(f, &k, d, decisions);
                let mut secs = agg(&p.kept);
                let gone = annotate_sections(&mut secs, &p.marked, &a, agg);
                let shown: u64 = secs.iter().map(|s| s.totals.flows).sum();
                assert_eq!(shown + sq, input, "{d:?} {decisions:?}");
                let rows = |s: &[AppSection]| s.iter().map(|s| s.rows.len()).sum::<usize>();
                assert_eq!(gone as usize, rows(&agg(&flows)) - rows(&secs));
            }
            // Both: each direction holds every record once; the squelched
            // records are counted once, not per section.
            let agg = |f: &[ResolvedFlow]| app_sections_both(f, &k, decisions);
            let mut secs = agg(&p.kept);
            annotate_sections(&mut secs, &p.marked, &a, agg);
            for d in [Direction::In, Direction::Out] {
                let shown: u64 = secs
                    .iter()
                    .filter(|s| s.direction == d)
                    .map(|s| s.totals.flows)
                    .sum();
                assert_eq!(shown + sq, input);
            }
        }
        let cols = MeshColumns {
            user: true,
            process: true,
        };
        let agg = |f: &[ResolvedFlow]| mesh(f, &k, cols);
        let mut rows = agg(&p.kept);
        let gone = annotate_mesh(&mut rows, &p.marked, &a, agg);
        assert_eq!(rows.iter().map(|r| r.flows).sum::<u64>() + 9, 30);
        assert_eq!(gone as usize, agg(&flows).len() - rows.len());
        let focus = Focus::new(vec!["shop".into(), "prod".into()], k.clone());
        for side in [Direction::In, Direction::Out] {
            let agg = |f: &[ResolvedFlow]| app_peers(f, &focus, side);
            let mut rows = agg(&p.kept);
            let gone = annotate_peers(&mut rows, &p.marked, &a, agg);
            let input: u64 = agg(&flows).iter().map(|r| r.flows).sum();
            let on_side = |f: &ResolvedFlow| {
                let ep = if side == Direction::In {
                    &f.dst
                } else {
                    &f.src
                };
                crate::traffic::group_key(ep, &k) == focus.app
            };
            assert_eq!(
                rows.iter().map(|r| r.flows).sum::<u64>() + p.marked.squelched_where(on_side),
                input,
                "{side:?}"
            );
            assert_eq!(gone as usize, agg(&flows).len() - rows.len());
        }
    }

    #[test]
    fn blocked_sections_count_accepted_records_per_row() {
        let flows = rich_flows();
        let s = set();
        let a = s.active(None, now(), false);
        let p = partition(&flows, &a, false);
        let agg = |f: &[ResolvedFlow]| app_sections(f, &keys(), Direction::In, Decisions::Blocked);
        let mut secs = agg(&p.kept);
        annotate_sections(&mut secs, &p.marked, &a, agg);
        let erp = secs.iter().find(|s| s.app_text() == "erp/prod").unwrap();
        let accepted: u64 = erp
            .rows
            .iter()
            .filter_map(|r| r.marks.as_ref())
            .map(|m| m.count(Action::AcceptBlocked))
            .sum();
        assert_eq!(accepted, 2, "erp/dev's 8443 and 1521");
        assert!(
            erp.rows
                .iter()
                .filter(|r| r.peer_text() == "erp/dev")
                .all(|r| r.marks.as_ref().and_then(RowMarks::marker) == Some(Marker::Accepted))
        );
    }

    #[test]
    fn row_marks_flatten_into_the_row_json() {
        let flows = rich_flows();
        let s = set();
        let a = s.active(None, now(), false);
        let p = partition(&flows, &a, false);
        let agg = |f: &[ResolvedFlow]| group_by_labels(f, &keys());
        let mut rows = agg(&p.kept);
        annotate_groups(&mut rows, &p.marked, &a, agg);
        let corp = serde_json::to_value(find(&rows, "ipl:Corp/-", "shop/prod").unwrap()).unwrap();
        assert_eq!(
            corp["mark"],
            serde_json::json!({"action": "critical", "id": "m-crit", "flows": 1})
        );
        assert_eq!(corp["marks"], serde_json::json!({"critical": 1}));
        assert_eq!(corp["squelched"], 1);
        let plain = serde_json::to_value(find(&rows, "crm/prod", "crm/prod").unwrap()).unwrap();
        assert!(plain.get("mark").is_none() && plain.get("marks").is_none());
        let dev = serde_json::to_value(find(&rows, "shop/dev", "shop/dev").unwrap()).unwrap();
        assert!(dev.get("mark").is_none(), "partly squelched: no action");
        assert_eq!(dev["squelched"], 1);
    }
}
