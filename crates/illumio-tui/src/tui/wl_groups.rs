//! The Workloads screen by label group (`x`): one section per group of the
//! traffic group keys (`g`, `app,env` by default), its header counting the
//! workloads, their VEN health and enforcement, the workloads under it
//! unless folded. Built from the flat table's rows as shown (filters,
//! label filter, search and sort applied), so the order within a section
//! is the flat table's sort. Pure and local: never a request.
use crate::tui::model::{DELTA_COLUMN, TRANSITION_COLUMN, WlRow, wl_transition, workload_columns};
use crate::tui::table::{Column, SortKey};
use pce::label_filter::{group_title, value_of};
use std::collections::{BTreeSet, HashMap};
use std::sync::{Arc, LazyLock};

/// `s` while grouped: the order of the sections.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum GroupOrder {
    /// Most workloads first.
    #[default]
    Size,
    /// By the group's name, A→Z, a missing label last.
    Name,
    /// Most warning, error and offline VENs first.
    Unhealthy,
}

impl GroupOrder {
    /// size ↓ → name A→Z → unhealthy ↓ → size ↓.
    pub fn next(self) -> Self {
        match self {
            GroupOrder::Size => GroupOrder::Name,
            GroupOrder::Name => GroupOrder::Unhealthy,
            GroupOrder::Unhealthy => GroupOrder::Size,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            GroupOrder::Size => "by size ↓",
            GroupOrder::Name => "by name A→Z",
            GroupOrder::Unhealthy => "by unhealthy ↓",
        }
    }
}

/// The enforcement modes a header counts (unmanaged is a health state),
/// with their short names.
pub const ENFORCEMENT: [(&str, &str); 4] = [
    ("full", "full"),
    ("selective", "sel"),
    ("visibility_only", "vis"),
    ("idle", "idle"),
];

/// One group: its value per key (`None`: no label), its counts and its
/// workloads in the flat table's order.
#[derive(Debug)]
pub struct WlGroup {
    pub key: Vec<Option<String>>,
    /// The keys `key` is the value of (the grouping's).
    pub keys: Vec<String>,
    /// `shop | prod`, `(no app) | prod`.
    pub title: String,
    /// Workloads per health in rank order: healthy, unmanaged, warning,
    /// offline, error ([`VenHealth::rank`]).
    pub health: [usize; 5],
    /// Workloads per [`ENFORCEMENT`] mode.
    pub enforcement: [usize; 4],
    /// In compare mode: its rows per change against the snapshot.
    pub changes: pce::traffic::CompareCounts,
    pub members: Vec<WlRow>,
}

impl WlGroup {
    /// The workloads there now (gone rows, in compare mode, not counted).
    pub fn count(&self) -> usize {
        self.members.len() - self.changes.gone as usize
    }

    /// Warning, offline and error VENs.
    pub fn unhealthy(&self) -> usize {
        self.health[2] + self.health[3] + self.health[4]
    }
}

/// A row of the grouped table: a group's header, or a workload under it.
#[derive(Debug, Clone)]
pub enum WlGroupRow {
    Header { group: Arc<WlGroup>, folded: bool },
    Member { row: WlRow, group: Arc<WlGroup> },
}

impl WlGroupRow {
    pub fn member(&self) -> Option<&WlRow> {
        match self {
            WlGroupRow::Member { row, .. } => Some(row),
            WlGroupRow::Header { .. } => None,
        }
    }

    pub fn is_header(&self) -> bool {
        matches!(self, WlGroupRow::Header { .. })
    }

    /// The row's group (a member's, or the header's own).
    pub fn group(&self) -> &Arc<WlGroup> {
        match self {
            WlGroupRow::Header { group, .. } | WlGroupRow::Member { group, .. } => group,
        }
    }
}

/// The groups of `rows` under `keys`, in `order`. One pass, keys compared
/// borrowed: a workload costs a lookup, not a copy.
pub fn build_groups<'a>(
    rows: impl IntoIterator<Item = &'a WlRow>,
    keys: &[String],
    order: GroupOrder,
) -> Vec<Arc<WlGroup>> {
    let mut at: HashMap<Vec<Option<&'a str>>, usize> = HashMap::new();
    let mut groups: Vec<WlGroup> = Vec::new();
    for r in rows {
        let key: Vec<Option<&'a str>> = keys.iter().map(|k| value_of(&r.workload, k)).collect();
        let i = match at.get(&key) {
            Some(&i) => i,
            None => {
                let owned: Vec<Option<String>> =
                    key.iter().map(|v| v.map(str::to_string)).collect();
                at.insert(key, groups.len());
                groups.push(WlGroup {
                    title: group_title(&owned, keys),
                    keys: keys.to_vec(),
                    key: owned,
                    health: [0; 5],
                    enforcement: [0; 4],
                    changes: Default::default(),
                    members: Vec::new(),
                });
                groups.len() - 1
            }
        };
        let g = &mut groups[i];
        if let Some(c) = &r.change {
            g.changes.add(c.change);
            // Gone (compare mode): listed, counted apart, never in the
            // health or enforcement of what is there now.
            if c.change == pce::traffic::Change::Gone {
                g.members.push(r.clone());
                continue;
            }
        }
        g.health[usize::from(r.health.rank())] += 1;
        if let Some(e) = r.workload.enforcement()
            && let Some(j) = ENFORCEMENT.iter().position(|(m, _)| *m == e)
        {
            g.enforcement[j] += 1;
        }
        g.members.push(r.clone());
    }
    let mut groups: Vec<Arc<WlGroup>> = groups.into_iter().map(Arc::new).collect();
    order_groups(&mut groups, order);
    groups
}

/// `groups` in `order`; ties by name.
pub fn order_groups(groups: &mut [Arc<WlGroup>], order: GroupOrder) {
    fn name(g: &WlGroup) -> Vec<(bool, String)> {
        g.key
            .iter()
            .map(|v| (v.is_none(), v.as_deref().unwrap_or("").to_lowercase()))
            .collect()
    }
    groups.sort_by(|a, b| {
        let by = match order {
            GroupOrder::Size => b.count().cmp(&a.count()),
            GroupOrder::Name => std::cmp::Ordering::Equal,
            GroupOrder::Unhealthy => b
                .unhealthy()
                .cmp(&a.unhealthy())
                .then_with(|| b.count().cmp(&a.count())),
        };
        by.then_with(|| name(a).cmp(&name(b)))
            .then_with(|| a.key.cmp(&b.key))
    });
}

/// The table rows: each group's header, then its workloads unless folded.
pub fn layout(groups: &[Arc<WlGroup>], folded: &BTreeSet<Vec<Option<String>>>) -> Vec<WlGroupRow> {
    let mut out =
        Vec::with_capacity(groups.len() + groups.iter().map(|g| g.count()).sum::<usize>());
    for g in groups {
        let is_folded = folded.contains(&g.key);
        out.push(WlGroupRow::Header {
            group: g.clone(),
            folded: is_folded,
        });
        if !is_folded {
            out.extend(g.members.iter().map(|r| WlGroupRow::Member {
                row: r.clone(),
                group: g.clone(),
            }));
        }
    }
    out
}

/// The flat table's cells, for a member row; a header has none (it spans
/// the table).
static WL_COLUMNS: LazyLock<Vec<Column<WlRow>>> = LazyLock::new(workload_columns);

fn member_cell<const I: usize>(r: &WlGroupRow) -> String {
    r.member()
        .map(|w| (WL_COLUMNS[I].cell)(w))
        .unwrap_or_default()
}

fn member_key<const I: usize>(r: &WlGroupRow) -> SortKey {
    r.member()
        .map_or(SortKey::Num(0), |w| (WL_COLUMNS[I].key)(w))
}

fn member_narrow<const I: usize>(r: &WlGroupRow) -> String {
    r.member()
        .and_then(|w| WL_COLUMNS[I].narrow.map(|n| n(w)))
        .unwrap_or_default()
}

fn member_full<const I: usize>(r: &WlGroupRow) -> String {
    r.member()
        .and_then(|w| WL_COLUMNS[I].full.map(|n| n(w)))
        .unwrap_or_default()
}

type Cell = fn(&WlGroupRow) -> String;
type Key = fn(&WlGroupRow) -> SortKey;

macro_rules! adapters {
    ($($i:literal)*) => {
        const CELLS: &[Cell] = &[$(member_cell::<$i>),*];
        const KEYS: &[Key] = &[$(member_key::<$i>),*];
        const NARROW: &[Cell] = &[$(member_narrow::<$i>),*];
        const FULL: &[Cell] = &[$(member_full::<$i>),*];
    };
}
adapters!(0 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15);

/// A member's labels without its group's own (the header names them):
/// `app=shop env=prod role=web` under `shop | prod` is `role=web`.
fn member_labels(r: &WlGroupRow) -> String {
    let WlGroupRow::Member { row, group } = r else {
        return String::new();
    };
    let mut text = format!(" {} ", row.labels);
    for (k, v) in group.keys.iter().zip(&group.key) {
        if let Some(v) = v {
            text = text.replacen(&format!(" {k}={v} "), " ", 1);
        }
    }
    text.trim().to_string()
}

/// The grouped table's columns: `src` (the flat table's, as shown at the
/// moment: widths, which hide) reading each member row's workload; the
/// labels column leaves out the group's keys. A column past the adapters
/// is left out (a test keeps them in step: `member_columns_read_the_member_cells`).
pub fn member_columns(src: &[Column<WlRow>]) -> Vec<Column<WlGroupRow>> {
    src.iter()
        .filter_map(|c| {
            // Compare mode's columns (first) read the member's change.
            let compare = |cell: Cell, key: Key, full: Option<Cell>| Column {
                title: c.title,
                width: c.width,
                cell,
                key,
                right: c.right,
                hide_below: c.hide_below,
                narrow: None,
                narrow_width: c.narrow_width,
                full,
                full_title: c.full_title,
            };
            match c.title {
                DELTA_COLUMN => {
                    return Some(compare(
                        member_delta,
                        |r| SortKey::Text(member_delta_full(r)),
                        Some(member_delta_full),
                    ));
                }
                TRANSITION_COLUMN => {
                    return Some(compare(
                        member_transition,
                        |r| SortKey::Text(member_transition(r)),
                        None,
                    ));
                }
                _ => {}
            }
            let i = WL_COLUMNS
                .iter()
                .position(|w| w.title == c.title)
                .filter(|&i| i < CELLS.len())?;
            Some(Column {
                title: c.title,
                width: c.width,
                cell: if c.title == "labels" {
                    member_labels
                } else {
                    CELLS[i]
                },
                key: KEYS[i],
                right: c.right,
                hide_below: c.hide_below,
                narrow: c.narrow.map(|_| NARROW[i]),
                narrow_width: c.narrow_width,
                full: c.full.map(|_| FULL[i]),
                full_title: c.full_title,
            })
        })
        .collect()
}

fn member_delta(r: &WlGroupRow) -> String {
    r.member()
        .and_then(|w| w.change.as_ref())
        .map(|c| c.change.tag().to_string())
        .unwrap_or_default()
}

fn member_delta_full(r: &WlGroupRow) -> String {
    r.member()
        .and_then(|w| w.change.as_ref())
        .map(|c| c.change.as_str().to_string())
        .unwrap_or_default()
}

fn member_transition(r: &WlGroupRow) -> String {
    r.member().map(wl_transition).unwrap_or_default()
}

/// A header's VEN health in the order the bar draws it: ok, warning,
/// error, offline, unmanaged.
pub fn bar_counts(g: &WlGroup) -> [usize; 5] {
    let [healthy, unmanaged, warning, offline, error] = g.health;
    [healthy, warning, error, offline, unmanaged]
}

/// `10 ok · 1 offline · 1 unmanaged`: the states with workloads.
pub fn health_text(g: &WlGroup) -> String {
    const NAMES: [&str; 5] = ["ok", "warning", "error", "offline", "unmanaged"];
    bar_counts(g)
        .iter()
        .zip(NAMES)
        .filter(|(n, _)| **n > 0)
        .map(|(n, name)| format!("{n} {name}"))
        .collect::<Vec<_>>()
        .join(" · ")
}

/// In compare mode, `2 new 1 chg`: the changes of the workloads there
/// now (empty when none, or not comparing); gone ones are counted apart.
pub fn changes_text(g: &WlGroup) -> String {
    let c = &g.changes;
    [(c.new, "new"), (c.changed, "chg")]
        .into_iter()
        .filter(|(n, _)| *n > 0)
        .map(|(n, w)| format!("{n} {w}"))
        .collect::<Vec<_>>()
        .join(" ")
}

/// `full 8 vis 3 idle 1`: the modes with workloads.
pub fn enforcement_text(g: &WlGroup) -> String {
    g.enforcement
        .iter()
        .zip(ENFORCEMENT)
        .filter(|(n, _)| **n > 0)
        .map(|(n, (_, short))| format!("{short} {n}"))
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::model::WorkloadRowView;
    use pce::KeyOrder;
    use pce::models::Workload;

    fn rows() -> Vec<WlRow> {
        let w: Vec<Workload> =
            serde_json::from_str(include_str!("../../../../fixtures/workloads.json")).unwrap();
        let now: jiff::Timestamp = "2026-10-10T08:00:00Z".parse().unwrap();
        w.into_iter()
            .map(|w| Arc::new(WorkloadRowView::new(w, now, &KeyOrder::default())))
            .collect()
    }

    fn keys() -> Vec<String> {
        vec!["app".into(), "env".into()]
    }

    #[test]
    fn groups_count_health_and_enforcement_and_name_missing_labels() {
        let r = rows();
        let g = build_groups(&r, &keys(), GroupOrder::Size);
        assert_eq!(g.len(), 2);
        assert_eq!(g[0].title, "shop | prod");
        assert_eq!(g[0].count(), 2);
        assert_eq!(g[1].title, "(no app) | (no env)");
        assert_eq!(g[1].key, [None, None]);
        assert_eq!(health_text(&g[0]), "1 ok · 1 offline");
        assert_eq!(enforcement_text(&g[0]), "full 1 vis 1");
        assert_eq!(
            enforcement_text(&g[1]),
            "sel 1",
            "unmanaged is a health state"
        );
        assert_eq!(health_text(&g[1]), "1 warning · 1 unmanaged");
        // Members keep the order they came in.
        assert_eq!(g[0].members[0].workload.href, "/orgs/5/workloads/aaaa-1");
    }

    #[test]
    fn orders_by_size_name_and_unhealthy() {
        let mut r = rows();
        // A third group, one workload: crm | prod, offline.
        let mut w = (*r[1]).clone();
        w.workload.href = "/w/crm".into();
        w.workload.labels[1].value = Some("crm".into());
        r.push(Arc::new(w));
        let titles = |r: &[WlRow], o| {
            build_groups(r, &keys(), o)
                .iter()
                .map(|g| g.title.clone())
                .collect::<Vec<_>>()
        };
        // Ties by name, a missing label last.
        assert_eq!(
            titles(&r, GroupOrder::Size),
            ["shop | prod", "(no app) | (no env)", "crm | prod"]
        );
        assert_eq!(
            titles(&r, GroupOrder::Name),
            ["crm | prod", "shop | prod", "(no app) | (no env)"]
        );
        // One unhealthy VEN each: then by size, then by name.
        assert_eq!(
            titles(&r, GroupOrder::Unhealthy),
            ["shop | prod", "(no app) | (no env)", "crm | prod"]
        );
        r.pop();
        let mut w = (*r[3]).clone();
        w.workload.href = "/w/second-warning".into();
        r.push(Arc::new(w));
        assert_eq!(titles(&r, GroupOrder::Unhealthy)[0], "(no app) | (no env)");
        assert_eq!(GroupOrder::Size.next().next().next(), GroupOrder::Size);
    }

    #[test]
    fn folded_groups_show_their_header_only() {
        let r = rows();
        let g = build_groups(&r, &keys(), GroupOrder::Size);
        assert_eq!(layout(&g, &BTreeSet::new()).len(), 6);
        let folded: BTreeSet<_> = [g[0].key.clone()].into();
        let rows = layout(&g, &folded);
        assert_eq!(rows.len(), 4);
        assert!(matches!(rows[0], WlGroupRow::Header { folded: true, .. }));
        assert!(rows[1].is_header());
        assert!(rows[2].member().is_some());
    }

    #[test]
    fn member_columns_read_the_member_cells() {
        let r = rows();
        let g = build_groups(&r, &keys(), GroupOrder::Size);
        let src = workload_columns();
        let cols = member_columns(&src);
        assert_eq!(cols.len(), src.len());
        let rows = layout(&g, &BTreeSet::new());
        for (i, c) in cols.iter().enumerate() {
            assert_eq!(c.title, src[i].title);
            if c.title == "labels" {
                // Under `shop | prod` only the role is left to say.
                assert_eq!((c.cell)(&rows[1]), "role=web");
            } else {
                assert_eq!((c.cell)(&rows[1]), (src[i].cell)(&r[0]), "{}", c.title);
            }
            assert_eq!((c.key)(&rows[1]), (src[i].key)(&r[0]), "{}", c.title);
            assert_eq!((c.cell)(&rows[0]), "", "a header has no cells");
            assert_eq!(c.narrow.is_some(), src[i].narrow.is_some());
            assert_eq!(c.full.is_some(), src[i].full.is_some());
        }
    }

    /// 10k workloads in 200 groups: a regrouping is one pass (well under
    /// a second even unoptimised; not a timing gate, a guard against a
    /// quadratic build).
    #[test]
    fn groups_10k_workloads_quickly() {
        let base = rows();
        let mut r: Vec<WlRow> = Vec::with_capacity(10_000);
        for i in 0..10_000 {
            let mut w = (*base[i % 2]).clone();
            w.workload.href = format!("/w/{i}");
            w.workload.labels[1].value = Some(format!("app{}", i % 200));
            r.push(Arc::new(w));
        }
        let start = std::time::Instant::now();
        let g = build_groups(&r, &keys(), GroupOrder::Unhealthy);
        let rows = layout(&g, &BTreeSet::new());
        let took = start.elapsed();
        assert_eq!(g.len(), 200);
        assert_eq!(rows.len(), 10_200);
        assert!(took < std::time::Duration::from_secs(2), "{took:?}");
    }
}
