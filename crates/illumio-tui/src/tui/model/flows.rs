//! The traffic result and its rows: the query state, flows, groups, the
//! mesh, the treemap and the app sections, with their table columns.
use super::*;

#[derive(Debug)]
pub enum QueryState {
    Idle,
    Running {
        seq: u64,
        started: Instant,
        status: String,
        cancel: CancellationToken,
        /// `r` on a partial focus: the focus this run was started for.
        /// Clearing or changing the focus cancels it.
        focus: Option<Focus>,
        /// `D`: a draft re-evaluation of the result on screen. Any change
        /// of that result (a new query, a focus change, a cap step, a
        /// profile switch) or `Esc` cancels it.
        draft: bool,
        /// `c`: compare mode's baseline query; `C` cancels it.
        baseline: bool,
    },
    Failed(String),
}

#[derive(Debug)]
pub struct TrafficData {
    pub flows: Vec<ResolvedFlow>,
    pub truncated: bool,
    /// A supercluster region did not respond: rows are missing.
    pub incomplete: bool,
    pub elapsed: Duration,
    pub cache_hit: bool,
    /// When the result arrived: the Result header's age.
    pub fetched_at: Instant,
    pub filter: TrafficFilter,
    /// The time range the query asked for (`-` moves its start); `None`
    /// when the outcome carried no bounds.
    pub range: Option<Range>,
    /// The async query hrefs behind the flows (`in` first when focused):
    /// `D` re-evaluates them against the draft policy without a new POST.
    /// Empty when unknown.
    pub hrefs: Vec<String>,
    /// `D` loaded the draft decisions into `flows`: what they change.
    pub draft: Option<pce::traffic::DraftSummary>,
}

impl TrafficData {
    /// Where the window the data was asked for begins: the range's end
    /// minus the filter's window.
    pub fn window_start(&self) -> Option<jiff::Timestamp> {
        let end = self.range?.end;
        let span = jiff::Span::try_from(self.filter.window_duration().ok()?).ok()?;
        end.checked_sub(span).ok()
    }

    /// The Result header's count: `1234 flows`, or `1000 flows, capped at
    /// 1000; B raises the cap` for a truncated result (which covers an
    /// unknown part of its window, so it claims no span).
    pub fn count_text(&self) -> String {
        let n = self.flows.len();
        if self.truncated {
            format!(
                "{n} flows, capped at {}; B raises the cap",
                self.filter.max_results
            )
        } else {
            format!("{n} flows")
        }
    }
}

/// The time range a query asked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Range {
    pub start: jiff::Timestamp,
    pub end: jiff::Timestamp,
}

/// A flow with its grouping text for the keys in force (`shop/prod`): a
/// `Column`'s `fn` cannot see the keys, so the cells are computed when the
/// rows are built. Shared (`Arc`), so the Flows sections copy a row for a
/// few reference counts.
#[derive(Debug, Clone, PartialEq)]
pub struct FlowRow {
    pub flow: Arc<ResolvedFlow>,
    pub src_group: Arc<str>,
    pub dst_group: Arc<str>,
    /// The flow's winning mark, while marks are active.
    pub mark: Option<Hit>,
    /// Compare mode: the flow's change against the baseline.
    pub change: Option<pce::traffic::RowChange>,
}

impl FlowRow {
    pub fn new(flow: ResolvedFlow, keys: &[String]) -> Self {
        Self {
            src_group: group_text(&flow.src, keys).into(),
            dst_group: group_text(&flow.dst, keys).into(),
            flow: Arc::new(flow),
            mark: None,
            change: None,
        }
    }
}

impl std::ops::Deref for FlowRow {
    type Target = ResolvedFlow;

    fn deref(&self) -> &ResolvedFlow {
        &self.flow
    }
}

/// A row the Flows columns can draw: a flow (the flat table), or a row of
/// the Flows view's app sections (`x`), whose header rows have no cells.
pub trait FlowCells {
    fn flow_row(&self) -> Option<&FlowRow>;
}

impl FlowCells for FlowRow {
    fn flow_row(&self) -> Option<&FlowRow> {
        Some(self)
    }
}

/// `f` of the row's flow; empty on a header row.
pub(super) fn on_flow<T: FlowCells>(r: &T, f: impl Fn(&FlowRow) -> String) -> String {
    r.flow_row().map(f).unwrap_or_default()
}

/// The sort key `f` gives the row's flow; `0` on a header row.
pub(super) fn flow_key<T: FlowCells>(r: &T, f: impl Fn(&FlowRow) -> SortKey) -> SortKey {
    r.flow_row().map_or(SortKey::Num(0), f)
}

/// Last seen (from 140 columns), src, src group, dst, dst group, service,
/// conns, decision: the group columns fit at 80.
pub fn flow_columns() -> Vec<Column<FlowRow>> {
    flow_columns_of()
}

/// `flow_columns` for any row holding flows: the flat table and the
/// Flows view's app sections draw the same cells.
pub fn flow_columns_of<T: FlowCells>() -> Vec<Column<T>> {
    vec![
        Column {
            title: "last seen",
            width: 16,
            cell: |r| {
                on_flow(r, |f| {
                    f.last_detected
                        .as_deref()
                        .map(short_time)
                        .unwrap_or_default()
                })
            },
            key: |r| {
                flow_key(r, |f| {
                    SortKey::Text(f.last_detected.clone().unwrap_or_default())
                })
            },
            right: false,
            hide_below: 140,
            narrow: None,
            narrow_width: 0,
            full_title: None,
            full: Some(|r| on_flow(r, |f| f.last_detected.clone().unwrap_or_default())),
        },
        Column {
            title: "src",
            width: 19,
            cell: |r| on_flow(r, |f| f.src.display_name()),
            key: |r| flow_key(r, |f| SortKey::Text(f.src.display_name())),
            right: false,
            hide_below: 0,
            narrow: None,
            narrow_width: 14,
            full_title: None,
            full: None,
        },
        Column {
            title: "src group",
            width: 18,
            cell: |r| on_flow(r, |f| f.src_group.to_string()),
            key: |r| flow_key(r, |f| SortKey::Text(f.src_group.to_string())),
            right: false,
            hide_below: 0,
            narrow: None,
            narrow_width: 10,
            full_title: None,
            full: None,
        },
        Column {
            title: "dst",
            width: 19,
            cell: |r| on_flow(r, |f| f.dst.display_name()),
            key: |r| flow_key(r, |f| SortKey::Text(f.dst.display_name())),
            right: false,
            hide_below: 0,
            narrow: None,
            narrow_width: 14,
            full_title: None,
            full: None,
        },
        Column {
            title: "dst group",
            width: 18,
            cell: |r| on_flow(r, |f| f.dst_group.to_string()),
            key: |r| flow_key(r, |f| SortKey::Text(f.dst_group.to_string())),
            right: false,
            hide_below: 0,
            narrow: None,
            narrow_width: 10,
            full_title: None,
            full: None,
        },
        Column {
            title: "service",
            width: 10,
            cell: |r| on_flow(r, |f| f.service_text()),
            key: |r| flow_key(r, |f| SortKey::Num(u64::from(f.port.unwrap_or(0)))),
            right: false,
            hide_below: 0,
            narrow: None,
            narrow_width: 9,
            full_title: None,
            full: None,
        },
        Column {
            title: "conns",
            width: 7,
            cell: |r| on_flow(r, |f| f.num_connections.to_string()),
            key: |r| flow_key(r, |f| SortKey::Num(f.num_connections)),
            right: true,
            hide_below: 0,
            narrow: None,
            narrow_width: 5,
            full_title: None,
            full: None,
        },
        Column {
            title: "decision",
            width: 19,
            cell: |r| on_flow(r, |f| f.policy_decision.as_str().to_string()),
            key: |r| flow_key(r, |f| SortKey::Text(f.policy_decision.as_str().to_string())),
            right: false,
            hide_below: 0,
            narrow: Some(|r| {
                on_flow(r, |f| match f.policy_decision.as_str() {
                    "potentially_blocked" => "pot.blk".to_string(),
                    other => other.to_string(),
                })
            }),
            narrow_width: 8,
            full_title: None,
            full: None,
        },
    ]
}

/// The title of the column a view gains while draft decisions are loaded.
pub const DRAFT_COLUMN: &str = "draft";

/// `cols` with the group columns narrowed to make room for the 13-cell
/// draft column at 120 columns.
pub fn make_room_for_draft<T>(mut cols: Vec<Column<T>>) -> Vec<Column<T>> {
    for c in &mut cols {
        if matches!(c.title, "src group" | "dst group") {
            c.width = 11;
        }
    }
    cols
}

/// The Flows view's `draft` column.
pub fn draft_flow_column() -> Column<FlowRow> {
    draft_flow_column_of()
}

/// `draft_flow_column` for any row holding flows (`flow_columns_of`).
pub fn draft_flow_column_of<T: FlowCells>() -> Column<T> {
    Column {
        title: DRAFT_COLUMN,
        // `→ pot.blocked`, the longest known text: the reported decision
        // is the column beside it.
        width: 13,
        cell: |r| {
            on_flow(r, |f| {
                match (&f.flow.draft_policy_decision, f.flow.draft_change()) {
                    (_, Some(d)) => format!("→ {}", pce::traffic::short_decision(d.as_str())),
                    (Some(d), None) => pce::traffic::short_decision(d.as_str()),
                    (None, None) => String::new(),
                }
            })
        },
        key: |r| {
            flow_key(r, |f| {
                SortKey::Text(
                    f.flow
                        .draft_policy_decision
                        .as_ref()
                        .map(|d| d.as_str().to_string())
                        .unwrap_or_default(),
                )
            })
        },
        right: false,
        hide_below: 0,
        // Below 120 columns only the changes: `→ allow`, `→ block`,
        // `→ pot`; `·` where the draft keeps the verdict.
        narrow: Some(|r| {
            on_flow(r, |f| {
                match (&f.flow.draft_policy_decision, f.flow.draft_change()) {
                    (_, Some(d)) => format!(
                        "→ {}",
                        match d.class() {
                            pce::models::PolicyDecision::Allowed => "allow",
                            pce::models::PolicyDecision::Blocked => "block",
                            pce::models::PolicyDecision::PotentiallyBlocked => "pot",
                            pce::models::PolicyDecision::Unknown => "?",
                        }
                    ),
                    (Some(_), None) => "·".into(),
                    (None, None) => String::new(),
                }
            })
        }),
        narrow_width: 7,
        full_title: None,
        full: Some(|r| {
            on_flow(r, |f| {
                f.flow
                    .draft_policy_decision
                    .as_ref()
                    .map(|d| d.as_str().to_string())
                    .unwrap_or_default()
            })
        }),
    }
}

/// A draft cell that fits 20 columns: `2 would allow`, `1 would block`,
/// with both `+2 allow -1 block`, `-` with neither.
pub fn draft_counts_cell(d: Option<&DraftCounts>) -> String {
    match d {
        Some(c) if c.draft_allowed > 0 && c.draft_blocked > 0 => {
            format!("+{} allow -{} block", c.draft_allowed, c.draft_blocked)
        }
        _ => DraftCounts::text(d),
    }
}

/// App-to-app pairs' `draft` column: `2 would allow` (`draft_counts_cell`).
pub fn draft_group_column() -> Column<FlowGroup> {
    Column {
        title: DRAFT_COLUMN,
        width: 20,
        cell: |g| draft_counts_cell(g.draft.as_ref()),
        key: |g| SortKey::Num(g.draft.map_or(0, |d| d.draft_allowed)),
        right: false,
        // Below 100 columns the pairs have no room; `d` keeps `fixed by draft`.
        hide_below: 100,
        narrow: Some(|g| draft_short(g.draft.as_ref())),
        narrow_width: 9,
        full_title: None,
        full: None,
    }
}

/// `+2 -1`: would allow, would block; `-` with neither.
pub fn draft_short(d: Option<&DraftCounts>) -> String {
    match d {
        None => String::new(),
        Some(d) if d.draft_allowed == 0 && d.draft_blocked == 0 => "-".into(),
        Some(d) => {
            let mut parts = Vec::new();
            if d.draft_allowed > 0 {
                parts.push(format!("+{}", d.draft_allowed));
            }
            if d.draft_blocked > 0 {
                parts.push(format!("-{}", d.draft_blocked));
            }
            parts.join(" ")
        }
    }
}

/// A sectioned table's `draft` column, on its peer rows.
pub fn draft_section_column() -> Column<SectionRow> {
    Column {
        title: DRAFT_COLUMN,
        width: 20,
        cell: |r| r.with_row(|p| draft_counts_cell(p.draft.as_ref())),
        key: no_sort,
        right: false,
        hide_below: 0,
        narrow: Some(|r| r.with_row(|p| draft_short(p.draft.as_ref()))),
        narrow_width: 9,
        full_title: None,
        full: None,
    }
}

/// The Mesh view's `draft` column (`draft_counts_cell`).
pub fn draft_mesh_column() -> Column<MeshRow> {
    Column {
        title: DRAFT_COLUMN,
        width: 20,
        cell: |r| draft_counts_cell(r.draft.as_ref()),
        key: |r| SortKey::Num(r.draft.map_or(0, |d| d.draft_allowed)),
        right: false,
        // Below 100 columns the mesh has no room; `D` is in the Result header.
        hide_below: 100,
        narrow: Some(|r| draft_short(r.draft.as_ref())),
        narrow_width: 9,
        full_title: None,
        full: None,
    }
}

/// `cols`, plus `extra` at the end when `draft`.
pub(super) fn with_draft<T>(
    mut cols: Vec<Column<T>>,
    extra: Column<T>,
    draft: bool,
) -> Vec<Column<T>> {
    if draft {
        cols.push(extra);
    }
    cols
}

/// `2026-10-10T07:31:02Z` -> `10-10 07:31:02`
pub fn short_time(s: &str) -> String {
    s.get(5..19)
        .map(|x| x.replace('T', " "))
        .unwrap_or_else(|| s.to_string())
}

/// `2026-10-10T07:31:02Z` -> `10-10 07:31`
pub(super) fn short_time_narrow(s: &str) -> String {
    s.get(5..16)
        .map(|x| x.replace('T', " "))
        .unwrap_or_else(|| s.to_string())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrafficView {
    Flows,
    Aggregate,
    Mesh,
    Blocked,
    Treemap,
    /// `:exposure` / `9` (#82): risky services per destination group, on
    /// the exposure query's result. Not in `v`'s cycle (`v` leaves it).
    Exposure,
}

impl TrafficView {
    /// `v`: Flows, App-to-app, Mesh, Blocked, Treemap, then Flows again
    /// (from Exposure too).
    pub fn next(self) -> Self {
        match self {
            TrafficView::Flows => TrafficView::Aggregate,
            TrafficView::Aggregate => TrafficView::Mesh,
            TrafficView::Mesh => TrafficView::Blocked,
            TrafficView::Blocked => TrafficView::Treemap,
            TrafficView::Treemap | TrafficView::Exposure => TrafficView::Flows,
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            TrafficView::Flows => "Flows",
            TrafficView::Aggregate => "App-to-app",
            TrafficView::Mesh => "Mesh",
            TrafficView::Blocked => "Blocked",
            TrafficView::Treemap => "Treemap",
            TrafficView::Exposure => "Exposure",
        }
    }
}

/// A treemap tile the cursor can stand on: an outer (source) tile, or one
/// of its destinations (`inner`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TreemapTarget {
    pub outer: usize,
    pub inner: Option<usize>,
    pub rect: TreemapRect,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dir {
    Left,
    Right,
    Up,
    Down,
}

/// The Treemap view: what the renderer draws, computed when the groups,
/// the zoom, the weight or the terminal size change, never while drawing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TreemapState {
    pub weight: Weight,
    /// The source group zoomed into; its destinations fill the area.
    pub zoom: Option<Vec<String>>,
    /// Index into `targets()`.
    pub cursor: usize,
    pub tiles: Vec<Tile>,
    /// The pairs the tiles are laid out from: every loaded flow, whatever
    /// App-to-app's decision filter (`d`).
    pub groups: Vec<FlowGroup>,
    /// Some loaded flow carried byte counts: `b` may weigh by bytes, and
    /// the meta draws `b bytes` dim when not.
    pub has_bytes: bool,
    /// Where the tiles go: the Traffic table area minus its border for the
    /// terminal size (`ui::screens::traffic::treemap_area`).
    pub area: TreemapRect,
    /// The highlight (critical, important, revisit) of each pair with one,
    /// by (source, destination) text, and of each source: the glyph its
    /// tile shows before its label. Computed at relayout, never per draw.
    pub highlights: BTreeMap<(String, String), pce::marks::Marker>,
    pub source_highlights: BTreeMap<String, pce::marks::Marker>,
}

impl TreemapState {
    pub fn new(area: TreemapRect) -> Self {
        Self {
            weight: Weight::Connections,
            zoom: None,
            cursor: 0,
            tiles: Vec::new(),
            groups: Vec::new(),
            has_bytes: false,
            area,
            highlights: BTreeMap::new(),
            source_highlights: BTreeMap::new(),
        }
    }

    /// The highlight a tile shows: a destination by its pair, a source by
    /// the strongest of its pairs.
    pub fn highlight(&self, src: &str, dst: Option<&str>) -> Option<pce::marks::Marker> {
        match dst {
            Some(d) => self
                .highlights
                .get(&(src.to_string(), d.to_string()))
                .copied(),
            None => self.source_highlights.get(src).copied(),
        }
    }

    /// Every tile in drawing order: each outer tile, then its destinations.
    pub fn targets(&self) -> Vec<TreemapTarget> {
        let mut out = Vec::new();
        for (i, t) in self.tiles.iter().enumerate() {
            out.push(TreemapTarget {
                outer: i,
                inner: None,
                rect: t.rect,
            });
            out.extend(t.children.iter().enumerate().map(|(j, c)| TreemapTarget {
                outer: i,
                inner: Some(j),
                rect: c.rect,
            }));
        }
        out
    }

    pub fn current(&self) -> Option<TreemapTarget> {
        self.targets().get(self.cursor).copied()
    }

    pub fn tile(&self, t: TreemapTarget) -> &Tile {
        let outer = &self.tiles[t.outer];
        t.inner.map_or(outer, |j| &outer.children[j])
    }

    /// Moves to the nearest tile whose centre lies in `dir`: distance along
    /// `dir` plus twice the distance across it, a row counting as two
    /// columns. Nothing in that direction: the cursor stays.
    pub fn move_cursor(&mut self, dir: Dir) {
        let targets = self.targets();
        let Some(cur) = targets.get(self.cursor) else {
            return;
        };
        let centre = |r: TreemapRect| {
            (
                2 * i32::from(r.x) + i32::from(r.w),
                2 * (2 * i32::from(r.y) + i32::from(r.h)),
            )
        };
        let (cx, cy) = centre(cur.rect);
        let best = targets
            .iter()
            .enumerate()
            .filter(|(i, _)| *i != self.cursor)
            .filter_map(|(i, t)| {
                let (x, y) = centre(t.rect);
                let (dx, dy) = (x - cx, y - cy);
                let (ahead, side) = match dir {
                    Dir::Right => (dx, dy),
                    Dir::Left => (-dx, dy),
                    Dir::Down => (dy, dx),
                    Dir::Up => (-dy, dx),
                };
                (ahead > 0).then_some((ahead + 2 * side.abs(), i))
            })
            .min();
        if let Some((_, i)) = best {
            self.cursor = i;
        }
    }
}

/// `allowed`, `pot.blocked`, `blocked` or `unknown`: a mesh row's dominant
/// decision in the 11-cell column.
pub(super) fn mesh_decision(r: &MeshRow) -> String {
    match r.dominant() {
        pce::models::PolicyDecision::PotentiallyBlocked => "pot.blocked".into(),
        d => d.as_str().into(),
    }
}

/// The Mesh view's columns; user and process only when `cols` shows them.
pub fn mesh_columns(cols: MeshColumns) -> Vec<Column<MeshRow>> {
    let mut out: Vec<Column<MeshRow>> = Vec::new();
    if cols.user {
        out.push(Column {
            title: "user",
            width: 12,
            cell: |r| r.user.clone().unwrap_or_else(|| "-".into()),
            key: |r| SortKey::Text(r.user.clone().unwrap_or_default()),
            right: false,
            hide_below: 0,
            narrow: None,
            narrow_width: 8,
            full_title: None,
            full: Some(|r| r.user.clone().unwrap_or_default()),
        });
    }
    if cols.process {
        out.push(Column {
            title: "process",
            width: 14,
            cell: |r| r.process.clone().unwrap_or_else(|| "-".into()),
            key: |r| SortKey::Text(r.process.clone().unwrap_or_default()),
            right: false,
            hide_below: 0,
            narrow: None,
            narrow_width: 10,
            full_title: None,
            full: Some(|r| r.process.clone().unwrap_or_default()),
        });
    }
    let rest: [Column<MeshRow>; 6] = [
        Column {
            title: "src",
            width: 18,
            cell: |r| r.src.join("/"),
            key: |r| SortKey::Text(r.src.join("/")),
            right: false,
            hide_below: 0,
            narrow: None,
            narrow_width: 14,
            full_title: None,
            full: None,
        },
        Column {
            title: "dst",
            width: 18,
            cell: |r| r.dst.join("/"),
            key: |r| SortKey::Text(r.dst.join("/")),
            right: false,
            hide_below: 0,
            narrow: None,
            narrow_width: 14,
            full_title: None,
            full: None,
        },
        Column {
            title: "service",
            width: 10,
            cell: |r| r.service_text(),
            key: |r| SortKey::Num(u64::from(r.port.unwrap_or(0))),
            right: false,
            hide_below: 0,
            narrow: None,
            narrow_width: 9,
            full_title: None,
            full: None,
        },
        Column {
            title: "dst proc",
            width: 12,
            cell: |r| r.dst_process.clone().unwrap_or_else(|| "-".into()),
            key: |r| SortKey::Text(r.dst_process.clone().unwrap_or_default()),
            right: false,
            hide_below: 120,
            narrow: None,
            narrow_width: 0,
            full_title: None,
            full: Some(|r| r.dst_process.clone().unwrap_or_default()),
        },
        Column {
            title: "conns",
            width: 8,
            cell: |r| r.connections.to_string(),
            key: |r| SortKey::Num(r.connections),
            right: true,
            hide_below: 0,
            narrow: None,
            narrow_width: 6,
            full_title: None,
            full: None,
        },
        Column {
            title: "decision",
            width: 11,
            cell: mesh_decision,
            key: |r| SortKey::Text(mesh_decision(r)),
            right: false,
            hide_below: 0,
            narrow: Some(|r| match r.dominant() {
                pce::models::PolicyDecision::PotentiallyBlocked => "pot.blk".into(),
                d => d.as_str().into(),
            }),
            narrow_width: 8,
            full_title: None,
            full: Some(|r| r.dominant().as_str().into()),
        },
    ];
    out.extend(rest);
    out
}

/// The connections per decision (lifetime counts of the flow records),
/// shown from 140 columns: `traffic groups`' `allowed_conns`,
/// `pot_blocked_conns` and `blocked_conns`.
pub(super) fn conns_by_decision_columns() -> [Column<FlowGroup>; 3] {
    [
        Column {
            title: "allow.c",
            width: 8,
            cell: |g| g.conns.allowed_conns.to_string(),
            key: |g| SortKey::Num(g.conns.allowed_conns),
            right: true,
            hide_below: 140,
            narrow: None,
            narrow_width: 0,
            full_title: Some("allowed_conns"),
            full: None,
        },
        Column {
            title: "pot.c",
            width: 8,
            cell: |g| g.conns.pot_blocked_conns.to_string(),
            key: |g| SortKey::Num(g.conns.pot_blocked_conns),
            right: true,
            hide_below: 140,
            narrow: None,
            narrow_width: 0,
            full_title: Some("pot_blocked_conns"),
            full: None,
        },
        Column {
            title: "block.c",
            width: 8,
            cell: |g| g.conns.blocked_conns.to_string(),
            key: |g| SortKey::Num(g.conns.blocked_conns),
            right: true,
            hide_below: 140,
            narrow: None,
            narrow_width: 0,
            full_title: Some("blocked_conns"),
            full: None,
        },
    ]
}

/// The Exposure view (#82): one row per destination group and risky
/// service, exposed rows first.
pub fn exposure_columns() -> Vec<Column<ExposureRow>> {
    let col = |title, width, cell: fn(&ExposureRow) -> String, key, right, hide_below| Column {
        title,
        width,
        cell,
        key,
        right,
        hide_below,
        narrow: None,
        narrow_width: 0,
        full_title: None,
        full: None,
    };
    vec![
        Column {
            full: Some(|r| r.status.as_str().to_string()),
            narrow: Some(|r| match r.status {
                ExposureStatus::PotentiallyBlocked => "pot.bl".into(),
                s => s.short().to_string(),
            }),
            narrow_width: 7,
            ..col(
                "status",
                11,
                |r| r.status.short().to_string(),
                |r| SortKey::Num(r.status as u64),
                false,
                0,
            )
        },
        col(
            "service",
            8,
            |r| r.service.clone(),
            |r| SortKey::Text(r.service.clone()),
            false,
            0,
        ),
        col(
            "ports",
            18,
            |r| r.ports.join(","),
            |r| SortKey::Text(r.ports.join(",")),
            false,
            120,
        ),
        col(
            "dst",
            0,
            |r| r.dst_text(),
            |r| SortKey::Text(r.dst_text()),
            false,
            0,
        ),
        col(
            "sources",
            20,
            |r| r.sources.text(),
            |r| SortKey::Num(r.sources.apps + r.sources.ip_lists),
            false,
            0,
        ),
        col(
            "flows",
            6,
            |r| r.flows.to_string(),
            |r| SortKey::Num(r.flows),
            true,
            100,
        ),
        col(
            "conns",
            8,
            |r| r.connections.to_string(),
            |r| SortKey::Num(r.connections),
            true,
            100,
        ),
        col(
            "allowed",
            7,
            |r| r.allowed.to_string(),
            |r| SortKey::Num(r.allowed),
            true,
            0,
        ),
        col(
            "pot.bl",
            6,
            |r| r.potentially_blocked.to_string(),
            |r| SortKey::Num(r.potentially_blocked),
            true,
            0,
        ),
        col(
            "blocked",
            7,
            |r| r.blocked.to_string(),
            |r| SortKey::Num(r.blocked),
            true,
            0,
        ),
        col(
            "draft",
            20,
            |r| DraftCounts::text(r.draft.as_ref()),
            |r| SortKey::Text(DraftCounts::text(r.draft.as_ref())),
            false,
            110,
        ),
    ]
}

pub fn group_columns() -> Vec<Column<FlowGroup>> {
    let mut cols: Vec<Column<FlowGroup>> = vec![
        Column {
            title: "src",
            width: 26,
            cell: |g| g.src.join("/"),
            key: |g| SortKey::Text(g.src.join("/")),
            right: false,
            hide_below: 0,
            narrow: None,
            narrow_width: 16,
            full_title: None,
            full: None,
        },
        Column {
            title: "dst",
            width: 26,
            cell: |g| g.dst.join("/"),
            key: |g| SortKey::Text(g.dst.join("/")),
            right: false,
            hide_below: 0,
            narrow: None,
            narrow_width: 16,
            full_title: None,
            full: None,
        },
        Column {
            title: "flows",
            width: 6,
            cell: |g| flows_cell(g.flows, g.marks.as_ref()),
            key: |g| SortKey::Num(g.flows),
            right: true,
            hide_below: 0,
            narrow: None,
            narrow_width: 5,
            full_title: None,
            full: Some(|g| g.flows.to_string()),
        },
        Column {
            title: "conns",
            width: 8,
            cell: |g| g.connections.to_string(),
            key: |g| SortKey::Num(g.connections),
            right: true,
            hide_below: 0,
            narrow: None,
            narrow_width: 6,
            full_title: None,
            full: None,
        },
        Column {
            title: "allowed",
            width: 7,
            cell: |g| g.allowed.to_string(),
            key: |g| SortKey::Num(g.allowed),
            right: true,
            hide_below: 0,
            narrow: None,
            narrow_width: 0,
            full_title: None,
            full: None,
        },
        Column {
            title: "pot.bl",
            width: 6,
            cell: |g| g.potentially_blocked.to_string(),
            key: |g| SortKey::Num(g.potentially_blocked),
            right: true,
            hide_below: 0,
            narrow: None,
            narrow_width: 0,
            full_title: None,
            full: None,
        },
        Column {
            title: "blocked",
            width: 7,
            cell: |g| g.blocked.to_string(),
            key: |g| SortKey::Num(g.blocked),
            right: true,
            hide_below: 0,
            narrow: None,
            narrow_width: 0,
            full_title: None,
            full: None,
        },
    ];
    cols.extend(conns_by_decision_columns());
    cols
}

/// App-to-app `pairs` by service (`o`): one row per source group,
/// destination group and service, the policy tuple.
pub fn service_group_columns() -> Vec<Column<FlowGroup>> {
    let mut cols = group_columns();
    cols.truncate(4);
    cols.insert(
        2,
        Column {
            title: "service",
            width: 10,
            cell: |g| g.service.clone().unwrap_or_default(),
            key: |g| SortKey::Text(g.service.clone().unwrap_or_default()),
            right: false,
            hide_below: 0,
            narrow: None,
            narrow_width: 9,
            full_title: None,
            full: None,
        },
    );
    let tail: [Column<FlowGroup>; 2] = [
        Column {
            title: "bytes",
            width: 8,
            cell: |g| bytes_text(g.bytes),
            key: |g| SortKey::Num(g.bytes),
            right: true,
            hide_below: 0,
            narrow: None,
            narrow_width: 6,
            full_title: None,
            full: Some(|g| g.bytes.to_string()),
        },
        Column {
            title: "decision",
            width: 11,
            cell: |g| decision_text(g.dominant()),
            key: |g| SortKey::Text(decision_text(g.dominant())),
            right: false,
            hide_below: 0,
            narrow: Some(|g| match g.dominant() {
                pce::models::PolicyDecision::PotentiallyBlocked => "pot.blk".into(),
                d => d.as_str().into(),
            }),
            narrow_width: 8,
            full_title: None,
            full: Some(|g| g.dominant().as_str().into()),
        },
    ];
    cols.extend(tail);
    cols.extend(conns_by_decision_columns());
    cols
}

/// App-to-app's state (`x` / `X`): the flat pair table, or sections per
/// app inbound, outbound or both.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum By {
    #[default]
    Pairs,
    In,
    Out,
    Both,
}

impl By {
    /// `x`: pairs → in → out → both → pairs.
    pub fn next(self) -> Self {
        match self {
            By::Pairs => By::In,
            By::In => By::Out,
            By::Out => By::Both,
            By::Both => By::Pairs,
        }
    }

    /// `X`: the other way round.
    pub fn prev(self) -> Self {
        match self {
            By::Pairs => By::Both,
            By::In => By::Pairs,
            By::Out => By::In,
            By::Both => By::Out,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            By::Pairs => "pairs",
            By::In => "in",
            By::Out => "out",
            By::Both => "both",
        }
    }
}

/// App-to-app's session state: kept across `g`, `r`, `+` / `-`, `f` and
/// `:window` (the tables are rebuilt from it), reset by `:profile`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AppState {
    pub by: By,
    /// `o`: `pairs` one row per service as well (`services`, not `groups`).
    pub by_service: bool,
    pub decisions: Decisions,
    /// The apps whose sections show only their header row.
    pub folded: BTreeSet<Vec<String>>,
    /// `s` outside `pairs`: the order of the sections.
    pub order: SectionOrder,
}

/// The Flows view's grouping (`x` / `X`): the flat table, or the flows
/// in one section per source or destination app.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum FlowsBy {
    #[default]
    Flat,
    Src,
    Dst,
}

impl FlowsBy {
    /// `x`: flat → by source app → by destination app → flat.
    pub fn next(self) -> Self {
        match self {
            FlowsBy::Flat => FlowsBy::Src,
            FlowsBy::Src => FlowsBy::Dst,
            FlowsBy::Dst => FlowsBy::Flat,
        }
    }

    /// `X`: the other way round.
    pub fn prev(self) -> Self {
        match self {
            FlowsBy::Flat => FlowsBy::Dst,
            FlowsBy::Src => FlowsBy::Flat,
            FlowsBy::Dst => FlowsBy::Src,
        }
    }

    /// The side the sections' app is on: the source is `Out` (the app
    /// sends), the destination `In`, as in App-to-app. `None` when flat.
    pub fn direction(self) -> Option<Direction> {
        match self {
            FlowsBy::Flat => None,
            FlowsBy::Src => Some(Direction::Out),
            FlowsBy::Dst => Some(Direction::In),
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            FlowsBy::Flat => "flat",
            FlowsBy::Src => "by src app",
            FlowsBy::Dst => "by dst app",
        }
    }
}

/// The Flows view's session state, kept like `AppState` (across `g`, `r`,
/// `+` / `-`, `f`) and reset by `:profile`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FlowsState {
    pub by: FlowsBy,
    /// `s` while grouped: the order of the sections.
    pub order: SectionOrder,
    /// The apps whose sections show only their header row, by source app
    /// and by destination app (one app is a different section in each).
    pub folded_src: BTreeSet<Vec<String>>,
    pub folded_dst: BTreeSet<Vec<String>>,
}

impl FlowsState {
    /// The folds of the grouping in force (by source app while flat).
    pub fn folded(&self) -> &BTreeSet<Vec<String>> {
        match self.by {
            FlowsBy::Dst => &self.folded_dst,
            FlowsBy::Flat | FlowsBy::Src => &self.folded_src,
        }
    }

    pub(super) fn folded_mut(&mut self) -> &mut BTreeSet<Vec<String>> {
        match self.by {
            FlowsBy::Dst => &mut self.folded_dst,
            FlowsBy::Flat | FlowsBy::Src => &mut self.folded_src,
        }
    }
}

/// What a section row is: an app's header with its totals per direction
/// (one direction, or in then out for `both`), one of its peer rows, or
/// (the Flows view) one of its flows.
#[derive(Debug, Clone, PartialEq)]
pub enum RowKind {
    Header {
        totals: Vec<(Direction, Totals)>,
        folded: bool,
        /// The section's rows' marks summed: any highlight, dimmed when all
        /// are quiet. Boxed: a header is otherwise small.
        marks: Option<Box<RowMarks>>,
        /// Blocked: records accepted as blocked (left out of `totals`).
        accepted: u64,
        /// The Flows view's sections: the header counts flow records
        /// (`12 flows · 412 conns`), not peer groups.
        per_flow: bool,
    },
    Peer {
        direction: Direction,
        /// Draw `←` / `→` before the peer (`both`).
        arrow: bool,
        /// Boxed: a peer row is far larger than a header.
        row: Box<PeerRow>,
    },
    /// A flow record under its app (the Flows view's sections).
    Flow { row: Arc<FlowRow> },
}

/// One row of a sectioned table (App-to-app `in` / `out` / `both`,
/// Blocked, Flows by app): a header row per app, then its peers (or
/// flows) unless folded.
#[derive(Debug, Clone, PartialEq)]
pub struct SectionRow {
    pub app: Vec<String>,
    pub kind: RowKind,
    /// The section's header and every peer row, lowercased: `/` keeps a
    /// whole section when any of it matches.
    haystack: Arc<Haystack>,
}

/// What `/` searches in one section: its header and every row, lowercased.
/// A Flows section's text is written at its first search, not at every
/// regrouping (it covers every flow); App-to-app's is ready at once.
#[derive(Debug, Default)]
pub struct Haystack {
    text: std::sync::OnceLock<Box<str>>,
    /// A Flows section's header line and flows, until the text is built.
    head: String,
    flows: Vec<Arc<FlowRow>>,
}

impl Haystack {
    fn ready(text: &str) -> Self {
        Self {
            text: std::sync::OnceLock::from(lowercase(text.to_string()).into_boxed_str()),
            ..Self::default()
        }
    }

    fn of_flows(head: String, flows: Vec<Arc<FlowRow>>) -> Self {
        Self {
            head,
            flows,
            ..Self::default()
        }
    }

    pub fn text(&self) -> &str {
        self.text.get_or_init(|| {
            use std::fmt::Write as _;
            // Written in place, names borrowed (`display_name` without its
            // copy).
            fn name(e: &pce::traffic::ResolvedEndpoint) -> &str {
                [&e.hostname, &e.name, &e.fqdn, &e.ip]
                    .into_iter()
                    .flatten()
                    .find(|s| !s.is_empty())
                    .map_or("?", String::as_str)
            }
            let mut text = String::with_capacity(self.head.len() + self.flows.len() * 96);
            text.push_str(&self.head);
            for f in &self.flows {
                let _ = write!(
                    text,
                    "\n{} {} {} {} {} {}",
                    name(&f.src),
                    f.src_group,
                    name(&f.dst),
                    f.dst_group,
                    f.service_text(),
                    f.policy_decision.as_str()
                );
            }
            lowercase(text).into_boxed_str()
        })
    }
}

/// Two sections search alike when their texts do.
impl PartialEq for Haystack {
    fn eq(&self, other: &Self) -> bool {
        self.text() == other.text()
    }
}

/// `to_lowercase`, as the needle is; ASCII text in place.
pub(super) fn lowercase(mut text: String) -> String {
    if text.is_ascii() {
        text.make_ascii_lowercase();
        text
    } else {
        text.to_lowercase()
    }
}

impl SectionRow {
    pub fn is_header(&self) -> bool {
        matches!(self.kind, RowKind::Header { .. })
    }

    pub fn peer(&self) -> Option<(Direction, &PeerRow)> {
        match &self.kind {
            RowKind::Peer { direction, row, .. } => Some((*direction, row)),
            RowKind::Header { .. } | RowKind::Flow { .. } => None,
        }
    }

    /// Compare mode: a peer's or a flow's change; `None` on a header.
    pub fn change(&self) -> Option<&pce::traffic::RowChange> {
        match &self.kind {
            RowKind::Peer { row, .. } => row.change.as_ref(),
            RowKind::Flow { row } => row.change.as_ref(),
            RowKind::Header { .. } => None,
        }
    }

    /// The flow of a Flows section's row; `None` for a header or a peer.
    pub fn flow(&self) -> Option<&FlowRow> {
        match &self.kind {
            RowKind::Flow { row } => Some(row),
            RowKind::Header { .. } | RowKind::Peer { .. } => None,
        }
    }

    /// A header's one line: `▾ shop/prod  in · 7 groups · 412 conns`;
    /// `▹` when folded; `← 1545 conns · …  → 1004 conns` for `both`.
    /// `None` for a peer.
    pub fn header_line(&self) -> Option<String> {
        let RowKind::Header {
            totals,
            folded,
            accepted,
            per_flow,
            ..
        } = &self.kind
        else {
            return None;
        };
        let parts: Vec<(Direction, &Totals)> = totals.iter().map(|(d, t)| (*d, t)).collect();
        let accepted = if *accepted > 0 {
            format!(" · {accepted} accepted")
        } else {
            String::new()
        };
        let counts = match (per_flow, parts.as_slice()) {
            (true, [(_, t)]) => t.flows_text(),
            _ => app_header(&parts),
        };
        Some(format!(
            "{} {}  {counts}{accepted}",
            if *folded { "▹" } else { "▾" },
            self.app.join("/"),
        ))
    }

    fn peer_cell(&self) -> String {
        match &self.kind {
            RowKind::Peer {
                direction,
                arrow,
                row,
            } => {
                if *arrow {
                    format!("  {} {}", direction.arrow(), row.peer_text())
                } else {
                    format!("  {}", row.peer_text())
                }
            }
            RowKind::Header { .. } | RowKind::Flow { .. } => self.app.join("/"),
        }
    }

    pub(super) fn with_row<R: Default>(&self, f: impl Fn(&PeerRow) -> R) -> R {
        self.peer().map(|(_, r)| f(r)).unwrap_or_default()
    }
}

impl FlowCells for SectionRow {
    fn flow_row(&self) -> Option<&FlowRow> {
        self.flow()
    }
}

/// `/` on a sectioned table: the row's section matches.
pub(super) fn section_matches(r: &SectionRow, needle: &str) -> bool {
    needle.is_empty() || r.haystack.text().contains(needle)
}

/// `allowed`, `pot.blocked`, `blocked` or `unknown`.
pub(super) fn decision_text(d: pce::models::PolicyDecision) -> String {
    match d {
        pce::models::PolicyDecision::PotentiallyBlocked => "pot.blocked".into(),
        d => d.as_str().into(),
    }
}

/// A section's header row: the app, its totals per direction and its
/// rows' marks summed (`sum_marks`).
pub(super) fn header_row(
    app: &[String],
    totals: Vec<(Direction, Totals)>,
    folded: bool,
    marks: Option<RowMarks>,
    per_flow: bool,
) -> SectionRow {
    SectionRow {
        app: app.to_vec(),
        kind: RowKind::Header {
            totals,
            folded,
            marks: marks.map(Box::new),
            accepted: 0,
            per_flow,
        },
        haystack: Arc::default(),
    }
}

/// One section into `out`: its header, then (unless folded) its rows, all
/// sharing one search text (the header line and `texts`, lowercased), so
/// `/` keeps or drops the section whole.
pub(super) fn push_section(
    out: &mut Vec<SectionRow>,
    mut header: SectionRow,
    rows: Vec<SectionRow>,
    texts: impl Iterator<Item = String>,
) {
    let mut text = header.header_line().unwrap_or_default();
    for t in texts {
        text.push('\n');
        text.push_str(&t);
    }
    let hay = Arc::new(Haystack::ready(&text));
    let folded = matches!(header.kind, RowKind::Header { folded: true, .. });
    header.haystack = hay.clone();
    out.push(header);
    if !folded {
        out.extend(rows.into_iter().map(|mut r| {
            r.haystack = hay.clone();
            r
        }));
    }
}

/// The rows of `sections` (in table order): consecutive sections of one
/// app share a header (`both`); a folded app shows its header only.
pub fn section_rows(
    sections: &[AppSection],
    folded: &BTreeSet<Vec<String>>,
    arrows: bool,
) -> Vec<SectionRow> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < sections.len() {
        let app = sections[i].app.clone();
        let mut j = i;
        while j < sections.len() && sections[j].app == app {
            j += 1;
        }
        let group = &sections[i..j];
        let header = header_row(
            &app,
            group
                .iter()
                .map(|s| (s.direction, s.totals.clone()))
                .collect(),
            folded.contains(&app),
            sum_marks(
                group
                    .iter()
                    .flat_map(|s| &s.rows)
                    .map(|r| (r.flows, r.marks.as_ref())),
            ),
            false,
        );
        let peers: Vec<SectionRow> = group
            .iter()
            .flat_map(|s| {
                s.rows.iter().map(|r| SectionRow {
                    app: app.clone(),
                    kind: RowKind::Peer {
                        direction: s.direction,
                        arrow: arrows,
                        row: Box::new(r.clone()),
                    },
                    haystack: Arc::default(),
                })
            })
            .collect();
        let texts = group.iter().flat_map(|s| &s.rows).map(|r| {
            format!(
                "{} {} {} {} {}",
                r.peer_text(),
                r.peer_name_text(),
                r.app_name_text(),
                r.service,
                decision_text(r.dominant())
            )
        });
        push_section(&mut out, header, peers, texts);
        i = j;
    }
    out
}

/// One Flows section (`x`): its app's header and its flow records,
/// kept between a fold or `s` (which only lay the sections out again).
#[derive(Debug, Clone)]
pub struct FlowSection {
    pub section: AppSection,
    /// The flows in the table's order (shared with the flat table's rows).
    pub flows: Vec<Arc<FlowRow>>,
    /// The header's marks: the flows' hits, and the squelched records of
    /// the app left out (`hidden`).
    pub marks: Option<RowMarks>,
    /// What `/` searches: the header and every flow (`Haystack`).
    pub haystack: Arc<Haystack>,
}

impl FlowSection {
    pub fn new(section: AppSection, flows: Vec<Arc<FlowRow>>, marks: Option<RowMarks>) -> Self {
        let header = header_row(
            &section.app,
            vec![(section.direction, section.totals.clone())],
            false,
            None,
            true,
        );
        let haystack = Arc::new(Haystack::of_flows(
            header.header_line().unwrap_or_default(),
            flows.clone(),
        ));
        Self {
            section,
            flows,
            marks,
            haystack,
        }
    }
}

/// The rows of the Flows view's sections (in table order): each app's
/// header, then its flow records unless folded.
pub fn flow_section_rows(
    sections: &[FlowSection],
    folded: &BTreeSet<Vec<String>>,
) -> Vec<SectionRow> {
    let mut out = Vec::new();
    for fs in sections {
        let app = &fs.section.app;
        let is_folded = folded.contains(app);
        let mut header = header_row(
            app,
            vec![(fs.section.direction, fs.section.totals.clone())],
            is_folded,
            fs.marks.clone(),
            true,
        );
        header.haystack = fs.haystack.clone();
        out.push(header);
        if !is_folded {
            out.extend(fs.flows.iter().map(|f| SectionRow {
                app: app.clone(),
                kind: RowKind::Flow { row: f.clone() },
                haystack: fs.haystack.clone(),
            }));
        }
    }
    out
}

/// The search groups of a sectioned table: a section's rows share their
/// search text, so `/` decides once per section.
pub(super) fn section_group(r: &SectionRow) -> usize {
    Arc::as_ptr(&r.haystack) as *const u8 as usize
}

/// A flows cell: the records shown, then `~12` for the squelched ones of
/// the row left out (its sort key stays the shown count; `:export` writes
/// the count alone). With a hidden count both numbers are at most 4 cells
/// (`short_count`), so the text fits the 10-cell column marks give it.
pub(crate) fn flows_cell(flows: u64, m: Option<&RowMarks>) -> String {
    match m.map(|m| m.hidden).filter(|h| *h > 0) {
        Some(h) => format!("{} ~{}", short_count(flows), short_count(h)),
        None => flows.to_string(),
    }
}

/// The cells of the flows column below 120 columns while marks are active.
pub(crate) const FLOWS_NARROW_MARKED: usize = 7;

/// `flows_cell` for the narrow column: the hidden count is dropped (`1.2K
/// ~`) rather than cut into a wrong number when both do not fit.
pub(crate) fn flows_cell_narrow(flows: u64, m: Option<&RowMarks>) -> String {
    let full = flows_cell(flows, m);
    if full.chars().count() <= FLOWS_NARROW_MARKED || m.is_none_or(|m| m.hidden == 0) {
        full
    } else {
        format!("{} ~", short_count(flows))
    }
}

/// A count in at most 4 cells, truncated, never rounded up: `999`, `1.2K`,
/// `123K`, `9.9M`.
pub(super) fn short_count(n: u64) -> String {
    if n < 1_000 {
        return n.to_string();
    }
    let mut scale = 1_000u64;
    for unit in ['K', 'M', 'G', 'T', 'P'] {
        if n / scale < 10 {
            return format!("{}.{}{unit}", n / scale, n % scale / (scale / 10));
        }
        if n / scale < 1_000 {
            return format!("{}{unit}", n / scale);
        }
        scale *= 1_000;
    }
    format!("{}E", n / scale)
}

/// Bytes as `1.2K`, `-` when none were reported.
pub(super) fn bytes_text(b: u64) -> String {
    if b == 0 { "-".into() } else { compact_count(b) }
}

pub(super) fn no_sort(_: &SectionRow) -> SortKey {
    SortKey::Num(0)
}

/// App-to-app's `in` / `out` / `both` columns. Header rows span the
/// table (`SectionRow::header_line`); the cells are the peer rows'.
pub fn app_columns() -> Vec<Column<SectionRow>> {
    vec![
        Column {
            title: "peer",
            width: 0,
            cell: SectionRow::peer_cell,
            key: no_sort,
            right: false,
            hide_below: 0,
            narrow: None,
            narrow_width: 0,
            full_title: None,
            full: None,
        },
        Column {
            title: "hosts",
            width: 22,
            cell: |r| r.with_row(PeerRow::peer_name_text),
            key: no_sort,
            right: false,
            hide_below: 120,
            narrow: None,
            narrow_width: 0,
            full_title: None,
            full: None,
        },
        Column {
            title: "service",
            width: 10,
            cell: |r| r.with_row(|p| p.service.clone()),
            key: no_sort,
            right: false,
            hide_below: 0,
            narrow: None,
            narrow_width: 9,
            full_title: None,
            full: None,
        },
        Column {
            title: "flows",
            width: 6,
            cell: |r| r.with_row(|p| flows_cell(p.flows, p.marks.as_ref())),
            key: no_sort,
            right: true,
            hide_below: 0,
            narrow: None,
            narrow_width: 5,
            full_title: None,
            full: Some(|r| r.with_row(|p| p.flows.to_string())),
        },
        Column {
            title: "conns",
            width: 8,
            cell: |r| r.with_row(|p| p.connections.to_string()),
            key: no_sort,
            right: true,
            hide_below: 0,
            narrow: None,
            narrow_width: 6,
            full_title: None,
            full: None,
        },
        Column {
            title: "bytes",
            width: 8,
            cell: |r| r.with_row(|p| bytes_text(p.bytes)),
            key: no_sort,
            right: true,
            hide_below: 0,
            narrow: None,
            narrow_width: 6,
            full_title: None,
            full: None,
        },
        Column {
            title: "decision",
            width: 11,
            cell: |r| r.with_row(|p| decision_text(p.dominant())),
            key: no_sort,
            right: false,
            hide_below: 0,
            narrow: Some(|r| {
                r.with_row(|p| match p.dominant() {
                    pce::models::PolicyDecision::PotentiallyBlocked => "pot.blk".into(),
                    d => d.as_str().into(),
                })
            }),
            narrow_width: 8,
            full_title: None,
            full: None,
        },
    ]
}

/// Blocked's columns: App-to-app `in` with `blocked`, its peers the
/// sources, plus the destination host(s) and when the row was last seen.
pub fn blocked_columns() -> Vec<Column<SectionRow>> {
    vec![
        Column {
            title: "src",
            width: 0,
            cell: SectionRow::peer_cell,
            key: no_sort,
            right: false,
            hide_below: 0,
            narrow: None,
            narrow_width: 0,
            full_title: None,
            full: None,
        },
        Column {
            title: "dst name",
            width: 22,
            cell: |r| r.with_row(PeerRow::app_name_text),
            key: no_sort,
            right: false,
            hide_below: 0,
            narrow: None,
            narrow_width: 14,
            full_title: None,
            full: None,
        },
        Column {
            title: "service",
            width: 10,
            cell: |r| r.with_row(|p| p.service.clone()),
            key: no_sort,
            right: false,
            hide_below: 0,
            narrow: None,
            narrow_width: 9,
            full_title: None,
            full: None,
        },
        Column {
            title: "flows",
            width: 6,
            cell: |r| r.with_row(|p| flows_cell(p.flows, p.marks.as_ref())),
            key: no_sort,
            right: true,
            hide_below: 0,
            narrow: None,
            narrow_width: 5,
            full_title: None,
            full: Some(|r| r.with_row(|p| p.flows.to_string())),
        },
        Column {
            title: "conns",
            width: 7,
            cell: |r| r.with_row(|p| p.connections.to_string()),
            key: no_sort,
            right: true,
            hide_below: 0,
            narrow: None,
            narrow_width: 5,
            full_title: None,
            full: None,
        },
        Column {
            title: "last seen",
            width: 16,
            cell: |r| {
                r.with_row(|p| {
                    p.last_detected
                        .as_deref()
                        .map(short_time)
                        .unwrap_or_default()
                })
            },
            key: no_sort,
            right: false,
            hide_below: 0,
            narrow: Some(|r| {
                r.with_row(|p| {
                    p.last_detected
                        .as_deref()
                        .map(short_time_narrow)
                        .unwrap_or_default()
                })
            }),
            narrow_width: 11,
            full_title: None,
            full: None,
        },
    ]
}
