//! The Traffic screen's state: the auto-refresh, the focused result, the
//! drill and every view rebuilt from the loaded flows.
use super::*;

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

/// Why the loaded traffic may miss flows of an app (`complete_for`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Gap {
    /// No result is loaded.
    NoData,
    /// The result used hit its cap; `focused`: it is the app's own focused
    /// result, so only a larger cap or a narrower window helps.
    Capped { cap: u32, focused: bool },
    /// A supercluster region did not answer.
    Region { focused: bool },
    /// The only result loaded is a focused one that does not cover this
    /// side (an `in` query, and the `out` side is asked).
    Uncovered,
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
    /// The Flows view by app (`x`): `flows`' rows, in its sort order,
    /// under one header per app; empty while flat.
    pub flows_grouped: TableView<SectionRow>,
    pub flows_state: FlowsState,
    /// The sections behind `apps`, `blocked` and `flows_grouped` (with
    /// Blocked's accepted records per app): a fold or `s` lays them out
    /// again without regrouping the flows.
    apps_cache: Vec<AppSection>,
    blocked_cache: (Vec<AppSection>, BTreeMap<Vec<String>, u64>),
    flows_cache: Vec<FlowSection>,
    /// The Flows sections were not built (another view on screen): built
    /// when the Flows view shows again (`sync_flows_grouped`).
    flows_stale: bool,
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
    /// `marks.toml` (no file until `app::run` gives the path): refreshed
    /// at every `rebuild_views`, one `stat`, never a request.
    pub marks: MarksStore,
    /// The PCE the marks are scoped to: the profile's.
    pub marks_pce: PceId,
    /// When the marks were last judged (expiry).
    pub marks_now: jiff::Timestamp,
    /// `M`: squelched flows shown (dimmed `~`) instead of left out.
    pub show_squelched: bool,
    /// What the marks did to the result on screen; `None` without a valid
    /// file holding an active (or draft-waiting) mark.
    pub marked: Option<MarkedView>,
    /// The flows the views aggregate while squelched ones are left out.
    pub(crate) unsquelched: Option<Vec<ResolvedFlow>>,
    /// A status the marks file asks for; `update` shows it once.
    pub(crate) marks_notice: Option<StatusLine>,
    /// The startup line on marks expiring soon was considered.
    pub(crate) marks_announced: bool,
    /// `c`: compare mode, the baseline and what it shows; `None` off.
    pub compare: Option<CompareState>,
    /// The baseline's flows as the views compare them: the focus keeps
    /// them, squelch hides them as it hides the recent ones (unless `M`),
    /// merged on their own. Computed at `rebuild_views`, never per draw.
    baseline_view: Option<Vec<ResolvedFlow>>,
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
pub(super) fn decisions_suffix(d: Decisions) -> String {
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
                decisions.keeps(f)
                    && &group_key(&f.src, keys) == src
                    && &group_key(&f.dst, keys) == dst
            }
            Drill::Service(src, dst, service, decisions) => {
                decisions.keeps(f)
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
            flows_grouped: TableView::new(flow_columns_of())
                .with_matcher(section_matches)
                .with_match_group(section_group),
            flows_state: FlowsState::default(),
            apps_cache: Vec::new(),
            blocked_cache: (Vec::new(), BTreeMap::new()),
            flows_cache: Vec::new(),
            flows_stale: false,
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
            apps: TableView::new(app_columns())
                .with_matcher(section_matches)
                .with_match_group(section_group),
            apps_state: AppState::default(),
            blocked: TableView::new(blocked_columns())
                .with_matcher(section_matches)
                .with_match_group(section_group),
            blocked_folded: BTreeSet::new(),
            treemap: TreemapState::new(TreemapRect::default()),
            drill: None,
            auto: AutoRefresh::new(auto_min_seconds),
            rule_form: None,
            focus: None,
            focus_result: None,
            unfocused: None,
            focused: None,
            marks: MarksStore::none(),
            marks_pce: PceId {
                host: String::new(),
                org: 0,
            },
            marks_now: jiff::Timestamp::now(),
            show_squelched: false,
            marked: None,
            unsquelched: None,
            marks_notice: None,
            marks_announced: false,
            compare: None,
            baseline_view: None,
        }
    }

    /// The loaded flows (none before the first query).
    pub fn loaded_flows(&self) -> &[ResolvedFlow] {
        self.data.as_ref().map_or(&[], |d| d.flows.as_slice())
    }

    /// The loaded flows the focus keeps (all of them, borrowed, without
    /// one), squelched ones left out unless `M`: what every view
    /// aggregates, under the drill and `d`.
    pub fn view_flows(&self) -> &[ResolvedFlow] {
        match &self.unsquelched {
            Some(f) => f,
            None => self.focus_flows(),
        }
    }

    /// The loaded flows the focus keeps, marks not applied: what the
    /// marks are matched against.
    pub fn focus_flows(&self) -> &[ResolvedFlow] {
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
            self.cap_from_data();
        }
        stale
    }

    /// The filter's cap back to the result on screen: a focused cap step
    /// that was cancelled, or left behind with its focused result, never
    /// ran at the filter's cap.
    fn cap_from_data(&mut self) {
        if let Some(d) = &self.data {
            self.filter.max_results = d.filter.max_results;
        }
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
        self.cancel_draft_run();
        self.rebuild_views();
    }

    /// The focus gone, a focused re-query in flight cancelled and the
    /// unfocused result back, without a rebuild; whether there was a focus.
    pub fn drop_focus(&mut self) -> bool {
        self.restore_unfocused();
        let had = self.focus.take().is_some();
        self.cancel_stale_focused_run();
        if had {
            self.cancel_draft_run();
        }
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
                // Another result on screen: a draft run for the focused one is stale.
                self.cancel_draft_run();
                self.data = Some(d);
                self.cap_from_data();
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
        self.cancel_draft_run();
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
            TrafficView::Flows if self.flows_grouped() => self.section_apps(),
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
        match &r.kind {
            RowKind::Peer { row, .. } => vec![r.app.clone(), row.peer.clone()],
            // The section's app first (by destination app, the destination).
            RowKind::Flow { row } => {
                let (src, dst) = (
                    group_key(&row.flow.src, &self.keys),
                    group_key(&row.flow.dst, &self.keys),
                );
                if dst == r.app {
                    vec![dst, src]
                } else {
                    vec![src, dst]
                }
            }
            RowKind::Header { .. } => vec![r.app.clone()],
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
            // Only another side's focused result is loaded.
            (Some(_), None) => true,
            _ => self.data.as_ref().is_some_and(|d| d.truncated),
        }
    }

    /// Whether `flows_for(focus)` holds every flow of `focus`: the result
    /// actually used is neither truncated nor incomplete. Unlike
    /// [`Self::partial_for`], a covering focused result that is itself
    /// truncated is a gap, and so is a region that did not answer.
    pub fn complete_for(&self, focus: &Focus) -> Result<(), Gap> {
        let focused = self.focus_result.as_ref().is_some_and(|r| r.covers(focus));
        let d = self.used_for(focus)?;
        if d.truncated {
            return Err(Gap::Capped {
                cap: d.filter.max_results,
                focused,
            });
        }
        if d.incomplete {
            return Err(Gap::Region { focused });
        }
        Ok(())
    }

    /// The result `flows_for(focus)` reads, whatever its completeness: a
    /// focused result covering `focus`, else the unfocused one. A focused
    /// result for another side alone is [`Gap::Uncovered`].
    pub fn used_for(&self, focus: &Focus) -> Result<&TrafficData, Gap> {
        let focused = self.focus_result.as_ref().is_some_and(|r| r.covers(focus));
        match (&self.unfocused, focused) {
            (Some(u), false) => Ok(u),
            (None, false) if self.focus_result.is_some() => Err(Gap::Uncovered),
            _ => self.data.as_ref().ok_or(Gap::NoData),
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

    /// Recomputes every view from `data` with the keys in force. Pure
    /// apart from one `stat` of `marks.toml` (a local file, re-read only
    /// when it changed); never requests.
    pub fn rebuild_views(&mut self) {
        self.sync_draft_columns();
        self.keys = self.resolve_keys();
        self.refresh_focused();
        self.refresh_marks();
        self.refresh_baseline();
        let (shown, tree, mesh_rows, counts, compared) = {
            let keys = &self.keys;
            let flows = self.view_flows();
            let hits = self.marked.as_ref().map(|m| m.hits.as_slice());
            let drilled = |f: &ResolvedFlow| self.drill.as_ref().is_none_or(|d| d.matches(f, keys));
            let changes = self
                .baseline_view
                .as_ref()
                .map(|b| pce::traffic::compare_flows(flows, b));
            let mut shown: Vec<FlowRow> = flows
                .iter()
                .enumerate()
                .filter(|(_, f)| drilled(f))
                .map(|(i, f)| FlowRow {
                    mark: hits.and_then(|h| h.get(i).copied().flatten()),
                    change: changes.as_ref().map(|c| c.recent[i]),
                    ..FlowRow::new(f.clone(), keys)
                })
                .collect();
            // Gone flows, under the drill too, with the baseline's values.
            for (f, rc) in changes.into_iter().flat_map(|c| c.gone) {
                if drilled(&f) {
                    shown.push(FlowRow {
                        change: Some(rc),
                        ..FlowRow::new(f, keys)
                    });
                }
            }
            let tree = group_by_labels(flows, keys);
            let cols = self.mesh_cols;
            let mut mesh_rows = mesh(flows, keys, cols);
            let mut tree = tree;
            let counts = self.marks_ctx().map(|(a, mv)| {
                annotate_groups(&mut tree, &mv.marked, &a, |f| group_by_labels(f, keys));
                let gone = annotate_mesh(&mut mesh_rows, &mv.marked, &a, |f| mesh(f, keys, cols));
                let flows = mv.marked.squelched_where(drilled);
                (
                    Squelch { rows: flows, flows },
                    self.squelch(gone, |_| true),
                    self.squelch(0, |_| true),
                )
            });
            let compared = self.baseline_view.as_ref().map(|b| {
                let mesh_counts = pce::traffic::compare_mesh(&mut mesh_rows, &mesh(b, keys, cols));
                let mut flow_counts = pce::traffic::CompareCounts::default();
                for r in &shown {
                    if let Some(c) = &r.change {
                        flow_counts.add(c.change);
                    }
                }
                (flow_counts, mesh_counts)
            });
            (shown, tree, mesh_rows, counts, compared)
        };
        if let (Some(mv), Some((flows, mesh, treemap))) = (self.marked.as_mut(), counts) {
            mv.flows = flows;
            mv.mesh = mesh;
            mv.treemap = treemap;
        }
        let (mut shown, mut mesh_rows) = (shown, mesh_rows);
        if let (Some(cmp), Some((flows, mesh))) = (self.compare.as_mut(), compared) {
            cmp.counts.flows = flows;
            cmp.counts.mesh = mesh;
            let show = cmp.show.set();
            shown.retain(|r| show.keeps(r.change.as_ref()));
            mesh_rows.retain(|r| show.keeps(r.change.as_ref()));
        }
        self.flows.set_rows(shown);
        self.treemap.groups = tree;
        self.mesh.set_rows(mesh_rows);
        self.rebuild_pairs();
        self.rebuild_sections();
        self.relayout_treemap();
    }

    /// The baseline's flows the views compare with (`baseline_view`): the
    /// focus and squelch applied as to the recent flows, merged on their
    /// own (one row per record, never with the recent side).
    fn refresh_baseline(&mut self) {
        let Some(b) = self.compare.as_ref().and_then(|c| c.baseline.as_ref()) else {
            self.baseline_view = None;
            return;
        };
        let focused: Vec<ResolvedFlow> = match &self.focus {
            Some(f) => b.flows.iter().filter(|x| f.keeps(x)).cloned().collect(),
            None => b.flows.clone(),
        };
        let kept = match self.active_marks().filter(|a| !a.is_empty()) {
            Some(a) => pce::marks::partition(&focused, &a, self.show_squelched)
                .kept
                .into_owned(),
            None => focused,
        };
        let mut merged = Vec::with_capacity(kept.len());
        pce::traffic::merge_flows(&mut merged, kept);
        self.baseline_view = Some(merged);
    }

    /// Compare mode shows its `Δ` column (a baseline is loaded).
    pub fn comparing(&self) -> bool {
        self.compare.as_ref().is_some_and(|c| c.baseline.is_some())
    }

    /// The flows column of the tables that show `~N` (pairs, by service,
    /// sections, Blocked): 10 cells (7 narrow, `flows_cell_narrow`) while
    /// marks are active, else the usual 6 (5).
    pub(crate) fn sync_marks_columns(&mut self, on: bool) {
        fn set<T>(t: &mut TableView<T>, on: bool, narrow: fn(&T) -> String) {
            if let Some(c) = t.columns.iter_mut().find(|c| c.title == "flows") {
                (c.width, c.narrow_width, c.narrow) = if on {
                    (10, FLOWS_NARROW_MARKED as u16, Some(narrow))
                } else {
                    (6, 5, None)
                };
            }
        }
        let group: fn(&FlowGroup) -> String = |g| flows_cell_narrow(g.flows, g.marks.as_ref());
        let section: fn(&SectionRow) -> String =
            |r| r.with_row(|p| flows_cell_narrow(p.flows, p.marks.as_ref()));
        set(&mut self.groups, on, group);
        set(&mut self.services, on, group);
        set(&mut self.apps, on, section);
        set(&mut self.blocked, on, section);
    }

    /// The `draft` columns while draft decisions are loaded, none
    /// otherwise; `fixed by draft` falls back to `all` without them.
    /// The `Δ` columns too, first, in compare mode with a baseline.
    fn sync_draft_columns(&mut self) {
        let draft = self.draft_loaded();
        let cmp = self.comparing();
        if !draft && self.apps_state.decisions == Decisions::FixedByDraft {
            self.apps_state.decisions = Decisions::All;
        }
        fn stale<T>(t: &TableView<T>, draft: bool, cmp: bool) -> bool {
            shows_draft(t) != draft || shows_delta(t) != cmp
        }
        if stale(&self.flows, draft, cmp) {
            self.flows.set_columns(with_delta(
                with_draft(
                    make_room_for_delta(
                        if draft {
                            make_room_for_draft(flow_columns())
                        } else {
                            flow_columns()
                        },
                        cmp,
                    ),
                    draft_flow_column(),
                    draft,
                ),
                delta_flow_column(),
                cmp,
            ));
        }
        if stale(&self.flows_grouped, draft, cmp) {
            self.flows_grouped.set_columns(with_delta(
                with_draft(
                    make_room_for_delta(
                        if draft {
                            make_room_for_draft(flow_columns_of())
                        } else {
                            flow_columns_of()
                        },
                        cmp,
                    ),
                    draft_flow_column_of(),
                    draft,
                ),
                delta_section_column(),
                cmp,
            ));
        }
        if stale(&self.groups, draft, cmp) {
            self.groups.set_columns(with_delta(
                with_draft(group_columns(), draft_group_column(), draft),
                delta_group_column(),
                cmp,
            ));
            self.services.set_columns(with_delta(
                with_draft(service_group_columns(), draft_group_column(), draft),
                delta_group_column(),
                cmp,
            ));
        }
        if stale(&self.mesh, draft, cmp) {
            self.mesh.set_columns(with_delta(
                with_draft(mesh_columns(self.mesh_cols), draft_mesh_column(), draft),
                delta_mesh_column(),
                cmp,
            ));
        }
        if stale(&self.apps, draft, cmp) {
            self.apps.set_columns(with_delta(
                with_draft(app_columns(), draft_section_column(), draft),
                delta_section_column(),
                cmp,
            ));
            self.blocked.set_columns(with_delta(
                with_draft(blocked_columns(), draft_section_column(), draft),
                delta_section_column(),
                cmp,
            ));
        }
    }

    /// App-to-app's pairs, and by service while `o` is on (none while off),
    /// from the flows the focus and the decision filter keep. Pure.
    pub fn rebuild_pairs(&mut self) {
        let (groups, services, counts) = {
            let st = &self.apps_state;
            let keys = &self.keys;
            let flows = self.view_flows();
            let kept = st.decisions.kept(flows);
            let mut groups = group_by_labels(&kept, keys);
            let mut services = if st.by_service {
                group_by_service(&kept, keys)
            } else {
                Vec::new()
            };
            let d = st.decisions;
            let counts = self.marks_ctx().map(|(a, mv)| {
                let pairs = annotate_groups(&mut groups, &mv.marked, &a, |f| {
                    group_by_labels(&d.kept(f), keys)
                });
                let by_service = annotate_groups(&mut services, &mv.marked, &a, |f| {
                    group_by_service(&d.kept(f), keys)
                });
                (
                    self.squelch(pairs, |f| d.keeps(f)),
                    self.squelch(by_service, |f| d.keeps(f)),
                )
            });
            (groups, services, counts)
        };
        if let (Some(mv), Some((pairs, services))) = (self.marked.as_mut(), counts) {
            mv.pairs = pairs;
            mv.services = services;
        }
        let (mut groups, mut services) = (groups, services);
        if let (Some(cmp), Some(b)) = (self.compare.as_mut(), self.baseline_view.as_ref()) {
            let (keys, d) = (&self.keys, self.apps_state.decisions);
            let kept = d.kept(b);
            cmp.counts.pairs =
                pce::traffic::compare_groups(&mut groups, &group_by_labels(&kept, keys));
            if self.apps_state.by_service {
                cmp.counts.services =
                    pce::traffic::compare_groups(&mut services, &group_by_service(&kept, keys));
            }
            let show = cmp.show.set();
            pce::traffic::keep_changes(&mut groups, show, |g| g.change.as_ref());
            pce::traffic::keep_changes(&mut services, show, |g| g.change.as_ref());
        }
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

    /// App-to-app's sections (none in `pairs`), Blocked's and the Flows
    /// view's, from the loaded flows, the keys in force, the state and the
    /// folds. Pure.
    pub fn rebuild_sections(&mut self) {
        self.rebuild_apps();
        self.rebuild_blocked();
        self.rebuild_flows_grouped();
    }

    /// App-to-app's sections and their squelched counts, then its rows.
    pub fn rebuild_apps(&mut self) {
        let mut sections = self.apps_sections(self.view_flows());
        let d = self.apps_state.decisions;
        let sq = self
            .marks_ctx()
            .map(|(a, mv)| {
                let gone =
                    annotate_sections(&mut sections, &mv.marked, &a, |f| self.apps_sections(f));
                self.squelch(gone, |f| d.keeps(f))
            })
            .unwrap_or_default();
        if let Some(mv) = self.marked.as_mut() {
            mv.sections = sq;
        }
        if let Some(b) = self.baseline_view.as_ref() {
            let base = self.apps_sections(b);
            if let Some(cmp) = self.compare.as_mut() {
                cmp.counts.apps = pce::traffic::compare_sections(&mut sections, &base);
                pce::traffic::keep_section_changes(&mut sections, cmp.show.set());
            }
        }
        self.apps_cache = sections;
        let rows = self.apps_rows(&self.apps_state.folded);
        self.apps.set_rows(rows);
    }

    /// Blocked's sections, its squelched counts and its records still
    /// blocked and accepted (the accepted ones taken out of each header's
    /// blocked counts), then its rows.
    pub fn rebuild_blocked(&mut self) {
        let keys = &self.keys;
        let blocked = |f: &[ResolvedFlow]| app_sections(f, keys, Direction::In, Decisions::Blocked);
        let mut sections = blocked(self.view_flows());
        let mut accepted: BTreeMap<Vec<String>, u64> = BTreeMap::new();
        let mut sq = Squelch::default();
        if let Some((a, mv)) = self.marks_ctx() {
            let gone = annotate_sections(&mut sections, &mv.marked, &a, blocked);
            sq = self.squelch(gone, |f| Decisions::Blocked.keeps(f));
            for acc in blocked(&self.accepted_flows()) {
                if let Some(s) = sections.iter_mut().find(|s| s.app == acc.app) {
                    // A subset of the section's own records; saturating
                    // all the same, so a count can never panic.
                    s.totals.blocked = s.totals.blocked.saturating_sub(acc.totals.blocked);
                    s.totals.potentially_blocked = s
                        .totals
                        .potentially_blocked
                        .saturating_sub(acc.totals.potentially_blocked);
                    accepted.insert(acc.app, acc.totals.flows);
                }
            }
        }
        let open = sections
            .iter()
            .map(|s| s.totals.blocked + s.totals.potentially_blocked)
            .sum();
        if let Some(mv) = self.marked.as_mut() {
            mv.blocked = sq;
            mv.blocked_open = open;
            mv.accepted = accepted.values().sum();
        }
        if let (Some(cmp), Some(b)) = (self.compare.as_mut(), self.baseline_view.as_ref()) {
            cmp.counts.blocked = pce::traffic::compare_sections(&mut sections, &blocked(b));
            pce::traffic::keep_section_changes(&mut sections, cmp.show.set());
        }
        self.blocked_cache = (sections, accepted);
        let rows = self.blocked_rows(&self.blocked_folded);
        self.blocked.set_rows(rows);
    }

    /// The Flows view's sections, while it shows them; otherwise none,
    /// built when it shows again (`sync_flows_grouped`). The flat table's
    /// rows as shown (drilled, its search and sort applied, squelched
    /// flows left out unless `M`) under their app on the grouping's side.
    /// The marks counts are the flat view's: the same flows, so kept +
    /// squelched still equals all; a header sums its flows' hits and the
    /// squelched records of its app left out, and an app whose every flow
    /// is squelched has no section.
    pub fn rebuild_flows_grouped(&mut self) {
        let dir = self.flows_state.by.direction();
        let Some(dir) = dir.filter(|_| self.view == TrafficView::Flows) else {
            self.flows_cache = Vec::new();
            self.flows_grouped.set_rows(Vec::new());
            self.flows_stale = dir.is_some();
            return;
        };
        self.flows_stale = false;
        let keys = &self.keys;
        let rows = self.flows.rows();
        let shown = self.flows.visible();
        let sections = flow_sections(
            shown.iter().map(|&i| &*rows[i].flow),
            keys,
            dir,
            self.flows_state.order,
        );
        let ctx = self.marks_ctx();
        let mut hidden: BTreeMap<Vec<String>, u64> = BTreeMap::new();
        if let Some((_, mv)) = &ctx
            && !mv.marked.keep_squelched
        {
            let drilled = |f: &ResolvedFlow| self.drill.as_ref().is_none_or(|d| d.matches(f, keys));
            for f in mv.marked.squelched().filter(|f| drilled(f)) {
                *hidden
                    .entry(pce::traffic::sides(f, keys, dir).0)
                    .or_default() += 1;
            }
        }
        let cache = sections
            .into_iter()
            .map(|(section, idx)| {
                let flows: Vec<Arc<FlowRow>> = idx
                    .iter()
                    .map(|&k| Arc::new(rows[shown[k]].clone()))
                    .collect();
                let marks = ctx.as_ref().map(|(a, _)| {
                    let mut rm = RowMarks {
                        flows: flows.len() as u64,
                        hidden: hidden.get(&section.app).copied().unwrap_or(0),
                        ..RowMarks::default()
                    };
                    for h in flows.iter().filter_map(|f| f.mark) {
                        let i = h.action.index();
                        rm.by_action[i] += 1;
                        if rm.top[i]
                            .as_ref()
                            .is_none_or(|t| a.rank(h.mark) < a.rank(t.mark))
                        {
                            rm.top[i] = Some(pce::marks::TopMark {
                                mark: h.mark,
                                id: a.mark(h.mark).id.clone(),
                            });
                        }
                    }
                    rm
                });
                FlowSection::new(section, flows, marks)
            })
            .collect();
        self.flows_cache = cache;
        let rows = flow_section_rows(&self.flows_cache, self.flows_state.folded());
        self.flows_grouped.set_rows(rows);
    }

    /// The Flows sections, if the Flows view shows them and they were
    /// left unbuilt while another view was on screen.
    pub fn sync_flows_grouped(&mut self) {
        if self.flows_stale && self.view == TrafficView::Flows {
            self.rebuild_flows_grouped();
        }
    }

    /// The sectioned table on screen laid out again from its sections (a
    /// fold, `s`): no regrouping.
    fn relayout_sections(&mut self) {
        match self.view {
            TrafficView::Aggregate => {
                let rows = self.apps_rows(&self.apps_state.folded);
                self.apps.set_rows(rows);
            }
            TrafficView::Blocked => {
                let rows = self.blocked_rows(&self.blocked_folded);
                self.blocked.set_rows(rows);
            }
            TrafficView::Flows => {
                let rows = flow_section_rows(&self.flows_cache, self.flows_state.folded());
                self.flows_grouped.set_rows(rows);
            }
            TrafficView::Mesh | TrafficView::Treemap => {}
        }
    }

    /// App-to-app's sections in force (none in `pairs`), from `flows`.
    fn apps_sections(&self, flows: &[ResolvedFlow]) -> Vec<AppSection> {
        let keys = &self.keys;
        let st = &self.apps_state;
        let sections = match st.by {
            By::Pairs => Vec::new(),
            By::In => app_sections(flows, keys, Direction::In, st.decisions),
            By::Out => app_sections(flows, keys, Direction::Out, st.decisions),
            By::Both => app_sections_both(flows, keys, st.decisions),
        };
        order_sections(sections, st.order, |s| s)
    }

    /// App-to-app's rows under `folded` (none in `pairs`).
    fn apps_rows(&self, folded: &BTreeSet<Vec<String>>) -> Vec<SectionRow> {
        let both = self.apps_state.by == By::Both;
        section_rows(&self.apps_cache, folded, both)
    }

    /// Blocked's rows under `folded`, each header with its accepted records.
    fn blocked_rows(&self, folded: &BTreeSet<Vec<String>>) -> Vec<SectionRow> {
        let (sections, accepted) = &self.blocked_cache;
        let mut rows = section_rows(sections, folded, false);
        for r in &mut rows {
            if let RowKind::Header { accepted: n, .. } = &mut r.kind {
                *n = accepted.get(&r.app).copied().unwrap_or(0);
            }
        }
        rows
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
            TrafficView::Flows if self.flows_grouped() => (
                &self.flows_grouped,
                flow_section_rows(&self.flows_cache, &none),
            ),
            _ => return None,
        };
        let shown = table.visible_of(&rows);
        let mut rows: Vec<Option<SectionRow>> = rows.into_iter().map(Some).collect();
        Some(shown.into_iter().filter_map(|i| rows[i].take()).collect())
    }

    /// The sectioned table the active view shows, if any: App-to-app
    /// outside `pairs`, Blocked, or Flows by app.
    pub fn section_table(&self) -> Option<&TableView<SectionRow>> {
        match self.view {
            TrafficView::Aggregate if self.apps_state.by != By::Pairs => Some(&self.apps),
            TrafficView::Blocked => Some(&self.blocked),
            TrafficView::Flows if self.flows_grouped() => Some(&self.flows_grouped),
            _ => None,
        }
    }

    /// The Flows view shows app sections (`x`), not the flat table.
    pub fn flows_grouped(&self) -> bool {
        self.flows_state.by != FlowsBy::Flat
    }

    /// The flow under the cursor of the Flows view, flat or by app (none
    /// on a section header).
    pub fn selected_flow(&self) -> Option<&FlowRow> {
        if self.flows_grouped() {
            self.flows_grouped
                .selected_item()
                .and_then(SectionRow::flow)
        } else {
            self.flows.selected_item()
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
        self.relayout_sections();
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
                RowKind::Peer { .. } | RowKind::Flow { .. } => None,
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
        self.relayout_sections();
        if let (Some(app), Some(t)) = (app, self.section_table_mut()) {
            t.select_where(|r| r.is_header() && r.app == app);
        }
    }

    pub fn section_table_mut(&mut self) -> Option<&mut TableView<SectionRow>> {
        match self.view {
            TrafficView::Aggregate if self.apps_state.by != By::Pairs => Some(&mut self.apps),
            TrafficView::Blocked => Some(&mut self.blocked),
            TrafficView::Flows if self.flows_grouped() => Some(&mut self.flows_grouped),
            _ => None,
        }
    }

    fn folds_mut(&mut self) -> &mut BTreeSet<Vec<String>> {
        match self.view {
            TrafficView::Blocked => &mut self.blocked_folded,
            TrafficView::Flows => self.flows_state.folded_mut(),
            _ => &mut self.apps_state.folded,
        }
    }

    /// `s` on a sectioned view: the next section order (App-to-app, Flows
    /// by app), the cursor staying on its app's header. `None` where the
    /// order is fixed (Blocked).
    pub fn cycle_section_order(&mut self) -> Option<SectionOrder> {
        let order = match self.view {
            TrafficView::Aggregate => &mut self.apps_state.order,
            TrafficView::Flows => &mut self.flows_state.order,
            _ => return None,
        };
        *order = order.next();
        let order = *order;
        let app = self
            .section_table()
            .and_then(|t| t.selected_item())
            .map(|r| r.app.clone());
        // The same sections in another order: no regrouping.
        if self.view == TrafficView::Flows {
            let cache = std::mem::take(&mut self.flows_cache);
            self.flows_cache = order_sections(cache, order, |f| &f.section);
        } else {
            let cache = std::mem::take(&mut self.apps_cache);
            self.apps_cache = order_sections(cache, order, |s| s);
        }
        self.relayout_sections();
        if let (Some(app), Some(t)) = (app, self.section_table_mut()) {
            t.select_where(|r| r.is_header() && r.app == app);
        }
        Some(order)
    }

    /// The decision filter of the sectioned table on screen (Flows has
    /// none: every flow).
    pub fn section_decisions(&self) -> Decisions {
        match self.view {
            TrafficView::Blocked => Decisions::Blocked,
            TrafficView::Flows => Decisions::All,
            _ => self.apps_state.decisions,
        }
    }

    /// `Enter` on a section row: the drill for its header or peer row
    /// (`None` on a Flows section's flow: it opens the detail).
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
            RowKind::Flow { .. } => return None,
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
        tm.highlights.clear();
        tm.source_highlights.clear();
        for g in &tm.groups {
            let Some(m) = g
                .marks
                .as_ref()
                .and_then(RowMarks::marker)
                .filter(|m| m.action().is_some_and(|a| a.is_highlight()))
            else {
                continue;
            };
            let src = g.src.join("/");
            tm.highlights.insert((src.clone(), g.dst.join("/")), m);
            let strongest = |x: pce::marks::Marker| x.action();
            match tm.source_highlights.get(&src) {
                Some(old) if strongest(*old) <= strongest(m) => {}
                _ => {
                    tm.source_highlights.insert(src, m);
                }
            }
        }
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
            TrafficView::Flows if self.flows_grouped() => self.flows_grouped.is_searching(),
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

    /// `D` loaded draft decisions into the result on screen.
    pub fn draft_loaded(&self) -> bool {
        self.data.as_ref().is_some_and(|d| d.draft.is_some())
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
            draft: false,
            baseline: false,
        };
        (self.seq, cancel)
    }

    /// `start_query` for compare mode's baseline (`c`, `B` on a truncated
    /// baseline): the result on screen stays.
    pub fn start_baseline_run(&mut self, now: Instant) -> (u64, CancellationToken) {
        let last_run = self.auto.last_run;
        let started = self.start_query(now);
        // Not a refresh of the result: the auto-refresh clock keeps its time.
        self.auto.last_run = last_run;
        if let QueryState::Running { baseline, .. } = &mut self.query {
            *baseline = true;
        }
        started
    }

    /// The baseline query is on its way.
    pub fn is_baseline_running(&self) -> bool {
        matches!(self.query, QueryState::Running { baseline: true, .. })
    }

    /// Compare mode off (`C`, a filter or window change, a focused
    /// result), a baseline run cancelled; whether it was on. Rebuilds.
    pub fn end_compare(&mut self) -> bool {
        if self.is_baseline_running() {
            self.cancel_query();
        }
        let was = self.compare.take().is_some();
        if was {
            self.rebuild_views();
        }
        was
    }

    /// `start_query` for `D`: a draft re-evaluation of the result on
    /// screen.
    /// The run keeps no focus (that is for `r`'s focused re-queries): any
    /// focus change cancels a draft run (`cancel_draft_run`).
    pub fn start_draft_run(&mut self, now: Instant) -> (u64, CancellationToken) {
        let started = self.start_query(now);
        if let QueryState::Running { draft, .. } = &mut self.query {
            *draft = true;
        }
        started
    }

    /// A `D` re-evaluation is on its way.
    pub fn is_draft_running(&self) -> bool {
        matches!(self.query, QueryState::Running { draft: true, .. })
    }

    /// Cancels a `D` re-evaluation in flight (its answer would land on a
    /// result that changed); whether one was cancelled.
    pub fn cancel_draft_run(&mut self) -> bool {
        let running = self.is_draft_running();
        if running {
            self.cancel_query();
        }
        running
    }

    /// After a write: draft decisions on screen (and on the unfocused
    /// result kept aside) are dropped, and a draft run in flight is
    /// cancelled, since the draft policy they came from may have changed.
    /// Whether anything was dropped. No request.
    pub fn drop_draft(&mut self) -> bool {
        let running = self.cancel_draft_run();
        let mut dropped = false;
        if let Some(d) = self.unfocused.as_mut().filter(|d| d.draft.is_some()) {
            for f in &mut d.flows {
                f.draft_policy_decision = None;
            }
            d.draft = None;
            dropped = true;
        }
        self.clear_draft() || dropped || running
    }

    /// `D` on a result with draft decisions: back to the reported ones, no
    /// request (the draft result stays cached for the next `D`).
    pub fn clear_draft(&mut self) -> bool {
        let Some(d) = self.data.as_mut().filter(|d| d.draft.is_some()) else {
            return false;
        };
        for f in &mut d.flows {
            f.draft_policy_decision = None;
        }
        d.draft = None;
        self.rebuild_views();
        true
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
