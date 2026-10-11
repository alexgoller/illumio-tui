//! All TUI state. No I/O, no terminal types beyond sizes.
use crate::tui::diff::ObjectDiff;
use crate::tui::filter::{FilterEditor, TrafficFilter};
use crate::tui::form::Form;
use crate::tui::picker::KeyPicker;
use crate::tui::table::{Column, SortKey, TableView};
use crate::tui::ui::theme::Theme;
use pce::impact::RuleActors;
use pce::models::{
    Actor, DenyRule, Event, EventFilter, IpList, Label, LabelGroup, PendingChange, PolicyScope,
    Rule, RuleSet, Service, VenHealth, Workload,
};
use pce::traffic::treemap::{Rect as TreemapRect, Tile, Weight, layout, layout_zoomed};
use pce::traffic::{
    BlockedGroup, FlowGroup, MeshColumns, MeshKey, MeshRow, ResolvedFlow, blocked_summary,
    group_by_labels, group_key, group_text, keys_or_default, keys_seen, mesh, mesh_key,
};
use pce::write::Plan;
use pce::{IpListSize, NameIndex, RuleKind, RuleSetView, RuleView, StatsSnapshot, ip_list_size};
use std::collections::{BTreeSet, HashMap, HashSet};
use std::time::{Duration, Instant};
use tokio_util::sync::CancellationToken;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Screen {
    Traffic,
    Policy,
    Objects,
    Workloads,
    Events,
}

impl Screen {
    /// In tab order; a screen's key is its position plus one.
    pub const ALL: [Screen; 5] = [
        Screen::Traffic,
        Screen::Policy,
        Screen::Objects,
        Screen::Workloads,
        Screen::Events,
    ];

    pub fn title(self) -> &'static str {
        match self {
            Screen::Traffic => "Traffic",
            Screen::Policy => "Policy",
            Screen::Objects => "Objects",
            Screen::Workloads => "Workloads",
            Screen::Events => "Events",
        }
    }

    /// The tab row's label when the full titles do not fit.
    pub fn short_title(self) -> &'static str {
        match self {
            Screen::Traffic => "Traf",
            Screen::Policy => "Pol",
            Screen::Objects => "Obj",
            Screen::Workloads => "Wl",
            Screen::Events => "Ev",
        }
    }

    pub fn index(self) -> usize {
        Screen::ALL.iter().position(|s| *s == self).unwrap_or(0)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProfileInfo {
    pub name: String,
    pub host: String,
    pub org_id: u64,
}

#[derive(Debug)]
pub enum Connection {
    Connecting,
    /// Health passed; the name index may still be loading (`index` None).
    Connected {
        index: Option<std::sync::Arc<pce::NameIndex>>,
        index_age_secs: u64,
    },
    Disconnected {
        error: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatusKind {
    Info,
    Warn,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatusLine {
    pub text: String,
    pub kind: StatusKind,
}

impl StatusLine {
    pub fn info(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            kind: StatusKind::Info,
        }
    }
    pub fn warn(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            kind: StatusKind::Warn,
        }
    }
    pub fn error(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            kind: StatusKind::Error,
        }
    }
}

/// Why writes are off when the session toggle is: at startup without
/// `--allow-writes`, and after `:writes off`.
pub const WRITES_OFF: &str = "writes off; :writes on to enable";

/// Whether editing keys may start a write flow. The guard itself lives in
/// the runtime (`Runtime::writer`); the model never holds it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WritesState {
    /// The session toggle on, the profile allowing writes and the audit sink writable.
    pub enabled: bool,
    /// Why not, for the status line.
    pub reason: Option<String>,
    /// A plan or a send is on its way: set when `Cmd::PlanWrite` or
    /// `Cmd::ExecuteWrite` is emitted, cleared when its answer lands (and by
    /// `Model::reset_data`). Editing keys, `:profile` and `:writes` wait for it.
    pub in_flight: bool,
}

impl WritesState {
    pub fn on() -> Self {
        Self {
            enabled: true,
            reason: None,
            in_flight: false,
        }
    }

    pub fn off(reason: impl Into<String>) -> Self {
        Self {
            enabled: false,
            reason: Some(reason.into()),
            in_flight: false,
        }
    }

    /// The status-line text for an editing key pressed while writes are off.
    pub fn refusal(&self) -> String {
        self.reason
            .clone()
            .unwrap_or_else(|| "writes are not enabled".into())
    }
}

/// A modal drawn on top of the current screen. Later tasks add `Error`, `Detail`.
#[derive(Debug, Clone)]
pub enum Overlay {
    None,
    Help,
    Prompt {
        input: String,
    },
    FilterEditor(FilterEditor),
    Confirm {
        reason: String,
        action: PendingAction,
    },
    /// `g`: the label keys every traffic view groups by.
    GroupKeys(KeyPicker),
    /// The selected policy rule in full.
    RuleDetail(RuleView),
    /// Pending (unprovisioned) changes; the list lives in `PolicyState::pending`.
    Pending,
    /// A failure too long for the status line, or an authentication failure.
    Error(String),
    /// Events filter: severity, type (local), window.
    EventsEditor {
        values: [String; 3],
        active: usize,
        error: Option<String>,
    },
    /// The selected audit event as pretty JSON; `scroll` is the first shown line.
    EventDetail {
        event: Event,
        scroll: usize,
    },
    /// Create or edit form for a label, IP list, service or workload.
    Form(Form),
    /// The exact request a write would send; `y` sends it, `n`/`Esc` cancel.
    /// `scroll` is the first body line shown.
    ConfirmWrite {
        plan: Plan,
        kind: pce::write::ObjectKind,
        scroll: u16,
    },
    /// A pending IP list or service against its active version.
    Diff {
        diff: ObjectDiff,
        scroll: u16,
    },
}

impl PartialEq for Overlay {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Overlay::None, Overlay::None)
            | (Overlay::Help, Overlay::Help)
            | (Overlay::Pending, Overlay::Pending) => true,
            (Overlay::Prompt { input: a }, Overlay::Prompt { input: b })
            | (Overlay::Error(a), Overlay::Error(b)) => a == b,
            (Overlay::GroupKeys(a), Overlay::GroupKeys(b)) => a == b,
            (Overlay::FilterEditor(a), Overlay::FilterEditor(b)) => a == b,
            (Overlay::Form(a), Overlay::Form(b)) => a == b,
            (
                Overlay::ConfirmWrite {
                    plan: pa,
                    kind: ka,
                    scroll: sa,
                },
                Overlay::ConfirmWrite {
                    plan: pb,
                    kind: kb,
                    scroll: sb,
                },
            ) => pa.request() == pb.request() && ka == kb && sa == sb,
            (
                Overlay::Confirm {
                    reason: ra,
                    action: aa,
                },
                Overlay::Confirm {
                    reason: rb,
                    action: ab,
                },
            ) => ra == rb && aa == ab,
            (
                Overlay::Diff {
                    diff: a,
                    scroll: sa,
                },
                Overlay::Diff {
                    diff: b,
                    scroll: sb,
                },
            ) => a == b && sa == sb,
            // `RuleView` is not `PartialEq`; a rule is identified by its href.
            (Overlay::RuleDetail(a), Overlay::RuleDetail(b)) => a.href == b.href,
            (
                Overlay::EventsEditor {
                    values: va,
                    active: aa,
                    error: ea,
                },
                Overlay::EventsEditor {
                    values: vb,
                    active: ab,
                    error: eb,
                },
            ) => va == vb && aa == ab && ea == eb,
            (
                Overlay::EventDetail {
                    event: a,
                    scroll: sa,
                },
                Overlay::EventDetail {
                    event: b,
                    scroll: sb,
                },
            ) => a.href == b.href && sa == sb,
            _ => false,
        }
    }
}

impl Eq for Overlay {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PendingAction {
    /// A new filter from the editor or a command: runs through the cache.
    RunQuery(TrafficFilter),
    /// `r` on a filter that did not produce the data on screen: bypasses the cache.
    RunNow(TrafficFilter),
}

#[derive(Debug)]
pub enum QueryState {
    Idle,
    Running {
        seq: u64,
        started: Instant,
        status: String,
        cancel: CancellationToken,
    },
    Failed(String),
}

#[derive(Debug)]
pub struct TrafficData {
    pub flows: Vec<ResolvedFlow>,
    pub truncated: bool,
    pub matches_count: Option<u64>,
    pub elapsed: Duration,
    pub cache_hit: bool,
    pub fetched_at: Instant,
    pub filter: TrafficFilter,
}

/// A flow with its grouping text for the keys in force (`shop/prod`): a
/// `Column`'s `fn` cannot see the keys, so the cells are computed when the
/// rows are built.
#[derive(Debug, Clone)]
pub struct FlowRow {
    pub flow: ResolvedFlow,
    pub src_group: String,
    pub dst_group: String,
}

impl FlowRow {
    pub fn new(flow: ResolvedFlow, keys: &[String]) -> Self {
        Self {
            src_group: group_text(&flow.src, keys),
            dst_group: group_text(&flow.dst, keys),
            flow,
        }
    }
}

impl std::ops::Deref for FlowRow {
    type Target = ResolvedFlow;

    fn deref(&self) -> &ResolvedFlow {
        &self.flow
    }
}

/// Last seen (from 140 columns), src, src group, dst, dst group, service,
/// conns, decision: the group columns fit at 80.
pub fn flow_columns() -> Vec<Column<FlowRow>> {
    vec![
        Column {
            title: "last seen",
            width: 16,
            cell: |f| {
                f.last_detected
                    .as_deref()
                    .map(short_time)
                    .unwrap_or_default()
            },
            key: |f| SortKey::Text(f.last_detected.clone().unwrap_or_default()),
            right: false,
            hide_below: 140,
            narrow: None,
            narrow_width: 0,
        },
        Column {
            title: "src",
            width: 19,
            cell: |f| f.src.display_name(),
            key: |f| SortKey::Text(f.src.display_name()),
            right: false,
            hide_below: 0,
            narrow: None,
            narrow_width: 14,
        },
        Column {
            title: "src group",
            width: 18,
            cell: |f| f.src_group.clone(),
            key: |f| SortKey::Text(f.src_group.clone()),
            right: false,
            hide_below: 0,
            narrow: None,
            narrow_width: 10,
        },
        Column {
            title: "dst",
            width: 19,
            cell: |f| f.dst.display_name(),
            key: |f| SortKey::Text(f.dst.display_name()),
            right: false,
            hide_below: 0,
            narrow: None,
            narrow_width: 14,
        },
        Column {
            title: "dst group",
            width: 18,
            cell: |f| f.dst_group.clone(),
            key: |f| SortKey::Text(f.dst_group.clone()),
            right: false,
            hide_below: 0,
            narrow: None,
            narrow_width: 10,
        },
        Column {
            title: "service",
            width: 10,
            cell: |f| f.service_text(),
            key: |f| SortKey::Num(u64::from(f.port.unwrap_or(0))),
            right: false,
            hide_below: 0,
            narrow: None,
            narrow_width: 9,
        },
        Column {
            title: "conns",
            width: 7,
            cell: |f| f.num_connections.to_string(),
            key: |f| SortKey::Num(f.num_connections),
            right: true,
            hide_below: 0,
            narrow: None,
            narrow_width: 5,
        },
        Column {
            title: "decision",
            width: 19,
            cell: |f| f.policy_decision.as_str().to_string(),
            key: |f| SortKey::Text(f.policy_decision.as_str().to_string()),
            right: false,
            hide_below: 0,
            narrow: Some(|f| match f.policy_decision.as_str() {
                "potentially_blocked" => "pot.blk".to_string(),
                other => other.to_string(),
            }),
            narrow_width: 8,
        },
    ]
}

/// `2026-10-10T07:31:02Z` -> `10-10 07:31:02`
pub fn short_time(s: &str) -> String {
    s.get(5..19)
        .map(|x| x.replace('T', " "))
        .unwrap_or_else(|| s.to_string())
}

/// `2026-10-10T07:31:02Z` -> `10-10 07:31`
fn short_time_narrow(s: &str) -> String {
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
}

impl TrafficView {
    /// `v`: Flows, App-to-app, Mesh, Blocked, Treemap, then Flows again.
    pub fn next(self) -> Self {
        match self {
            TrafficView::Flows => TrafficView::Aggregate,
            TrafficView::Aggregate => TrafficView::Mesh,
            TrafficView::Mesh => TrafficView::Blocked,
            TrafficView::Blocked => TrafficView::Treemap,
            TrafficView::Treemap => TrafficView::Flows,
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            TrafficView::Flows => "Flows",
            TrafficView::Aggregate => "App-to-app",
            TrafficView::Mesh => "Mesh",
            TrafficView::Blocked => "Blocked",
            TrafficView::Treemap => "Treemap",
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
    /// Some loaded flow carried byte counts: `b` may weigh by bytes, and
    /// the meta draws `b bytes` dim when not.
    pub has_bytes: bool,
    /// Where the tiles go: the Traffic table area minus its border for the
    /// terminal size (`ui::screens::traffic::treemap_area`).
    pub area: TreemapRect,
}

impl TreemapState {
    pub fn new(area: TreemapRect) -> Self {
        Self {
            weight: Weight::Connections,
            zoom: None,
            cursor: 0,
            tiles: Vec::new(),
            has_bytes: false,
            area,
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
fn mesh_decision(r: &MeshRow) -> String {
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
        },
    ];
    out.extend(rest);
    out
}

pub fn group_columns() -> Vec<Column<FlowGroup>> {
    vec![
        Column {
            title: "src",
            width: 26,
            cell: |g| g.src.join("/"),
            key: |g| SortKey::Text(g.src.join("/")),
            right: false,
            hide_below: 0,
            narrow: None,
            narrow_width: 16,
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
        },
        Column {
            title: "flows",
            width: 6,
            cell: |g| g.flows.to_string(),
            key: |g| SortKey::Num(g.flows),
            right: true,
            hide_below: 0,
            narrow: None,
            narrow_width: 5,
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
        },
    ]
}

pub fn blocked_columns() -> Vec<Column<BlockedGroup>> {
    vec![
        Column {
            title: "last seen",
            width: 16,
            cell: |b| {
                b.last_detected
                    .as_deref()
                    .map(short_time)
                    .unwrap_or_default()
            },
            key: |b| SortKey::Text(b.last_detected.clone().unwrap_or_default()),
            right: false,
            hide_below: 0,
            narrow: Some(|b| {
                b.last_detected
                    .as_deref()
                    .map(short_time_narrow)
                    .unwrap_or_default()
            }),
            narrow_width: 11,
        },
        Column {
            title: "dst",
            width: 18,
            cell: |b| b.dst_group.join("/"),
            key: |b| SortKey::Text(b.dst_group.join("/")),
            right: false,
            hide_below: 0,
            narrow: None,
            narrow_width: 12,
        },
        Column {
            title: "dst name",
            width: 22,
            cell: blocked_dst_name,
            key: |b| SortKey::Text(blocked_dst_name(b)),
            right: false,
            hide_below: 0,
            narrow: None,
            narrow_width: 14,
        },
        Column {
            title: "service",
            width: 10,
            cell: |b| match b.port {
                Some(p) => format!("{p}/{}", b.proto),
                None => b.proto.clone(),
            },
            key: |b| SortKey::Num(u64::from(b.port.unwrap_or(0))),
            right: false,
            hide_below: 0,
            narrow: None,
            narrow_width: 9,
        },
        Column {
            title: "flows",
            width: 6,
            cell: |b| b.flows.to_string(),
            key: |b| SortKey::Num(b.flows),
            right: true,
            hide_below: 0,
            narrow: None,
            narrow_width: 5,
        },
        Column {
            title: "conns",
            width: 7,
            cell: |b| b.connections.to_string(),
            key: |b| SortKey::Num(b.connections),
            right: true,
            hide_below: 0,
            narrow: None,
            narrow_width: 5,
        },
        Column {
            title: "sources",
            width: 0,
            cell: blocked_sources,
            key: |b| SortKey::Text(blocked_sources(b)),
            right: false,
            hide_below: 0,
            narrow: None,
            narrow_width: 0,
        },
    ]
}

/// The destination's name when the row has one, else `N hosts`.
fn blocked_dst_name(b: &BlockedGroup) -> String {
    match b.dst_names.as_slice() {
        [one] if b.dst_hosts <= 1 => one.clone(),
        _ => format!("{} hosts", b.dst_hosts),
    }
}

/// `shop/dev, hr/prod +3`: the first source groups, then how many more.
fn blocked_sources(b: &BlockedGroup) -> String {
    let mut s = b.sources.join(", ");
    let more = b.source_count.saturating_sub(b.sources.len() as u64);
    if more > 0 {
        s.push_str(&format!(" +{more}"));
    }
    s
}

/// Opt-in periodic refresh. Never below 60 s nor the profile minimum.
#[derive(Debug, Clone)]
pub struct AutoRefresh {
    pub enabled: bool,
    pub interval: Duration,
    pub last_run: Option<Instant>,
    /// The request in flight was started by the auto-refresh tick.
    pub from_tick: bool,
    /// A failed tick switched auto-refresh off; a manual `r` re-arms it.
    pub tripped: bool,
}

impl AutoRefresh {
    pub fn new(min_seconds: u64) -> Self {
        Self {
            enabled: false,
            interval: Duration::from_secs(min_seconds.max(60)),
            last_run: None,
            from_tick: false,
            tripped: false,
        }
    }

    /// A failed auto-refresh request: disable, never retry unattended.
    /// Returns whether this failure was the tick's own.
    pub fn take_tick_failure(&mut self) -> bool {
        let was = std::mem::take(&mut self.from_tick);
        if was {
            self.enabled = false;
            self.tripped = true;
        }
        was
    }

    /// `a`: switches auto-refresh on or off (clearing a tripped state) and
    /// returns the status line to show.
    pub fn toggle(&mut self) -> StatusLine {
        self.enabled = !self.enabled;
        self.tripped = false;
        if self.enabled {
            StatusLine::info(format!("auto-refresh every {}s", self.interval.as_secs()))
        } else {
            StatusLine::info("auto-refresh off")
        }
    }

    /// A manual refresh re-enables what a failed tick switched off.
    pub fn rearm(&mut self) {
        if std::mem::take(&mut self.tripped) {
            self.enabled = true;
        }
    }

    /// Panel meta suffix: the countdown while on, the reason after a failed tick.
    pub fn meta(&self, now: Instant) -> String {
        if self.enabled {
            format!(" · auto in {}s", self.remaining(now).as_secs())
        } else if self.tripped {
            " · auto off · failed".into()
        } else {
            String::new()
        }
    }

    pub fn due(&self, now: Instant) -> bool {
        self.enabled
            && self
                .last_run
                .is_none_or(|l| now.saturating_duration_since(l) >= self.interval)
    }

    pub fn remaining(&self, now: Instant) -> Duration {
        self.last_run
            .map(|l| {
                self.interval
                    .saturating_sub(now.saturating_duration_since(l))
            })
            .unwrap_or(Duration::ZERO)
    }
}

#[derive(Debug)]
pub struct TrafficState {
    pub filter: TrafficFilter,
    pub query: QueryState,
    pub data: Option<TrafficData>,
    pub seq: u64,
    pub flows: TableView<FlowRow>,
    pub show_detail: bool,
    pub view: TrafficView,
    /// The label keys every view groups by: the profile's scope keys at
    /// startup and after `:profile`, then whatever `g` chose. Not persisted.
    pub group_keys: Vec<String>,
    /// `g` chose `all labels`: every key seen in the loaded flows.
    pub all_keys: bool,
    /// The keys in force, resolved by `rebuild_views` (the `all labels` scan
    /// runs there, never per draw): what every view and column groups by.
    pub keys: Vec<String>,
    pub groups: TableView<FlowGroup>,
    pub mesh: TableView<MeshRow>,
    /// The Mesh view's optional columns (`u`, `p`); both shown at first.
    pub mesh_cols: MeshColumns,
    pub blocked: TableView<BlockedGroup>,
    pub treemap: TreemapState,
    /// What the Flows view is narrowed to.
    pub drill: Option<Drill>,
    pub auto: AutoRefresh,
    /// A submitted `w` form while its plan is on the way: a planner error
    /// (an unknown ruleset name) brings it back with the error.
    pub rule_form: Option<Form>,
}

/// Both optional Mesh columns shown.
pub const MESH_ALL: MeshColumns = MeshColumns {
    user: true,
    process: true,
};

/// The keys `all labels` lists first when the flows carry them.
pub const PREFERRED_KEYS: [&str; 4] = ["app", "env", "role", "loc"];

/// What `Enter` narrowed the Flows view to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Drill {
    /// An App-to-app or treemap pair: source and destination group keys.
    Pair(Vec<String>, Vec<String>),
    /// A Mesh row, under the columns it was grouped with.
    Mesh { key: MeshKey, cols: MeshColumns },
}

impl Drill {
    /// Whether `f` belongs to the drilled row under `keys`.
    pub fn matches(&self, f: &ResolvedFlow, keys: &[String]) -> bool {
        match self {
            Drill::Pair(src, dst) => {
                &group_key(&f.src, keys) == src && &group_key(&f.dst, keys) == dst
            }
            Drill::Mesh { key, cols } => &mesh_key(f, keys, *cols) == key,
        }
    }

    /// The Flows title after `Flows ▸ `.
    pub fn title(&self) -> String {
        match self {
            Drill::Pair(src, dst) => format!("{} → {}", src.join("/"), dst.join("/")),
            Drill::Mesh { key, .. } => format!(
                "{} → {} {}",
                key.src.join("/"),
                key.dst.join("/"),
                key.service_text()
            ),
        }
    }
}

impl TrafficState {
    pub fn new(window: &str, max_results: u32, auto_min_seconds: u64, keys: &[String]) -> Self {
        Self {
            filter: TrafficFilter::defaults(window, max_results),
            query: QueryState::Idle,
            data: None,
            seq: 0,
            flows: TableView::new(flow_columns()),
            show_detail: false,
            view: TrafficView::Flows,
            group_keys: keys_or_default(keys),
            all_keys: false,
            keys: keys_or_default(keys),
            groups: TableView::new(group_columns()),
            mesh: TableView::new(mesh_columns(MESH_ALL)),
            mesh_cols: MESH_ALL,
            blocked: TableView::new(blocked_columns()),
            treemap: TreemapState::new(TreemapRect::default()),
            drill: None,
            auto: AutoRefresh::new(auto_min_seconds),
            rule_form: None,
        }
    }

    /// The loaded flows (none before the first query).
    pub fn loaded_flows(&self) -> &[ResolvedFlow] {
        self.data.as_ref().map_or(&[], |d| d.flows.as_slice())
    }

    /// The keys in force, as the last `rebuild_views` resolved them.
    pub fn effective_keys(&self) -> Vec<String> {
        self.keys.clone()
    }

    /// Every key seen in the loaded flows when `all_keys` (`app`, `env`,
    /// `role`, `loc` first), else `group_keys` (`app,env` when empty).
    fn resolve_keys(&self) -> Vec<String> {
        if self.all_keys {
            let preferred = PREFERRED_KEYS.map(String::from);
            let seen: Vec<String> = keys_seen(self.loaded_flows(), &preferred)
                .into_iter()
                .map(|(k, _)| k)
                .collect();
            if !seen.is_empty() {
                return seen;
            }
        }
        keys_or_default(&self.group_keys)
    }

    /// Whether any loaded flow carries a byte count.
    pub fn has_bytes(&self) -> bool {
        self.loaded_flows().iter().any(ResolvedFlow::has_bytes)
    }

    /// The first loaded flow `pred` accepts that is blocked or potentially
    /// blocked, else the first it accepts at all: what `w` drafts a rule
    /// from on an aggregated row.
    pub fn first_flow(&self, pred: impl Fn(&ResolvedFlow) -> bool) -> Option<ResolvedFlow> {
        let flows = self.loaded_flows();
        flows
            .iter()
            .filter(|f| pred(f))
            .find(|f| {
                matches!(
                    f.policy_decision,
                    pce::models::PolicyDecision::Blocked
                        | pce::models::PolicyDecision::PotentiallyBlocked
                )
            })
            .or_else(|| flows.iter().find(|f| pred(f)))
            .cloned()
    }

    /// Recomputes every view from `data` with the keys in force. Pure;
    /// never requests.
    pub fn rebuild_views(&mut self) {
        self.keys = self.resolve_keys();
        let keys = &self.keys;
        let flows = self.loaded_flows();
        let shown: Vec<FlowRow> = flows
            .iter()
            .filter(|f| self.drill.as_ref().is_none_or(|d| d.matches(f, keys)))
            .map(|f| FlowRow::new(f.clone(), keys))
            .collect();
        let groups = group_by_labels(flows, keys);
        let mesh_rows = mesh(flows, keys, self.mesh_cols);
        let blocked = blocked_summary(flows, keys);
        self.flows.set_rows(shown);
        self.groups.set_rows(groups);
        self.mesh.set_rows(mesh_rows);
        self.blocked.set_rows(blocked);
        self.relayout_treemap();
    }

    /// The treemap's tiles again from the current groups, zoom, weight and
    /// area. A zoom whose source is gone is dropped; byte weight falls back
    /// to connections when no flow carries bytes; the cursor is clamped.
    pub fn relayout_treemap(&mut self) {
        self.treemap.has_bytes = self.has_bytes();
        if !self.treemap.has_bytes {
            self.treemap.weight = Weight::Connections;
        }
        let groups = self.groups.rows();
        let tm = &mut self.treemap;
        if tm
            .zoom
            .as_ref()
            .is_some_and(|z| !groups.iter().any(|g| &g.src == z))
        {
            tm.zoom = None;
        }
        tm.tiles = match &tm.zoom {
            Some(z) => layout_zoomed(groups, z, tm.area, tm.weight),
            None => layout(groups, tm.area, tm.weight),
        };
        tm.cursor = tm.cursor.min(tm.targets().len().saturating_sub(1));
    }

    /// `u` / `p` on the Mesh view: show or hide a column, regroup, and start
    /// again at the first row (a new table: sort and search reset too).
    pub fn toggle_mesh_column(&mut self, user: bool) {
        if user {
            self.mesh_cols.user = !self.mesh_cols.user;
        } else {
            self.mesh_cols.process = !self.mesh_cols.process;
        }
        self.mesh = TableView::new(mesh_columns(self.mesh_cols));
        self.rebuild_views();
    }

    /// Whether the active view's table is capturing keys for a search.
    pub fn is_searching(&self) -> bool {
        match self.view {
            TrafficView::Flows => self.flows.is_searching(),
            TrafficView::Aggregate => self.groups.is_searching(),
            TrafficView::Mesh => self.mesh.is_searching(),
            TrafficView::Blocked => self.blocked.is_searching(),
            TrafficView::Treemap => false,
        }
    }

    pub fn is_running(&self) -> bool {
        matches!(self.query, QueryState::Running { .. })
    }

    /// Cancels a running query (if any) and returns the next sequence number.
    pub fn start_query(&mut self, now: Instant) -> (u64, CancellationToken) {
        if let QueryState::Running { cancel, .. } = &self.query {
            cancel.cancel();
        }
        self.seq += 1;
        self.auto.last_run = Some(now);
        self.auto.from_tick = false;
        let cancel = CancellationToken::new();
        self.query = QueryState::Running {
            seq: self.seq,
            started: now,
            status: "submitting".into(),
            cancel: cancel.clone(),
        };
        (self.seq, cancel)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PolicyPane {
    RuleSets,
    Rules,
}

fn plain_names<'a>(v: impl Iterator<Item = &'a str>) -> String {
    v.collect::<Vec<_>>().join(", ")
}

fn actor_names(v: &[pce::ResolvedActor]) -> String {
    plain_names(v.iter().map(|a| a.name.as_str()))
}

fn service_names(v: &[pce::ResolvedService]) -> String {
    plain_names(v.iter().map(|s| s.name.as_str()))
}

/// Evaluation order of the rule kinds: an override deny beats an allow,
/// which beats a plain deny.
fn rule_rank(kind: RuleKind) -> u8 {
    match kind {
        RuleKind::OverrideDeny => 0,
        RuleKind::Allow => 1,
        RuleKind::Deny => 2,
    }
}

pub fn rule_kind_name(kind: RuleKind) -> &'static str {
    match kind {
        RuleKind::Allow => "allow",
        RuleKind::Deny => "deny",
        RuleKind::OverrideDeny => "override_deny",
    }
}

/// A rule on one line as the rules pane names it: `allow role=db ← role=web
/// PostgreSQL` (kind, providers ← consumers, services), ` (disabled)` when off.
pub fn rule_line(r: &RuleView) -> String {
    let mut s = format!(
        "{} {} ← {} {}",
        rule_kind_name(r.kind),
        actor_names(&r.providers),
        actor_names(&r.consumers),
        service_names(&r.services)
    );
    if !r.enabled {
        s.push_str(" (disabled)");
    }
    s
}

fn on_off(enabled: bool) -> String {
    if enabled { "✓".into() } else { "✗".into() }
}

pub fn ruleset_columns() -> Vec<Column<RuleSetView>> {
    vec![
        Column {
            title: "ruleset",
            width: 22,
            cell: |r| r.name.clone(),
            key: |r| SortKey::Text(r.name.clone()),
            right: false,
            hide_below: 0,
            narrow: None,
            narrow_width: 0,
        },
        Column {
            title: "on",
            width: 3,
            cell: |r| on_off(r.enabled),
            key: |r| SortKey::Num(u64::from(r.enabled)),
            right: false,
            hide_below: 0,
            narrow: None,
            narrow_width: 0,
        },
        Column {
            title: "scopes",
            width: 0,
            cell: |r| r.scopes.join(" | "),
            key: |r| SortKey::Text(r.scopes.join(" | ")),
            right: false,
            hide_below: 0,
            narrow: None,
            narrow_width: 0,
        },
        Column {
            title: "rules",
            width: 5,
            cell: |r| r.rule_count.to_string(),
            key: |r| SortKey::Num(r.rule_count as u64),
            right: true,
            hide_below: 0,
            narrow: None,
            narrow_width: 0,
        },
        Column {
            title: "pending",
            width: 7,
            cell: |r| r.update_type.clone().unwrap_or_default(),
            key: |r| SortKey::Text(r.update_type.clone().unwrap_or_default()),
            right: false,
            hide_below: 70,
            narrow: None,
            narrow_width: 0,
        },
    ]
}

/// A rules-pane row: the resolved rule, its impact over the cached workloads
/// (`None` until they are cached) and the `P/C` cell made from it (`12/3`,
/// `1.2K/5.7K` from 1000 on, `-`). Both are computed once, when the rows are built: a column's `fn`
/// pointer cannot see the workload cache, and the rule detail reads the
/// stored impact instead of evaluating it on every draw.
#[derive(Debug, Clone)]
pub struct RuleRow {
    pub view: RuleView,
    pub impact: Option<pce::Impact>,
    pub pc: String,
}

impl RuleRow {
    /// Sort key of the `P/C` column: providers, then consumers (`≥` sorts by
    /// its number); rows without an impact last.
    pub(crate) fn pc_key(&self) -> u64 {
        self.impact.as_ref().map_or(u64::MAX, |i| {
            ((i.providers as u64) << 32) | (i.consumers as u64 & 0xFFFF_FFFF)
        })
    }
}

impl std::ops::Deref for RuleRow {
    type Target = RuleView;

    fn deref(&self) -> &RuleView {
        &self.view
    }
}

pub fn rule_columns() -> Vec<Column<RuleRow>> {
    vec![
        Column {
            title: "kind",
            width: 13,
            cell: |r| rule_kind_name(r.kind).to_string(),
            key: |r| SortKey::Text(rule_kind_name(r.kind).to_string()),
            right: false,
            hide_below: 0,
            narrow: None,
            narrow_width: 0,
        },
        Column {
            title: "on",
            width: 3,
            cell: |r| on_off(r.enabled),
            key: |r| SortKey::Num(u64::from(r.enabled)),
            right: false,
            hide_below: 0,
            narrow: None,
            narrow_width: 0,
        },
        Column {
            title: "consumers",
            width: 0,
            cell: |r| actor_names(&r.consumers),
            key: |r| SortKey::Text(actor_names(&r.consumers)),
            right: false,
            hide_below: 0,
            narrow: None,
            narrow_width: 0,
        },
        Column {
            title: "providers",
            width: 0,
            cell: |r| actor_names(&r.providers),
            key: |r| SortKey::Text(actor_names(&r.providers)),
            right: false,
            hide_below: 0,
            narrow: None,
            narrow_width: 0,
        },
        Column {
            title: "services",
            width: 0,
            cell: |r| service_names(&r.services),
            key: |r| SortKey::Text(service_names(&r.services)),
            right: false,
            hide_below: 0,
            narrow: None,
            narrow_width: 0,
        },
        Column {
            title: "P/C",
            // `Impact::pc_compact`'s widest form, `≥123K/≥456K`.
            width: 11,
            cell: |r| r.pc.clone(),
            key: |r| SortKey::Num(r.pc_key()),
            right: true,
            hide_below: 0,
            narrow: None,
            narrow_width: 0,
        },
        Column {
            title: "pending",
            width: 7,
            cell: |r| r.update_type.clone().unwrap_or_default(),
            key: |r| SortKey::Text(r.update_type.clone().unwrap_or_default()),
            right: false,
            hide_below: 90,
            narrow: None,
            narrow_width: 0,
        },
    ]
}

#[derive(Debug)]
pub struct PolicyState {
    pub scope: PolicyScope,
    pub pane: PolicyPane,
    pub rulesets: TableView<RuleSetView>,
    pub rules: TableView<RuleRow>,
    pub raw: Vec<RuleSet>,
    /// Deny rules fetched from the endpoint for rulesets whose inline list is empty.
    pub fallback_deny: HashMap<String, Vec<DenyRule>>,
    /// Rulesets whose deny-rule fallback was already requested (once per load).
    pub deny_requested: HashSet<String>,
    pub loaded_at: Option<Instant>,
    pub age_secs: u64,
    pub loading: bool,
    /// Set by an explicit `r`: the next ruleset reload also bypasses the cache for deny rules.
    pub bypass_deny: bool,
    pub pending: Option<Vec<PendingChange>>,
    /// A pending-changes request is in flight (`p` does not send another).
    pub pending_loading: bool,
    /// The highlighted entry of the pending overlay.
    pub pending_selected: usize,
    /// A diff request is in flight (`Enter` does not send another).
    pub diff_loading: bool,
    /// The next rulesets result was asked for by a write: it keeps the
    /// write's report in the status line instead of `N rulesets`.
    pub quiet_reload: bool,
    /// The open rule confirm's impact line, evaluated when the confirm
    /// opens and when the workloads or rulesets load (never per draw).
    pub confirm_impact: Option<String>,
    /// Pending entries marked with `Space` for `P` (hrefs); cleared when the
    /// list is loaded again.
    pub pending_marked: BTreeSet<String>,
    /// The counts of the provision just sent (`ip_lists=1, rule_sets=2`),
    /// taken from its body at `y`: the PCE's answer does not repeat them.
    pub provision_counts: Option<String>,
    /// The report of the provision just done; the pending reload it caused
    /// shows it in front of the new count.
    pub provisioned: Option<String>,
    /// A `w` rule waiting for the ruleset `Ctrl-N` creates: planned into
    /// it once that write is done, dropped by any failure or cancel.
    /// The form comes along so a failed rule plan can reopen it.
    pub rule_after_ruleset: Option<(pce::write::RuleDraft, crate::tui::form::Form)>,
    pub auto: AutoRefresh,
}

/// One loaded rule that names an IP list, for the IP lists tab.
#[derive(Debug, Clone)]
pub struct IpListUse {
    pub ruleset: String,
    pub kind: RuleKind,
    pub providers: Vec<Actor>,
    pub consumers: Vec<Actor>,
}

impl PolicyState {
    pub fn new(min_refresh: u64) -> Self {
        Self {
            scope: PolicyScope::Draft,
            pane: PolicyPane::RuleSets,
            rulesets: TableView::new(ruleset_columns()),
            rules: TableView::new(rule_columns()),
            raw: Vec::new(),
            fallback_deny: HashMap::new(),
            deny_requested: HashSet::new(),
            loaded_at: None,
            age_secs: 0,
            loading: false,
            bypass_deny: false,
            pending: None,
            pending_loading: false,
            pending_selected: 0,
            diff_loading: false,
            quiet_reload: false,
            confirm_impact: None,
            pending_marked: BTreeSet::new(),
            provision_counts: None,
            provisioned: None,
            rule_after_ruleset: None,
            auto: AutoRefresh::new(min_refresh),
        }
    }

    /// Whether the focused pane's table is capturing keys for a search.
    pub fn is_searching(&self) -> bool {
        match self.pane {
            PolicyPane::RuleSets => self.rulesets.is_searching(),
            PolicyPane::Rules => self.rules.is_searching(),
        }
    }

    pub fn set_rule_sets(
        &mut self,
        raw: Vec<RuleSet>,
        idx: &NameIndex,
        workloads: Option<(&[Workload], bool)>,
    ) {
        self.rulesets.set_rows(
            raw.iter()
                .map(|rs| RuleSetView::from_rule_set(rs, idx))
                .collect(),
        );
        self.raw = raw;
        self.rebuild_rules(idx, workloads);
    }

    /// Forgets everything loaded (a scope switch shows nothing of the old scope).
    pub fn clear(&mut self) {
        self.rulesets.set_rows(Vec::new());
        self.rules.set_rows(Vec::new());
        self.raw.clear();
        self.fallback_deny.clear();
        self.deny_requested.clear();
        self.loaded_at = None;
        self.age_secs = 0;
        self.bypass_deny = false;
        self.pending_loading = false;
    }

    /// Rules of the selected ruleset in evaluation order: override deny rules,
    /// then allow rules, then deny rules (inline or fetched fallback). The
    /// order within each group is the PCE's. `workloads` (the cache, when
    /// loaded) fills the `P/C` column.
    pub fn rebuild_rules(&mut self, idx: &NameIndex, workloads: Option<(&[Workload], bool)>) {
        let Some(href) = self.selected_href() else {
            self.rules.set_rows(Vec::new());
            return;
        };
        let Some(rs) = self.raw.iter().find(|r| r.href == href) else {
            self.rules.set_rows(Vec::new());
            return;
        };
        // The workloads' label sets are built once for all the rules.
        let warm = workloads.map(|(w, truncated)| (w, truncated, pce::workload_labels(w)));
        let cells = |a: &RuleActors| {
            let impact = warm.as_ref().map(|(w, truncated, labels)| {
                crate::tui::impact::compute_with(&rs.scopes, a, idx, w, *truncated, labels)
            });
            (
                impact
                    .as_ref()
                    .map_or_else(|| "-".to_string(), pce::Impact::pc_compact),
                impact,
            )
        };
        let mut rows: Vec<RuleRow> = rs
            .rules
            .iter()
            .map(|r| {
                let (pc, impact) = cells(&RuleActors::of_rule(r));
                RuleRow {
                    view: RuleView::from_rule(r, idx),
                    impact,
                    pc,
                }
            })
            .collect();
        let deny: &[DenyRule] = if rs.deny_rules.is_empty() {
            self.fallback_deny.get(&href).map_or(&[], Vec::as_slice)
        } else {
            &rs.deny_rules
        };
        rows.extend(deny.iter().map(|d| {
            let (pc, impact) = cells(&RuleActors::of_deny(d));
            RuleRow {
                view: RuleView::from_deny(d, idx),
                impact,
                pc,
            }
        }));
        rows.sort_by_key(|r| rule_rank(r.kind));
        self.rules.set_rows(rows);
    }

    pub fn selected_href(&self) -> Option<String> {
        self.rulesets.selected_item().map(|r| r.href.clone())
    }

    /// The selected ruleset as loaded, for an edit form.
    pub fn selected_rule_set(&self) -> Option<&RuleSet> {
        let href = self.selected_href()?;
        self.raw.iter().find(|r| r.href == href)
    }

    /// An allow rule by href, with its ruleset's href.
    pub fn find_rule(&self, href: &str) -> Option<(Rule, String)> {
        self.raw.iter().find_map(|rs| {
            rs.rules
                .iter()
                .find(|r| r.href == href)
                .map(|r| (r.clone(), rs.href.clone()))
        })
    }

    /// A deny rule by href (inline, or fetched from the fallback endpoint),
    /// with its ruleset's href.
    pub fn find_deny_rule(&self, href: &str) -> Option<(DenyRule, String)> {
        self.raw
            .iter()
            .find_map(|rs| {
                rs.deny_rules
                    .iter()
                    .find(|d| d.href == href)
                    .map(|d| (d.clone(), rs.href.clone()))
            })
            .or_else(|| {
                self.fallback_deny.iter().find_map(|(rs, list)| {
                    list.iter()
                        .find(|d| d.href == href)
                        .map(|d| (d.clone(), rs.clone()))
                })
            })
    }

    /// The rules of the loaded draft rulesets (allow, deny, fetched deny)
    /// that name the IP list `href` on either side; `None` until draft
    /// rulesets are loaded. Costs no request.
    pub fn ip_list_uses(&self, href: &str) -> Option<Vec<IpListUse>> {
        if self.scope != PolicyScope::Draft || self.loaded_at.is_none() {
            return None;
        }
        let names = |v: &[Actor]| {
            v.iter()
                .any(|a| a.ip_list.as_ref().is_some_and(|r| r.href == href))
        };
        let mut out = Vec::new();
        for rs in &self.raw {
            for r in &rs.rules {
                if names(&r.providers) || names(&r.consumers) {
                    out.push(IpListUse {
                        ruleset: rs.name.clone(),
                        kind: RuleKind::Allow,
                        providers: r.providers.clone(),
                        consumers: r.consumers.clone(),
                    });
                }
            }
            let deny: &[DenyRule] = if rs.deny_rules.is_empty() {
                self.fallback_deny.get(&rs.href).map_or(&[], Vec::as_slice)
            } else {
                &rs.deny_rules
            };
            for d in deny {
                if names(&d.providers) || names(&d.consumers) {
                    out.push(IpListUse {
                        ruleset: rs.name.clone(),
                        kind: if d.override_ {
                            RuleKind::OverrideDeny
                        } else {
                            RuleKind::Deny
                        },
                        providers: d.providers.clone(),
                        consumers: d.consumers.clone(),
                    });
                }
            }
        }
        Some(out)
    }

    /// The selected ruleset has no inline deny rules and its endpoint list was
    /// neither fetched nor requested: the rules pane may be missing deny rules.
    pub fn deny_not_loaded(&self) -> bool {
        let Some(href) = self.selected_href() else {
            return false;
        };
        self.raw
            .iter()
            .find(|r| r.href == href)
            .is_some_and(|r| r.deny_rules.is_empty())
            && !self.fallback_deny.contains_key(&href)
            && !self.deny_requested.contains(&href)
    }

    /// The selected ruleset's href when its inline deny list is empty and the
    /// endpoint has not been asked yet. Marks it as requested.
    pub fn take_deny_fallback(&mut self) -> Option<String> {
        let href = self.selected_href()?;
        let inline_empty = self
            .raw
            .iter()
            .find(|r| r.href == href)
            .is_some_and(|r| r.deny_rules.is_empty());
        (inline_empty && self.deny_requested.insert(href.clone())).then_some(href)
    }
}

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

pub fn label_columns() -> Vec<Column<Label>> {
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
            hide_below: 0,
            narrow: None,
            narrow_width: 0,
        },
        Column {
            title: "href",
            width: 24,
            cell: |l| l.href.clone(),
            key: |l| SortKey::Text(l.href.clone()),
            right: false,
            hide_below: 70,
            narrow: None,
            narrow_width: 0,
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
        },
    ]
}

#[derive(Debug)]
pub struct ObjectsState {
    pub kind: ObjectKind,
    pub labels: TableView<Label>,
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
}

impl ObjectsState {
    pub fn new(min_refresh: u64) -> Self {
        Self {
            kind: ObjectKind::Labels,
            labels: TableView::new(label_columns()),
            groups: TableView::new(group_columns_objects()),
            services: TableView::new(service_columns()),
            ip_lists: IpListsState::new(min_refresh),
            ages: [None; 3],
            loaded_at: [None; 3],
            loading: [false; 3],
            show_detail: false,
            index_failed: false,
            auto: AutoRefresh::new(min_refresh),
        }
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
}

impl WorkloadRowView {
    pub fn new(workload: Workload, now: jiff::Timestamp) -> Self {
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
            health,
            workload,
        }
    }
}

type RowCell = fn(&WorkloadRowView) -> String;
type RowKey = fn(&WorkloadRowView) -> SortKey;

fn text_col(
    title: &'static str,
    width: u16,
    cell: RowCell,
    key: RowKey,
) -> Column<WorkloadRowView> {
    Column {
        title,
        width,
        cell,
        key,
        right: false,
        hide_below: 0,
        narrow: None,
        narrow_width: 0,
    }
}

fn hostname_of(r: &WorkloadRowView) -> String {
    r.workload.hostname.clone().unwrap_or_default()
}

fn name_of(r: &WorkloadRowView) -> String {
    r.workload.name.clone().unwrap_or_default()
}

fn ip_of(r: &WorkloadRowView) -> String {
    r.workload.primary_ip().unwrap_or("").to_string()
}

fn enforcement_of(r: &WorkloadRowView) -> String {
    r.workload.enforcement_mode.clone().unwrap_or_default()
}

fn visibility_of(r: &WorkloadRowView) -> String {
    r.workload.visibility_level.clone().unwrap_or_default()
}

fn online_of(r: &WorkloadRowView) -> String {
    match r.workload.online {
        Some(true) => "yes",
        Some(false) => "no",
        None => "-",
    }
    .to_string()
}

fn agent_of(r: &WorkloadRowView) -> String {
    r.workload.agent_status().unwrap_or_else(|| "-".into())
}

/// Index of the `health` column in `workload_columns()`.
const HEALTH_COL: usize = 8;

pub fn workload_columns() -> Vec<Column<WorkloadRowView>> {
    // Below 120 columns only hostname, ip, enforcement, heartbeat and health
    // stay, so the table is readable at 80.
    let wide = |c: Column<WorkloadRowView>| Column {
        hide_below: 120,
        ..c
    };
    vec![
        Column {
            // A workload without a hostname is still identifiable when the name column is hidden.
            narrow: Some(|r| r.workload.display_name()),
            narrow_width: 20,
            ..text_col("hostname", 22, hostname_of, |r| {
                SortKey::Text(hostname_of(r))
            })
        },
        wide(text_col("name", 14, name_of, |r| SortKey::Text(name_of(r)))),
        text_col("ip", 15, ip_of, |r| SortKey::Text(ip_of(r))),
        wide(text_col(
            "labels",
            0,
            |r| r.workload.label_text(),
            |r| SortKey::Text(r.workload.label_text()),
        )),
        Column {
            narrow: Some(|r| enforcement_of(r).replace("visibility_only", "visibility")),
            narrow_width: 11,
            ..text_col("enforcement", 15, enforcement_of, |r| {
                SortKey::Text(enforcement_of(r))
            })
        },
        Column {
            // Secondary: gives way to the health columns until the terminal is very wide.
            hide_below: 180,
            ..text_col("visibility", 13, visibility_of, |r| {
                SortKey::Text(visibility_of(r))
            })
        },
        wide(text_col("online", 6, online_of, |r| {
            SortKey::Num(u64::from(r.workload.online.unwrap_or(false)))
        })),
        wide(text_col("agent", 9, agent_of, |r| {
            SortKey::Text(agent_of(r))
        })),
        text_col(
            "health",
            9,
            |r| r.health.as_str().to_string(),
            |r| SortKey::Num(u64::from(r.health.rank())),
        ),
        text_col(
            "heartbeat",
            9,
            |r| r.heartbeat.clone(),
            |r| SortKey::Num(r.heartbeat_secs.unwrap_or(u64::MAX)),
        ),
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
    ]
}

#[derive(Debug)]
pub struct WorkloadsState {
    pub table: TableView<WorkloadRowView>,
    /// The workloads as fetched; `table` is a projection of these.
    pub raw: Vec<Workload>,
    /// The last read was cut short: impact counts are lower bounds.
    pub truncated: bool,
    /// Why the last read failed, until one succeeds (`impact: unavailable`).
    pub error: Option<String>,
    pub unhealthy_only: bool,
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
}

impl WorkloadsState {
    pub fn new(min_refresh: u64) -> Self {
        Self {
            table: TableView::new(workload_columns()),
            raw: Vec::new(),
            truncated: false,
            error: None,
            unhealthy_only: false,
            now: None,
            counts: [0; 5],
            age_secs: 0,
            loaded_at: None,
            loading: false,
            show_detail: false,
            index_failed: false,
            auto: AutoRefresh::new(min_refresh),
        }
    }

    /// The cached workloads and their `truncated` flag, once loaded.
    pub fn impact_source(&self) -> Option<(&[Workload], bool)> {
        self.loaded_at
            .is_some()
            .then_some((self.raw.as_slice(), self.truncated))
    }

    /// Recomputes the table rows (and health counts) from `raw`.
    pub fn rebuild(&mut self) {
        let now = self.now.unwrap_or_else(jiff::Timestamp::now);
        let views: Vec<WorkloadRowView> = self
            .raw
            .iter()
            .cloned()
            .map(|w| WorkloadRowView::new(w, now))
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
        let rows = if self.unhealthy_only {
            views.into_iter().filter(|v| unhealthy(v.health)).collect()
        } else {
            views
        };
        self.table.set_rows(rows);
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
    }
}

/// The severity column, the target of `S`.
const SEVERITY_COL: usize = 1;

pub fn event_columns() -> Vec<Column<Event>> {
    vec![
        Column {
            title: "timestamp",
            width: 20,
            cell: |e| {
                e.timestamp
                    .as_deref()
                    .map(|t| t.get(..19).unwrap_or(t).replace('T', " "))
                    .unwrap_or_default()
            },
            key: |e| SortKey::Text(e.timestamp.clone().unwrap_or_default()),
            right: false,
            hide_below: 0,
            narrow: Some(|e| {
                e.timestamp
                    .as_deref()
                    .map(|t| t.get(5..19).unwrap_or(t).replace('T', " "))
                    .unwrap_or_default()
            }),
            narrow_width: 14,
        },
        Column {
            title: "severity",
            width: 8,
            cell: |e| e.severity.clone().unwrap_or_default(),
            key: |e| SortKey::Num(u64::from(e.severity_rank())),
            right: false,
            hide_below: 0,
            narrow: None,
            narrow_width: 0,
        },
        Column {
            title: "type",
            width: 28,
            cell: |e| e.event_type.clone().unwrap_or_default(),
            key: |e| SortKey::Text(e.event_type.clone().unwrap_or_default()),
            right: false,
            hide_below: 0,
            narrow: None,
            narrow_width: 24,
        },
        Column {
            title: "actor",
            width: 22,
            cell: |e| e.actor(),
            key: |e| SortKey::Text(e.actor()),
            right: false,
            hide_below: 0,
            narrow: None,
            narrow_width: 20,
        },
        Column {
            title: "resource",
            width: 0,
            cell: |e| e.resource(),
            key: |e| SortKey::Text(e.resource()),
            right: false,
            // Secondary: only on wide terminals.
            hide_below: 120,
            narrow: None,
            narrow_width: 0,
        },
        Column {
            title: "status",
            width: 8,
            cell: |e| e.status.clone().unwrap_or_default(),
            key: |e| SortKey::Text(e.status.clone().unwrap_or_default()),
            right: false,
            hide_below: 100,
            narrow: None,
            narrow_width: 0,
        },
    ]
}

/// What the events editor holds: the server-side severity and window, and the
/// local `type` substring.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EventsFilterEdit {
    pub severity: String,
    pub event_type: String,
    pub window: String,
}

impl EventsFilterEdit {
    /// The request filter. `since` is rounded down to the whole minute and
    /// `until` is left open, so applying the same filter again within the cache
    /// TTL produces the same query (and a cache hit) instead of a new one every second.
    pub fn to_filter(&self, now: jiff::Timestamp) -> Result<EventFilter, String> {
        let w = pce::traffic::parse_window(&self.window).map_err(|e| format!("window: {e}"))?;
        let span = jiff::Span::try_from(w).map_err(|e| format!("window: {e}"))?;
        let minute = jiff::Timestamp::from_second(now.as_second().div_euclid(60) * 60)
            .map_err(|e| e.to_string())?;
        let since = minute
            .checked_sub(span)
            .map_err(|e| format!("window: {e}"))?;
        Ok(EventFilter {
            severity: Some(self.severity.trim().to_string()).filter(|s| !s.is_empty()),
            event_type: None,
            status: None,
            since: Some(since),
            until: None,
            max_results: None,
        })
    }
}

#[derive(Debug)]
pub struct EventsState {
    pub table: TableView<Event>,
    /// The events as fetched; `table` is the local `type` projection of these.
    pub raw: Vec<Event>,
    pub filter: EventsFilterEdit,
    /// The `since` and `until` of the last answered request (`until` is empty when open).
    pub window: Option<(String, String)>,
    pub loaded_at: Option<Instant>,
    pub age_secs: u64,
    pub loading: bool,
    pub auto: AutoRefresh,
}

impl EventsState {
    pub fn new(min_refresh: u64) -> Self {
        Self {
            table: TableView::new(event_columns()),
            raw: Vec::new(),
            filter: EventsFilterEdit {
                severity: String::new(),
                event_type: String::new(),
                window: "24h".into(),
            },
            window: None,
            loaded_at: None,
            age_secs: 0,
            loading: false,
            auto: AutoRefresh::new(min_refresh),
        }
    }

    /// Applies the local type substring filter.
    pub fn rebuild(&mut self) {
        let needle = self.filter.event_type.trim().to_ascii_lowercase();
        let rows: Vec<Event> = self
            .raw
            .iter()
            .filter(|e| {
                needle.is_empty()
                    || e.event_type
                        .as_deref()
                        .unwrap_or("")
                        .to_ascii_lowercase()
                        .contains(&needle)
            })
            .cloned()
            .collect();
        self.table.set_rows(rows);
    }

    /// `S`: worst severity first; pressing it again flips the direction.
    pub fn sort_by_severity(&mut self) {
        let desc = self.table.sort() != Some((SEVERITY_COL, true));
        self.table.sort_by(SEVERITY_COL, desc);
    }
}

/// Limits copied from the resolved profile so `update` never touches config.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Limits {
    pub auto_refresh_min_seconds: u64,
    pub traffic_max_results: u32,
    pub traffic_default_window: String,
    /// Label keys a ruleset scope is made of (`app`, `env`).
    pub scope_keys: Vec<String>,
    /// The label key that names a tier (`role`).
    pub tier_key: String,
}

#[derive(Debug)]
pub struct Model {
    pub profile: ProfileInfo,
    pub theme_name: String,
    pub theme: Theme,
    pub limits: Limits,
    pub screen: Screen,
    pub overlay: Overlay,
    pub traffic: TrafficState,
    pub policy: PolicyState,
    pub objects: ObjectsState,
    pub workloads: WorkloadsState,
    pub events: EventsState,
    pub connection: Connection,
    pub stats: StatsSnapshot,
    pub size: (u16, u16),
    pub focused: bool,
    pub status: StatusLine,
    /// Set at startup, on every profile switch and on `:writes on|off` from
    /// the runtime's guard.
    pub writes: WritesState,
    /// First help line shown; `Overlay::Help` is reopened at the top.
    pub help_scroll: usize,
    /// Number of background tasks in flight; the runtime ticks at 250 ms while > 0.
    pub busy: usize,
    /// A `:profile` switch is being built off the event loop; a second one
    /// waits until it lands so two switches cannot race.
    pub switching: bool,
    pub quit: bool,
    pub started: Instant,
}

impl Model {
    pub fn new_connecting(
        profile: ProfileInfo,
        theme_name: String,
        auto_refresh_min_seconds: u64,
        traffic_max_results: u32,
        traffic_default_window: String,
        writes: WritesState,
    ) -> Self {
        let theme = Theme::from_name(&theme_name);
        let scope_keys: Vec<String> = crate::config::DEFAULT_SCOPE_KEYS.map(String::from).to_vec();
        let size = (80, 24);
        let mut traffic = TrafficState::new(
            &traffic_default_window,
            traffic_max_results,
            auto_refresh_min_seconds,
            &scope_keys,
        );
        traffic.treemap.area = crate::tui::ui::screens::traffic::treemap_area(size);
        let policy = PolicyState::new(auto_refresh_min_seconds);
        let objects = ObjectsState::new(auto_refresh_min_seconds);
        let workloads = WorkloadsState::new(auto_refresh_min_seconds);
        let events = EventsState::new(auto_refresh_min_seconds);
        Self {
            profile,
            theme_name,
            theme,
            // The defaults; `app::run` and a profile switch set the profile's.
            limits: Limits {
                auto_refresh_min_seconds,
                traffic_max_results,
                traffic_default_window,
                scope_keys,
                tier_key: crate::config::DEFAULT_TIER_KEY.into(),
            },
            screen: Screen::Traffic,
            overlay: Overlay::None,
            traffic,
            policy,
            objects,
            workloads,
            events,
            connection: Connection::Connecting,
            stats: StatsSnapshot { calls: 0, bytes: 0 },
            size,
            focused: true,
            status: StatusLine::info("connecting…"),
            writes,
            help_scroll: 0,
            busy: 0,
            switching: false,
            quit: false,
            started: Instant::now(),
        }
    }

    pub fn set_theme(&mut self, name: &str) {
        self.theme_name = name.to_string();
        self.theme = Theme::from_name(name);
    }

    /// The profile's label model: the `w` form's dimensions, and the keys
    /// every traffic view groups by until `g` changes them.
    pub fn set_label_model(&mut self, scope_keys: Vec<String>, tier_key: String) {
        self.traffic.group_keys = keys_or_default(&scope_keys);
        self.traffic.all_keys = false;
        self.traffic.rebuild_views();
        self.limits.scope_keys = scope_keys;
        self.limits.tier_key = tier_key;
    }

    /// What a `w` form is pre-filled from: the profile's label model and
    /// the draft rulesets the Policy screen loaded (none for the active scope).
    pub fn flow_prefill(&self) -> crate::tui::edit::FlowPrefill<'_> {
        crate::tui::edit::FlowPrefill {
            scope_keys: &self.limits.scope_keys,
            tier_key: &self.limits.tier_key,
            rulesets: if self.policy.scope == PolicyScope::Draft {
                &self.policy.raw
            } else {
                &[]
            },
        }
    }

    pub fn index(&self) -> Option<&std::sync::Arc<pce::NameIndex>> {
        match &self.connection {
            Connection::Connected { index, .. } => index.as_ref(),
            _ => None,
        }
    }

    /// Rebuilds the rules pane from what is loaded, its `P/C` column from
    /// the cached workloads (`-` without them). No request.
    pub fn rebuild_rules(&mut self) {
        let Some(idx) = self.index().cloned() else {
            return;
        };
        let workloads = self.workloads.impact_source();
        self.policy.rebuild_rules(&idx, workloads);
    }

    /// Evaluates the open confirm's impact line into `policy.confirm_impact`
    /// (`None` unless a rule write is on screen). No request.
    pub fn refresh_confirm_impact(&mut self) {
        self.policy.confirm_impact = match &self.overlay {
            Overlay::ConfirmWrite { plan, .. } => crate::tui::impact::confirm_line(self, plan),
            _ => None,
        };
    }

    /// Whether any screen has opted in to auto-refresh (drives the idle tick).
    pub fn any_auto_enabled(&self) -> bool {
        self.traffic.auto.enabled
            || self.policy.auto.enabled
            || self.objects.auto.enabled
            || self.objects.ip_lists.auto.enabled
            || self.workloads.auto.enabled
            || self.events.auto.enabled
    }

    pub fn is_connected(&self) -> bool {
        matches!(self.connection, Connection::Connected { .. })
    }

    /// Forgets everything loaded from the previous PCE: a running query is
    /// cancelled, every screen starts empty with auto-refresh off, counters and
    /// overlay are cleared. The traffic filter returns to the defaults in `limits`.
    pub fn reset_data(&mut self) {
        if let QueryState::Running { cancel, .. } = &self.traffic.query {
            cancel.cancel();
        }
        let min = self.limits.auto_refresh_min_seconds;
        let mut traffic = TrafficState::new(
            &self.limits.traffic_default_window,
            self.limits.traffic_max_results,
            min,
            &self.limits.scope_keys,
        );
        traffic.seq = self.traffic.seq + 1; // a late result of the old query is stale
        traffic.treemap.area = crate::tui::ui::screens::traffic::treemap_area(self.size);
        self.traffic = traffic;
        self.policy = PolicyState::new(min);
        self.objects = ObjectsState::new(min);
        self.workloads = WorkloadsState::new(min);
        self.events = EventsState::new(min);
        self.stats = StatsSnapshot { calls: 0, bytes: 0 };
        self.busy = 0;
        self.overlay = Overlay::None;
        self.writes.in_flight = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(s: &str) -> jiff::Timestamp {
        s.parse().unwrap()
    }

    #[test]
    fn events_window_is_whole_minutes_and_open_ended_so_the_cache_can_hit() {
        let edit = EventsFilterEdit {
            severity: " err ".into(),
            event_type: "agent".into(),
            window: "24h".into(),
        };
        let a = edit.to_filter(at("2026-10-10T08:00:07Z")).unwrap();
        let b = edit.to_filter(at("2026-10-10T08:00:52Z")).unwrap();
        assert_eq!(a.to_query(), b.to_query(), "same minute, same query");
        assert_eq!(
            a.to_query(),
            vec![
                ("severity", "err".to_string()),
                ("timestamp[gte]", "2026-10-09T08:00:00Z".to_string()),
            ],
            "severity trimmed, `until` left open, the local type not sent"
        );
        let c = edit.to_filter(at("2026-10-10T08:01:00Z")).unwrap();
        assert_ne!(
            a.to_query(),
            c.to_query(),
            "the next minute moves the window"
        );
    }

    #[test]
    fn events_empty_severity_is_none_and_a_bad_window_is_an_error() {
        let mut edit = EventsFilterEdit {
            severity: "  ".into(),
            event_type: String::new(),
            window: "1h".into(),
        };
        assert!(
            edit.to_filter(at("2026-10-10T08:00:00Z"))
                .unwrap()
                .severity
                .is_none()
        );
        edit.window = "soon".into();
        assert!(
            edit.to_filter(at("2026-10-10T08:00:00Z"))
                .unwrap_err()
                .starts_with("window:")
        );
    }

    #[test]
    fn an_event_without_a_timestamp_renders_blank() {
        let e: Event = serde_json::from_str(r#"{"href":"/orgs/5/events/x"}"#).unwrap();
        for c in event_columns() {
            assert!((c.cell)(&e).len() < 5, "{}", c.title);
        }
        assert_eq!((event_columns()[0].cell)(&e), "");
    }

    #[test]
    fn rules_are_found_by_href_inline_or_in_the_deny_fallback() {
        let mut p = PolicyState::new(60);
        p.raw = serde_json::from_str(include_str!("../../../../fixtures/rule_sets.json")).unwrap();
        let (r, rs) = p
            .find_rule("/orgs/5/sec_policy/draft/rule_sets/100/sec_rules/2")
            .unwrap();
        assert_eq!(r.description.as_deref(), Some("corp to web"));
        assert_eq!(rs, "/orgs/5/sec_policy/draft/rule_sets/100");
        let (d, _) = p
            .find_deny_rule("/orgs/5/sec_policy/draft/rule_sets/100/deny_rules/8")
            .unwrap();
        assert!(d.override_);
        assert!(
            p.find_rule("/orgs/5/sec_policy/draft/rule_sets/100/deny_rules/8")
                .is_none()
        );
        let fetched: DenyRule = serde_json::from_value(serde_json::json!({
            "href": "/orgs/5/sec_policy/draft/rule_sets/101/deny_rules/9",
            "providers": [{"actors": "ams"}], "consumers": [{"actors": "ams"}],
            "ingress_services": [{"port": 23, "proto": 6}]
        }))
        .unwrap();
        p.fallback_deny.insert(
            "/orgs/5/sec_policy/draft/rule_sets/101".into(),
            vec![fetched],
        );
        let (_, rs) = p
            .find_deny_rule("/orgs/5/sec_policy/draft/rule_sets/101/deny_rules/9")
            .unwrap();
        assert_eq!(rs, "/orgs/5/sec_policy/draft/rule_sets/101");
    }

    #[test]
    fn the_flow_and_blocked_columns_fit_80_columns_with_the_group_columns() {
        use crate::tui::ui::widgets::table::visible_columns;
        let row_width = |cols: &[(usize, u16, bool)]| -> u16 {
            cols.iter().map(|c| c.1).sum::<u16>() + cols.len() as u16 - 1 + 2
        };
        let flows = flow_columns();
        let at80 = visible_columns(&flows, 80);
        let titles: Vec<&str> = at80.iter().map(|c| flows[c.0].title).collect();
        assert_eq!(
            titles,
            vec![
                "src",
                "src group",
                "dst",
                "dst group",
                "service",
                "conns",
                "decision"
            ]
        );
        assert!(
            row_width(&at80) <= 78,
            "inside the border at 80: {}",
            row_width(&at80)
        );
        assert!(row_width(&visible_columns(&flows, 120)) <= 118);
        let at160 = visible_columns(&flows, 160);
        assert_eq!(flows[at160[0].0].title, "last seen");
        assert!(row_width(&at160) <= 158);
        let blocked = blocked_columns();
        let titles: Vec<&str> = blocked.iter().map(|c| c.title).collect();
        assert_eq!(
            titles,
            vec![
                "last seen",
                "dst",
                "dst name",
                "service",
                "flows",
                "conns",
                "sources"
            ]
        );
    }

    #[test]
    fn blocked_cells_name_one_host_or_count_them_and_count_more_sources() {
        let flows = crate::tui::test_support::fixture_flows_rich();
        let keys = vec!["app".to_string(), "env".to_string()];
        let rows = pce::traffic::blocked_summary(&flows, &keys);
        let cols = blocked_columns();
        let shop = rows
            .iter()
            .find(|r| r.dst_group == vec!["shop", "prod"])
            .unwrap();
        assert_eq!((cols[1].cell)(shop), "shop/prod");
        assert_eq!((cols[2].cell)(shop), "shop-db-p.example");
        assert_eq!((cols[6].cell)(shop), "hr/prod, shop/dev, ipl:Corp/-");
        let mut many = shop.clone();
        many.dst_hosts = 8;
        many.source_count = 10;
        many.sources = vec!["a".into(), "b".into(), "c".into(), "d".into(), "e".into()];
        assert_eq!((cols[2].cell)(&many), "8 hosts");
        assert_eq!((cols[6].cell)(&many), "a, b, c, d, e +5");
    }

    #[test]
    fn mesh_columns_follow_the_toggles_and_fit_80_columns() {
        use crate::tui::ui::widgets::table::visible_columns;
        let titles = |cols: MeshColumns| -> Vec<&'static str> {
            mesh_columns(cols).iter().map(|c| c.title).collect()
        };
        assert_eq!(
            titles(MESH_ALL),
            vec![
                "user", "process", "src", "dst", "service", "dst proc", "conns", "decision"
            ]
        );
        assert_eq!(
            titles(MeshColumns::default()),
            vec!["src", "dst", "service", "dst proc", "conns", "decision"]
        );
        let cols = mesh_columns(MESH_ALL);
        let at80 = visible_columns(&cols, 80);
        let width: u16 = at80.iter().map(|c| c.1).sum::<u16>() + at80.len() as u16 - 1 + 2;
        assert!(width <= 78, "{width}");
        assert!(
            !at80.iter().any(|c| cols[c.0].title == "dst proc"),
            "from 120 columns"
        );
        let at160 = visible_columns(&cols, 160);
        let width: u16 = at160.iter().map(|c| c.1).sum::<u16>() + at160.len() as u16 - 1 + 2;
        assert!(width <= 158, "{width}");
        let rows = mesh(
            &crate::tui::test_support::fixture_flows_rich(),
            &["app".to_string(), "env".to_string()],
            MESH_ALL,
        );
        let pb = rows
            .iter()
            .find(|r| r.dominant() == pce::models::PolicyDecision::PotentiallyBlocked)
            .unwrap();
        assert_eq!((cols[7].cell)(pb), "pot.blocked");
        assert_eq!((cols[7].narrow.unwrap())(pb), "pot.blk");
        let icmp = rows.iter().find(|r| r.proto == "icmp").unwrap();
        assert_eq!((cols[1].cell)(icmp), "-", "no process");
    }

    #[test]
    fn the_label_model_sets_the_group_keys_and_empty_falls_back_to_app_env() {
        let mut m = crate::tui::test_support::model_fixture();
        assert_eq!(m.traffic.group_keys, vec!["app", "env"]);
        m.set_label_model(vec!["site".into(), "zone".into()], "tier".into());
        assert_eq!(m.traffic.group_keys, vec!["site", "zone"]);
        assert_eq!(m.limits.tier_key, "tier");
        m.set_label_model(Vec::new(), "role".into());
        assert_eq!(m.traffic.group_keys, vec!["app", "env"]);
        assert!(
            m.limits.scope_keys.is_empty(),
            "the profile value itself is kept"
        );
    }

    #[test]
    fn ip_list_rows_carry_their_size_and_how_many_loaded_rules_use_them() {
        let lists: Vec<IpList> =
            serde_json::from_str(include_str!("../../../../fixtures/ip_lists.json")).unwrap();
        let mut s = IpListsState::new(60);
        let mut p = PolicyState::new(60);
        s.set_lists(lists, &p);
        assert_eq!(s.table.rows()[1].size.display(), "65.5K");
        assert_eq!(s.table.rows()[1].used_by, None, "no rulesets loaded");
        p.raw = serde_json::from_str(include_str!("../../../../fixtures/rule_sets.json")).unwrap();
        p.loaded_at = Some(Instant::now());
        s.recount(&p);
        let used: Vec<Option<usize>> = s.table.rows().iter().map(|r| r.used_by).collect();
        assert_eq!(used, vec![Some(1), Some(1)], "deny rule 7 and allow rule 2");
        let uses = p
            .ip_list_uses("/orgs/5/sec_policy/draft/ip_lists/21")
            .unwrap();
        assert_eq!(uses[0].ruleset, "Shop-Prod");
        assert_eq!(uses[0].kind, RuleKind::Allow);
        p.scope = PolicyScope::Active;
        s.recount(&p);
        assert_eq!(s.table.rows()[0].used_by, None, "only draft rulesets count");
        let cols = ip_list_columns();
        let titles: Vec<&str> = cols.iter().map(|c| c.title).collect();
        assert_eq!(
            titles,
            vec!["name", "size", "ranges", "fqdns", "used by", "pending"]
        );
        let rows = s.table.rows();
        assert_eq!((cols[1].cell)(&rows[0]), "any");
        assert_eq!((cols[1].cell)(&rows[1]), "65.5K");
        assert_eq!((cols[2].cell)(&rows[0]), "2");
        assert_eq!((cols[3].cell)(&rows[1]), "1");
        assert_eq!((cols[4].cell)(&rows[0]), "-");
        assert!(
            (cols[1].key)(&rows[0]) > (cols[1].key)(&rows[1]),
            "any sorts last"
        );
        assert_eq!(Screen::ALL.len(), 5);
        assert_eq!(Screen::Events.index(), 4);
        assert_eq!(Screen::Objects.title(), "Objects");
        assert_eq!(ObjectKind::ALL.len(), 4);
        assert_eq!(ObjectKind::IpLists.title(), "IP lists");
        assert_eq!(ObjectKind::IpLists.index(), 3);
        assert_eq!(ObjectKind::Labels.prev(), ObjectKind::IpLists);
        assert_eq!(ObjectKind::Services.next(), ObjectKind::IpLists);
        assert_eq!(ObjectKind::IpLists.next(), ObjectKind::Labels);
        assert!(!ObjectKind::FETCHED.contains(&ObjectKind::IpLists));
    }

    #[test]
    fn ip_list_uses_match_the_provider_side_and_fetched_deny_rules() {
        let mut sets: Vec<serde_json::Value> =
            serde_json::from_str(include_str!("../../../../fixtures/rule_sets.json")).unwrap();
        let deny_on_20 = sets[0]["deny_rules"][0].clone();
        let mut edge = sets[1].clone();
        edge["name"] = "Edge".into();
        edge["href"] = "/orgs/5/sec_policy/draft/rule_sets/102".into();
        edge["rules"][0]["providers"] =
            serde_json::json!([{"ip_list": {"href": "/orgs/5/sec_policy/draft/ip_lists/21"}}]);
        sets.push(edge);
        let mut p = PolicyState::new(60);
        p.raw = serde_json::from_value(serde_json::Value::Array(sets)).unwrap();
        p.loaded_at = Some(Instant::now());
        p.fallback_deny.insert(
            "/orgs/5/sec_policy/draft/rule_sets/102".into(),
            vec![serde_json::from_value(deny_on_20).unwrap()],
        );
        let corp = p
            .ip_list_uses("/orgs/5/sec_policy/draft/ip_lists/21")
            .unwrap();
        let corp: Vec<(&str, RuleKind)> =
            corp.iter().map(|u| (u.ruleset.as_str(), u.kind)).collect();
        assert_eq!(
            corp,
            vec![("Shop-Prod", RuleKind::Allow), ("Edge", RuleKind::Allow)],
            "Edge names the list as a provider"
        );
        let any = p
            .ip_list_uses("/orgs/5/sec_policy/draft/ip_lists/20")
            .unwrap();
        let any: Vec<(&str, RuleKind)> = any.iter().map(|u| (u.ruleset.as_str(), u.kind)).collect();
        assert_eq!(
            any,
            vec![("Shop-Prod", RuleKind::Deny), ("Edge", RuleKind::Deny)],
            "the second deny rule comes from fallback_deny"
        );
    }
}
