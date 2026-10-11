# Traffic views: label grouping everywhere, mesh and treemap (issue #6)

Date: 2026-10-11. Status: approved in chat by Alex ("looks right, go ahead").
Issue: https://github.com/alexgoller/illumio-tui/issues/6. Also folds in issue #5
(IP lists under Objects) because both touch the tab row.

## 1. Intent

People look at traffic through their label model. For most of them that is
application and environment; role and location come next; some have more
keys. The Traffic screen today groups only its App-to-app view, only by
`app,env`, and shows every label everywhere else. This plan makes the
chosen label keys the lens for every traffic view, adds a mesh view like the
Illumio UI's (user, process, source, destination, service, destination
process) and a treemap view of the same data, and exposes the aggregations
to the CLI for agents. Nothing in it sends a new request: every view is a
projection of the flows the current query already returned.

What Alex said, verbatim where it matters:

- "app|env is a given for most people"; "App, Env, Role, Location are the 4
  basics, more is definitely a plus, but we should stay user centric".
- Grouping must apply "not only by app|env (cycling through it with v), but
  also in the main view and the blocked view".
- Mesh: "source user, source process, source (group labels as specified by
  user, app|env as default), destination (same groupings), port/proto,
  destination process if available".
- Treemap: "a treemap view of the traffic (always use the current query data
  so we are smart on querying)".
- Mesh sits beside App-to-app (not instead of it).
- Treemap nests source → destination, area by connections.
- Default keys from the profile label model; `g` picks any keys.
- CLI `traffic mesh` and `traffic groups` for agents.

## 2. Global constraints

- Rust edition 2024, rust-version 1.88. No new dependencies (ratatui 0.30,
  crossterm, serde, jiff, clap 4, insta, wiremock, assert_cmd as today).
- Never hammer the PCE: no view, key, zoom or column change may emit a
  request `Cmd`. Only `r`, `f`+Enter and auto-refresh query, as today.
- Elm-style TUI: pure `update`, `Runtime` executes `Cmd`s. Pure aggregation
  lives in the `pce` crate under `pce::traffic`.
- Label model: profile `[profiles.<p>.labels] scope_keys` (default
  `["app","env"]`), `tier_key` (default `role`), env overrides
  `ILLUMIO_SCOPE_KEYS` / `ILLUMIO_TIER_KEY`. Existing. Nothing here writes
  config.
- Commit trailer: `Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>`.
- Smokes against demo100 are read-only and never press `y`.

## 3. IP lists under Objects (issue #5)

Alex, after the first reorder ask: IP lists belong under Objects, as a tab
beside Labels, Label groups and Services (not under Services).
`Screen::IpLists` goes away; IP lists become the fourth Objects tab
(Labels, Label groups, Services, IP lists), a sibling of the others, keeping the
Plan 7 table (name, size, ranges, FQDNs, used by), the detail (size, used
by, ranges, FQDNs, capped), the editing keys and the cache-aware load on
first visit. Screens are five again: `1` Traffic, `2` Policy, `3` Objects,
`4` Workloads, `5` Events. `:iplists` / `:ip` open Objects on that tab. Tab
row, help overlay, status hints, README, architecture doc and index.html
follow; every screen snapshot re-records with the tab row as the only
change, and the IP lists snapshots move to Objects. No behaviour change in
the IP lists table, detail, sizes or writes.

## 4. Display label keys

### 4.1 Model

`TrafficState.group_keys: Vec<String>` already exists. It is initialised from
`limits.scope_keys` (the resolved profile label model) instead of a
hard-coded `app,env`, when the profile loads and on every `:profile` switch.
It is session state; `g` changes it; nothing persists it.

`TrafficState.all_keys: bool` (default false): when true, the chosen keys are
"every key seen in the loaded flows", in sorted order with `app`, `env`,
`role`, `loc` first when present. `rebuild_views` resolves the effective keys
once (`effective_keys()`) and uses them for every view.

### 4.2 The `g` picker

`g` replaces the free-text prompt with `Overlay::GroupKeys(KeyPicker)`:

```
┌ Group by ──────────────────────────────┐
│ [x] app                                │
│ [x] env                                │
│ [ ] role                               │
│ [ ] loc                                │
│ [ ] datacenter      (seen in 12 flows) │
│ [ ] all labels                         │
│                                        │
│ keys: app,env                          │
│ Space toggle · Enter apply · Esc close │
└────────────────────────────────────────┘
```

- Entries: the profile scope keys and tier key first, then `loc`, then every
  other key seen in the loaded flows (count of flows carrying it in dim
  text), then `all labels`. A key seen in no flow is still listed if it is a
  profile key.
- `j/k` or arrows move, Space toggles, Enter applies, Esc closes unchanged.
  Toggling `all labels` on unchecks the others and sets `all_keys`; toggling
  any key on clears `all_keys`.
- The `keys:` line is editable: typing a printable character or Backspace
  edits the text form (`app,env,role`); Enter on an edited text applies it
  split on commas, trimmed, empties dropped, unknown keys allowed (they show
  as `-`). Empty text falls back to the profile scope keys. This keeps the
  old `g` then `app,env,role` muscle memory working.
- Applying clears `traffic.drill` and any treemap zoom and calls
  `rebuild_views()`. No `Cmd`.
- The status hint on Traffic names `g group`.

### 4.3 Where the keys show

Every traffic view uses the effective keys:

- **Flows**: the `src labels` column (today hidden below 120 cols, full
  text) becomes `src` grouping values, `shop/prod`, width 18, visible at 80
  columns; a `dst` grouping column of the same shape is added after `dst`.
  Missing values show `-`. An endpoint that is not a workload shows `-` in
  every slot (its IP list or FQDN is already in the name column). The full
  `app=shop env=prod` text moves to the detail pane, which gains `src labels`
  and `dst labels` lines and `bytes in/out` when present.
- **App-to-app**: unchanged behaviour, now driven by the same keys.
- **Blocked**: grouped by `(dst group, port, proto)` instead of
  `(dst display name, port, proto)`. Columns: last seen, dst (group values),
  dst name (the display name when the group has one destination, else
  `N hosts`), service, flows, conns, sources (distinct source group values,
  max 5, `+N`). `w` on a blocked row keeps working through the first flow
  behind the row, as today.
- **Mesh** and **Treemap**: see §5 and §6.

### 4.4 `pce::traffic` additions

```rust
/// The grouping values of an endpoint for `keys`: one entry per key, `-`
/// when missing or when the endpoint is not a workload.
pub fn group_values(ep: &ResolvedEndpoint, keys: &[String]) -> Vec<String>;
/// `shop/prod`; `-` for every slot of a non-workload.
pub fn group_text(ep: &ResolvedEndpoint, keys: &[String]) -> String;
/// Every label key seen in `flows`, with the number of flows carrying it,
/// sorted by the preferred order then name.
pub fn keys_seen(flows: &[ResolvedFlow], preferred: &[String]) -> Vec<(String, usize)>;
```

`group_by_labels` keeps its signature and uses `group_values`.
`blocked_summary(flows, keys)` gains the keys parameter; `BlockedGroup` gains
`dst_group: Vec<String>` and `dst_names: Vec<String>` (distinct, max 5), and
`sources` holds source group texts.

`ResolvedFlow` gains `bytes_in: Option<u64>` and `bytes_out: Option<u64>`
from the raw flow's `dst_bi`/`dst_bo`; `resolve_flows` copies them.
`FlowGroup` gains `bytes: u64` (sum of in+out, 0 when absent). Serialised
field names are those; the JSON envelope for `traffic query --group` grows
one field, additive.

## 5. Mesh view

### 5.1 Data

```rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MeshRow {
    pub user: Option<String>,      // None when hidden or absent
    pub process: Option<String>,   // source process (flow.process_name)
    pub src: Vec<String>,          // group values
    pub dst: Vec<String>,
    pub port: Option<u16>,
    pub proto: String,
    pub dst_process: Option<String>,
    pub flows: u64,
    pub connections: u64,
    pub bytes: u64,
    pub allowed: u64,
    pub potentially_blocked: u64,
    pub blocked: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct MeshColumns { pub user: bool, pub process: bool }

pub fn mesh(flows: &[ResolvedFlow], keys: &[String], cols: MeshColumns) -> Vec<MeshRow>;
```

Rows group on `(user?, process?, src, dst, port, proto, dst_process)` where
`user`/`process` take part only when their column is shown. Sorted by
connections descending, then src, dst, service. Decisions counted like
`FlowGroup` (Unknown counts in flows/connections only).

Destination process: the PCE reports one `process_name` per flow, observed
on the reporting VEN; when `flow_direction` is `inbound` the process is the
destination's, when `outbound` the source's. `mesh` places it accordingly
and leaves the other side `None`. A flow without direction counts as source
process (today's behaviour in the Flows detail).

### 5.2 TUI

`TrafficView::Mesh`, title `Mesh`, position in the `v` cycle: Flows,
App-to-app, Mesh, Blocked, Treemap. `TrafficState` gains
`mesh: TableView<MeshRow>` and `mesh_cols: MeshColumns` (default both true).

Columns (`mesh_columns(cols)`): user 12 (hidden when off), process 14
(hidden when off), src 18, dst 18, service 10, dst proc 12 (hide_below 120),
conns 8 right, decision 10 (narrow `pot.blk`). Rows styled by dominant
decision like App-to-app.

Keys on the Mesh view: `u` toggles the user column, `p` the process column;
each regroups (`rebuild_views`) and keeps the cursor on the first row.
`Enter` drills: `traffic.drill` gains a `MeshKey` variant so the Flows view
filters to exactly the flows behind the row; the Flows title shows
`Flows ▸ <src> → <dst> <service>`; Esc clears as today. `s` sort, `/`
search, `w` on a blocked row drafts a rule from its first flow, like the
Blocked view.

The panel meta reads `keys app,env · u user · p process · Enter drills`.

## 6. Treemap view

### 6.1 Layout, pure

`pce::traffic::treemap`:

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tile {
    pub label: String,           // group text, e.g. shop/prod
    pub weight: u64,             // connections or bytes
    pub rect: Rect,              // x, y, w, h in cells, within the parent
    pub decision: Dominant,      // Allowed | PotentiallyBlocked | Blocked | Unknown
    pub children: Vec<Tile>,     // destinations, laid out inside rect
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect { pub x: u16, pub y: u16, pub w: u16, pub h: u16 }
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Weight { Connections, Bytes }

/// Squarified treemap of `groups` (from `group_by_labels`) into `area`:
/// outer tiles per src, inner per dst. Tiles narrower than 1 cell are
/// dropped and their weight is folded into an `other (N)` tile. Zero total
/// weight gives no tiles.
pub fn layout(groups: &[FlowGroup], area: Rect, weight: Weight) -> Vec<Tile>;
```

Squarified algorithm (Bruls, Huizing, van Wijk): rows are laid along the
shorter side, a tile joins the current row while the worst aspect ratio does
not get worse. Cell aspect is corrected by treating one row as two columns
(terminal cells are about 1:2), so tiles look square on screen. Determinism:
inputs sorted by weight descending then label; ties keep input order.

### 6.2 TUI

`TrafficView::Treemap`, title `Treemap`. `TrafficState` gains
`treemap: TreemapState { weight: Weight, zoom: Option<Vec<String>>, cursor: usize, tiles: Vec<Tile> }`.
`rebuild_views` recomputes `tiles` from the current groups and the treemap
area derived from the terminal size the model already stores from
`Msg::Resize` (the same layout arithmetic as the renderer: body minus the
filter and stats panels and the border). `Msg::Resize` on the Treemap view
recomputes the tiles, so the renderer only draws what the model holds.

Rendering: each outer tile is a bordered block (plain borders, title = label
truncated, dim) filled with its inner tiles; inner tiles are filled runs of
`▒` in the decision colour (`t.good`, `t.warn`, `t.bad`, dim for unknown)
with the destination label written over the fill when the tile is at least
`len+2` wide and 1 high, plus the weight (`1.2K`) on a second line when at
least 2 high. The cursor tile is drawn with reversed style. Zoomed in, the
chosen source's destinations fill the whole area as outer tiles (one level),
and the title reads `Treemap ▸ shop/prod`.

Keys: `h/j/k/l` or arrows move the cursor to the nearest tile in that
direction (by rect centre); `Enter` on an outer tile zooms in, on an inner
tile (or when zoomed) switches to the Flows view drilled to that src→dst
pair; `Esc` zooms out, or when not zoomed does nothing; `b` toggles weight
between connections and bytes (bytes greyed in the meta when no flow carried
bytes, and the toggle is refused with an info status). Meta reads
`by conns · b bytes · Enter zoom · Esc out`. Resize recomputes the layout
on the next draw with the same zoom and cursor clamped.

### 6.3 Not in scope

No animation, no colour legend beyond the decision colours already used in
the stats bar, no treemap in the CLI.

## 7. CLI

```
illumio-tui traffic groups [query flags] [--keys app,env]
illumio-tui traffic mesh   [query flags] [--keys app,env] [--no-user] [--no-process]
```

Both share `QueryArgs` minus `--group`/`--blocked` (those stay on `query`
for compatibility). `--keys` defaults to the profile's scope keys. Output is
the same envelope as `traffic query` (`data`, `meta` with cache, truncated,
window); `groups` emits `Vec<FlowGroup>` (now with `bytes`), `mesh` emits
`Vec<MeshRow>`. Tables: `TableRows` impls with headers
`src, dst, flows, conns, bytes, allowed, pot.blocked, blocked` and
`user, process, src, dst, service, dst process, flows, conns, bytes, decision`
(decision = dominant). Dry run, cost confirmation and caching behave exactly
as `traffic query`.

## 8. Error handling

- Unknown keys are allowed and show `-`; the picker only lists what it has
  seen plus the profile keys. No error.
- Treemap with no flows draws the empty-result text the Flows table uses.
- Bytes absent on every flow: `b` refuses with `no byte counts in this
  result`, `bytes` columns show `-` in tables and `0` in JSON.
- Mesh and treemap on a truncated result show the existing `⚠ truncated`
  marker in the stats panel; nothing else changes.

## 9. Testing

- `pce`: unit tests for `group_values`/`group_text` (missing key, non-workload,
  ordering), `keys_seen` (preferred-first ordering, counts), `blocked_summary`
  with keys (grouping by dst group, names capped), `mesh` (user/process
  toggles change the grouping; direction places the process; sort), `layout`
  (weights sum to the area within rounding, no overlaps, determinism,
  `other` folding, zero weight, 1-cell areas), bytes copied by
  `resolve_flows`.
- A richer fixture `fixtures/traffic_flows_rich.json` (about 30 flows, 4
  apps × 2 envs, roles, a `loc` key, users, processes, both directions, bytes
  on some) used by the new snapshots and update tests; the existing 4-flow
  fixture stays for the existing snapshots.
- TUI update tests: `v` cycles five views; `g` picker toggles, text edit,
  all-labels, Esc; keys apply to flows/blocked/mesh/treemap rows; `u`/`p`;
  mesh Enter drills and Esc clears; treemap cursor moves, zoom, Enter to
  flows, `b` refusal; every one asserts `update` returned no `Cmd`.
- Snapshots: picker 80x24; flows with group columns 80x24; blocked 80x24;
  mesh 80x24 and 160x48; treemap 80x24, 160x48 and zoomed 80x24; light
  theme 160x48 for mesh and treemap; help overlay. Screen-order change:
  every existing screen snapshot re-recorded (tab row only).
- CLI: `traffic groups` and `traffic mesh` on recorded responses, JSON
  shape and table headers, `--keys`, `--no-user`, dry run.
- Headless smoke on demo100 (read-only): run one traffic query, cycle
  views, open the picker, zoom the treemap; assert the request counter did
  not move after the query.
