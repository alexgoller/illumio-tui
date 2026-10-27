//! The Objects screen's state: labels, label groups, services and IP
//! lists, their rows and columns, and the label usage.
use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObjectKind {
    Labels,
    LabelGroups,
    Services,
    /// The IP lists tab: its table, detail and load live in `IpListsState`
    /// (`ObjectsState::ip_lists`) and go through `Cmd::LoadIpLists`.
    IpLists,
}

impl ObjectKind {
    /// The tabs, in order.
    pub const ALL: [ObjectKind; 4] = [
        ObjectKind::Labels,
        ObjectKind::LabelGroups,
        ObjectKind::Services,
        ObjectKind::IpLists,
    ];

    /// The kinds `Cmd::LoadObjects` reads into `ObjectsState`'s own tables;
    /// `ObjectsState::{ages, loaded_at, loading}` are indexed by these.
    pub const FETCHED: [ObjectKind; 3] = [
        ObjectKind::Labels,
        ObjectKind::LabelGroups,
        ObjectKind::Services,
    ];

    /// Position in `ALL` (the tab strip). For the `FETCHED` kinds it is also
    /// the slot in `ObjectsState`'s arrays; IP lists have no slot there.
    pub fn index(self) -> usize {
        Self::ALL.iter().position(|k| *k == self).unwrap_or(0)
    }

    pub fn next(self) -> Self {
        Self::ALL[(self.index() + 1) % Self::ALL.len()]
    }

    pub fn prev(self) -> Self {
        Self::ALL[(self.index() + Self::ALL.len() - 1) % Self::ALL.len()]
    }

    pub fn title(self) -> &'static str {
        match self {
            ObjectKind::Labels => "Labels",
            ObjectKind::LabelGroups => "Label groups",
            ObjectKind::Services => "Services",
            ObjectKind::IpLists => "IP lists",
        }
    }
}

/// A Labels-tab row: the label, its usage once the workloads, the policy
/// and the label groups are loaded (`None` before), and whether the `f`
/// key filter keeps it. Built by `ObjectsState::rebuild_labels`, so the
/// column `fn` pointers and the matcher stay pure.
#[derive(Debug, Clone)]
pub struct LabelRow {
    pub label: Label,
    pub usage: Option<pce::LabelUsage>,
    /// What holds the label, as the PCE says (`u` asks); `None` before
    /// that or from a PCE without usage flags.
    pub held_by: Option<pce::models::LabelUsageFlags>,
    pub in_keys: bool,
}

impl LabelRow {
    /// Whether nothing holds the label: by the PCE's flags (and our counts
    /// when loaded) when `by_pce`, else by our counts alone. Unknown is used.
    pub fn is_unused(&self, by_pce: bool) -> bool {
        let counted = |u: pce::LabelUsage| !u.is_used();
        if by_pce {
            self.held_by
                .as_ref()
                .is_some_and(|f| f.decided() && !f.is_used())
                && self.usage.is_none_or(counted)
        } else {
            self.usage.is_some_and(counted)
        }
    }

    /// The PCE's kinds holding the label, comma-joined; `None` before they
    /// are known.
    pub fn held_by_kinds(&self) -> Option<String> {
        self.held_by.as_ref().map(|f| f.kinds().join(","))
    }

    /// The `held by` cell: `held_by_kinds`, `-` before they are known.
    pub fn held_by_text(&self) -> String {
        self.held_by_kinds().unwrap_or_else(|| "-".into())
    }
}

impl std::ops::Deref for LabelRow {
    type Target = Label;

    fn deref(&self) -> &Label {
        &self.label
    }
}

/// A usage count cell: the number, or `-` before the data is loaded.
pub(super) fn usage_cell(r: &LabelRow, n: fn(&pce::LabelUsage) -> usize) -> String {
    r.usage.map_or_else(|| "-".into(), |u| n(&u).to_string())
}

/// `usage_cell` for `:export`: empty before the data is loaded.
pub(super) fn usage_full(r: &LabelRow, n: fn(&pce::LabelUsage) -> usize) -> String {
    r.usage.map(|u| n(&u).to_string()).unwrap_or_default()
}

/// Its sort key: rows without usage sort last.
pub(super) fn usage_key(r: &LabelRow, n: fn(&pce::LabelUsage) -> usize) -> SortKey {
    SortKey::Num(r.usage.map_or(u64::MAX, |u| n(&u) as u64))
}

/// The Labels-tab search (`needle` is lowercased): `=text` matches values,
/// `key=text` the values of that key (even outside the `f` filter),
/// anything else the key, value or href. Rows the `f` filter drops are
/// shown only by a `key=text` search.
pub fn label_matches(r: &LabelRow, needle: &str) -> bool {
    let value_has = |v: &str| r.value.to_lowercase().contains(v);
    match needle.split_once('=') {
        Some((key, v)) if !key.trim().is_empty() => {
            r.key.to_lowercase() == key.trim() && value_has(v)
        }
        Some((_, v)) => r.in_keys && value_has(v),
        None => {
            r.in_keys
                && (needle.is_empty()
                    || r.key.to_lowercase().contains(needle)
                    || value_has(needle)
                    || r.href.to_lowercase().contains(needle))
        }
    }
}

pub fn label_columns() -> Vec<Column<LabelRow>> {
    vec![
        Column {
            title: "key",
            width: 16,
            cell: |l| l.key.clone(),
            key: |l| SortKey::Text(l.key.clone()),
            right: false,
            hide_below: 0,
            narrow: None,
            narrow_width: 0,
            full_title: None,
            full: None,
        },
        Column {
            title: "value",
            width: 0,
            cell: |l| l.value.clone(),
            key: |l| SortKey::Text(l.value.clone()),
            right: false,
            hide_below: 0,
            narrow: None,
            narrow_width: 0,
            full_title: None,
            full: None,
        },
        Column {
            title: "workloads",
            width: 9,
            cell: |l| usage_cell(l, |u| u.workloads),
            key: |l| usage_key(l, |u| u.workloads),
            right: false,
            hide_below: 0,
            narrow: None,
            narrow_width: 0,
            full_title: None,
            full: Some(|l| usage_full(l, |u| u.workloads)),
        },
        Column {
            title: "rules",
            // One spare, left-aligned cell: the last column below 90
            // columns, and an overlay box leaves the table's last cell
            // visible beside it.
            width: 6,
            cell: |l| usage_cell(l, |u| u.rules),
            key: |l| usage_key(l, |u| u.rules),
            right: false,
            hide_below: 0,
            narrow: None,
            narrow_width: 0,
            full_title: None,
            full: Some(|l| usage_full(l, |u| u.rules)),
        },
        Column {
            title: "held by",
            width: 28,
            cell: |l| l.held_by_text(),
            key: |l| SortKey::Text(l.held_by_text()),
            right: false,
            // Wide terminals only: the counts and the value come first.
            hide_below: 120,
            narrow: None,
            narrow_width: 0,
            full_title: None,
            full: Some(|l| l.held_by_kinds().unwrap_or_default()),
        },
        Column {
            title: "deleted",
            width: 7,
            cell: |l| {
                if l.is_deleted() {
                    "yes".into()
                } else {
                    String::new()
                }
            },
            key: |l| SortKey::Num(u64::from(l.is_deleted())),
            right: false,
            // Deleted rows are dimmed anyway; at 80 columns the counts need the room.
            hide_below: 90,
            narrow: None,
            narrow_width: 0,
            full_title: None,
            full: Some(|l| l.is_deleted().to_string()),
        },
        Column {
            title: "href",
            width: 24,
            cell: |l| l.href.clone(),
            key: |l| SortKey::Text(l.href.clone()),
            right: false,
            // Below 100 columns the usage counts keep the value readable.
            hide_below: 100,
            narrow: None,
            narrow_width: 0,
            full_title: None,
            full: None,
        },
    ]
}

pub fn group_columns_objects() -> Vec<Column<LabelGroup>> {
    vec![
        Column {
            title: "name",
            width: 0,
            cell: |g| g.name.clone(),
            key: |g| SortKey::Text(g.name.clone()),
            right: false,
            hide_below: 0,
            narrow: None,
            narrow_width: 0,
            full_title: None,
            full: None,
        },
        Column {
            title: "key",
            width: 12,
            cell: |g| g.key.clone(),
            key: |g| SortKey::Text(g.key.clone()),
            right: false,
            hide_below: 0,
            narrow: None,
            narrow_width: 0,
            full_title: None,
            full: None,
        },
        Column {
            title: "labels",
            width: 6,
            cell: |g| g.labels.len().to_string(),
            key: |g| SortKey::Num(g.labels.len() as u64),
            right: true,
            hide_below: 0,
            narrow: None,
            narrow_width: 0,
            full_title: None,
            full: Some(|g| join_list(g.labels.iter().map(|r| &r.href))),
        },
        Column {
            title: "groups",
            width: 6,
            cell: |g| g.sub_groups.len().to_string(),
            key: |g| SortKey::Num(g.sub_groups.len() as u64),
            right: true,
            hide_below: 0,
            narrow: None,
            narrow_width: 0,
            full_title: None,
            full: Some(|g| join_list(g.sub_groups.iter().map(|r| &r.href))),
        },
        Column {
            title: "pending",
            width: 7,
            cell: |g| g.update_type.clone().unwrap_or_default(),
            key: |g| SortKey::Text(g.update_type.clone().unwrap_or_default()),
            right: false,
            hide_below: 70,
            narrow: None,
            narrow_width: 0,
            full_title: None,
            full: None,
        },
    ]
}

pub fn service_columns() -> Vec<Column<Service>> {
    vec![
        Column {
            title: "name",
            width: 28,
            cell: |s| s.name.clone(),
            key: |s| SortKey::Text(s.name.clone()),
            right: false,
            hide_below: 0,
            narrow: None,
            narrow_width: 24,
            full_title: None,
            full: None,
        },
        Column {
            title: "ports",
            width: 0,
            cell: |s| s.summary(),
            key: |s| SortKey::Num(s.service_ports.len() as u64),
            right: false,
            hide_below: 0,
            narrow: None,
            narrow_width: 0,
            full_title: None,
            full: Some(|s| join_list(s.service_ports.iter().map(|p| p.display()))),
        },
        Column {
            title: "pending",
            width: 7,
            cell: |s| s.update_type.clone().unwrap_or_default(),
            key: |s| SortKey::Text(s.update_type.clone().unwrap_or_default()),
            right: false,
            hide_below: 70,
            narrow: None,
            narrow_width: 0,
            full_title: None,
            full: None,
        },
    ]
}

/// What the PCE said about label usage (`u` on the Labels tab).
#[derive(Debug, Clone, Default, PartialEq)]
pub enum PceUsage {
    /// Not asked yet (or the ask failed).
    #[default]
    Unknown,
    /// Its flags by label href: they decide what is unused.
    Flags(HashMap<String, pce::models::LabelUsageFlags>),
    /// It answered without usage flags (an older PCE): our counts from
    /// workloads, rules, scopes and label groups decide.
    Unsupported,
}

impl PceUsage {
    /// From a `usage=true` answer: flags when every label has them.
    pub fn from_labels(labels: Vec<Label>) -> Self {
        if !labels.iter().all(|l| l.usage_flags().is_some()) {
            return Self::Unsupported;
        }
        Self::Flags(
            labels
                .into_iter()
                .filter_map(|l| l.usage.map(|u| (l.href, u)))
                .collect(),
        )
    }
}

#[derive(Debug)]
pub struct ObjectsState {
    pub kind: ObjectKind,
    /// The Labels tab: a projection of `all_labels` through the filters below.
    pub labels: TableView<LabelRow>,
    /// The labels as fetched.
    pub all_labels: Vec<Label>,
    /// `f`: the keys the Labels tab shows (empty: every key). Kept across
    /// `r`; a profile switch starts over.
    pub label_keys: Vec<String>,
    /// `i`: labels whose key the profile ignores are left out (on by
    /// default when it has `ignore_keys`).
    pub hide_ignored: bool,
    /// How many labels `hide_ignored` leaves out.
    pub ignored_hidden: usize,
    /// `u`: only labels nothing holds; applies once the PCE's usage flags
    /// arrive, or, from a PCE without them, once `label_usage` exists.
    pub unused_only: bool,
    /// Usage by label href, from the cached workloads, policy and label
    /// groups (`Model::recount_labels`); `None` until all three are loaded.
    pub label_usage: Option<HashMap<String, pce::LabelUsage>>,
    /// The workload list behind `label_usage` was cut short: a label it
    /// counts as unused may be carried by a workload not read.
    pub usage_truncated: bool,
    /// The PCE's own usage flags (`GET /labels?usage=true`), asked for by `u`.
    pub pce_usage: PceUsage,
    /// The `u` request is on its way.
    pub pce_usage_loading: bool,
    pub groups: TableView<LabelGroup>,
    pub services: TableView<Service>,
    /// The IP lists tab.
    pub ip_lists: IpListsState,
    /// Cache age in seconds when each kind (indexed by `ObjectKind::index`,
    /// `FETCHED` kinds only) was loaded.
    pub ages: [Option<u64>; 3],
    /// When each kind arrived, so the shown age keeps growing.
    pub loaded_at: [Option<Instant>; 3],
    pub loading: [bool; 3],
    pub show_detail: bool,
    /// The startup index load failed; `r` retries it.
    pub index_failed: bool,
    pub auto: AutoRefresh,
    /// `W` / `:used`: the open where-used overlay (`Overlay::WhereUsed`).
    pub where_used: Option<crate::tui::where_used::WhereUsedView>,
    /// Workload counts of labels from the one-row probe, by label href:
    /// a second `W` on the label sends none. Dropped when the workload list
    /// arrives (it counts instead) and by a write.
    pub workload_probes: HashMap<String, pce::where_used::WorkloadCount>,
    /// Probes on their way, by label href.
    pub probes_loading: HashSet<String>,
    /// The pairing profiles, read once by the first `W` on a label: no
    /// reload by a second `W`, a reopen or a timer.
    pub pairing_profiles: Option<Vec<pce::models::PairingProfile>>,
    pub pairing_loading: bool,
    /// Why the last read of them failed, for the overlay (until W asks again).
    pub pairing_error: Option<String>,
    /// `:hygiene`: the report (`Overlay::Hygiene`); kept while `W` checks
    /// one of its rows, so `Esc` there comes back to it.
    pub hygiene: Option<crate::tui::hygiene::HygieneView>,
    /// `Enter` on a `:hygiene` row whose tab is still loading: the tab and
    /// the href selected once it arrives.
    pub select_on_load: Option<(ObjectKind, String)>,
}

impl ObjectsState {
    pub fn new(min_refresh: u64) -> Self {
        Self {
            kind: ObjectKind::Labels,
            labels: TableView::new(label_columns()).with_matcher(label_matches),
            all_labels: Vec::new(),
            label_keys: Vec::new(),
            hide_ignored: false,
            ignored_hidden: 0,
            unused_only: false,
            label_usage: None,
            usage_truncated: false,
            pce_usage: PceUsage::Unknown,
            pce_usage_loading: false,
            groups: TableView::new(group_columns_objects()),
            services: TableView::new(service_columns()),
            ip_lists: IpListsState::new(min_refresh),
            ages: [None; 3],
            loaded_at: [None; 3],
            loading: [false; 3],
            show_detail: false,
            index_failed: false,
            auto: AutoRefresh::new(min_refresh),
            where_used: None,
            workload_probes: HashMap::new(),
            probes_loading: HashSet::new(),
            pairing_profiles: None,
            pairing_loading: false,
            pairing_error: None,
            hygiene: None,
            select_on_load: None,
        }
    }

    /// The fetched labels; the tab's rows follow.
    pub fn set_labels(&mut self, labels: Vec<Label>, order: &KeyOrder) {
        self.all_labels = labels;
        self.rebuild_labels(order);
    }

    /// The Labels tab's rows again from `all_labels`, the filters and the
    /// stored usage (`order` says which keys are ignored). No request.
    pub fn rebuild_labels(&mut self, order: &KeyOrder) {
        let usage = self.label_usage.as_ref();
        let flags = match &self.pce_usage {
            PceUsage::Flags(m) => Some(m),
            _ => None,
        };
        let unused_only = self.unused_only && self.unused_decided();
        let mut hidden = 0;
        let rows: Vec<LabelRow> = self
            .all_labels
            .iter()
            .filter(|l| {
                let hide = self.hide_ignored && !order.shows(&l.key);
                hidden += usize::from(hide);
                !hide
            })
            .map(|l| LabelRow {
                usage: usage.map(|u| u.get(&l.href).copied().unwrap_or_default()),
                held_by: flags.and_then(|m| m.get(&l.href).cloned()),
                in_keys: self.label_keys.is_empty() || self.label_keys.contains(&l.key),
                label: l.clone(),
            })
            .filter(|r| !unused_only || r.is_unused(flags.is_some()))
            .collect();
        self.ignored_hidden = hidden;
        self.labels.set_rows(rows);
    }

    /// Whether `u` can tell used from unused: the PCE's flags are in, or
    /// the PCE has none and our own counts are.
    pub fn unused_decided(&self) -> bool {
        match self.pce_usage {
            PceUsage::Flags(_) => true,
            PceUsage::Unsupported => self.label_usage.is_some(),
            PceUsage::Unknown => false,
        }
    }

    /// Whether any Labels-tab filter (`f`, `i`, `u`) leaves rows out.
    pub fn labels_filtered(&self) -> bool {
        !self.label_keys.is_empty() || self.ignored_hidden > 0 || self.unused_only
    }

    /// Whether the three `FETCHED` kinds have arrived.
    pub fn loaded(&self) -> bool {
        self.ages.iter().all(Option::is_some)
    }

    /// Whether the current kind's table is capturing keys for a search.
    pub fn is_searching(&self) -> bool {
        match self.kind {
            ObjectKind::Labels => self.labels.is_searching(),
            ObjectKind::LabelGroups => self.groups.is_searching(),
            ObjectKind::Services => self.services.is_searching(),
            ObjectKind::IpLists => self.ip_lists.table.is_searching(),
        }
    }

    /// Whether the current kind has a selected row.
    pub fn has_selection(&self) -> bool {
        match self.kind {
            ObjectKind::Labels => self.labels.selected_item().is_some(),
            ObjectKind::LabelGroups => self.groups.selected_item().is_some(),
            ObjectKind::Services => self.services.selected_item().is_some(),
            ObjectKind::IpLists => self.ip_lists.table.selected_item().is_some(),
        }
    }

    /// Age of the current kind in seconds, growing since it was loaded.
    pub fn age_secs(&self, kind: ObjectKind) -> Option<u64> {
        if kind == ObjectKind::IpLists {
            let s = &self.ip_lists;
            return s.loaded_at.map(|t| s.age_secs + t.elapsed().as_secs());
        }
        let i = kind.index();
        self.ages[i].map(|a| a + self.loaded_at[i].map_or(0, |t| t.elapsed().as_secs()))
    }
}

/// An IP list with what its screen derives from it once per load.
#[derive(Debug, Clone)]
pub struct IpListRow {
    pub list: IpList,
    pub size: IpListSize,
    /// Rules of the loaded draft rulesets that name the list; `None` until
    /// the Policy screen loaded them.
    pub used_by: Option<usize>,
}

pub fn ip_list_columns() -> Vec<Column<IpListRow>> {
    vec![
        Column {
            title: "name",
            width: 28,
            cell: |r| r.list.name.clone(),
            key: |r| SortKey::Text(r.list.name.clone()),
            right: false,
            hide_below: 0,
            narrow: None,
            narrow_width: 24,
            full_title: None,
            full: None,
        },
        Column {
            title: "size",
            width: 10,
            // `any (0.0.0.0/0)` does not fit; the detail shows the reason.
            cell: |r| {
                if r.size.unbounded.is_some() {
                    "any".to_string()
                } else {
                    r.size.display()
                }
            },
            key: |r| SortKey::Num(r.size.sort_key()),
            right: true,
            hide_below: 0,
            narrow: None,
            narrow_width: 0,
            full_title: None,
            full: Some(|r| {
                if r.size.unbounded.is_some() {
                    r.size.display()
                } else {
                    r.size.total().to_string()
                }
            }),
        },
        Column {
            title: "ranges",
            width: 6,
            cell: |r| r.list.ip_ranges.len().to_string(),
            key: |r| SortKey::Num(r.list.ip_ranges.len() as u64),
            right: true,
            hide_below: 0,
            narrow: None,
            narrow_width: 0,
            full_title: None,
            full: Some(|r| join_list(r.list.entries().iter().take(r.list.ip_ranges.len()))),
        },
        Column {
            title: "fqdns",
            width: 5,
            cell: |r| r.list.fqdns.len().to_string(),
            key: |r| SortKey::Num(r.list.fqdns.len() as u64),
            right: true,
            hide_below: 0,
            narrow: None,
            narrow_width: 0,
            full_title: None,
            full: Some(|r| join_list(r.list.fqdns.iter().map(|f| &f.fqdn))),
        },
        Column {
            title: "used by",
            width: 7,
            cell: |r| r.used_by.map_or_else(|| "-".to_string(), |n| n.to_string()),
            key: |r| SortKey::Num(r.used_by.map_or(0, |n| n as u64 + 1)),
            right: true,
            hide_below: 0,
            narrow: None,
            narrow_width: 0,
            full_title: None,
            full: Some(|r| r.used_by.map(|n| n.to_string()).unwrap_or_default()),
        },
        Column {
            title: "pending",
            width: 7,
            cell: |r| r.list.update_type.clone().unwrap_or_default(),
            key: |r| SortKey::Text(r.list.update_type.clone().unwrap_or_default()),
            right: false,
            hide_below: 70,
            narrow: None,
            narrow_width: 0,
            full_title: None,
            full: None,
        },
    ]
}

#[derive(Debug)]
pub struct IpListsState {
    pub table: TableView<IpListRow>,
    /// Cache age in seconds when the lists arrived.
    pub age_secs: u64,
    pub loaded_at: Option<Instant>,
    pub loading: bool,
    pub show_detail: bool,
    pub auto: AutoRefresh,
}

impl IpListsState {
    pub fn new(min_refresh: u64) -> Self {
        Self {
            table: TableView::new(ip_list_columns()),
            age_secs: 0,
            loaded_at: None,
            loading: false,
            show_detail: false,
            auto: AutoRefresh::new(min_refresh),
        }
    }

    /// The rows for `lists`: sizes computed, `used_by` from the loaded rulesets.
    pub fn set_lists(&mut self, lists: Vec<IpList>, policy: &PolicyState) {
        self.table.set_rows(
            lists
                .into_iter()
                .map(|list| IpListRow {
                    size: ip_list_size(&list),
                    used_by: policy.ip_list_uses(&list.href).map(|u| u.len()),
                    list,
                })
                .collect(),
        );
    }

    /// `used_by` again from `policy` (the rulesets changed); no request.
    pub fn recount(&mut self, policy: &PolicyState) {
        let rows = self
            .table
            .rows()
            .iter()
            .cloned()
            .map(|mut r| {
                r.used_by = policy.ip_list_uses(&r.list.href).map(|u| u.len());
                r
            })
            .collect();
        self.table.set_rows(rows);
    }
}
