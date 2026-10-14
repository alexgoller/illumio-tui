//! All TUI state. No I/O, no terminal types beyond sizes.
use crate::cli::output::join_list;
use crate::tui::app_picker::AppPicker;
use crate::tui::app_view::AppView;
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
    AppSection, Decisions, Direction, FlowGroup, Focus, FocusDirection, GroupKey, MeshColumns,
    MeshKey, MeshRow, PeerRow, ResolvedFlow, Totals, app_header, app_sections, app_sections_both,
    apps_in, compact_count, group_by_labels, group_by_service, group_key, group_text, in_app,
    in_peer, keys_or_default, keys_seen, mesh, mesh_key,
};
use pce::write::Plan;
use pce::{
    IpListSize, KeyOrder, NameIndex, RuleKind, RuleSetView, RuleView, StatsSnapshot, ip_list_size,
};
use std::collections::{BTreeSet, HashMap, HashSet};
use std::sync::Arc;
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
    /// `F` without an app under the cursor, `:focus` alone: the apps of the
    /// loaded result.
    AppPicker(AppPicker),
    /// `A` / `:app`: the app overlay; its state is `Model::app_view`.
    App,
    /// `f` on the Labels tab: the label keys it shows.
    LabelKeys(KeyPicker),
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
            | (Overlay::Pending, Overlay::Pending)
            | (Overlay::App, Overlay::App) => true,
            (Overlay::Prompt { input: a }, Overlay::Prompt { input: b })
            | (Overlay::Error(a), Overlay::Error(b)) => a == b,
            (Overlay::GroupKeys(a), Overlay::GroupKeys(b))
            | (Overlay::LabelKeys(a), Overlay::LabelKeys(b)) => a == b,
            (Overlay::FilterEditor(a), Overlay::FilterEditor(b)) => a == b,
            (Overlay::AppPicker(a), Overlay::AppPicker(b)) => a == b,
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
    /// `u` on the Labels tab: the workloads and the rulesets not cached
    /// yet, read with their screens' own loads.
    LoadLabelUsage,
    /// `r` on a partial focus past a cost threshold: one query per side.
    RunFocused(TrafficFilter, pce::traffic::Focus, pce::CacheMode),
    /// `r` in the app overlay: the workloads and the rulesets again, past the cache.
    ReloadApp,
    /// `t` in the app overlay: the exact timeline's missing buckets, as many
    /// queries as the prompt named (`y` asks again when that changed).
    AppTimeline(u32),
}

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
            full_title: None,
            full: Some(|f| f.last_detected.clone().unwrap_or_default()),
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
            full_title: None,
            full: None,
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
            full_title: None,
            full: None,
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
            full_title: None,
            full: None,
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
            full_title: None,
            full: None,
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
            full_title: None,
            full: None,
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
            full_title: None,
            full: None,
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
            full_title: None,
            full: None,
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
    /// The pairs the tiles are laid out from: every loaded flow, whatever
    /// App-to-app's decision filter (`d`).
    pub groups: Vec<FlowGroup>,
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
            groups: Vec::new(),
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
            cell: |g| g.flows.to_string(),
            key: |g| SortKey::Num(g.flows),
            right: true,
            hide_below: 0,
            narrow: None,
            narrow_width: 5,
            full_title: None,
            full: None,
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
    ]
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
}

/// What a section row is: an app's header with its totals per direction
/// (one direction, or in then out for `both`), or one of its peer rows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RowKind {
    Header {
        totals: Vec<(Direction, Totals)>,
        folded: bool,
    },
    Peer {
        direction: Direction,
        /// Draw `←` / `→` before the peer (`both`).
        arrow: bool,
        row: PeerRow,
    },
}

/// One row of a sectioned table (App-to-app `in` / `out` / `both`,
/// Blocked): a header row per app, then its peers unless folded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SectionRow {
    pub app: Vec<String>,
    pub kind: RowKind,
    /// The section's header and every peer row, lowercased: `/` keeps a
    /// whole section when any of it matches.
    haystack: Arc<str>,
}

impl SectionRow {
    pub fn is_header(&self) -> bool {
        matches!(self.kind, RowKind::Header { .. })
    }

    pub fn peer(&self) -> Option<(Direction, &PeerRow)> {
        match &self.kind {
            RowKind::Peer { direction, row, .. } => Some((*direction, row)),
            RowKind::Header { .. } => None,
        }
    }

    /// A header's one line: `▾ shop/prod  in · 7 groups · 412 conns`;
    /// `▹` when folded; `← 1545 conns · …  → 1004 conns` for `both`.
    /// `None` for a peer.
    pub fn header_line(&self) -> Option<String> {
        let RowKind::Header { totals, folded } = &self.kind else {
            return None;
        };
        let parts: Vec<(Direction, &Totals)> = totals.iter().map(|(d, t)| (*d, t)).collect();
        Some(format!(
            "{} {}  {}",
            if *folded { "▹" } else { "▾" },
            self.app.join("/"),
            app_header(&parts)
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
            RowKind::Header { .. } => self.app.join("/"),
        }
    }

    fn with_row<R: Default>(&self, f: impl Fn(&PeerRow) -> R) -> R {
        self.peer().map(|(_, r)| f(r)).unwrap_or_default()
    }
}

/// `/` on a sectioned table: the row's section matches.
fn section_matches(r: &SectionRow, needle: &str) -> bool {
    needle.is_empty() || r.haystack.contains(needle)
}

/// `allowed`, `pot.blocked`, `blocked` or `unknown`.
fn decision_text(d: pce::models::PolicyDecision) -> String {
    match d {
        pce::models::PolicyDecision::PotentiallyBlocked => "pot.blocked".into(),
        d => d.as_str().into(),
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
        let is_folded = folded.contains(&app);
        let mut header = SectionRow {
            app: app.clone(),
            kind: RowKind::Header {
                totals: group
                    .iter()
                    .map(|s| (s.direction, s.totals.clone()))
                    .collect(),
                folded: is_folded,
            },
            haystack: Arc::from(""),
        };
        let mut text = header.header_line().unwrap_or_default();
        let peers: Vec<SectionRow> = group
            .iter()
            .flat_map(|s| {
                s.rows.iter().map(|r| SectionRow {
                    app: app.clone(),
                    kind: RowKind::Peer {
                        direction: s.direction,
                        arrow: arrows,
                        row: r.clone(),
                    },
                    haystack: Arc::from(""),
                })
            })
            .collect();
        for p in &peers {
            if let Some((_, r)) = p.peer() {
                text.push_str(&format!(
                    "\n{} {} {} {} {}",
                    r.peer_text(),
                    r.peer_name_text(),
                    r.app_name_text(),
                    r.service,
                    decision_text(r.dominant())
                ));
            }
        }
        let hay: Arc<str> = Arc::from(text.to_lowercase());
        header.haystack = hay.clone();
        out.push(header);
        if !is_folded {
            out.extend(peers.into_iter().map(|mut p| {
                p.haystack = hay.clone();
                p
            }));
        }
        i = j;
    }
    out
}

/// Bytes as `1.2K`, `-` when none were reported.
fn bytes_text(b: u64) -> String {
    if b == 0 { "-".into() } else { compact_count(b) }
}

fn no_sort(_: &SectionRow) -> SortKey {
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
            cell: |r| r.with_row(|p| p.flows.to_string()),
            key: no_sort,
            right: true,
            hide_below: 0,
            narrow: None,
            narrow_width: 5,
            full_title: None,
            full: None,
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
            cell: |r| r.with_row(|p| p.flows.to_string()),
            key: no_sort,
            right: true,
            hide_below: 0,
            narrow: None,
            narrow_width: 5,
            full_title: None,
            full: None,
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

/// What a focused re-query (`r` on a partial focus) loaded into `TrafficData`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FocusResult {
    pub focus: Focus,
    /// The Explorer queries it took: 2 for `both`.
    pub queries: u32,
}

impl FocusResult {
    /// Whether this result holds every flow of `f`: the same app under the
    /// same keys, queried for `f`'s direction or for both.
    pub fn covers(&self, f: &Focus) -> bool {
        self.focus.same_app(f)
            && (self.focus.direction == FocusDirection::Both || self.focus.direction == f.direction)
    }
}

/// `1 query`, `2 queries` (shared with the CLI's meta line).
pub use pce::traffic::queries_text;

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
    /// The profile's key order: what `all labels` and the `g` picker list
    /// first (scope keys, then the tier key; the rest alphabetically) and
    /// which keys they leave out (`ignore_keys`). Set with the label model.
    pub key_order: KeyOrder,
    /// The keys in force, resolved by `rebuild_views` (the `all labels` scan
    /// runs there, never per draw): what every view and column groups by.
    pub keys: Vec<String>,
    /// App-to-app `pairs`: the flows the decision filter (`d`) keeps.
    pub groups: TableView<FlowGroup>,
    /// App-to-app `pairs` by service (`o`), under the decision filter too;
    /// empty while off.
    pub services: TableView<FlowGroup>,
    pub mesh: TableView<MeshRow>,
    /// The Mesh view's optional columns (`u`, `p`); both shown at first.
    pub mesh_cols: MeshColumns,
    /// App-to-app's `in` / `out` / `both` sections (`pairs` uses `groups`).
    pub apps: TableView<SectionRow>,
    pub apps_state: AppState,
    /// Blocked: App-to-app `in` with `blocked`, with its own cursor and folds.
    pub blocked: TableView<SectionRow>,
    pub blocked_folded: BTreeSet<Vec<String>>,
    pub treemap: TreemapState,
    /// What the Flows view is narrowed to.
    pub drill: Option<Drill>,
    pub auto: AutoRefresh,
    /// A submitted `w` form while its plan is on the way: a planner error
    /// (an unknown ruleset name) brings it back with the error.
    pub rule_form: Option<Form>,
    /// `F` / `:focus`: every view shows only this app's traffic
    /// (`Focus::keeps`) in its direction (`X`). Cleared by `Esc`, `g` and
    /// `:profile`; a new result keeps it.
    pub focus: Option<Focus>,
    /// `data` came from `r` on a partial focus: the focus it was queried
    /// for and the number of queries.
    pub focus_result: Option<FocusResult>,
    /// The unfocused result `focus_result` replaced; put back, without a
    /// request, when the focus is cleared or moves to another app.
    pub unfocused: Option<TrafficData>,
    /// The loaded flows the focus keeps, computed once whenever the focus
    /// or the data changes (`refresh_focused`); `None` without a focus.
    focused: Option<Vec<ResolvedFlow>>,
}

/// Both optional Mesh columns shown.
pub const MESH_ALL: MeshColumns = MeshColumns {
    user: true,
    process: true,
};

/// What `Enter` narrowed the Flows view to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Drill {
    /// An App-to-app or treemap pair: source and destination group keys,
    /// and the decisions App-to-app's filter kept (`All` from the treemap).
    Pair(Vec<String>, Vec<String>, Decisions),
    /// An App-to-app pair by service (`o`): the groups, the service and
    /// the decisions kept.
    Service(Vec<String>, Vec<String>, String, Decisions),
    /// A Mesh row, under the columns it was grouped with.
    Mesh { key: MeshKey, cols: MeshColumns },
    /// A section header: every flow of `app` in `direction` (either side
    /// when `None`, from `both`) that `decisions` keeps.
    App {
        app: Vec<String>,
        direction: Option<Direction>,
        decisions: Decisions,
    },
    /// A section's peer row: its peer and service under `app`.
    Peer {
        app: Vec<String>,
        direction: Direction,
        peer: Vec<String>,
        service: String,
        decisions: Decisions,
    },
}

/// ` · blocked` when the drill keeps only some decisions.
fn decisions_suffix(d: Decisions) -> String {
    if d == Decisions::All {
        String::new()
    } else {
        format!(" · {}", d.label())
    }
}

impl Drill {
    /// Whether `f` belongs to the drilled row under `keys`.
    pub fn matches(&self, f: &ResolvedFlow, keys: &[String]) -> bool {
        match self {
            Drill::Pair(src, dst, decisions) => {
                decisions.keeps(f.policy_decision)
                    && &group_key(&f.src, keys) == src
                    && &group_key(&f.dst, keys) == dst
            }
            Drill::Service(src, dst, service, decisions) => {
                decisions.keeps(f.policy_decision)
                    && &group_key(&f.src, keys) == src
                    && &group_key(&f.dst, keys) == dst
                    && &f.service_text() == service
            }
            Drill::Mesh { key, cols } => &mesh_key(f, keys, *cols) == key,
            Drill::App {
                app,
                direction,
                decisions,
            } => in_app(f, keys, app, *direction, *decisions),
            Drill::Peer {
                app,
                direction,
                peer,
                service,
                decisions,
            } => in_peer(f, keys, app, *direction, peer, service, *decisions),
        }
    }

    /// The Flows title after `Flows ▸ `.
    pub fn title(&self) -> String {
        match self {
            Drill::Pair(src, dst, decisions) => format!(
                "{} → {}{}",
                src.join("/"),
                dst.join("/"),
                decisions_suffix(*decisions)
            ),
            Drill::Service(src, dst, service, decisions) => format!(
                "{} → {} {service}{}",
                src.join("/"),
                dst.join("/"),
                decisions_suffix(*decisions)
            ),
            Drill::Mesh { key, .. } => format!(
                "{} → {} {}",
                key.src.join("/"),
                key.dst.join("/"),
                key.service_text()
            ),
            Drill::App {
                app,
                direction,
                decisions,
            } => {
                let app = app.join("/");
                let pair = match direction {
                    Some(Direction::In) => format!("* → {app}"),
                    Some(Direction::Out) => format!("{app} → *"),
                    None => format!("{app} ↔ *"),
                };
                format!("{pair}{}", decisions_suffix(*decisions))
            }
            Drill::Peer {
                app,
                direction,
                peer,
                service,
                decisions,
            } => {
                let (src, dst) = match direction {
                    Direction::In => (peer, app),
                    Direction::Out => (app, peer),
                };
                format!(
                    "{} → {} {service}{}",
                    src.join("/"),
                    dst.join("/"),
                    decisions_suffix(*decisions)
                )
            }
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
            key_order: KeyOrder::default(),
            keys: keys_or_default(keys),
            groups: TableView::new(group_columns()),
            services: TableView::new(service_group_columns()),
            mesh: TableView::new(mesh_columns(MESH_ALL)),
            mesh_cols: MESH_ALL,
            apps: TableView::new(app_columns()).with_matcher(section_matches),
            apps_state: AppState::default(),
            blocked: TableView::new(blocked_columns()).with_matcher(section_matches),
            blocked_folded: BTreeSet::new(),
            treemap: TreemapState::new(TreemapRect::default()),
            drill: None,
            auto: AutoRefresh::new(auto_min_seconds),
            rule_form: None,
            focus: None,
            focus_result: None,
            unfocused: None,
            focused: None,
        }
    }

    /// The loaded flows (none before the first query).
    pub fn loaded_flows(&self) -> &[ResolvedFlow] {
        self.data.as_ref().map_or(&[], |d| d.flows.as_slice())
    }

    /// The loaded flows the focus keeps (all of them, borrowed, without
    /// one): what every view aggregates, under the drill and `d`.
    pub fn view_flows(&self) -> &[ResolvedFlow] {
        match &self.focused {
            Some(f) => f,
            None => self.loaded_flows(),
        }
    }

    /// Recomputes the flows the focus keeps from `data`: once per change
    /// of the focus or the data, never per frame.
    fn refresh_focused(&mut self) {
        self.focused = self.focus.as_ref().map(|f| {
            self.loaded_flows()
                .iter()
                .filter(|x| f.keeps(x))
                .cloned()
                .collect()
        });
    }

    /// The focus a running focused re-query was started for.
    pub fn running_focus(&self) -> Option<&Focus> {
        match &self.query {
            QueryState::Running { focus, .. } => focus.as_ref(),
            _ => None,
        }
    }

    /// Cancels a running focused re-query unless it was started for
    /// exactly the focus now set; whether one was cancelled. Its answer
    /// would bring back a focus the user cleared or changed.
    pub fn cancel_stale_focused_run(&mut self) -> bool {
        let stale = self
            .running_focus()
            .is_some_and(|r| self.focus.as_ref() != Some(r));
        if stale {
            self.cancel_query();
        }
        stale
    }

    /// `F` / `:focus`: narrows every view to `focus`. A focused result of
    /// another app is put back first: it holds only that app's flows.
    /// No request.
    pub fn set_focus(&mut self, focus: Focus) {
        if self
            .focus_result
            .as_ref()
            .is_some_and(|r| !r.focus.same_app(&focus))
        {
            self.restore_unfocused();
        }
        self.focus = Some(focus);
        self.cancel_stale_focused_run();
        self.rebuild_views();
    }

    /// The focus gone, a focused re-query in flight cancelled and the
    /// unfocused result back, without a rebuild; whether there was a focus.
    pub fn drop_focus(&mut self) -> bool {
        self.restore_unfocused();
        let had = self.focus.take().is_some();
        self.cancel_stale_focused_run();
        self.refresh_focused();
        had
    }

    /// `Esc`: `drop_focus`, then every view again. No request.
    pub fn clear_focus(&mut self) -> bool {
        let had = self.drop_focus();
        self.rebuild_views();
        had
    }

    /// The unfocused result back in `data` if a focused one replaced it
    /// (no rebuild); whether one was put back.
    pub fn restore_unfocused(&mut self) -> bool {
        self.focus_result = None;
        match self.unfocused.take() {
            Some(d) => {
                self.data = Some(d);
                self.refresh_focused();
                true
            }
            None => false,
        }
    }

    /// `+`, `-`, `]`, `[` and `B` work on the unfocused result: a focused
    /// one on screen goes back first and every view is rebuilt (the focus
    /// stays and narrows it again). Whether one went back.
    pub fn back_to_unfocused(&mut self) -> bool {
        if self.focus_result.is_none() {
            return false;
        }
        self.restore_unfocused();
        self.rebuild_views();
        true
    }

    /// `X`: the next direction, every view rebuilt; `None` without a focus.
    pub fn cycle_focus_direction(&mut self) -> Option<FocusDirection> {
        let f = self.focus.as_mut()?;
        f.direction = f.direction.next();
        let d = f.direction;
        self.cancel_stale_focused_run();
        self.rebuild_views();
        Some(d)
    }

    /// A focused re-query's answer: the unfocused result kept aside (once),
    /// `data` the focused flows, the drill and the zoom dropped (a new
    /// result), every view rebuilt.
    pub fn apply_focused(&mut self, focus: Focus, queries: u32, data: TrafficData) {
        if self.unfocused.is_none() {
            self.unfocused = self.data.take();
        }
        self.data = Some(data);
        self.focus_result = Some(FocusResult {
            focus: focus.clone(),
            queries,
        });
        self.focus = Some(focus);
        self.drill = None;
        self.treemap.zoom = None;
        self.rebuild_views();
    }

    /// The focus may be missing flows: the result it narrows hit the cap
    /// and no focused query covering it is loaded.
    pub fn focus_partial(&self) -> bool {
        let Some(f) = &self.focus else {
            return false;
        };
        match &self.focus_result {
            Some(r) => !r.covers(f),
            None => self.data.as_ref().is_some_and(|d| d.truncated),
        }
    }

    /// What `r` re-queries: the focus while it is partial, or while the
    /// result on screen is its focused query (then `r` refreshes it).
    pub fn focus_requery(&self) -> Option<Focus> {
        let f = self.focus.as_ref()?;
        let shown = self.focus_result.as_ref().is_some_and(|r| r.covers(f));
        (shown || self.focus_partial()).then(|| f.clone())
    }

    /// The Result header's focus part: `focus shop/prod · in`, with `· 2
    /// queries` when a focused query loaded the data, or `focus is partial:
    /// r re-queries shop/prod · in`.
    pub fn focus_meta(&self) -> Option<String> {
        let f = self.focus.as_ref()?;
        if self.focus_partial() {
            return Some(format!(
                "focus is partial: r re-queries {} · {}",
                f.app_text(),
                f.direction.as_str()
            ));
        }
        Some(match &self.focus_result {
            Some(r) if r.covers(f) => format!("{} · {}", f.badge(), queries_text(r.queries)),
            _ => f.badge(),
        })
    }

    /// The apps of the row under the cursor, in the order `F` takes them: a
    /// section header its app; a peer row its section's app, then the peer
    /// (so the destination first in an inbound section, App-to-app `in`
    /// and `both` or Blocked); any other pair, flow, Mesh row or treemap
    /// tile the source, then the destination. A flow
    /// from an app to itself has one. Empty without a row (or on an `other
    /// (N)` tile).
    pub fn row_apps(&self) -> Vec<GroupKey> {
        let keys = &self.keys;
        let mut apps: Vec<GroupKey> = match self.view {
            TrafficView::Flows => self
                .flows
                .selected_item()
                .map(|r| vec![group_key(&r.flow.src, keys), group_key(&r.flow.dst, keys)])
                .unwrap_or_default(),
            TrafficView::Aggregate if self.apps_state.by == By::Pairs => self
                .pairs_table()
                .selected_item()
                .map(|g| vec![g.src.clone(), g.dst.clone()])
                .unwrap_or_default(),
            TrafficView::Aggregate | TrafficView::Blocked => self.section_apps(),
            TrafficView::Mesh => self
                .mesh
                .selected_item()
                .map(|r| vec![r.src.clone(), r.dst.clone()])
                .unwrap_or_default(),
            TrafficView::Treemap => self.treemap_apps(),
        };
        apps.dedup();
        apps
    }

    fn section_apps(&self) -> Vec<GroupKey> {
        let Some(r) = self.section_table().and_then(|t| t.selected_item()) else {
            return Vec::new();
        };
        match r.peer() {
            Some((_, row)) => vec![r.app.clone(), row.peer.clone()],
            None => vec![r.app.clone()],
        }
    }

    fn treemap_apps(&self) -> Vec<GroupKey> {
        let tm = &self.treemap;
        let Some(target) = tm.current() else {
            return Vec::new();
        };
        let tile = tm.tile(target);
        if tile.is_other() {
            return Vec::new();
        }
        let src_of = |text: &str| {
            tm.groups
                .iter()
                .find(|g| g.src.join("/") == text)
                .map(|g| g.src.clone())
        };
        let dst_of = |text: &str| {
            tm.groups
                .iter()
                .find(|g| g.dst.join("/") == text)
                .map(|g| g.dst.clone())
        };
        match (&tm.zoom, target.inner) {
            (None, None) => src_of(&tile.label).into_iter().collect(),
            (Some(z), _) => [Some(z.clone()), dst_of(&tile.label)]
                .into_iter()
                .flatten()
                .collect(),
            (None, Some(_)) => [src_of(&tm.tiles[target.outer].label), dst_of(&tile.label)]
                .into_iter()
                .flatten()
                .collect(),
        }
    }

    /// The app `F` focuses on the selected row: its first app, or its
    /// second when the focus is already on the first (a second `F` switches
    /// sides). `None` without an app under the cursor.
    pub fn next_focus_app(&self) -> Option<GroupKey> {
        let apps = self.row_apps();
        let first = apps.first()?;
        let on_first = self
            .focus
            .as_ref()
            .is_some_and(|f| &f.app == first && f.keys == self.keys);
        match apps.get(1) {
            Some(second) if on_first => Some(second.clone()),
            _ => Some(first.clone()),
        }
    }

    /// The apps the picker and `:focus` complete over: every labelled
    /// workload group of the unfocused result under the keys in force.
    pub fn loaded_apps(&self) -> Vec<String> {
        let flows = match &self.unfocused {
            Some(d) => d.flows.as_slice(),
            None => self.loaded_flows(),
        };
        apps_in(flows, &self.keys)
    }

    /// The loaded flows an app overlay for `focus` reads: a focused result
    /// that covers it, else the unfocused result.
    pub fn flows_for(&self, focus: &Focus) -> &[ResolvedFlow] {
        match (&self.focus_result, &self.unfocused) {
            (Some(r), _) if r.covers(focus) => self.loaded_flows(),
            (_, Some(u)) => &u.flows,
            _ => self.loaded_flows(),
        }
    }

    /// Whether `flows_for(focus)` may miss some of its flows: truncated,
    /// and not a focused result covering it.
    pub fn partial_for(&self, focus: &Focus) -> bool {
        match (&self.focus_result, &self.unfocused) {
            (Some(r), _) if r.covers(focus) => false,
            (_, Some(u)) => u.truncated,
            _ => self.data.as_ref().is_some_and(|d| d.truncated),
        }
    }

    /// The loaded result's window, `(start, end)`: its range's end and the
    /// filter's window back from it. `None` before a result with bounds.
    pub fn window_bounds(&self) -> Option<(jiff::Timestamp, jiff::Timestamp)> {
        let d = self.unfocused.as_ref().or(self.data.as_ref())?;
        Some((d.window_start()?, d.range?.end))
    }

    /// The order `all labels` and the `g` picker list keys in, and the
    /// keys they leave out.
    pub fn set_key_order(&mut self, order: &KeyOrder) {
        self.key_order = order.clone();
    }

    /// The keys in force, as the last `rebuild_views` resolved them.
    pub fn effective_keys(&self) -> Vec<String> {
        self.keys.clone()
    }

    /// Every key seen in the loaded flows when `all_keys` (in `key_order`,
    /// ignored keys left out), else `group_keys` (`app,env` when empty).
    fn resolve_keys(&self) -> Vec<String> {
        if self.all_keys {
            let seen: Vec<String> = keys_seen(self.loaded_flows(), &self.key_order)
                .into_iter()
                .map(|(k, _)| k)
                .collect();
            if !seen.is_empty() {
                return seen;
            }
        }
        keys_or_default(&self.group_keys)
    }

    /// Whether any loaded flow the focus keeps carries a byte count.
    pub fn has_bytes(&self) -> bool {
        self.view_flows().iter().any(ResolvedFlow::has_bytes)
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
        self.refresh_focused();
        let (shown, tree, mesh_rows) = {
            let keys = &self.keys;
            let flows = self.view_flows();
            let shown: Vec<FlowRow> = flows
                .iter()
                .filter(|f| self.drill.as_ref().is_none_or(|d| d.matches(f, keys)))
                .map(|f| FlowRow::new(f.clone(), keys))
                .collect();
            (
                shown,
                group_by_labels(flows, keys),
                mesh(flows, keys, self.mesh_cols),
            )
        };
        self.flows.set_rows(shown);
        self.treemap.groups = tree;
        self.mesh.set_rows(mesh_rows);
        self.rebuild_pairs();
        self.rebuild_sections();
        self.relayout_treemap();
    }

    /// App-to-app's pairs, and by service while `o` is on (none while off),
    /// from the flows the focus and the decision filter keep. Pure.
    pub fn rebuild_pairs(&mut self) {
        let (groups, services) = {
            let st = &self.apps_state;
            let flows = self.view_flows();
            let kept = st.decisions.kept(flows);
            let groups = group_by_labels(&kept, &self.keys);
            let services = if st.by_service {
                group_by_service(&kept, &self.keys)
            } else {
                Vec::new()
            };
            (groups, services)
        };
        self.groups.set_rows(groups);
        self.services.set_rows(services);
    }

    /// App-to-app `pairs`' table: by service when `o` is on, else the pairs.
    pub fn pairs_table(&self) -> &TableView<FlowGroup> {
        if self.apps_state.by_service {
            &self.services
        } else {
            &self.groups
        }
    }

    pub fn pairs_table_mut(&mut self) -> &mut TableView<FlowGroup> {
        if self.apps_state.by_service {
            &mut self.services
        } else {
            &mut self.groups
        }
    }

    /// App-to-app's sections (none in `pairs`) and Blocked's, from the
    /// loaded flows, the keys in force, the state and the folds. Pure.
    pub fn rebuild_sections(&mut self) {
        let apps = self.apps_rows(&self.apps_state.folded);
        let blocked = self.blocked_rows(&self.blocked_folded);
        self.apps.set_rows(apps);
        self.blocked.set_rows(blocked);
    }

    /// App-to-app's rows under `folded`; none in `pairs`.
    fn apps_rows(&self, folded: &BTreeSet<Vec<String>>) -> Vec<SectionRow> {
        let keys = &self.keys;
        let flows = self.view_flows();
        let st = &self.apps_state;
        match st.by {
            By::Pairs => Vec::new(),
            By::In => section_rows(
                &app_sections(flows, keys, Direction::In, st.decisions),
                folded,
                false,
            ),
            By::Out => section_rows(
                &app_sections(flows, keys, Direction::Out, st.decisions),
                folded,
                false,
            ),
            By::Both => section_rows(&app_sections_both(flows, keys, st.decisions), folded, true),
        }
    }

    /// Blocked's rows under `folded`.
    fn blocked_rows(&self, folded: &BTreeSet<Vec<String>>) -> Vec<SectionRow> {
        section_rows(
            &app_sections(
                self.view_flows(),
                &self.keys,
                Direction::In,
                Decisions::Blocked,
            ),
            folded,
            false,
        )
    }

    /// The sectioned table on screen with every section unfolded, its
    /// search and sort applied (`:export`); `None` outside one.
    pub fn unfolded_section_rows(&self) -> Option<Vec<SectionRow>> {
        let none = BTreeSet::new();
        let (table, rows) = match self.view {
            TrafficView::Aggregate if self.apps_state.by != By::Pairs => {
                (&self.apps, self.apps_rows(&none))
            }
            TrafficView::Blocked => (&self.blocked, self.blocked_rows(&none)),
            _ => return None,
        };
        let shown = table.visible_of(&rows);
        let mut rows: Vec<Option<SectionRow>> = rows.into_iter().map(Some).collect();
        Some(shown.into_iter().filter_map(|i| rows[i].take()).collect())
    }

    /// The sectioned table the active view shows, if any: App-to-app
    /// outside `pairs`, or Blocked.
    pub fn section_table(&self) -> Option<&TableView<SectionRow>> {
        match self.view {
            TrafficView::Aggregate if self.apps_state.by != By::Pairs => Some(&self.apps),
            TrafficView::Blocked => Some(&self.blocked),
            _ => None,
        }
    }

    /// `Space`: folds or unfolds the selected row's section; the cursor
    /// lands on its header.
    pub fn toggle_fold(&mut self) {
        let Some(app) = self
            .section_table()
            .and_then(|t| t.selected_item())
            .map(|r| r.app.clone())
        else {
            return;
        };
        let folded = self.folds_mut();
        if !folded.remove(&app) {
            folded.insert(app.clone());
        }
        self.rebuild_sections();
        if let Some(t) = self.section_table_mut() {
            t.select_where(|r| r.is_header() && r.app == app);
        }
    }

    /// `z`: folds every section, or unfolds them all when all are folded.
    pub fn toggle_fold_all(&mut self) {
        let Some(t) = self.section_table() else {
            return;
        };
        let headers: Vec<(Vec<String>, bool)> = t
            .rows()
            .iter()
            .filter_map(|r| match &r.kind {
                RowKind::Header { folded, .. } => Some((r.app.clone(), *folded)),
                RowKind::Peer { .. } => None,
            })
            .collect();
        let app = t.selected_item().map(|r| r.app.clone());
        let all_folded = headers.iter().all(|(_, f)| *f);
        let folded = self.folds_mut();
        if all_folded {
            folded.clear();
        } else {
            folded.extend(headers.into_iter().map(|(a, _)| a));
        }
        self.rebuild_sections();
        if let (Some(app), Some(t)) = (app, self.section_table_mut()) {
            t.select_where(|r| r.is_header() && r.app == app);
        }
    }

    fn section_table_mut(&mut self) -> Option<&mut TableView<SectionRow>> {
        match self.view {
            TrafficView::Aggregate if self.apps_state.by != By::Pairs => Some(&mut self.apps),
            TrafficView::Blocked => Some(&mut self.blocked),
            _ => None,
        }
    }

    fn folds_mut(&mut self) -> &mut BTreeSet<Vec<String>> {
        if self.view == TrafficView::Blocked {
            &mut self.blocked_folded
        } else {
            &mut self.apps_state.folded
        }
    }

    /// The decision filter of the sectioned table on screen.
    pub fn section_decisions(&self) -> Decisions {
        if self.view == TrafficView::Blocked {
            Decisions::Blocked
        } else {
            self.apps_state.decisions
        }
    }

    /// `Enter` on a section row: the drill for its header or peer row.
    pub fn section_drill(&self) -> Option<Drill> {
        let r = self.section_table()?.selected_item()?;
        let decisions = self.section_decisions();
        Some(match &r.kind {
            RowKind::Header { totals, .. } => Drill::App {
                app: r.app.clone(),
                direction: match totals.as_slice() {
                    [(d, _)] => Some(*d),
                    _ => None,
                },
                decisions,
            },
            RowKind::Peer { direction, row, .. } => Drill::Peer {
                app: r.app.clone(),
                direction: *direction,
                peer: row.peer.clone(),
                service: row.service.clone(),
                decisions,
            },
        })
    }

    /// The treemap's tiles again from its (unfiltered) groups, zoom, weight and
    /// area. A zoom whose source is gone is dropped; byte weight falls back
    /// to connections when no flow carries bytes; the cursor is clamped.
    pub fn relayout_treemap(&mut self) {
        self.treemap.has_bytes = self.has_bytes();
        if !self.treemap.has_bytes {
            self.treemap.weight = Weight::Connections;
        }
        let tm = &mut self.treemap;
        let groups = &tm.groups;
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
            TrafficView::Aggregate if self.apps_state.by == By::Pairs => {
                self.pairs_table().is_searching()
            }
            TrafficView::Aggregate => self.apps.is_searching(),
            TrafficView::Mesh => self.mesh.is_searching(),
            TrafficView::Blocked => self.blocked.is_searching(),
            TrafficView::Treemap => false,
        }
    }

    pub fn is_running(&self) -> bool {
        matches!(self.query, QueryState::Running { .. })
    }

    /// Cancels a running query without starting another: its answer is
    /// stale from here on.
    pub fn cancel_query(&mut self) {
        if let QueryState::Running { cancel, .. } = &self.query {
            cancel.cancel();
            self.seq += 1;
            self.query = QueryState::Idle;
        }
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
            focus: None,
        };
        (self.seq, cancel)
    }

    /// `start_query` for `r` on a partial focus: the run remembers its
    /// focus, so clearing or changing the focus cancels it.
    pub fn start_focused_query(&mut self, now: Instant, focus: Focus) -> (u64, CancellationToken) {
        let started = self.start_query(now);
        if let QueryState::Running { focus: f, .. } = &mut self.query {
            *f = Some(focus);
        }
        started
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
            full_title: None,
            full: None,
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
            full_title: None,
            full: Some(|r| r.enabled.to_string()),
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
            full_title: None,
            full: Some(|r| join_list(&r.scopes)),
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
            full_title: None,
            full: None,
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
            full_title: None,
            full: None,
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
            full_title: None,
            full: None,
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
            full_title: None,
            full: Some(|r| r.enabled.to_string()),
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
            full_title: None,
            full: Some(|r| join_list(r.consumers.iter().map(|a| &a.name))),
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
            full_title: None,
            full: Some(|r| join_list(r.providers.iter().map(|a| &a.name))),
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
            full_title: None,
            full: Some(|r| join_list(r.services.iter().map(|s| &s.name))),
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
            full_title: None,
            full: Some(|r| r.impact.as_ref().map(pce::Impact::pc).unwrap_or_default()),
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
            full_title: None,
            full: None,
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
    /// Pending entries marked with `Space` for `P` and `R` (hrefs); cleared
    /// when the list is loaded again.
    pub pending_marked: BTreeSet<String>,
    /// The counts of the provision just sent (`ip_lists=1, rule_sets=2`),
    /// taken from its body at `y`: the PCE's answer does not repeat them.
    pub provision_counts: Option<String>,
    /// The report of the provision just done; the pending reload it caused
    /// shows it in front of the new count.
    pub provisioned: Option<String>,
    /// What the user typed into an open revert confirm; only `revert`
    /// followed by `Enter` sends it.
    pub revert_typed: String,
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
            revert_typed: String::new(),
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
fn usage_cell(r: &LabelRow, n: fn(&pce::LabelUsage) -> usize) -> String {
    r.usage.map_or_else(|| "-".into(), |u| n(&u).to_string())
}

/// `usage_cell` for `:export`: empty before the data is loaded.
fn usage_full(r: &LabelRow, n: fn(&pce::LabelUsage) -> usize) -> String {
    r.usage.map(|u| n(&u).to_string()).unwrap_or_default()
}

/// Its sort key: rows without usage sort last.
fn usage_key(r: &LabelRow, n: fn(&pce::LabelUsage) -> usize) -> SortKey {
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
        full_title: None,
        full: None,
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

/// The effective enforcement: `unmanaged` without a VEN, else the PCE's mode.
fn enforcement_of(r: &WorkloadRowView) -> String {
    r.workload.enforcement().unwrap_or_default().to_string()
}

/// `-` without a VEN: nothing reports flows there, whatever the PCE says.
fn visibility_of(r: &WorkloadRowView) -> String {
    if r.workload.is_unmanaged() {
        return "-".into();
    }
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
            full_title: None,
            full: None,
            ..text_col("hostname", 22, hostname_of, |r| {
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
    /// `Enter` on a workload in the app overlay while this list loads: the
    /// workload selected, its detail open, once the list arrives.
    pub open_on_load: Option<String>,
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
            open_on_load: None,
        }
    }

    /// The cached workloads and their `truncated` flag, once loaded.
    pub fn impact_source(&self) -> Option<(&[Workload], bool)> {
        self.loaded_at
            .is_some()
            .then_some((self.raw.as_slice(), self.truncated))
    }

    /// Recomputes the table rows (and health counts) from `raw`, labels in
    /// `order`.
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
            full_title: None,
            full: Some(|e| e.timestamp.clone().unwrap_or_default()),
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
            full_title: None,
            full: None,
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
            full_title: None,
            full: None,
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
            full_title: None,
            full: None,
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
            full_title: None,
            full: Some(|e| join_list(e.resources())),
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
            full_title: None,
            full: None,
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
    /// After a truncated query below 10000, re-run it at 10000 without `B`.
    pub traffic_auto_backfill: bool,
    /// Label keys a ruleset scope is made of (`app`, `env`).
    pub scope_keys: Vec<String>,
    /// The label key that names a tier (`role`).
    pub tier_key: String,
    /// Label keys hidden from label text (columns, the `g` picker).
    pub ignore_keys: Vec<String>,
}

impl Limits {
    /// The resolved profile's limits and label model (startup's and a
    /// `:profile` switch's).
    pub fn from_config(cfg: &crate::config::ResolvedConfig) -> Self {
        Self {
            auto_refresh_min_seconds: cfg.auto_refresh_min_seconds,
            traffic_max_results: cfg.traffic_max_results,
            traffic_default_window: cfg.traffic_default_window.clone(),
            traffic_auto_backfill: cfg.traffic_auto_backfill,
            scope_keys: cfg.scope_keys.clone(),
            tier_key: cfg.tier_key.clone(),
            ignore_keys: cfg.ignore_keys.clone(),
        }
    }

    /// The order label keys are shown in: `scope_keys`, `tier_key`, then
    /// the rest alphabetically; `ignore_keys` hidden.
    pub fn key_order(&self) -> KeyOrder {
        KeyOrder::new(&self.scope_keys, &self.tier_key).with_ignored(&self.ignore_keys)
    }
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
    /// The app overlay's state while it is open (and after, until the next
    /// `A`: reopening rebuilds it).
    pub app_view: Option<AppView>,
    /// The last app overlay's sequence number: answers of older ones are dropped.
    pub app_seq: u64,
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
        traffic.set_key_order(&KeyOrder::new(&scope_keys, crate::config::DEFAULT_TIER_KEY));
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
                traffic_auto_backfill: false,
                scope_keys,
                tier_key: crate::config::DEFAULT_TIER_KEY.into(),
                ignore_keys: Vec::new(),
            },
            screen: Screen::Traffic,
            overlay: Overlay::None,
            traffic,
            policy,
            objects,
            workloads,
            events,
            app_view: None,
            app_seq: 0,
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

    /// The profile's label model: the `w` form's dimensions, the keys
    /// every traffic view groups by until `g` changes them, and the keys
    /// label text hides.
    pub fn set_label_model(
        &mut self,
        scope_keys: Vec<String>,
        tier_key: String,
        ignore_keys: Vec<String>,
    ) {
        self.traffic.group_keys = keys_or_default(&scope_keys);
        self.traffic.all_keys = false;
        self.limits.scope_keys = scope_keys;
        self.limits.tier_key = tier_key;
        self.limits.ignore_keys = ignore_keys;
        let order = self.limits.key_order();
        self.traffic.set_key_order(&order);
        self.traffic.rebuild_views();
        self.workloads.rebuild(&order);
        self.objects.hide_ignored = !self.limits.ignore_keys.is_empty();
        self.objects.rebuild_labels(&order);
    }

    /// Whether the Labels tab's usage can be counted: the workloads, the
    /// rulesets and the label groups are all cached.
    pub fn usage_ready(&self) -> bool {
        self.workloads.loaded_at.is_some()
            && self.policy.loaded_at.is_some()
            && self.objects.ages[ObjectKind::LabelGroups.index()].is_some()
    }

    /// Counts label usage from what is cached (none until `usage_ready`)
    /// and rebuilds the Labels tab. No request.
    pub fn recount_labels(&mut self) {
        self.objects.label_usage = self.usage_ready().then(|| {
            pce::label_usage(
                &self.workloads.raw,
                &self.policy.raw,
                self.policy.fallback_deny.values().flatten(),
                self.objects.groups.rows(),
            )
        });
        self.objects.usage_truncated = self.workloads.truncated;
        self.objects.rebuild_labels(&self.limits.key_order());
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
            group_keys: None,
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
        traffic.set_key_order(&self.limits.key_order());
        traffic.seq = self.traffic.seq + 1; // a late result of the old query is stale
        traffic.treemap.area = crate::tui::ui::screens::traffic::treemap_area(self.size);
        self.traffic = traffic;
        self.policy = PolicyState::new(min);
        self.objects = ObjectsState::new(min);
        self.objects.hide_ignored = !self.limits.ignore_keys.is_empty();
        self.workloads = WorkloadsState::new(min);
        self.events = EventsState::new(min);
        // An exact timeline's bucket in flight is the old PCE's: cancelled.
        if let Some(e) = self.app_view.as_mut().and_then(|v| v.exact.as_mut()) {
            e.stop();
        }
        self.app_view = None;
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
            vec!["src", "dst name", "service", "flows", "conns", "last seen"]
        );
        // A fill column (width 0) gets at least 8 cells.
        let fill_width = |cols: &[(usize, u16, bool)]| -> u16 {
            cols.iter()
                .map(|c| if c.1 == 0 { 8 } else { c.1 })
                .sum::<u16>()
                + cols.len() as u16
                - 1
                + 2
        };
        let at80 = visible_columns(&blocked, 80);
        assert!(fill_width(&at80) <= 78 - 16, "room for the sources at 80");
        let apps = app_columns();
        let at80 = visible_columns(&apps, 80);
        let titles: Vec<&str> = at80.iter().map(|c| apps[c.0].title).collect();
        assert_eq!(
            titles,
            vec!["peer", "service", "flows", "conns", "bytes", "decision"]
        );
        assert!(fill_width(&at80) <= 78 - 16, "room for the peer at 80");
        let at160 = visible_columns(&apps, 160);
        assert_eq!(apps[at160[1].0].title, "hosts");
    }

    fn rich_sections(direction: Direction, decisions: Decisions, arrows: bool) -> Vec<SectionRow> {
        let flows = crate::tui::test_support::fixture_flows_rich();
        let keys = vec!["app".to_string(), "env".to_string()];
        let sections = if arrows {
            app_sections_both(&flows, &keys, decisions)
        } else {
            app_sections(&flows, &keys, direction, decisions)
        };
        section_rows(&sections, &BTreeSet::new(), arrows)
    }

    #[test]
    fn blocked_cells_name_one_host_or_count_them() {
        let rows = rich_sections(Direction::In, Decisions::Blocked, false);
        let cols = blocked_columns();
        let header = rows
            .iter()
            .find(|r| r.is_header() && r.app == vec!["shop", "prod"])
            .unwrap();
        assert_eq!(
            header.header_line().unwrap(),
            "▾ shop/prod  in · 3 groups · 24 conns · 2 blocked · 1 pot.blocked"
        );
        assert!(
            cols.iter()
                .all(|c| (c.cell)(header).is_empty() || c.title == "src")
        );
        let peers: Vec<Vec<String>> = rows
            .iter()
            .filter(|r| !r.is_header() && r.app == vec!["shop", "prod"])
            .map(|r| cols.iter().map(|c| (c.cell)(r)).collect())
            .collect();
        assert_eq!(peers.len(), 3);
        assert!(
            peers
                .iter()
                .all(|p| p[1] == "shop-db-p.example" && p[2] == "5432/tcp")
        );
        let srcs: BTreeSet<&str> = peers.iter().map(|p| p[0].as_str()).collect();
        assert_eq!(
            srcs,
            BTreeSet::from(["  hr/prod", "  shop/dev", "  ipl:Corp/-"]),
            "indented sources"
        );
        let mut many = rows
            .iter()
            .find(|r| !r.is_header() && r.app == vec!["shop", "prod"])
            .unwrap()
            .clone();
        if let RowKind::Peer { row, .. } = &mut many.kind {
            row.app_hosts = 8;
            row.app_names = vec!["a".into(), "b".into()];
        }
        assert_eq!((cols[1].cell)(&many), "8 hosts");
    }

    #[test]
    fn both_rows_mark_the_direction_and_share_one_header_per_app() {
        let rows = rich_sections(Direction::In, Decisions::All, true);
        let header = &rows[0];
        let line = header.header_line().unwrap();
        assert_eq!(
            line,
            "▾ shop/prod  ← 1545 conns · 2 blocked · 1 pot.blocked  → 1004 conns"
        );
        let peers: Vec<String> = rows
            .iter()
            .skip(1)
            .take_while(|r| !r.is_header())
            .map(|r| (app_columns()[0].cell)(r))
            .collect();
        assert!(peers.iter().any(|p| p.starts_with("  ← ")), "{peers:?}");
        assert!(peers.iter().any(|p| p.starts_with("  → ")), "{peers:?}");
        let first_out = peers.iter().position(|p| p.starts_with("  → ")).unwrap();
        assert!(
            peers[first_out..].iter().all(|p| p.starts_with("  → ")),
            "inbound rows, then outbound"
        );
        let headers = rows.iter().filter(|r| r.is_header()).count();
        let apps: BTreeSet<&Vec<String>> = rows.iter().map(|r| &r.app).collect();
        assert_eq!(headers, apps.len());
    }

    #[test]
    fn a_folded_section_keeps_its_header_and_search_keeps_whole_sections() {
        let flows = crate::tui::test_support::fixture_flows_rich();
        let keys = vec!["app".to_string(), "env".to_string()];
        let sections = app_sections(&flows, &keys, Direction::In, Decisions::All);
        let folded = BTreeSet::from([vec!["crm".to_string(), "prod".to_string()]]);
        let rows = section_rows(&sections, &folded, false);
        assert!(rows[0].header_line().unwrap().starts_with("▹ crm/prod"));
        assert!(rows[1].is_header(), "no peers under a folded header");
        let mut t = TableView::new(app_columns()).with_matcher(section_matches);
        t.set_rows(section_rows(&sections, &BTreeSet::new(), false));
        let all = t.len();
        t.start_search();
        for c in "api.partner".chars() {
            t.search_push(c);
        }
        assert!(t.len() < all && !t.is_empty());
        let shown: Vec<&SectionRow> = t.visible().iter().map(|&i| &t.rows()[i]).collect();
        let app = &shown[0].app;
        assert!(shown[0].is_header());
        assert!(shown.iter().all(|r| &r.app == app), "one whole section");
        assert!(shown.len() > 2, "with its rows that do not match too");
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
    fn workload_labels_column_follows_the_key_order_and_sorts_by_it() {
        let mut m = crate::tui::test_support::model_with_workloads();
        m.set_label_model(vec!["env".into()], "role".into(), Vec::new());
        // host-1 sorts first alphabetically (`app=aaa`), host-2 by the order (`role=db`).
        m.workloads.raw[0].labels[1].value = Some("aaa".into());
        m.workloads.rebuild(&m.limits.key_order());
        let t = &m.workloads.table;
        let col = t.columns.iter().position(|c| c.title == "labels").unwrap();
        assert_eq!(t.cell(&t.rows()[0], col), "env=prod role=web app=aaa");
        assert_eq!(t.rows()[0].labels, "env=prod role=web app=aaa");
        m.workloads.table.sort_by(col, false);
        let t = &m.workloads.table;
        let sorted: Vec<&str> = t
            .visible()
            .iter()
            .map(|&i| t.rows()[i].labels.as_str())
            .filter(|l| !l.is_empty())
            .collect();
        assert_eq!(
            sorted,
            vec!["env=prod role=db app=shop", "env=prod role=web app=aaa"]
        );
    }

    #[test]
    fn workload_visibility_is_a_dash_without_a_ven() {
        let mut m = crate::tui::test_support::model_with_workloads();
        for w in &mut m.workloads.raw {
            w.visibility_level = Some("flow_summary".into());
        }
        m.workloads.rebuild(&m.limits.key_order());
        let t = &m.workloads.table;
        let col = t
            .columns
            .iter()
            .position(|c| c.title == "visibility")
            .unwrap();
        let cells: Vec<String> = t.rows().iter().map(|r| t.cell(r, col)).collect();
        let nas = t
            .rows()
            .iter()
            .position(|r| r.workload.is_unmanaged())
            .unwrap();
        assert_eq!(cells[nas], "-");
        assert_eq!(cells[0], "flow_summary");
    }

    #[test]
    fn workload_labels_column_hides_ignored_keys() {
        let mut m = crate::tui::test_support::model_with_workloads();
        m.set_label_model(vec!["app".into()], "role".into(), vec!["env".into()]);
        m.workloads.rebuild(&m.limits.key_order());
        let t = &m.workloads.table;
        let col = t.columns.iter().position(|c| c.title == "labels").unwrap();
        assert_eq!(t.cell(&t.rows()[0], col), "app=shop role=web");
        assert!(
            m.workloads.raw[0]
                .labels
                .iter()
                .any(|l| l.key.as_deref() == Some("env")),
            "the workload itself keeps the label"
        );
    }

    #[test]
    fn limits_from_config_carry_the_ignored_keys() {
        let file: crate::config::ConfigFile = toml::from_str(
            r#"
[profiles.default]
host = "h"
org_id = 1
api_key = "k"
api_secret = "s"

[profiles.default.labels]
ignore_keys = ["os", "app"]
"#,
        )
        .unwrap();
        let cfg = crate::config::resolve(
            &file,
            &crate::config::Overrides::default(),
            &Default::default(),
        )
        .unwrap();
        let limits = Limits::from_config(&cfg);
        assert_eq!(limits.ignore_keys, vec!["os"], "app is a scope key");
        assert_eq!(limits.key_order().ignored(), vec!["os"]);
        let mut m = crate::tui::test_support::model_fixture();
        m.limits = limits;
        m.reset_data();
        assert!(!m.traffic.key_order.shows("os"));
    }

    #[test]
    fn the_label_model_sets_the_group_keys_and_empty_falls_back_to_app_env() {
        let mut m = crate::tui::test_support::model_fixture();
        assert_eq!(m.traffic.group_keys, vec!["app", "env"]);
        assert_eq!(m.traffic.key_order.first(), vec!["app", "env", "role"]);
        m.set_label_model(
            vec!["site".into(), "zone".into()],
            "tier".into(),
            vec!["os".into()],
        );
        assert_eq!(m.traffic.group_keys, vec!["site", "zone"]);
        assert_eq!(m.limits.tier_key, "tier");
        assert_eq!(m.traffic.key_order.first(), vec!["site", "zone", "tier"]);
        assert_eq!(m.traffic.key_order.ignored(), vec!["os"]);
        m.reset_data();
        assert_eq!(
            m.traffic.key_order.first(),
            vec!["site", "zone", "tier"],
            "a profile switch's reset keeps the profile's order"
        );
        assert_eq!(m.traffic.key_order.ignored(), vec!["os"]);
        m.set_label_model(Vec::new(), "role".into(), Vec::new());
        assert_eq!(m.traffic.group_keys, vec!["app", "env"]);
        assert_eq!(m.traffic.key_order.first(), vec!["role"]);
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

#[cfg(test)]
mod focus_tests {
    use super::*;
    use crate::tui::test_support::{model_with_flows, model_with_rich_flows};
    use pce::traffic::{Focus, FocusDirection};

    fn app(s: &str) -> Vec<String> {
        s.split('/').map(str::to_string).collect()
    }

    fn keys() -> Vec<String> {
        vec!["app".into(), "env".into()]
    }

    fn data_of(t: &TrafficState, n: usize) -> TrafficData {
        TrafficData {
            flows: t.loaded_flows()[..n].to_vec(),
            truncated: false,
            incomplete: false,
            elapsed: Duration::from_millis(900),
            cache_hit: false,
            fetched_at: Instant::now(),
            filter: t.filter.clone(),
            range: None,
        }
    }

    #[test]
    fn the_focus_narrows_every_view_and_clearing_brings_them_back() {
        let mut m = model_with_rich_flows();
        let t = &mut m.traffic;
        assert_eq!(t.flows.len(), 30);
        let crm = app("crm/prod");
        t.set_focus(Focus::new(crm.clone(), keys()).with_direction(FocusDirection::In));
        assert_eq!(t.flows.len(), 7);
        assert!(t.flows.rows().iter().all(|r| r.dst_group == "crm/prod"));
        assert!(!t.groups.is_empty() && t.groups.rows().iter().all(|g| g.dst == crm));
        assert!(!t.mesh.is_empty() && t.mesh.rows().iter().all(|r| r.dst == crm));
        assert!(t.treemap.groups.iter().all(|g| g.dst == crm));
        assert!(
            !t.blocked.is_empty() && t.blocked.rows().iter().all(|r| r.app == crm),
            "flow 6 is blocked into crm/prod"
        );
        assert_eq!(t.cycle_focus_direction(), Some(FocusDirection::Out));
        assert_eq!(t.flows.len(), 4);
        assert_eq!(t.cycle_focus_direction(), Some(FocusDirection::Both));
        assert_eq!(
            t.flows.len(),
            9,
            "flows 3 and 4 are crm/prod to itself: once"
        );
        assert!(t.clear_focus());
        assert_eq!(t.flows.len(), 30);
        assert!(!t.clear_focus(), "nothing left to clear");
        assert_eq!(t.cycle_focus_direction(), None);
    }

    #[test]
    fn the_decision_filter_and_by_service_work_on_top_of_the_focus() {
        let mut m = model_with_rich_flows();
        let t = &mut m.traffic;
        t.set_focus(Focus::new(app("crm/prod"), keys()));
        t.apps_state.decisions = Decisions::BlockedOnly;
        t.apps_state.by_service = true;
        t.rebuild_pairs();
        let rows: Vec<String> = t
            .services
            .rows()
            .iter()
            .map(|g| {
                format!(
                    "{}>{} {}",
                    g.src.join("/"),
                    g.dst.join("/"),
                    g.service.clone().unwrap_or_default()
                )
            })
            .collect();
        assert_eq!(
            rows,
            vec!["crm/dev>crm/prod 3306/tcp"],
            "the one blocked flow of crm/prod"
        );
    }

    #[test]
    fn f_takes_the_source_then_the_destination_and_in_sections_the_destination_first() {
        let mut m = model_with_flows();
        let t = &mut m.traffic;
        t.flows
            .select_where(|r| r.flow.dst.display_name() == "fqdn-1.example");
        assert_eq!(
            t.row_apps(),
            vec![app("shop/prod"), app("fqdn-1.example/-")]
        );
        assert_eq!(t.next_focus_app(), Some(app("shop/prod")));
        t.set_focus(Focus::new(app("shop/prod"), keys()));
        t.flows
            .select_where(|r| r.flow.dst.display_name() == "fqdn-1.example");
        assert_eq!(
            t.next_focus_app(),
            Some(app("fqdn-1.example/-")),
            "a second F switches sides"
        );
        t.flows.select_where(|r| r.flow.num_connections == 310);
        assert_eq!(
            t.row_apps(),
            vec![app("shop/prod")],
            "a flow to itself has one app"
        );
        t.view = TrafficView::Blocked;
        t.blocked.select_where(|r| !r.is_header());
        assert_eq!(
            t.row_apps()[0],
            app("shop/prod"),
            "Blocked is in: destination first"
        );
        t.blocked.select_where(|r| r.is_header());
        assert_eq!(t.row_apps(), vec![app("shop/prod")], "a header is its app");
        t.view = TrafficView::Mesh;
        t.mesh
            .select_where(|r| r.dst.join("/") == "fqdn-1.example/-");
        assert_eq!(
            t.row_apps(),
            vec![app("shop/prod"), app("fqdn-1.example/-")],
            "a Mesh row: the source, then the destination"
        );
        t.mesh.select_where(|r| r.src == r.dst);
        assert_eq!(t.row_apps(), vec![app("shop/prod")], "to itself: one app");
        t.flows.set_rows(Vec::new());
        t.view = TrafficView::Flows;
        assert!(t.row_apps().is_empty());
        assert_eq!(t.next_focus_app(), None);
        assert_eq!(t.loaded_apps(), vec!["shop/prod"]);
    }

    #[test]
    fn a_focus_on_a_truncated_result_is_partial_until_its_focused_query_is_loaded() {
        let mut m = model_with_flows();
        let t = &mut m.traffic;
        t.data.as_mut().unwrap().truncated = true;
        let f = Focus::new(app("shop/prod"), keys());
        t.set_focus(f.clone());
        assert!(t.focus_partial());
        assert_eq!(t.focus_requery(), Some(f.clone()));
        assert_eq!(
            t.focus_meta().as_deref(),
            Some("focus is partial: r re-queries shop/prod · both")
        );
        let focused = data_of(t, 2);
        t.apply_focused(f.clone(), 2, focused);
        assert!(!t.focus_partial());
        assert_eq!(
            t.focus_meta().as_deref(),
            Some("focus shop/prod · both · 2 queries")
        );
        assert_eq!(t.loaded_flows().len(), 2);
        assert_eq!(
            t.focus_requery(),
            Some(f.clone()),
            "r refreshes the focused result"
        );
        assert_eq!(t.cycle_focus_direction(), Some(FocusDirection::In));
        assert!(!t.focus_partial(), "a both result covers in");
        assert!(t.clear_focus());
        assert_eq!(t.loaded_flows().len(), 4, "the unfocused result is back");
        assert!(t.data.as_ref().unwrap().truncated);
        assert!(t.unfocused.is_none() && t.focus_result.is_none());
    }

    #[test]
    fn a_focused_result_of_another_app_goes_back_first_and_window_keys_go_back_too() {
        let mut m = model_with_flows();
        let t = &mut m.traffic;
        t.data.as_mut().unwrap().truncated = true;
        let f = Focus::new(app("shop/prod"), keys()).with_direction(FocusDirection::In);
        let focused = data_of(t, 1);
        t.apply_focused(f, 1, focused);
        assert_eq!(t.loaded_flows().len(), 1);
        t.set_focus(Focus::new(app("unmanaged-nas/-"), keys()));
        assert!(
            t.unfocused.is_none(),
            "another app: the unfocused result is back first"
        );
        assert_eq!(t.loaded_flows().len(), 4);
        assert!(t.focus_partial(), "still truncated: partial");
        assert!(!t.back_to_unfocused(), "nothing focused to go back from");
        let f = Focus::new(app("shop/prod"), keys());
        let focused = data_of(t, 2);
        t.apply_focused(f, 2, focused);
        assert!(t.back_to_unfocused());
        assert_eq!(t.loaded_flows().len(), 4);
        assert!(t.focus.is_some(), "the focus stays");
        assert!(t.drop_focus(), "drop_focus reports the focus it dropped");
        assert!(t.focus.is_none() && t.unfocused.is_none());
    }

    #[test]
    fn queries_text_counts() {
        assert_eq!(queries_text(1), "1 query");
        assert_eq!(queries_text(2), "2 queries");
    }

    #[test]
    fn the_focused_flows_are_computed_once_per_focus_or_data_change() {
        let mut m = model_with_rich_flows();
        let t = &mut m.traffic;
        assert!(
            std::ptr::eq(t.view_flows().as_ptr(), t.loaded_flows().as_ptr()),
            "no focus: the loaded flows, borrowed"
        );
        t.set_focus(Focus::new(app("crm/prod"), keys()).with_direction(FocusDirection::In));
        assert_eq!(t.view_flows().len(), 7, "focus set");
        let cached = t.view_flows().as_ptr();
        t.rebuild_pairs();
        assert_eq!(
            t.view_flows().as_ptr(),
            cached,
            "a rebuild of one view reuses it"
        );
        t.cycle_focus_direction();
        assert_eq!(t.view_flows().len(), 4, "X: recomputed");
        let more = t.loaded_flows()[..3].to_vec();
        t.data.as_mut().unwrap().flows = more;
        t.rebuild_views();
        assert_eq!(
            t.view_flows().len(),
            t.loaded_flows()
                .iter()
                .filter(|f| t.focus.as_ref().unwrap().keeps(f))
                .count(),
            "new data: recomputed"
        );
        assert!(t.drop_focus());
        assert_eq!(
            t.view_flows().len(),
            3,
            "dropped without a rebuild: every flow"
        );
        let data = data_of(t, 3);
        t.apply_focused(Focus::new(app("crm/prod"), keys()), 2, data);
        assert!(t.restore_unfocused());
        assert_eq!(
            t.view_flows().len(),
            t.loaded_flows()
                .iter()
                .filter(|f| t.focus.as_ref().unwrap().keeps(f))
                .count(),
            "the unfocused result back: recomputed without a rebuild"
        );
    }

    #[test]
    fn a_truncated_result_names_its_cap_and_claims_no_span() {
        let mut m = model_with_flows();
        let d = m.traffic.data.as_mut().unwrap();
        let n = d.flows.len();
        assert_eq!(d.count_text(), format!("{n} flows"));
        d.truncated = true;
        d.filter.max_results = 1000;
        assert_eq!(
            d.count_text(),
            format!("{n} flows, capped at 1000; B raises the cap")
        );
    }

    #[test]
    fn f_on_a_peer_row_of_both_sections_takes_its_sections_app_first() {
        let mut m = model_with_flows();
        let t = &mut m.traffic;
        t.view = TrafficView::Aggregate;
        t.apps_state.by = By::Both;
        t.rebuild_sections();
        let peer = |d: Direction, text: &'static str| {
            move |r: &SectionRow| {
                r.peer()
                    .is_some_and(|(rd, p)| rd == d && p.peer_text() == text)
            }
        };
        t.apps.select_where(peer(Direction::In, "unmanaged-nas/-"));
        assert_eq!(
            t.row_apps(),
            vec![app("shop/prod"), app("unmanaged-nas/-")],
            "an inbound section: its app (the destination) first"
        );
        t.apps
            .select_where(peer(Direction::Out, "fqdn-1.example/-"));
        assert_eq!(
            t.row_apps(),
            vec![app("shop/prod"), app("fqdn-1.example/-")],
            "an outbound section: its app (the source) first"
        );
    }
}
