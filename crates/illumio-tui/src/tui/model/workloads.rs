//! The Workloads screen's state: its rows and columns, the filters and
//! marks, the groups and the loaded workloads.
use super::*;

/// Whether a Workloads row is marked for a bulk change (`Space`, `V`): set
/// from `WorkloadsState::marked` (the truth, by href) on every rebuild and
/// mark change, read by the hostname cell (`●`). Atomic so the shared row
/// (flat and grouped tables hold the same `Arc`) changes without a rebuild.
#[derive(Debug, Default)]
pub struct MarkFlag(std::sync::atomic::AtomicBool);

impl MarkFlag {
    pub fn get(&self) -> bool {
        self.0.load(std::sync::atomic::Ordering::Relaxed)
    }

    pub fn set(&self, on: bool) {
        self.0.store(on, std::sync::atomic::Ordering::Relaxed);
    }
}

impl Clone for MarkFlag {
    fn clone(&self) -> Self {
        Self(std::sync::atomic::AtomicBool::new(self.get()))
    }
}

/// A workload with its derived health and display strings, computed once per
/// rebuild so the column `fn` pointers stay pure.
#[derive(Debug, Clone)]
pub struct WorkloadRowView {
    pub workload: Workload,
    pub health: VenHealth,
    pub heartbeat: String,
    /// Heartbeat age in seconds, for sorting (`None` sorts last when ascending).
    pub heartbeat_secs: Option<u64>,
    pub sync: String,
    pub ven: String,
    /// `env=prod role=web app=shop`, keys in the profile's [`KeyOrder`].
    pub labels: String,
    /// `:export`'s labels: every label (ignored keys too), `; `-joined, as
    /// `workloads --csv` prints them.
    pub label_list: String,
    /// The label model's keys (scope keys, then the tier key) this workload
    /// has no label for: non-empty means unlabeled (`u`).
    pub missing: Vec<String>,
    /// Marked for a bulk change.
    pub marked: MarkFlag,
}

impl WorkloadRowView {
    pub fn new(workload: Workload, now: jiff::Timestamp, order: &KeyOrder) -> Self {
        let health = workload.ven_health(now);
        let a = workload.agent_status_view();
        let age = workload.heartbeat_age(now);
        let heartbeat = age
            .map(crate::tui::ui::widgets::bars::age_text)
            .unwrap_or_default();
        Self {
            heartbeat,
            heartbeat_secs: age.map(|d| d.as_secs()),
            sync: a
                .as_ref()
                .and_then(|a| a.sync_state.clone())
                .unwrap_or_default(),
            ven: a.and_then(|a| a.version).unwrap_or_default(),
            labels: workload.label_text(order),
            label_list: join_list(workload.label_entries(order)),
            missing: workload.missing_label_keys(order.first()),
            marked: MarkFlag::default(),
            health,
            workload,
        }
    }
}

pub(super) type RowCell = fn(&WlRow) -> String;

pub(super) type RowKey = fn(&WlRow) -> SortKey;

pub(super) fn text_col(
    title: &'static str,
    width: u16,
    cell: RowCell,
    key: RowKey,
) -> Column<WlRow> {
    Column {
        title,
        width,
        cell,
        key,
        right: false,
        hide_below: 0,
        narrow: None,
        narrow_width: 0,
        full_title: None,
        full: None,
    }
}

pub(super) fn hostname_of(r: &WlRow) -> String {
    r.workload.hostname.clone().unwrap_or_default()
}

/// The hostname cell: `● ` first on a row marked for a bulk change.
pub(super) fn hostname_cell(r: &WlRow) -> String {
    marked_cell(r, hostname_of(r))
}

pub(super) fn marked_cell(r: &WlRow, text: String) -> String {
    if r.marked.get() {
        format!("● {text}")
    } else {
        text
    }
}

pub(super) fn name_of(r: &WlRow) -> String {
    r.workload.name.clone().unwrap_or_default()
}

pub(super) fn ip_of(r: &WlRow) -> String {
    r.workload.primary_ip().unwrap_or("").to_string()
}

/// The effective enforcement: `unmanaged` without a VEN, else the PCE's mode.
pub(super) fn enforcement_of(r: &WlRow) -> String {
    r.workload.enforcement().unwrap_or_default().to_string()
}

/// `-` without a VEN: nothing reports flows there, whatever the PCE says.
pub(super) fn visibility_of(r: &WlRow) -> String {
    if r.workload.is_unmanaged() {
        return "-".into();
    }
    r.workload.visibility_level.clone().unwrap_or_default()
}

pub(super) fn online_of(r: &WlRow) -> String {
    match r.workload.online {
        Some(true) => "yes",
        Some(false) => "no",
        None => "-",
    }
    .to_string()
}

pub(super) fn agent_of(r: &WlRow) -> String {
    r.workload.agent_status().unwrap_or_else(|| "-".into())
}

/// Index of the `health` column in `workload_columns()`.
pub(super) const HEALTH_COL: usize = 8;

/// Index of the `heartbeat` column in `workload_columns()`.
pub(super) const HEARTBEAT_COL: usize = 9;

/// Index of the `missing` column (last) in `workload_columns()`.
pub(super) const MISSING_COL: usize = 12;

/// The columns, with `missing` shown at every width while `u` (unlabeled)
/// is on, the heartbeat giving it room below 100 columns; otherwise only
/// on very wide terminals.
pub(super) fn workload_columns_for(unlabeled: bool) -> Vec<Column<WlRow>> {
    let mut cols = workload_columns();
    if unlabeled {
        cols[MISSING_COL].hide_below = 0;
        cols[HEARTBEAT_COL].hide_below = 100;
    }
    cols
}

pub fn workload_columns() -> Vec<Column<WlRow>> {
    // Below 120 columns only hostname, ip, enforcement, heartbeat and health
    // stay, so the table is readable at 80.
    let wide = |c: Column<WlRow>| Column {
        hide_below: 120,
        ..c
    };
    vec![
        Column {
            // A workload without a hostname is still identifiable when the name column is hidden.
            narrow: Some(|r| marked_cell(r, r.workload.display_name())),
            narrow_width: 20,
            full_title: None,
            // `:export` writes the hostname, never the mark.
            full: Some(hostname_of),
            ..text_col("hostname", 22, hostname_cell, |r| {
                SortKey::Text(hostname_of(r))
            })
        },
        wide(text_col("name", 14, name_of, |r| SortKey::Text(name_of(r)))),
        Column {
            full: Some(|r| join_list(r.workload.interfaces.iter().map(|i| &i.address))),
            ..text_col("ip", 15, ip_of, |r| SortKey::Text(ip_of(r)))
        },
        wide(Column {
            full: Some(|r| r.label_list.clone()),
            ..text_col(
                "labels",
                0,
                |r| r.labels.clone(),
                |r| SortKey::Text(r.labels.clone()),
            )
        }),
        Column {
            narrow: Some(|r| enforcement_of(r).replace("visibility_only", "visibility")),
            narrow_width: 11,
            full_title: None,
            full: None,
            ..text_col("enforcement", 15, enforcement_of, |r| {
                SortKey::Text(enforcement_of(r))
            })
        },
        Column {
            // Secondary: gives way to the health columns until the terminal is very wide.
            hide_below: 180,
            // `:export` leaves the cell empty for nothing, as CSV does.
            full: Some(|r| {
                Some(visibility_of(r))
                    .filter(|v| v != "-")
                    .unwrap_or_default()
            }),
            ..text_col("visibility", 13, visibility_of, |r| {
                SortKey::Text(visibility_of(r))
            })
        },
        wide(Column {
            full: Some(|r| r.workload.online.map(|b| b.to_string()).unwrap_or_default()),
            ..text_col("online", 6, online_of, |r| {
                SortKey::Num(u64::from(r.workload.online.unwrap_or(false)))
            })
        }),
        wide(Column {
            full: Some(|r| r.workload.agent_status().unwrap_or_default()),
            ..text_col("agent", 9, agent_of, |r| SortKey::Text(agent_of(r)))
        }),
        text_col(
            "health",
            9,
            |r| r.health.as_str().to_string(),
            |r| SortKey::Num(u64::from(r.health.rank())),
        ),
        Column {
            full: Some(|r| r.heartbeat_secs.map(|s| s.to_string()).unwrap_or_default()),
            full_title: Some("heartbeat_age_secs"),
            ..text_col(
                "heartbeat",
                9,
                |r| r.heartbeat.clone(),
                |r| SortKey::Num(r.heartbeat_secs.unwrap_or(u64::MAX)),
            )
        },
        wide(text_col(
            "sync",
            8,
            |r| r.sync.clone(),
            |r| SortKey::Text(r.sync.clone()),
        )),
        Column {
            hide_below: 140,
            ..text_col(
                "ven",
                12,
                |r| r.ven.clone(),
                |r| SortKey::Text(r.ven.clone()),
            )
        },
        Column {
            // Names what `u` filters on; shown with `u` (`workload_columns_for`).
            hide_below: 200,
            full: Some(|r| join_list(&r.missing)),
            ..text_col(
                "missing",
                18,
                |r| r.missing.join(", "),
                |r| SortKey::Text(r.missing.join(", ")),
            )
        },
    ]
}

/// The columns `/` searches: every one but `missing`, which names keys
/// (`/role` would match every unlabeled row).
pub(super) static SEARCHED_COLUMNS: std::sync::LazyLock<Vec<Column<WlRow>>> =
    std::sync::LazyLock::new(|| {
        let mut cols = workload_columns();

        cols.truncate(MISSING_COL);
        cols
    });

/// The Workloads search: the default substring match minus `missing`.
pub(super) fn workload_matches(r: &WlRow, needle: &str) -> bool {
    needle.is_empty()
        || SEARCHED_COLUMNS
            .iter()
            .any(|c| (c.cell)(r).to_lowercase().contains(needle))
}

/// `E`'s steps after `all`: the effective enforcement ([`Workload::enforcement`]).
pub(super) const ENFORCEMENT_CYCLE: [&str; 5] = [
    "full",
    "selective",
    "visibility_only",
    "idle",
    pce::models::UNMANAGED,
];

/// The Workloads screen's local filters (`E` `m` `o` `u`, `c` clears):
/// applied to the loaded list, no request. `h` stays its own toggle.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct WorkloadFilters {
    /// The effective enforcement, `unmanaged` included.
    pub enforcement: Option<&'static str>,
    /// With a VEN (`true`) or without (`false`).
    pub managed: Option<bool>,
    /// The VEN is offline: [`VenHealth::Offline`] (`online` false or a
    /// stale heartbeat), as `workloads --offline`.
    pub offline: bool,
    /// Some label model key has no label ([`Workload::missing_label_keys`]).
    pub unlabeled: bool,
}

impl WorkloadFilters {
    /// `E`: all → full → selective → visibility_only → idle → unmanaged → all.
    pub fn next_enforcement(&mut self) {
        let at = self
            .enforcement
            .and_then(|e| ENFORCEMENT_CYCLE.iter().position(|c| *c == e));
        self.enforcement = match at {
            None => Some(ENFORCEMENT_CYCLE[0]),
            Some(i) => ENFORCEMENT_CYCLE.get(i + 1).copied(),
        };
    }

    /// `m`: all → managed → unmanaged → all.
    pub fn next_managed(&mut self) {
        self.managed = match self.managed {
            None => Some(true),
            Some(true) => Some(false),
            Some(false) => None,
        };
    }

    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }

    fn keeps(&self, v: &WorkloadRowView) -> bool {
        let w = &v.workload;
        self.enforcement.is_none_or(|e| w.enforcement() == Some(e))
            && self.managed.is_none_or(|m| m != w.is_unmanaged())
            // `health` is `ven_health` at the row's clock.
            && (!self.offline || v.health == VenHealth::Offline)
            && (!self.unlabeled || !v.missing.is_empty())
    }
}

/// A Workloads row: shared, so the grouped view (`x`) holds the same rows.
pub type WlRow = Arc<WorkloadRowView>;

#[derive(Debug)]
pub struct WorkloadsState {
    pub table: TableView<WlRow>,
    /// The workloads as fetched; `table` is a projection of these.
    pub raw: Vec<Workload>,
    /// The last read was cut short: impact counts are lower bounds.
    pub truncated: bool,
    /// Why the last read failed, until one succeeds (`impact: unavailable`).
    pub error: Option<String>,
    pub unhealthy_only: bool,
    /// `E` `m` `o` `u`: local filters on top of `unhealthy_only`.
    pub filters: WorkloadFilters,
    /// `f` / `:filter`: the label filter (`app=shop,crm !role=db`), local.
    pub labels: LabelFilter,
    /// The label filter's keys and values no label has (no match).
    pub label_warnings: Vec<String>,
    /// `x`: sections per label group instead of the flat table.
    pub grouped: bool,
    /// The keys the sections group by: the traffic group keys (`g`).
    pub group_keys: Vec<String>,
    /// `s` while grouped.
    pub group_order: GroupOrder,
    /// The groups whose sections show their header only (`Space`, `z`).
    pub folded: BTreeSet<Vec<Option<String>>>,
    /// The groups of the flat table's shown rows, while grouped; a fold or
    /// `s` lays them out again without regrouping.
    pub groups: Vec<Arc<WlGroup>>,
    /// The grouped table (`x`): headers and the workloads under them.
    pub sections: TableView<WlGroupRow>,
    /// The label model keys the last `rebuild` checked for `u`.
    model_keys: Vec<String>,
    /// Test-only fixed clock so ages in snapshots are deterministic.
    pub now: Option<jiff::Timestamp>,
    /// Workloads per health state in rank order (healthy, unmanaged, warning, offline, error).
    counts: [usize; 5],
    pub age_secs: u64,
    pub loaded_at: Option<Instant>,
    pub loading: bool,
    pub show_detail: bool,
    /// The name index failed, so the screen could not load; `r` retries the index.
    pub index_failed: bool,
    pub auto: AutoRefresh,
    /// `Enter` on a workload in the app overlay while this list loads: the
    /// workload selected, its detail open, once the list arrives.
    pub open_on_load: Option<String>,
    /// Workloads marked for a bulk change (`Space`, `V`), by href: they
    /// survive filters, search, sort and grouping; a reload drops the ones
    /// gone, a sent change and `Esc` clear them.
    pub marked: BTreeSet<String>,
    /// The next load is the reload after a bulk change: its report stays
    /// on the status line.
    pub keep_status: bool,
    /// `Esc` with many marks asked once (`Esc again clears N marks`).
    pub esc_armed: bool,
}

/// What `L` and `M` change: the marked workloads, else the group under
/// the cursor, else the row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BulkSource {
    Marked,
    Group,
    Row,
}

/// What the cursor is on in the grouped table, kept across a regrouping.
pub(super) enum WlCursor {
    Header(Vec<Option<String>>),
    Workload(String),
}

impl WorkloadsState {
    pub fn new(min_refresh: u64) -> Self {
        let columns = workload_columns();
        Self {
            sections: TableView::new(member_columns(&columns)),
            table: TableView::new(columns).with_matcher(workload_matches),
            raw: Vec::new(),
            truncated: false,
            error: None,
            unhealthy_only: false,
            filters: WorkloadFilters::default(),
            labels: LabelFilter::default(),
            label_warnings: Vec::new(),
            grouped: false,
            group_keys: keys_or_default(&[]),
            group_order: GroupOrder::default(),
            folded: BTreeSet::new(),
            groups: Vec::new(),
            model_keys: Vec::new(),
            now: None,
            counts: [0; 5],
            age_secs: 0,
            loaded_at: None,
            loading: false,
            show_detail: false,
            index_failed: false,
            auto: AutoRefresh::new(min_refresh),
            open_on_load: None,
            marked: BTreeSet::new(),
            keep_status: false,
            esc_armed: false,
        }
    }

    /// The rows' `●` from `marked` (the flat and grouped tables share them).
    pub fn sync_marks(&self) {
        for r in self.table.rows() {
            r.marked.set(self.marked.contains(&r.workload.href));
        }
    }

    /// `Space` on a workload: marks it, or unmarks it; whether it is now marked.
    pub fn toggle_mark(&mut self, href: &str) -> bool {
        let on = if self.marked.remove(href) {
            false
        } else {
            self.marked.insert(href.to_string());
            true
        };
        self.sync_marks();
        on
    }

    /// `V`: the hrefs it marks: grouped, the group under the cursor (on a
    /// header or one of its workloads); flat, every workload shown.
    pub fn mark_scope(&self) -> Vec<String> {
        if self.grouped {
            let key = match self.sections.selected_item() {
                Some(WlGroupRow::Header { group, .. }) => Some(group.key.clone()),
                Some(WlGroupRow::Member { row, .. }) => {
                    group_key_of(&self.raw, &row.workload.href, &self.group_keys)
                }
                None => None,
            };
            return self
                .groups
                .iter()
                .find(|g| Some(&g.key) == key.as_ref())
                .map(|g| g.members.iter().map(|m| m.workload.href.clone()).collect())
                .unwrap_or_default();
        }
        let rows = self.table.rows();
        self.table
            .visible()
            .iter()
            .map(|&i| rows[i].workload.href.clone())
            .collect()
    }

    /// `V`: marks every href of `scope`, or unmarks them when all are
    /// marked already; `(count, now marked)`.
    pub fn toggle_marks(&mut self, scope: &[String]) -> (usize, bool) {
        let all = !scope.is_empty() && scope.iter().all(|h| self.marked.contains(h));
        for h in scope {
            if all {
                self.marked.remove(h);
            } else {
                self.marked.insert(h.clone());
            }
        }
        self.sync_marks();
        (scope.len(), !all)
    }

    /// `Esc` or a sent change: no marks; how many there were.
    pub fn clear_marks(&mut self) -> usize {
        let n = self.marked.len();
        self.marked.clear();
        self.sync_marks();
        n
    }

    /// Marked workloads the filters (or search) hide from the table.
    pub fn marked_hidden(&self) -> usize {
        let rows = self.table.rows();
        let shown = self
            .table
            .visible()
            .iter()
            .filter(|&&i| rows[i].marked.get())
            .count();
        self.marked.len().saturating_sub(shown)
    }

    /// The marked workloads still loaded (hidden by a filter or not).
    pub fn marked_workloads(&self) -> Vec<&Workload> {
        self.raw
            .iter()
            .filter(|w| self.marked.contains(&w.href))
            .collect()
    }

    /// What `L` and `M` change: the marked workloads, else the group under
    /// the cursor (on its header), else the row; with what names them
    /// (`3 marked`, `group shop | prod`, `host-1`). Empty: nothing.
    pub fn bulk_targets(&self) -> (Vec<&Workload>, String, BulkSource) {
        if !self.marked.is_empty() {
            let ws = self.marked_workloads();
            let what = format!("{} marked", ws.len());
            return (ws, what, BulkSource::Marked);
        }
        if let Some(g) = self.selected_group() {
            let ws = g.members.iter().map(|m| &m.workload).collect();
            return (ws, format!("group {}", g.title), BulkSource::Group);
        }
        match self.selected_row() {
            Some(r) => (
                vec![&r.workload],
                r.workload.display_name(),
                BulkSource::Row,
            ),
            None => (Vec::new(), String::new(), BulkSource::Row),
        }
    }

    /// The cached workloads and their `truncated` flag, once loaded.
    pub fn impact_source(&self) -> Option<(&[Workload], bool)> {
        self.loaded_at
            .is_some()
            .then_some((self.raw.as_slice(), self.truncated))
    }

    /// Recomputes the table rows (and health counts) from `raw`, labels in
    /// `order`, and the sections from them while grouped.
    pub fn rebuild(&mut self, order: &KeyOrder) {
        let now = self.now.unwrap_or_else(jiff::Timestamp::now);
        let views: Vec<WorkloadRowView> = self
            .raw
            .iter()
            .cloned()
            .map(|w| WorkloadRowView::new(w, now, order))
            .collect();
        self.counts = [0; 5];
        for v in &views {
            self.counts[usize::from(v.health.rank())] += 1;
        }
        let unhealthy = |h: VenHealth| {
            matches!(
                h,
                VenHealth::Warning | VenHealth::Error | VenHealth::Offline
            )
        };
        let rows = views
            .into_iter()
            .filter(|v| !self.unhealthy_only || unhealthy(v.health))
            .filter(|v| self.filters.keeps(v))
            .filter(|v| self.labels.keeps(&v.workload))
            .map(Arc::new)
            .collect();
        self.model_keys = order.first().to_vec();
        let unlabeled = self.filters.unlabeled;
        if (self.table.columns[MISSING_COL].hide_below == 0) != unlabeled {
            let columns = workload_columns_for(unlabeled);
            self.sections.set_columns(member_columns(&columns));
            self.table.set_columns(columns);
        }
        self.table.set_rows(rows);
        self.sync_marks();
        self.regroup();
    }

    /// The sections again from the flat table's shown rows (after a
    /// filter, search, sort or key change); nothing while flat. The cursor
    /// stays on its workload or header when still shown.
    pub fn regroup(&mut self) {
        if !self.grouped {
            if !self.groups.is_empty() || !self.sections.rows().is_empty() {
                self.groups = Vec::new();
                self.sections.set_rows(Vec::new());
            }
            return;
        }
        let rows = self.table.rows();
        let shown = self.table.visible().iter().map(|&i| &rows[i]);
        self.groups = build_groups(shown, &self.group_keys, self.group_order);
        self.relayout();
    }

    /// The sections laid out again from `groups` (a fold, `s`), the cursor
    /// kept on its row.
    fn relayout(&mut self) {
        let cursor = self.cursor();
        self.sections
            .set_rows(wl_layout(&self.groups, &self.folded));
        match cursor {
            Some(WlCursor::Header(key)) => self
                .sections
                .select_where(|r| r.is_header() && r.group().key == key),
            Some(WlCursor::Workload(href)) => self
                .sections
                .select_where(|r| r.member().is_some_and(|w| w.workload.href == href)),
            None => {}
        }
    }

    fn cursor(&self) -> Option<WlCursor> {
        self.sections.selected_item().map(|r| match r {
            WlGroupRow::Header { group, .. } => WlCursor::Header(group.key.clone()),
            WlGroupRow::Member { row, .. } => WlCursor::Workload(row.workload.href.clone()),
        })
    }

    /// The workload under the cursor, flat or grouped (none on a header).
    pub fn selected_row(&self) -> Option<&WlRow> {
        if self.grouped {
            self.sections.selected_item().and_then(WlGroupRow::member)
        } else {
            self.table.selected_item()
        }
    }

    /// The group header under the cursor, while grouped.
    pub fn selected_group(&self) -> Option<&Arc<WlGroup>> {
        match self.sections.selected_item() {
            Some(WlGroupRow::Header { group, .. }) if self.grouped => Some(group),
            _ => None,
        }
    }

    /// Selects the workload `href` in the table on screen; whether it is
    /// now selected (a folded section's workload is not shown).
    pub fn select_href(&mut self, href: &str) -> bool {
        if self.grouped {
            self.sections
                .select_where(|r| r.member().is_some_and(|w| w.workload.href == href));
        } else {
            self.table.select_where(|r| r.workload.href == href);
        }
        self.selected_row().is_some_and(|r| r.workload.href == href)
    }

    /// Keeps the cursor of the table on screen inside `page` rows.
    pub fn ensure_visible(&mut self, page: usize) {
        if self.grouped {
            self.sections.ensure_visible(page);
        } else {
            self.table.ensure_visible(page);
        }
    }

    /// `x`: flat ↔ by label group under `keys`; the cursor stays on its
    /// workload (grouped: on its section's header when that is folded).
    pub fn toggle_grouped(&mut self, keys: Vec<String>) {
        let href = self.selected_row().map(|r| r.workload.href.clone());
        let group = self.selected_group().map(|g| g.members.first().cloned());
        self.grouped = !self.grouped;
        self.group_keys = keys;
        self.regroup();
        let href = href.or_else(|| group.flatten().map(|w| w.workload.href.clone()));
        if let Some(href) = href {
            if self.grouped && !self.select_href(&href) {
                let key = group_key_of(&self.raw, &href, &self.group_keys);
                self.sections
                    .select_where(|r| r.is_header() && Some(&r.group().key) == key.as_ref());
            } else if !self.grouped {
                self.select_href(&href);
            }
        }
    }

    /// New group keys (`g`): the sections again, folds dropped.
    pub fn set_group_keys(&mut self, keys: Vec<String>) {
        if keys != self.group_keys {
            self.group_keys = keys;
            self.folded.clear();
            self.regroup();
        }
    }

    /// `Space`: folds or unfolds the section under the cursor; the cursor
    /// lands on its header.
    pub fn toggle_fold(&mut self) {
        let Some(key) = self.sections.selected_item().map(|r| r.group().key.clone()) else {
            return;
        };
        if !self.folded.remove(&key) {
            self.folded.insert(key.clone());
        }
        self.sections
            .set_rows(wl_layout(&self.groups, &self.folded));
        self.sections
            .select_where(|r| r.is_header() && r.group().key == key);
    }

    /// `z`: folds every section, or unfolds them all when all are folded.
    pub fn toggle_fold_all(&mut self) {
        let key = self.sections.selected_item().map(|r| r.group().key.clone());
        let all = self.groups.iter().all(|g| self.folded.contains(&g.key));
        if all {
            self.folded.clear();
        } else {
            self.folded
                .extend(self.groups.iter().map(|g| g.key.clone()));
        }
        self.sections
            .set_rows(wl_layout(&self.groups, &self.folded));
        if let Some(key) = key {
            self.sections
                .select_where(|r| r.is_header() && r.group().key == key);
        }
    }

    /// `s` while grouped: the next section order, no regrouping.
    pub fn cycle_group_order(&mut self) -> GroupOrder {
        self.group_order = self.group_order.next();
        order_groups(&mut self.groups, self.group_order);
        self.relayout();
        self.group_order
    }

    /// Sets the label filter (`f`, `:filter`, `Enter` on a header) and
    /// names its unknown keys and values (from `idx`, when loaded).
    pub fn set_labels(&mut self, labels: LabelFilter, idx: Option<&NameIndex>, order: &KeyOrder) {
        self.label_warnings = self.unknown_labels(&labels, idx);
        self.labels = labels;
        self.rebuild(order);
    }

    /// The keys and values of `labels` neither the label list (`idx`) nor
    /// any loaded workload has: they match nothing.
    pub fn unknown_labels(&self, labels: &LabelFilter, idx: Option<&NameIndex>) -> Vec<String> {
        labels.unknown(|k| {
            let mut values = idx.map(|i| i.values_for_key(k)).unwrap_or_default();
            values.extend(
                self.raw
                    .iter()
                    .filter_map(|w| pce::label_filter::value_of(w, k))
                    .map(str::to_string),
            );
            (!values.is_empty()).then_some(values)
        })
    }

    /// Any filter beyond search: `h`, `E` `m` `o` `u` or the label filter.
    pub fn filtered(&self) -> bool {
        self.unhealthy_only || !self.filters.is_empty() || !self.labels.is_empty()
    }

    /// `c`: every filter off, the label filter too (not `/`).
    pub fn clear_filters(&mut self) {
        self.unhealthy_only = false;
        self.filters = WorkloadFilters::default();
        self.labels = LabelFilter::default();
        self.label_warnings.clear();
    }

    /// The line above the table while a filter is on: what is active and
    /// the rows shown of those loaded (`labels app=shop,crm · enforcement
    /// unmanaged · offline · unlabeled (app, env, role) · 24 of 206`); a
    /// truncated list says its counts are partial.
    pub fn filter_line(&self) -> Option<String> {
        if !self.filtered() {
            return None;
        }
        let f = &self.filters;
        let mut parts: Vec<String> = Vec::new();
        if !self.labels.is_empty() {
            parts.push(format!("labels {}", self.labels));
        }
        if self.unhealthy_only {
            parts.push("unhealthy".into());
        }
        if let Some(e) = f.enforcement {
            parts.push(format!("enforcement {e}"));
        }
        match f.managed {
            Some(true) => parts.push("managed".into()),
            Some(false) => parts.push("unmanaged".into()),
            None => {}
        }
        if f.offline {
            parts.push("offline".into());
        }
        if f.unlabeled {
            parts.push(format!("unlabeled ({})", self.model_keys.join(", ")));
        }
        parts.push(format!("{} of {}", self.table.len(), self.raw.len()));
        if self.truncated {
            parts.push("partial: the list is truncated".into());
        }
        Some(parts.join(" · "))
    }

    /// Counts per state in rank order (healthy, unmanaged, warning, offline, error)
    /// over all loaded workloads, ignoring the unhealthy-only filter.
    pub fn summary(&self) -> [usize; 5] {
        self.counts
    }

    /// `H`: worst health first; pressing it again flips the direction.
    pub fn sort_by_health(&mut self) {
        let desc = self.table.sort() != Some((HEALTH_COL, true));
        self.table.sort_by(HEALTH_COL, desc);
        self.regroup();
    }
}

/// The group of the loaded workload `href` under `keys`.
pub(super) fn group_key_of(
    raw: &[Workload],
    href: &str,
    keys: &[String],
) -> Option<Vec<Option<String>>> {
    raw.iter()
        .find(|w| w.href == href)
        .map(|w| pce::label_filter::group_key(w, keys))
}
