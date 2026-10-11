# Traffic Views Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** IP lists become the fourth Objects tab (five screens again), and the Traffic screen groups every view by the label keys the user chose (the profile's scope keys by default, `g` opens a picker): Flows gets `src group` / `dst group` columns and a detail with both label sets and bytes, Blocked groups by destination group, two new views (Mesh: user, process, source, destination, service, destination process; Treemap: sources as boxes, destinations as filled tiles, sized by connections or bytes) join the `v` cycle, and the CLI gains `traffic groups` and `traffic mesh` for agents. Nothing sends a new request: every view is a projection of the flows the current query returned.

**Architecture:** Pure aggregation in the `pce` crate: `pce::traffic::aggregate` gains `group_values` / `group_text` / `group_key` / `keys_seen` / `compact_count` / `Dominant`, bytes on `ResolvedFlow` and `FlowGroup`, and a key-aware `blocked_summary`; `pce::traffic::mesh` (rows, columns, keys, process placement by direction) and `pce::traffic::treemap` (squarified layout in terminal cells, `other (N)` folding, a zoomed layout) are new. The TUI keeps Elm style: `TrafficState` holds the keys (`group_keys`, `all_keys`), `FlowRow` (a flow plus its group texts, because a `Column` cell is a plain `fn`), the Mesh table and its columns, a `Drill` enum (pair or mesh row) and a `TreemapState` whose tiles are computed when the data, keys, zoom, weight or terminal size change, never while drawing. The `g` picker is a pure `KeyPicker` in `tui/picker.rs`. On the Objects side, `ObjectKind::IpLists` routes to the Plan 7 IP list code, whose `IpListsState` moves into `ObjectsState.ip_lists`. The CLI splits the shared filter flags into `FilterArgs` and adds two subcommands over one `fetch`.

**Tech Stack:** Rust 2024 (rust-version 1.88), tokio, ratatui 0.30, crossterm, serde/serde_json, clap 4, jiff, insta (plain snapshot files), wiremock 0.6, assert_cmd. No new dependency.

**Spec:** `docs/superpowers/specs/2026-10-11-traffic-views-design.md` (binding).

**Tasks:** 9, in this order: 1 IP lists as the fourth Objects tab; 2 `pce::traffic` grouping primitives and the rich fixture; 3 `pce::traffic::mesh`; 4 `pce::traffic::treemap`; 5 TUI display keys and the `g` picker; 6 TUI Mesh view; 7 TUI Treemap view; 8 CLI `traffic groups` and `traffic mesh`; 9 docs, tracking, reads-only smoke, final gate. Task 1 is independent of the rest and goes first because it re-records every screen snapshot (the tab row): later tasks then re-record only what they change. Task 2 changes `blocked_summary`'s signature, so it updates its two callers (TUI model, CLI) in the same task and re-records the one snapshot that shows the new `sources` text. Every task boundary is green: this plan was replayed task by task on a scratch worktree at `34b4fa9`, and the gate passed after each task with these workspace test totals: 728 before, 729, 735, 739, 745, 758, 763, 772, 776.

## Global Constraints

- Rust edition 2024, rust-version 1.88. No new dependencies (ratatui 0.30, crossterm, serde, jiff, clap 4, insta, wiremock, assert_cmd as today). `git diff 34b4fa9 HEAD -- Cargo.toml crates/*/Cargo.toml Cargo.lock` stays empty.
- Never hammer the PCE: no view, key, zoom or column change may emit a request `Cmd`. Only `r`, `f`+Enter and auto-refresh query, as today. Every TUI test that changes keys, views, zoom, weight or columns asserts that `update(...)` returned an empty `Vec<Cmd>`.
- Elm-style TUI: pure `update`, `Runtime` executes `Cmd`s. Pure aggregation lives in the `pce` crate under `pce::traffic`.
- Label model: profile `[profiles.<p>.labels] scope_keys` (default `["app","env"]`), `tier_key` (default `role`), env overrides `ILLUMIO_SCOPE_KEYS` / `ILLUMIO_TIER_KEY`. Existing. Nothing here writes config.
- Commit trailer on every commit, exactly (after a blank line): `Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>`.
- Smokes against demo100 are read-only and never press `y`.
- Gate at the end of every task, in this order: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`. Run `cargo fmt --all` first and accept its formatting: the code blocks below are what was replayed, `rustfmt` may re-wrap what you paste.
- Edition 2024 clippy may ask for let-chains or iterator loops: accept the rewrite and list it as a deviation in the task report.
- Snapshots are plain insta files; `cargo insta` is not installed. Create and update them with `INSTA_UPDATE=always cargo test -p illumio-tui --test tui_snapshots`, then delete strays with `find crates/illumio-tui/tests/snapshots -name '*.snap.new' -delete`, read every new or changed `.snap`, and check `git status --short crates/illumio-tui/tests/snapshots` against the list the task gives.
- Tests: `pce` unit tests in the module's own `#[cfg(test)] mod tests`; TUI unit tests in `#[cfg(test)]` modules (`tui/update.rs`, `tui/model.rs`, `tui/picker.rs`); snapshots in `crates/illumio-tui/tests/tui_snapshots.rs`; runtime tests in `crates/illumio-tui/tests/tui_runtime.rs`; CLI tests in `crates/illumio-tui/tests/cli.rs` (assert_cmd + wiremock, `mount_traffic(server, expected_posts)`, `cmd`, `stdout_json`). Test names are sentences (`hiding_the_user_or_the_process_merges_rows`).
- The TUI's `ObjectKind` (Objects tabs, `tui/model.rs`) and `pce::write::ObjectKind` collide: the latter is always written fully qualified.
- Lessons that apply (`tasks/lessons.md`): compact cells need a width test against the real column (Tasks 5 and 6 test the 80-column budgets with `visible_columns`); computed-per-draw work is a hidden hot loop (the treemap is laid out in `update`, never in `view`); a late result must not swap state under the user (a new query result drops the drill and the zoom).
- Spec refinements this plan makes (reviewers: these are intended):
  - Two grouping functions, not one. `group_values` / `group_text` do exactly what §4.4 says (`-` in every slot of a non-workload) and feed the Flows `src group` / `dst group` columns. The aggregations (App-to-app, Blocked, Mesh, Treemap) group by `pub fn group_key`, which is `group_values` for a labelled workload and keeps today's fallback otherwise (the name or `ipl:<list>` in the first slot, `-` after it): with `group_values` alone every IP list, FQDN and unlabelled host would merge into one `-/-` row, and §4.3 says App-to-app is unchanged. `group_by_labels` keeps its signature and goes through `group_key`.
  - `w` on a Blocked row: §4.3 says "keeps working … as today", but today `w` refuses outside Flows. The plan implements it (Task 5) the way §5.2 asks for Mesh rows (Task 6): the first blocked or potentially blocked flow behind the row. App-to-app and Treemap still refuse with today's text.
  - `BlockedGroup` also gains `dst_hosts` and `source_count` (distinct, uncapped) next to the capped `dst_names` / `sources`, so the `N hosts` cell and the `+N` suffix are true beyond five. `dst` and `dst_labels` keep their meaning (the first destination) for existing JSON consumers; `BLOCKED_NAMES = 5`.
  - The rich fixture needs seven labels (`app=crm|erp|hr`, `env=dev`, `role=api`, `loc=us`, `datacenter=fra1`) that `fixtures/labels.json` lacks; adding them there would change the Objects screen and its tests, so they live in `fixtures/traffic_labels_rich.json` and `fixture_flows_rich()` resolves with both files. Both fixtures come from a generator script in Task 2 (not committed) with fixed checksums.
  - Flows at 80 columns: the group columns fit only when `last seen` hides below 140 columns and the names narrow to 14 cells (groups 10); from 120 columns src/dst are 19 and the groups 18. The detail pane grows from 6 to 8 rows for the `src labels` / `dst labels` lines; `bytes in … out …` goes on the `seen` line when present.
  - `Dominant` is `pub type Dominant = PolicyDecision` (the same four variants), so the theme's `decision_style` colours rows and tiles; `dominant(a, p, b)` is blocked > potentially blocked > allowed > unknown.
  - Treemap: `Tile.rect` is in the coordinates of the `area` passed in (absolute screen cells, so the renderer and the cursor use it as is); an outer tile lays its destinations out in `rect.inner()` (one cell of border), none when it is smaller than 3x3; `other (N)` comes last, has no children and the Unknown decision; when `other` itself gets no cell the smallest named tile joins it, until `other` alone fills the area. `layout_zoomed(groups, src, area, weight)` lays one source's destinations out for the zoom. `TreemapState` also stores `area` (from `ui::screens::traffic::treemap_area(size)`, set at startup, on `Msg::Resize` and in `reset_data`).
  - Treemap cursor: the targets are every outer tile followed by its destinations, in drawing order; a direction key moves to the nearest target whose centre lies ahead, scoring the distance ahead plus twice the distance across (a row counts as two columns), and stays put when nothing is ahead. Enter on an `other (N)` tile only explains itself in the status line.
  - Mesh: `u` / `p` rebuild the Mesh table (sort and search reset, cursor on the first row); `Drill::Mesh { key, cols }` keeps the columns it was made with. The decision column is 11 cells (`pot.blocked`) and 8 narrow (`pot.blk`) so its header fits; narrow widths user 8, process 10, src 14, dst 14, service 9, conns 6.
  - Picker: `j` / `k` / Space move and toggle on the key rows and are text on the `keys:` line; any other printable character or Backspace edits the text and moves the cursor to it (so `g` then `app,env,role` Enter still works); toggled keys are appended in the order chosen; counts show for keys outside the profile model and `loc`. Applying sets the status `grouped by <keys>`.
  - Objects: `:iplists` / `:ip` become `Command::IpLists`. `ObjectsState` keeps its three-slot arrays for `ObjectKind::FETCHED` (labels, label groups, services); `IpListsState` moves into `ObjectsState.ip_lists`; a `Cmd::LoadObjects { kind: IpLists }` is forwarded to `Cmd::LoadIpLists` (never emitted, but total). The IP lists load on the first visit to their tab (Tab / Shift-Tab onto it, `:iplists`, or `3` while it is the current tab): a cache hit after startup.
  - Status hints at 80 columns: the Traffic hint trades `s sort` for `g group` (writes off) and `/ search` for `g group` (writes on). The Blocked panel meta reads `by dst <keys>`; the Treemap meta reads `by conns · b bytes · Enter zoom · Esc out` (or `by bytes · b conns · …`).
  - CLI: the filter flags move into `FilterArgs`, flattened into `QueryArgs`, `GroupsArgs` and `MeshArgs`; `--keys` defaults to the profile's scope keys; `traffic query --blocked` groups by them too; a `bytes` table cell is `-` in every row when no row carried bytes (JSON keeps `0`).
  - Help: 32 lines today; the `Objects  Tab / Enter` line names the IP lists tab (Task 1, still 32), Task 6 adds `Mesh     u / p` (33), Task 7 adds `Treemap  hjkl Enter b` (34).

## Review Focus

1. The IP lists tab driven by every key, the wheel and the auto-refresh tick must never index the three-slot `ObjectsState` arrays with `ObjectKind::IpLists` (index 3, a panic) or touch the other kinds' state. Test: Task 1 `every_key_and_the_wheel_on_the_ip_lists_tab_stay_on_its_own_state`.
2. Endpoints that are not labelled workloads (an IP list, an FQDN, an unlabelled workload) under any keys: Flows shows `-` in every slot, the aggregations keep them apart and readable (`ipl:Corp/-`, `fqdn-1.example/-`, `unmanaged-nas/-`). Tests: Task 2 `group_values_dash_a_missing_key_and_every_slot_of_a_non_workload`, Task 5 `g_opens_the_picker_and_enter_regroups_every_view_without_requests` (the `-/-/-` row).
3. Terminal sizes the treemap meets: a 1-cell area, a 1-row area, zero width or height, zero weights, sources without bytes; tiles must fill the area exactly with no overlap and never panic, and a resize must re-lay the tiles. Tests: Task 4 `small_tiles_fold_into_other_and_a_single_cell_still_draws`, `zero_weight_and_empty_areas_give_no_tiles`; Task 7 `the_treemap_follows_the_terminal_size_and_v_comes_back_to_flows`.
4. Stale drill-downs and zooms: new keys drop the drill and the zoom, a zoomed source that no longer exists is dropped, and a new query result clears both. Tests: Task 5 `esc_keeps_the_keys_and_the_drill_and_applying_clears_the_drill`; Task 7 `new_keys_and_a_new_result_drop_the_zoom`.
5. A blocked row with more than five destinations or source groups: names stop at five while `dst_hosts` / `source_count` keep counting, the cells read `8 hosts` and `… +5`, and `w` still finds a flow behind the row. Tests: Task 2 `blocked_summary_groups_by_the_destination_group_and_caps_the_names`; Task 5 `blocked_cells_name_one_host_or_count_them_and_count_more_sources`.

---

## File structure

| File | Responsibility |
| --- | --- |
| `fixtures/traffic_flows_rich.json`, `fixtures/traffic_labels_rich.json` (new) | 30 raw flows and the 7 extra labels they use (Task 2 generator) |
| `crates/pce/src/traffic/resolved.rs` | `ResolvedFlow.{bytes_in, bytes_out}`, `bytes()`, `has_bytes()`; test helpers `rich_index()`, `rich_flows()` |
| `crates/pce/src/traffic/aggregate.rs` | `Dominant`, `dominant`, `keys_or_default`, `compact_count`, `group_values`, `group_text`, `group_key`, `keys_seen`; `FlowGroup.bytes`, `dominant()`, `contains()`; `BlockedGroup` new fields, `contains()`; `blocked_summary(flows, keys)` |
| `crates/pce/src/traffic/mesh.rs` (new) | `MeshRow`, `MeshColumns`, `MeshKey`, `process_sides`, `mesh_key`, `mesh` |
| `crates/pce/src/traffic/treemap.rs` (new) | `Rect`, `Weight`, `Tile`, `layout`, `layout_zoomed`; squarify, cell rounding, `other (N)` folding |
| `crates/pce/src/traffic/mod.rs` | modules and re-exports |
| `crates/illumio-tui/src/tui/model.rs` | Task 1: `Screen` (5), `ObjectKind::IpLists` / `FETCHED`, `ObjectsState.ip_lists`; Tasks 5-7: `FlowRow`, `flow_columns`, blocked cells, `Drill`, `PREFERRED_KEYS`, `TrafficState.{all_keys, mesh, mesh_cols, treemap}`, `effective_keys`, `has_bytes`, `first_flow`, `rebuild_views`, `toggle_mesh_column`, `relayout_treemap`, `mesh_columns`, `MESH_ALL`, `TreemapState`, `TreemapTarget`, `Dir`, `Model::set_label_model` |
| `crates/illumio-tui/src/tui/picker.rs` (new) | `KeyPicker`, `PickerEntry`, `PickerOutcome`, `parse_keys` |
| `crates/illumio-tui/src/tui/update.rs` | Task 1 routing of the IP lists tab; Tasks 5-7 traffic keys (`g`, `u`, `p`, treemap keys, `b`), `apply_group_keys`, `open_key_picker`, `treemap_enter`, `treemap_out`, `w` on rows, `Msg::Resize` |
| `crates/illumio-tui/src/tui/commands.rs` | `Command::IpLists` |
| `crates/illumio-tui/src/tui/app.rs` | `LoadObjects{IpLists}` forwarding; `set_label_model` at startup |
| `crates/illumio-tui/src/tui/ui/mod.rs` | tab row tests; `render_body` without the sixth screen; picker overlay dispatch |
| `crates/illumio-tui/src/tui/ui/screens/objects.rs`, `iplists.rs` | strip plus the IP lists tab (`strip_split`, `render_tab`) |
| `crates/illumio-tui/src/tui/ui/screens/traffic.rs` | detail 8 rows, group detail lines, `render_mesh`, `render_treemap`, `treemap_area` |
| `crates/illumio-tui/src/tui/ui/widgets/overlay.rs` | `HELP_LINES`, `screen_hint`, `render_key_picker` (replaces `render_group_prompt`) |
| `crates/illumio-tui/src/tui/test_support.rs` | `on_ip_lists_tab`, `fixture_flows_rich`, `model_with_rich_flows` |
| `crates/illumio-tui/src/cli/traffic.rs` | `FilterArgs`, `GroupsArgs`, `MeshArgs`, `fetch`, `groups`, `mesh_cmd`, `TableRows for Vec<MeshRow>`, bytes column |
| `crates/illumio-tui/tests/{tui_snapshots,tui_runtime,cli}.rs` | tests |
| `README.md`, `docs/architecture.md`, `docs/index.html`, `tasks/todo.md` | docs and tracking |

---

### Task 1: IP lists as the fourth Objects tab (issue #5)

**Files:**
- Modify: `crates/illumio-tui/src/tui/model.rs` (`Screen` ~lines 19-66; `ObjectKind` ~1290-1323; `ObjectsState` ~1466-1527; `Model` ~2100-2271; test `ip_list_rows_carry_their_size_and_how_many_loaded_rules_use_them` ~2370)
- Modify: `crates/illumio-tui/src/tui/commands.rs` (`Command` line 5, `parse` line 47, test line 84)
- Modify: `crates/illumio-tui/src/tui/app.rs` (`Cmd::LoadObjects` arm ~line 335)
- Modify: `crates/illumio-tui/src/tui/update.rs` (`Msg::Tick` ~50-82, `Msg::IndexLoaded` ~152, `on_screen_key` ~1135-1182, `scroll_focused` ~1375-1396, `switch` ~1413, `load_missing_objects` / `load_objects` ~1546-1563, `ip_lists_page` ~1605, `load_missing_ip_lists` doc ~1613, `objects_key` ~2014-2065, `objects_edit_key` ~2116, `on_command` ~2299; tests)
- Modify: `crates/illumio-tui/src/tui/ui/mod.rs` (`render_body` ~255-258; tests ~238-276)
- Modify: `crates/illumio-tui/src/tui/ui/screens/objects.rs` (module doc, `layout` ~line 18, `render_screen` ~53, `render_table` ~80-138, `render_detail` ~145)
- Modify: `crates/illumio-tui/src/tui/ui/screens/iplists.rs` (module doc, `layout` ~line 21, `render_screen` ~37 becomes `render_tab`)
- Modify: `crates/illumio-tui/src/tui/ui/widgets/overlay.rs` (`HELP_LINES` ~17-80, `screen_hint` ~683)
- Modify: `crates/illumio-tui/src/tui/test_support.rs` (imports line 3, `model_with_ip_lists` ~125, new `on_ip_lists_tab`)
- Test: `crates/illumio-tui/tests/tui_runtime.rs` (`writes_disabled_editing_keys_send_nothing` ~1033, `update_ip_list_puts_only_the_changed_ranges` ~1106, `opening_ip_lists_costs_nothing_after_startup_and_r_bypasses_once` ~2095, `create_ip_list_on_the_ip_lists_screen_posts_once_and_reloads_once` ~2110), `crates/illumio-tui/tests/tui_snapshots.rs`
- Delete: `crates/illumio-tui/tests/snapshots/tui_snapshots__{ip_lists_80x24,ip_lists_160x48,ip_lists_light_styled_80x24,ip_list_detail_many_ranges_80x24,ip_list_detail_many_ranges_light_styled_80x24}.snap`
- Docs: `README.md` (screens paragraph ~80-86, Objects and IP lists bullets ~103-108, startup paragraph ~124-127, editing paragraph ~299-301), `docs/architecture.md` (~261-265), `docs/index.html` (~199-201)

**Interfaces:**
- Consumes: Plan 7's `IpListsState` / `IpListRow` / `ip_list_columns` (`model.rs` ~1530-1660), `Cmd::LoadIpLists(CacheMode)`, `Msg::IpListsLoaded`, `ip_lists_key`, `ip_lists_edit_key`, `load_ip_lists`, `load_missing_ip_lists` (`update.rs` ~1605-1695), `iplists::{layout, table_rows, render_detail, detail_lines}`; test helpers `model_fixture`, `model_with_ip_lists`, `key`, `kc`, `enter`, `mouse`, `writes_on`, `key_session`, `mount_lists`, `ch`.
- Produces:

```rust
// tui/model.rs
pub enum Screen { Traffic, Policy, Objects, Workloads, Events }          // ALL: [Screen; 5]
pub enum ObjectKind { Labels, LabelGroups, Services, IpLists }            // ALL: [ObjectKind; 4]
impl ObjectKind { pub const FETCHED: [ObjectKind; 3]; }                   // Labels, LabelGroups, Services
pub struct ObjectsState { /* … */ pub ip_lists: IpListsState, pub ages: [Option<u64>; 3], .. }
pub struct Model { /* `ip_lists` field removed: use `objects.ip_lists` */ }
// tui/commands.rs
pub enum Command { Screen(Screen), IpLists, /* … */ }
// tui/ui/screens/objects.rs
pub fn strip_split(body: Rect) -> (Rect, Rect);
// tui/ui/screens/iplists.rs
pub fn render_tab(model: &Model, f: &mut Frame, body: Rect); // replaces render_screen; body is below the strip
// tui/test_support.rs
pub fn on_ip_lists_tab(m: &mut Model);
```

- [ ] **Step 1: Write the failing tests**

In `crates/illumio-tui/src/tui/model.rs`:

replace

```rust
        assert_eq!(Screen::ALL.len(), 6);
        assert_eq!(Screen::IpLists.title(), "IP lists");
        assert_eq!(Screen::IpLists.short_title(), "IPs");
        assert_eq!(Screen::Objects.title(), "Objects");
        assert_eq!(ObjectKind::ALL.len(), 3);
        assert_eq!(ObjectKind::Labels.prev(), ObjectKind::Services);
        assert_eq!(ObjectKind::Services.next(), ObjectKind::Labels);
```

with

```rust
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
```

In `crates/illumio-tui/src/tui/commands.rs`:

replace

```rust
        assert_eq!(parse("iplists").unwrap(), Command::Screen(Screen::IpLists));
        assert_eq!(parse("ip").unwrap(), Command::Screen(Screen::IpLists));
```

with

```rust
        assert_eq!(parse("iplists").unwrap(), Command::IpLists);
        assert_eq!(parse("ip").unwrap(), Command::IpLists);
```

In `crates/illumio-tui/src/tui/ui/mod.rs`:

replace

```rust
    #[test]
    fn the_tab_row_shortens_only_when_the_full_row_does_not_fit() {
        let full = tab_labels(160);
        assert_eq!(row_width(&full), 84, "six full titles and five dividers");
        assert_eq!(full[2], " 3 Objects ");
        assert_eq!(full[5], " 6 IP lists ");
        assert_eq!(tab_labels(84)[5], " 6 IP lists ");
        assert_eq!(tab_labels(83)[5], " 6 IPs ");
        assert_eq!(tab_labels(83)[0], " 1 Traf ");
        assert_eq!(row_width(&tab_labels(80)), 58);
    }
```

with

```rust
    #[test]
    fn the_tab_row_shortens_only_when_the_full_row_does_not_fit() {
        let full = tab_labels(160);
        assert_eq!(full.len(), 5);
        assert_eq!(row_width(&full), 69, "five full titles and four dividers");
        assert_eq!(full[2], " 3 Objects ");
        assert_eq!(full[4], " 5 Events ");
        assert_eq!(tab_labels(80), full, "the full row fits at 80 columns");
        assert_eq!(tab_labels(69)[4], " 5 Events ");
        assert_eq!(tab_labels(68)[4], " 5 Ev ");
        assert_eq!(tab_labels(68)[0], " 1 Traf ");
        assert_eq!(row_width(&tab_labels(60)), 48);
    }
```

replace

```rust
        // The rendered row puts each label exactly where tab_at expects it,
        // with the short titles at 80 columns and the full ones at 160.
        let m = crate::tui::test_support::model_fixture();
        for (w, h) in [(80u16, 24u16), (160, 48)] {
```

with

```rust
        // The rendered row puts each label exactly where tab_at expects it,
        // with the short titles at 64 columns and the full ones from 80.
        let m = crate::tui::test_support::model_fixture();
        for (w, h) in [(64u16, 24u16), (80, 24), (160, 48)] {
```

In `crates/illumio-tui/src/tui/update.rs`:

replace

```rust
        assert_eq!(m.screen, Screen::Workloads);
        // entering IP lists for the first time loads them once (a cache hit)
        assert_eq!(
            update(&mut m, key('6')),
            vec![Cmd::LoadIpLists(pce::CacheMode::Use)]
        );
        assert_eq!(m.screen, Screen::IpLists);
        assert!(update(&mut m, key('1')).is_empty());
```

with

```rust
        assert_eq!(m.screen, Screen::Workloads);
        // there is no sixth screen: 6 does nothing
        assert!(update(&mut m, key('6')).is_empty());
        assert_eq!(m.screen, Screen::Workloads);
        assert!(update(&mut m, key('1')).is_empty());
```

replace

```rust
    #[test]
    fn six_opens_ip_lists_once_and_r_bypasses_the_cache_once() {
        let mut m = model_fixture();
        assert_eq!(
            update(&mut m, key('6')),
            vec![Cmd::LoadIpLists(pce::CacheMode::Use)]
        );
        assert_eq!(m.screen, Screen::IpLists);
        assert!(update(&mut m, key('6')).is_empty(), "already loading");
```

with

```rust
    #[test]
    fn the_ip_lists_tab_loads_once_and_r_bypasses_the_cache_once() {
        use crate::tui::model::ObjectKind;
        let mut m = model_fixture();
        assert_eq!(update(&mut m, key('3')).len(), 3, "the three other kinds");
        assert_eq!(
            update(&mut m, kc(KeyCode::BackTab)),
            vec![Cmd::LoadIpLists(pce::CacheMode::Use)],
            "Labels back to IP lists: the first visit loads them"
        );
        assert_eq!(m.objects.kind, ObjectKind::IpLists);
        assert_eq!(m.screen, Screen::Objects);
        update(&mut m, kc(KeyCode::Tab));
        assert!(
            update(&mut m, kc(KeyCode::BackTab)).is_empty(),
            "already loading"
        );
```

replace

```rust
        update(&mut m, key('1'));
        assert!(
            update(&mut m, key('6')).is_empty(),
            "coming back costs nothing"
        );
```

with

```rust
        update(&mut m, key('1'));
        assert!(
            update(&mut m, key('3')).is_empty(),
            "coming back to the tab costs nothing"
        );
        assert_eq!(m.objects.kind, ObjectKind::IpLists, "the tab is kept");
```

replace

```rust
    #[test]
    fn ip_lists_wait_for_the_name_index_and_the_command_opens_them() {
        let mut m = model_fixture();
        let Connection::Connected { index, .. } = &mut m.connection else {
            unreachable!()
        };
        let idx = index.take();
        assert!(
            update(&mut m, key('6')).is_empty(),
            "the startup load reads the same list"
        );
        let cmds = update(
            &mut m,
            Msg::IndexLoaded {
                index: idx.unwrap(),
                age_secs: 0,
            },
        );
        assert_eq!(cmds, vec![Cmd::LoadIpLists(pce::CacheMode::Use)]);
        let mut m = model_fixture();
        assert_eq!(
            on_command(&mut m, Command::Screen(Screen::IpLists)),
            vec![Cmd::LoadIpLists(pce::CacheMode::Use)]
        );
    }
```

with

```rust
    #[test]
    fn ip_lists_wait_for_the_name_index_and_the_command_opens_their_tab() {
        use crate::tui::model::ObjectKind;
        let mut m = model_fixture();
        let Connection::Connected { index, .. } = &mut m.connection else {
            unreachable!()
        };
        let idx = index.take();
        assert!(
            on_command(&mut m, Command::IpLists).is_empty(),
            "the startup load reads the same list"
        );
        assert_eq!(
            (m.screen, m.objects.kind),
            (Screen::Objects, ObjectKind::IpLists)
        );
        let cmds = update(
            &mut m,
            Msg::IndexLoaded {
                index: idx.unwrap(),
                age_secs: 0,
            },
        );
        assert_eq!(cmds.len(), 4, "the three other kinds and the IP lists");
        assert!(cmds.contains(&Cmd::LoadIpLists(pce::CacheMode::Use)));
        let mut m = model_fixture();
        let cmds = on_command(&mut m, Command::IpLists);
        assert_eq!(cmds.len(), 4);
        assert_eq!(cmds[3], Cmd::LoadIpLists(pce::CacheMode::Use));
        assert_eq!(m.objects.kind, ObjectKind::IpLists);
    }
```

replace

```rust
        assert_eq!(m.screen, Screen::IpLists, "6 is typed into the search");
```

with

```rust
        assert_eq!(m.screen, Screen::Objects, "6 is typed into the search");
```

replace

```rust
    #[test]
    fn ip_lists_auto_refresh_is_opt_in_and_bypasses_when_due() {
```

with

```rust
    #[test]
    fn every_key_and_the_wheel_on_the_ip_lists_tab_stay_on_its_own_state() {
        use crate::tui::model::ObjectKind;
        use crate::tui::test_support::{model_with_ip_lists, on_ip_lists_tab};
        use crossterm::event::MouseEventKind;
        let mut m = model_with_ip_lists();
        on_ip_lists_tab(&mut m);
        update(&mut m, Msg::Resize(80, 24));
        let keys = [
            key('j'),
            key('k'),
            key('s'),
            key('s'),
            enter(),
            enter(),
            key('a'),
            key('a'),
            kc(KeyCode::End),
            kc(KeyCode::Home),
            mouse(MouseEventKind::ScrollDown, 10, 10),
            mouse(MouseEventKind::ScrollUp, 10, 10),
            key('/'),
            key('c'),
            kc(KeyCode::Esc),
        ];
        for msg in keys {
            assert!(update(&mut m, msg).is_empty());
        }
        assert_eq!(m.objects.kind, ObjectKind::IpLists);
        assert_eq!(m.objects.ages, [None; 3], "the other kinds' slots are untouched");
        assert_eq!(m.objects.loading, [false; 3]);
        assert_eq!(
            update(&mut m, key('r')),
            vec![Cmd::LoadIpLists(pce::CacheMode::Bypass)]
        );
        assert_eq!(m.objects.loading, [false; 3], "r reloads only the IP lists");
        let later = std::time::Instant::now() + std::time::Duration::from_secs(3600);
        assert!(
            update(&mut m, Msg::Tick(later)).is_empty(),
            "a reload in flight and auto-refresh off: the tick does nothing"
        );
    }

    #[test]
    fn ip_lists_auto_refresh_is_opt_in_and_bypasses_when_due() {
```

In `crates/illumio-tui/tests/tui_runtime.rs`:

replace

```rust
        (50, ch('e')),
        (50, ch('6')),
        (300, ch('e')),
        (50, ch('x')),
        (200, KeyCode::F(12)),
    ];
    let model = key_session(&server, model(), None, keys).await;
    assert_eq!(model.screen, illumio_tui::tui::model::Screen::IpLists);
    assert_eq!(model.stats.calls, 5, "health + four lists");
    assert_eq!(
        server.received_requests().await.unwrap().len() as u64,
        model.stats.calls,
        "6 reads the cache; the refused e and x send nothing"
    );
```

with

```rust
        (50, ch('e')),
        (50, KeyCode::Tab),
        (300, ch('e')),
        (50, ch('x')),
        (200, KeyCode::F(12)),
    ];
    let model = key_session(&server, model(), None, keys).await;
    assert_eq!(model.screen, illumio_tui::tui::model::Screen::Objects);
    assert_eq!(
        model.objects.kind,
        illumio_tui::tui::model::ObjectKind::IpLists
    );
    assert_eq!(model.stats.calls, 5, "health + four lists");
    assert_eq!(
        server.received_requests().await.unwrap().len() as u64,
        model.stats.calls,
        "the IP lists tab reads the cache; the refused e and x send nothing"
    );
```

replace

```rust
    let mut keys = vec![
        (500, ch('6')),
        (200, ch('j')),
```

with

```rust
    let mut keys = vec![
        (500, ch('3')),
        (100, KeyCode::BackTab),
        (200, ch('j')),
```

replace

```rust
#[tokio::test]
async fn opening_ip_lists_costs_nothing_after_startup_and_r_bypasses_once() {
    let server = MockServer::start().await;
    mount_lists(&server, 1, 2, 1).await; // IP lists: the startup load and the r
    let model = objects_session(
        &server,
        &[(500, '6'), (300, '1'), (100, '6'), (300, 'r'), (400, 'q')],
    )
    .await;
```

with

```rust
#[tokio::test]
async fn opening_the_ip_lists_tab_costs_nothing_after_startup_and_r_bypasses_once() {
    let server = MockServer::start().await;
    mount_lists(&server, 1, 2, 1).await; // IP lists: the startup load and the r
    let keys = vec![
        (500, ch('3')),
        (100, KeyCode::BackTab),
        (300, ch('1')),
        (100, ch('3')),
        (300, ch('r')),
        (400, KeyCode::F(12)),
    ];
    let model = key_session(&server, model(), None, keys).await;
```

replace

```rust
async fn create_ip_list_on_the_ip_lists_screen_posts_once_and_reloads_once() {
```

with

```rust
async fn create_ip_list_on_the_objects_ip_lists_tab_posts_once_and_reloads_once() {
```

replace

```rust
    let mut keys = vec![(500, ch('6')), (200, ch('n'))];
```

with

```rust
    let mut keys = vec![(500, ch('3')), (100, KeyCode::BackTab), (200, ch('n'))];
```

In `crates/illumio-tui/tests/tui_snapshots.rs`:

replace

```rust
#[test]
fn ip_lists_screen_80x24_and_160x48_with_detail() {
    use illumio_tui::tui::test_support::model_with_ip_lists;
    let mut m = model_with_ip_lists();
    m.screen = Screen::IpLists;
    m.ip_lists.table.select_next(); // Corp
    m.ip_lists.show_detail = true;
    let out = render_to_string(&m, 80, 24);
    assert!(out.contains("6 IPs"), "{out}");
    assert!(out.contains("65.5K"), "{out}");
    assert!(out.contains("used by Shop-Prod · allow"), "{out}");
    insta::assert_snapshot!("ip_lists_80x24", out);
    let out = render_to_string(&m, 160, 48);
    assert!(out.contains("6 IP lists"), "{out}");
    assert!(out.contains("65536 IPv4"), "{out}");
    insta::assert_snapshot!("ip_lists_160x48", out);
    m.set_theme("light");
    insta::assert_snapshot!("ip_lists_light_styled_80x24", render_styled(&m, 80, 24));
}
```

with

```rust
#[test]
fn objects_ip_lists_tab_80x24_and_160x48_with_detail() {
    use illumio_tui::tui::test_support::model_with_ip_lists;
    let mut m = model_with_ip_lists();
    illumio_tui::tui::test_support::on_ip_lists_tab(&mut m);
    m.objects.ip_lists.table.select_next(); // Corp
    m.objects.ip_lists.show_detail = true;
    let out = render_to_string(&m, 80, 24);
    let strip = out.lines().nth(2).unwrap();
    assert!(
        strip.contains("Services") && strip.contains("IP lists"),
        "the fourth Objects tab: {strip}"
    );
    assert!(out.contains("3 Objects") && !out.contains(" 6 "), "{out}");
    assert!(out.contains("65.5K"), "{out}");
    assert!(out.contains("used by Shop-Prod · allow"), "{out}");
    insta::assert_snapshot!("objects_ip_lists_80x24", out);
    let out = render_to_string(&m, 160, 48);
    assert!(out.contains("65536 IPv4"), "{out}");
    insta::assert_snapshot!("objects_ip_lists_160x48", out);
    m.set_theme("light");
    insta::assert_snapshot!(
        "objects_ip_lists_light_styled_80x24",
        render_styled(&m, 80, 24)
    );
}
```

replace

```rust
    insta::assert_snapshot!("ip_list_detail_many_ranges_80x24", out);
    m.set_theme("light");
    insta::assert_snapshot!(
        "ip_list_detail_many_ranges_light_styled_80x24",
```

with

```rust
    insta::assert_snapshot!("objects_ip_list_detail_many_ranges_80x24", out);
    m.set_theme("light");
    insta::assert_snapshot!(
        "objects_ip_list_detail_many_ranges_light_styled_80x24",
```

replace

```rust
#[test]
fn the_tab_row_uses_short_titles_only_when_the_full_row_does_not_fit() {
    let m = model_fixture();
    let narrow = render_to_string(&m, 80, 24);
    let row = narrow.lines().nth(1).unwrap();
    assert!(
        row.contains("1 Traf") && row.contains("3 Obj") && row.contains("6 IPs"),
        "{row}"
    );
    let wide = render_to_string(&m, 160, 48);
    let row = wide.lines().nth(1).unwrap();
    assert!(
        row.contains("3 Objects") && row.contains("6 IP lists"),
        "{row}"
    );
}
```

with

```rust
#[test]
fn the_tab_row_uses_short_titles_only_when_the_full_row_does_not_fit() {
    let m = model_fixture();
    let narrow = render_to_string(&m, 64, 24);
    let row = narrow.lines().nth(1).unwrap();
    assert!(
        row.contains("1 Traf") && row.contains("3 Obj") && row.contains("5 Ev"),
        "{row}"
    );
    for w in [80, 160] {
        let out = render_to_string(&m, w, 24);
        let row = out.lines().nth(1).unwrap();
        assert!(
            row.contains("1 Traffic") && row.contains("5 Events") && !row.contains('6'),
            "{row}"
        );
    }
}
```

replace

```rust
    use illumio_tui::tui::form::{Field, Form};
    use illumio_tui::tui::model::{Overlay, Screen};
    use illumio_tui::tui::test_support::model_with_ip_lists;
```

with

```rust
    use illumio_tui::tui::form::{Field, Form};
    use illumio_tui::tui::model::Overlay;
    use illumio_tui::tui::test_support::model_with_ip_lists;
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p illumio-tui --lib 2>&1 | tail -5`
Expected: FAIL to compile (`Command::IpLists`, `ObjectKind::IpLists`, `ObjectKind::FETCHED`, `on_ip_lists_tab` and `m.objects.ip_lists` do not exist yet).

- [ ] **Step 3: Implement**

In `crates/illumio-tui/src/tui/model.rs`:

replace

```rust
    Events,
    IpLists,
}

impl Screen {
    /// In tab order; a screen's key is its position plus one.
    pub const ALL: [Screen; 6] = [
        Screen::Traffic,
        Screen::Policy,
        Screen::Objects,
        Screen::Workloads,
        Screen::Events,
        Screen::IpLists,
    ];
```

with

```rust
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
```

replace

```rust
            Screen::Events => "Events",
            Screen::IpLists => "IP lists",
```

with

```rust
            Screen::Events => "Events",
```

replace

```rust
            Screen::Events => "Ev",
            Screen::IpLists => "IPs",
```

with

```rust
            Screen::Events => "Ev",
```

replace

```rust
pub enum ObjectKind {
    Labels,
    LabelGroups,
    Services,
}

impl ObjectKind {
    pub const ALL: [ObjectKind; 3] = [
        ObjectKind::Labels,
        ObjectKind::LabelGroups,
        ObjectKind::Services,
    ];

    pub fn index(self) -> usize {
```

with

```rust
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
```

replace

```rust
            ObjectKind::Services => "Services",
        }
    }
}
```

with

```rust
            ObjectKind::Services => "Services",
            ObjectKind::IpLists => "IP lists",
        }
    }
}
```

replace

```rust
    pub services: TableView<Service>,
    /// Cache age in seconds when each kind (indexed by `ObjectKind::index`) was loaded.
```

with

```rust
    pub services: TableView<Service>,
    /// The IP lists tab.
    pub ip_lists: IpListsState,
    /// Cache age in seconds when each kind (indexed by `ObjectKind::index`,
    /// `FETCHED` kinds only) was loaded.
```

replace

```rust
            services: TableView::new(service_columns()),
            ages: [None; 3],
```

with

```rust
            services: TableView::new(service_columns()),
            ip_lists: IpListsState::new(min_refresh),
            ages: [None; 3],
```

replace

```rust
    pub fn loaded(&self) -> bool {
```

with

```rust
    /// Whether the three `FETCHED` kinds have arrived.
    pub fn loaded(&self) -> bool {
```

replace

```rust
            ObjectKind::Services => self.services.is_searching(),
        }
```

with

```rust
            ObjectKind::Services => self.services.is_searching(),
            ObjectKind::IpLists => self.ip_lists.table.is_searching(),
        }
```

replace

```rust
            ObjectKind::Services => self.services.selected_item().is_some(),
        }
```

with

```rust
            ObjectKind::Services => self.services.selected_item().is_some(),
            ObjectKind::IpLists => self.ip_lists.table.selected_item().is_some(),
        }
```

replace

```rust
    pub fn age_secs(&self, kind: ObjectKind) -> Option<u64> {
        let i = kind.index();
```

with

```rust
    pub fn age_secs(&self, kind: ObjectKind) -> Option<u64> {
        if kind == ObjectKind::IpLists {
            let s = &self.ip_lists;
            return s.loaded_at.map(|t| s.age_secs + t.elapsed().as_secs());
        }
        let i = kind.index();
```

replace

```rust
    pub objects: ObjectsState,
    pub ip_lists: IpListsState,
```

with

```rust
    pub objects: ObjectsState,
```

delete

```rust
        let ip_lists = IpListsState::new(auto_refresh_min_seconds);
```

replace

```rust
            objects,
            ip_lists,
```

with

```rust
            objects,
```

replace

```rust
            || self.ip_lists.auto.enabled
```

with

```rust
            || self.objects.ip_lists.auto.enabled
```

delete

```rust
        self.ip_lists = IpListsState::new(min);
```

In `crates/illumio-tui/src/tui/commands.rs`:

replace

```rust
    Screen(Screen),
    Filter(String),
```

with

```rust
    Screen(Screen),
    /// `:iplists` / `:ip`: the Objects screen on its IP lists tab.
    IpLists,
    Filter(String),
```

replace

```rust
"iplists" | "ip" => no_arg(Command::Screen(Screen::IpLists)),
```

with

```rust
"iplists" | "ip" => no_arg(Command::IpLists),
```

In `crates/illumio-tui/src/tui/app.rs`:

replace

```rust
            Cmd::LoadObjects { kind, mode } => {
                self.model.busy += 1;
```

with

```rust
            // The IP lists tab loads through its own command.
            Cmd::LoadObjects {
                kind: ObjectKind::IpLists,
                mode,
            } => self.execute(Cmd::LoadIpLists(mode)),
            Cmd::LoadObjects { kind, mode } => {
                self.model.busy += 1;
```

replace

```rust
                            .map(|c| ObjectsPayload::Services(c.value, c.age.as_secs())),
                    }
```

with

```rust
                            .map(|c| ObjectsPayload::Services(c.value, c.age.as_secs())),
                        ObjectKind::IpLists => unreachable!("forwarded to Cmd::LoadIpLists above"),
                    }
```

In `crates/illumio-tui/src/tui/ui/screens/objects.rs`:

replace

```rust
//! Objects screen: a tab strip of the three object kinds (labels, label groups,
//! services), the current kind's table, and an optional detail pane for the
//! selected object.
```

with

```rust
//! Objects screen: a tab strip of the four object kinds (labels, label
//! groups, services, IP lists), the current kind's table, and an optional
//! detail pane for the selected object. The IP lists tab is drawn by
//! `screens::iplists` below the strip.
```

replace

```rust
/// `(tab strip, table, detail)` areas. Shared by `view` and `update` (page sizes).
/// The detail pane is dropped when the terminal is too short to keep a usable table.
pub fn layout(body: Rect, show_detail: bool) -> (Rect, Rect, Rect) {
```

with

```rust
/// `(tab strip, everything below it)`: the IP lists tab lays out the rest
/// itself (`iplists::layout`).
pub fn strip_split(body: Rect) -> (Rect, Rect) {
    let rows = Layout::vertical([Constraint::Length(1), Constraint::Min(0)]).split(body);
    (rows[0], rows[1])
}

/// `(tab strip, table, detail)` areas. Shared by `view` and `update` (page sizes).
/// The detail pane is dropped when the terminal is too short to keep a usable table.
pub fn layout(body: Rect, show_detail: bool) -> (Rect, Rect, Rect) {
```

replace

```rust
    let (strip, table, detail) = layout(body, model.objects.show_detail);
    render_strip(model, f, strip);
    render_table(model, f, table);
```

with

```rust
    if model.objects.kind == ObjectKind::IpLists {
        let (strip, rest) = strip_split(body);
        render_strip(model, f, strip);
        super::iplists::render_tab(model, f, rest);
        return;
    }
    let (strip, table, detail) = layout(body, model.objects.show_detail);
    render_strip(model, f, strip);
    render_table(model, f, table);
```

replace

```rust
        ObjectKind::Services => (
            o.services.len(),
            o.services.rows().len(),
            header_meta(&o.services),
        ),
    };
```

with

```rust
        ObjectKind::Services => (
            o.services.len(),
            o.services.rows().len(),
            header_meta(&o.services),
        ),
        // Drawn by `iplists::render_tab`; `render_screen` never gets here.
        ObjectKind::IpLists => return,
    };
```

replace

```rust
        ObjectKind::Services => render(f, area, &o.services, t, block, &|_: &Service| {
            Style::default()
        }),
    }
```

with

```rust
        ObjectKind::Services => render(f, area, &o.services, t, block, &|_: &Service| {
            Style::default()
        }),
        ObjectKind::IpLists => {}
    }
```

replace

```rust
        ObjectKind::Services => o.services.selected_item().map(|s| service_lines(s)),
    };
```

with

```rust
        ObjectKind::Services => o.services.selected_item().map(|s| service_lines(s)),
        ObjectKind::IpLists => None,
    };
```

In `crates/illumio-tui/src/tui/ui/screens/iplists.rs`:

replace

```rust
//! IP lists screen: every draft IP list with its size, entry counts and how
//! many loaded rules use it, and a detail pane for the selected one (size,
//! the rules by name, the first ranges and FQDNs). No request beyond the list itself.
```

with

```rust
//! The Objects screen's IP lists tab: every draft IP list with its size,
//! entry counts and how many loaded rules use it, and a detail pane for the
//! selected one (size, the rules by name, the first ranges and FQDNs). No
//! request beyond the list itself.
```

replace

```rust
/// `(table, detail)` areas. Shared by the view and `update` (page sizes); the
/// detail is dropped when the terminal is too short to keep a usable table.
pub fn layout(body: Rect, show_detail: bool) -> (Rect, Rect) {
```

with

```rust
/// `(table, detail)` areas below the Objects tab strip. Shared by the view
/// and `update` (page sizes); the detail is dropped when the terminal is too
/// short to keep a usable table.
pub fn layout(body: Rect, show_detail: bool) -> (Rect, Rect) {
```

replace

```rust
pub fn render_screen(model: &Model, f: &mut Frame, body: Rect) {
    let t = &model.theme;
    if model.index().is_none() {
        let note = if model.objects.index_failed {
            "no objects loaded · r retries"
        } else {
            "loading objects…"
        };
        f.render_widget(
            Paragraph::new(Span::styled(note, t.dim_style())).block(panel("IP lists", "", t, true)),
            body,
        );
        return;
    }
    let s = &model.ip_lists;
```

with

```rust
/// The tab's table and detail in `body` (the area below the strip); the
/// Objects screen has already handled a missing name index.
pub fn render_tab(model: &Model, f: &mut Frame, body: Rect) {
    let t = &model.theme;
    let s = &model.objects.ip_lists;
```

In `crates/illumio-tui/src/tui/ui/mod.rs`:

delete

```rust
    if model.screen == Screen::IpLists && model.is_connected() {
        screens::iplists::render_screen(model, f, area);
        return;
    }
```

In `crates/illumio-tui/src/tui/ui/widgets/overlay.rs`:

replace

```rust
    (
        "1 2 3 4 5 6",
        "Traffic · Policy · Objects · Workloads · Events · IPs",
    ),
```

with

```rust
    ("1 2 3 4 5", "Traffic · Policy · Objects · Workloads · Events"),
```

replace

```rust
    ("Objects  Tab / Enter", "next object type · detail"),
```

with

```rust
    (
        "Objects  Tab / Enter",
        "next tab: labels, groups, services, IP lists · detail",
    ),
```

delete

```rust
        (Screen::IpLists, true) => "n:new e:edit x:del  Enter detail  r refresh",
        (Screen::IpLists, false) => "Enter detail  r refresh  a auto  s sort  / search",
```

In `crates/illumio-tui/src/tui/test_support.rs`:

replace

```rust
use crate::tui::model::{Connection, Model, ProfileInfo, QueryState, TrafficData};
```

with

```rust
use crate::tui::model::{
    Connection, Model, ObjectKind, ProfileInfo, QueryState, Screen, TrafficData,
};
```

replace

```rust
    m.ip_lists.set_lists(lists, &m.policy);
    m.ip_lists.loaded_at = Some(Instant::now() - Duration::from_secs(3));
    m
}
```

with

```rust
    m.objects.ip_lists.set_lists(lists, &m.policy);
    m.objects.ip_lists.loaded_at = Some(Instant::now() - Duration::from_secs(3));
    m
}

/// Puts `m` on the Objects screen with the IP lists tab current.
pub fn on_ip_lists_tab(m: &mut Model) {
    m.screen = Screen::Objects;
    m.objects.kind = ObjectKind::IpLists;
}
```

In `crates/illumio-tui/src/tui/update.rs`:

replace

```rust
            let o = &model.objects;
            let kind = o.kind;
            if model.screen == Screen::Objects
                && model.focused
                && model.overlay == Overlay::None
                && o.ages[kind.index()].is_some()
```

with

```rust
            let o = &model.objects;
            let kind = o.kind;
            if model.screen == Screen::Objects
                && kind != ObjectKind::IpLists
                && model.focused
                && model.overlay == Overlay::None
                && o.ages[kind.index()].is_some()
```

replace

```rust
            if model.screen == Screen::IpLists
                && model.focused
                && model.overlay == Overlay::None
                && model.ip_lists.loaded_at.is_some()
                && !model.ip_lists.loading
                && model.ip_lists.auto.due(now)
            {
                let cmds = load_ip_lists(model, CacheMode::Bypass);
                model.ip_lists.auto.from_tick = true;
                return cmds;
            }
```

with

```rust
            let ipl = &model.objects.ip_lists;
            if model.screen == Screen::Objects
                && kind == ObjectKind::IpLists
                && model.focused
                && model.overlay == Overlay::None
                && ipl.loaded_at.is_some()
                && !ipl.loading
                && ipl.auto.due(now)
            {
                let cmds = load_ip_lists(model, CacheMode::Bypass);
                model.objects.ip_lists.auto.from_tick = true;
                return cmds;
            }
```

replace

```rust
            // The Objects screen was opened while the startup load was still running.
            if model.screen == Screen::Objects {
                cmds.extend(load_missing_objects(model));
            }
            if model.screen == Screen::Workloads {
                cmds.extend(load_missing_workloads(model));
            }
            if model.screen == Screen::IpLists {
                cmds.extend(load_missing_ip_lists(model));
            }
```

with

```rust
            // The Objects screen (IP lists tab included) was opened while the
            // startup load was still running.
            if model.screen == Screen::Objects {
                cmds.extend(load_missing_objects(model));
            }
            if model.screen == Screen::Workloads {
                cmds.extend(load_missing_workloads(model));
            }
```

delete

```rust
        if model.screen == Screen::IpLists && model.ip_lists.table.is_searching() {
            return ip_lists_key(model, k);
        }
```

delete

```rust
        (KeyCode::Char('6'), _) => switch(model, Screen::IpLists),
```

delete

```rust
        _ if model.screen == Screen::IpLists => ip_lists_key(model, k),
```

replace

```rust
        Screen::Objects => {
            let page = objects_page(model);
            let o = &mut model.objects;
            match o.kind {
                ObjectKind::Labels => step(&mut o.labels, down, page),
                ObjectKind::LabelGroups => step(&mut o.groups, down, page),
                ObjectKind::Services => step(&mut o.services, down, page),
            }
        }
```

with

```rust
        Screen::Objects => {
            let page = if model.objects.kind == ObjectKind::IpLists {
                ip_lists_page(model)
            } else {
                objects_page(model)
            };
            let o = &mut model.objects;
            match o.kind {
                ObjectKind::Labels => step(&mut o.labels, down, page),
                ObjectKind::LabelGroups => step(&mut o.groups, down, page),
                ObjectKind::Services => step(&mut o.services, down, page),
                ObjectKind::IpLists => step(&mut o.ip_lists.table, down, page),
            }
        }
```

delete

```rust
        Screen::IpLists => {
            let page = ip_lists_page(model);
            step(&mut model.ip_lists.table, down, page);
        }
```

replace

```rust
        Screen::Events => load_missing_events(model),
        Screen::IpLists => load_missing_ip_lists(model),
```

with

```rust
        Screen::Events => load_missing_events(model),
```

replace

```rust
/// Loads every kind that has neither arrived nor is in flight. Needs the name
/// index: it is built from the same four lists, so these are cache hits and
/// the screen costs no request.
fn load_missing_objects(model: &mut Model) -> Vec<Cmd> {
    if model.index().is_none() {
        return Vec::new();
    }
    let missing: Vec<ObjectKind> = ObjectKind::ALL
        .into_iter()
        .filter(|k| model.objects.ages[k.index()].is_none() && !model.objects.loading[k.index()])
        .collect();
    missing
        .into_iter()
        .flat_map(|k| load_objects(model, k, CacheMode::Use))
        .collect()
}

fn load_objects(model: &mut Model, kind: ObjectKind, mode: CacheMode) -> Vec<Cmd> {
    model.objects.loading[kind.index()] = true;
```

with

```rust
/// Loads every fetched kind that has neither arrived nor is in flight, and
/// the IP lists when their tab is the current one. Needs the name index: it
/// is built from the same four lists, so these are cache hits and the screen
/// costs no request.
fn load_missing_objects(model: &mut Model) -> Vec<Cmd> {
    if model.index().is_none() {
        return Vec::new();
    }
    let missing: Vec<ObjectKind> = ObjectKind::FETCHED
        .into_iter()
        .filter(|k| model.objects.ages[k.index()].is_none() && !model.objects.loading[k.index()])
        .collect();
    let mut cmds: Vec<Cmd> = missing
        .into_iter()
        .flat_map(|k| load_objects(model, k, CacheMode::Use))
        .collect();
    if model.objects.kind == ObjectKind::IpLists {
        cmds.extend(load_missing_ip_lists(model));
    }
    cmds
}

fn load_objects(model: &mut Model, kind: ObjectKind, mode: CacheMode) -> Vec<Cmd> {
    if kind == ObjectKind::IpLists {
        return load_ip_lists(model, mode);
    }
    model.objects.loading[kind.index()] = true;
```

replace

```rust
fn ip_lists_page(model: &Model) -> usize {
    let area = ratatui::layout::Rect::new(0, 0, model.size.0, model.size.1);
    crate::tui::ui::layout::chrome(area)
        .map(|c| crate::tui::ui::screens::iplists::table_rows(c.body, model.ip_lists.show_detail))
        .unwrap_or(10)
        .max(1)
}
```

with

```rust
fn ip_lists_page(model: &Model) -> usize {
    use crate::tui::ui::screens::{iplists, objects};
    let area = ratatui::layout::Rect::new(0, 0, model.size.0, model.size.1);
    crate::tui::ui::layout::chrome(area)
        .map(|c| {
            iplists::table_rows(
                objects::strip_split(c.body).1,
                model.objects.ip_lists.show_detail,
            )
        })
        .unwrap_or(10)
        .max(1)
}
```

replace

```rust
/// Loads the IP lists once. Waits for the name index like Objects does: the
/// index reads the same list, so after it the load is a cache hit.
```

with

```rust
/// Loads the IP lists once (the Objects screen's IP lists tab). Waits for the
/// name index like the other kinds: the index reads the same list, so after
/// it the load is a cache hit.
```

replace

```rust
fn objects_key(model: &mut Model, k: KeyEvent) -> Vec<Cmd> {
    let page = objects_page(model);
    if !model.objects.is_searching() {
        let kind = model.objects.kind;
        match k.code {
            KeyCode::Tab | KeyCode::BackTab => {
                model.objects.kind = if k.code == KeyCode::Tab {
                    kind.next()
                } else {
                    kind.prev()
                };
                model.objects.show_detail = false;
                return Vec::new();
            }
```

with

```rust
fn objects_key(model: &mut Model, k: KeyEvent) -> Vec<Cmd> {
    let page = objects_page(model);
    if !model.objects.is_searching() {
        let kind = model.objects.kind;
        match k.code {
            KeyCode::Tab | KeyCode::BackTab => {
                let o = &mut model.objects;
                o.kind = if k.code == KeyCode::Tab {
                    kind.next()
                } else {
                    kind.prev()
                };
                o.show_detail = false;
                o.ip_lists.show_detail = false;
                // The first visit to the IP lists tab loads them (a cache hit).
                return if o.kind == ObjectKind::IpLists {
                    load_missing_ip_lists(model)
                } else {
                    Vec::new()
                };
            }
            _ if kind == ObjectKind::IpLists => return ip_lists_key(model, k),
```

replace

```rust
    let o = &mut model.objects;
    match o.kind {
        ObjectKind::Labels => drive_table(&mut o.labels, k, page),
        ObjectKind::LabelGroups => drive_table(&mut o.groups, k, page),
        ObjectKind::Services => drive_table(&mut o.services, k, page),
    }
    Vec::new()
}
```

with

```rust
    let o = &mut model.objects;
    match o.kind {
        ObjectKind::Labels => drive_table(&mut o.labels, k, page),
        ObjectKind::LabelGroups => drive_table(&mut o.groups, k, page),
        ObjectKind::Services => drive_table(&mut o.services, k, page),
        // A search on the IP lists tab: its own keys and page size.
        ObjectKind::IpLists => return ip_lists_key(model, k),
    }
    Vec::new()
}
```

replace

```rust
    let o = &model.objects;
    let intent = match (o.kind, c) {
        (ObjectKind::LabelGroups, _) => {
```

with

```rust
    let o = &model.objects;
    let intent = match (o.kind, c) {
        (ObjectKind::IpLists, _) => return ip_lists_edit_key(model, c),
        (ObjectKind::LabelGroups, _) => {
```

replace

```rust
        Command::Screen(s) => switch(model, s),
        Command::Quit => {
```

with

```rust
        Command::Screen(s) => switch(model, s),
        Command::IpLists => {
            model.objects.kind = ObjectKind::IpLists;
            model.objects.show_detail = false;
            switch(model, Screen::Objects)
        }
        Command::Quit => {
```

Then move every remaining use of `Model.ip_lists` to `ObjectsState.ip_lists`, and the tests that set `Screen::IpLists` to the new helper (mechanical; the code above is written for after this rename):

```bash
perl -pi -e 's/\b(m|l|model)\.ip_lists\b/$1.objects.ip_lists/g' \
  crates/illumio-tui/src/tui/update.rs crates/illumio-tui/src/tui/ui/screens/iplists.rs \
  crates/illumio-tui/tests/tui_runtime.rs crates/illumio-tui/tests/tui_snapshots.rs
perl -pi -e 's/^(\s*)(m|l)\.screen = Screen::IpLists;/$1crate::tui::test_support::on_ip_lists_tab(&mut $2);/' \
  crates/illumio-tui/src/tui/update.rs
perl -pi -e 's/^(\s*)(m|l)\.screen = Screen::IpLists;/$1illumio_tui::tui::test_support::on_ip_lists_tab(&mut $2);/' \
  crates/illumio-tui/tests/tui_snapshots.rs
grep -rn 'Screen::IpLists\|\bmodel\.ip_lists\|\bm\.ip_lists' crates/illumio-tui/src crates/illumio-tui/tests
```

Expected: the `grep` prints nothing.

- [ ] **Step 4: Run the unit and runtime tests**

Run: `cargo fmt --all && cargo test -p illumio-tui --lib && cargo test -p illumio-tui --test tui_runtime`
Expected: PASS, including `the_ip_lists_tab_loads_once_and_r_bypasses_the_cache_once`, `ip_lists_wait_for_the_name_index_and_the_command_opens_their_tab`, `every_key_and_the_wheel_on_the_ip_lists_tab_stay_on_its_own_state` and `opening_the_ip_lists_tab_costs_nothing_after_startup_and_r_bypasses_once`.

- [ ] **Step 5: Re-record the snapshots and check that only the tab row moved**

```bash
git rm -q crates/illumio-tui/tests/snapshots/tui_snapshots__ip_lists_80x24.snap \
  crates/illumio-tui/tests/snapshots/tui_snapshots__ip_lists_160x48.snap \
  crates/illumio-tui/tests/snapshots/tui_snapshots__ip_lists_light_styled_80x24.snap \
  crates/illumio-tui/tests/snapshots/tui_snapshots__ip_list_detail_many_ranges_80x24.snap \
  crates/illumio-tui/tests/snapshots/tui_snapshots__ip_list_detail_many_ranges_light_styled_80x24.snap
INSTA_UPDATE=always cargo test -p illumio-tui --test tui_snapshots
find crates/illumio-tui/tests/snapshots -name '*.snap.new' -delete
git status --short crates/illumio-tui/tests/snapshots | awk '{print $1}' | sort | uniq -c
```

Expected: `65 M`, `5 D` (the five above) and `5 ??`: `objects_ip_lists_80x24`, `objects_ip_lists_160x48`, `objects_ip_lists_light_styled_80x24`, `objects_ip_list_detail_many_ranges_80x24`, `objects_ip_list_detail_many_ranges_light_styled_80x24`. The 65 are every snapshot that shows the tab row (all but `table_widget_40x10`, `events_detail_end_80x24` and the two `confirm_rule_impact` ones) and the three help overlays.

Check that nothing else moved: outside the help overlays and the four IP-list overlays (drawn over the IP lists tab now, so their background gains the strip), every changed line is the tab row, the Objects strip, or a colour-run line of a styled snapshot:

```bash
git diff -U0 -- crates/illumio-tui/tests/snapshots ':!*help_overlay*' ':!*form_ip_list*' ':!*confirm_ip_list_delete*' \
  | grep '^[-+][^-+]' | grep -v 'Traf' | grep -v 'Label groups' | grep -v '^[-+]    @'
```

Expected: no output. The tab row reads `  1 Traffic  │  2 Policy  │  3 Objects  │  4 Workloads  │  5 Events` at 80 and 160 columns (the full row is 69 cells), and the strip `  Labels  │  Label groups  │  Services  │  IP lists`. Read the three help diffs (`1 2 3 4 5` with `Traffic · Policy · Objects · Workloads · Events`, and `next tab: labels, groups, services, IP lists · detail`), the four IP-list overlays (the form and the delete confirm over the strip and the IP lists table, the hint now the Objects one) and the five new `objects_ip_list*` snapshots (the strip with `IP lists` highlighted, the table `name size ranges fqdns used by pending`, the detail with `65.5K addresses (65536 IPv4, 0 IPv6)` and `used by Shop-Prod · allow · role=web ← ipl:Corp`).

- [ ] **Step 6: Docs**

In `README.md`, replace

```markdown
Six screens: `1` Traffic, `2` Policy, `3` Objects, `4` Workloads, `5` Events, `6` IP lists. The
`:` prompt takes `:traffic`, `:policy`, `:objects`, `:workloads`, `:events`, `:iplists`,
`:filter <expr>`, `:window <dur>`, `:profile <name>`, `:writes [on|off]`, `:draft`, `:active`,
`:help` and `:q`.
`?` shows the full key map, `Esc` closes overlays, `q` or `Ctrl-C` quits. The mouse wheel
scrolls the current table and a click on a tab switches screen; on a terminal narrower than
the full tab row the tabs read `Traf`, `Pol`, `Obj`, `Wl`, `Ev`, `IPs`.
```

with

```markdown
Five screens: `1` Traffic, `2` Policy, `3` Objects, `4` Workloads, `5` Events. The `:` prompt
takes `:traffic`, `:policy`, `:objects`, `:workloads`, `:events`, `:iplists` (Objects on its
IP lists tab), `:filter <expr>`, `:window <dur>`, `:profile <name>`, `:writes [on|off]`,
`:draft`, `:active`, `:help` and `:q`.
`?` shows the full key map, `Esc` closes overlays, `q` or `Ctrl-C` quits. The mouse wheel
scrolls the current table and a click on a tab switches screen; on a terminal narrower than
the full tab row (69 columns) the tabs read `Traf`, `Pol`, `Obj`, `Wl`, `Ev`.
```

replace

```markdown
- **Objects**: labels, label groups and services; `Tab` next type, `Enter` detail.
- **IP lists**: name, size (addresses after exclusions: `65.5K`, `16.8M` for a /8; `any` only
  for an included `0.0.0.0/0` or `::/0`; `5 FQDNs` for a list of names only), ranges, FQDNs,
  `used by` (rules of the loaded draft rulesets that name the list; `-` until Policy is loaded)
  and pending; `Enter` shows the size, those rules, then the first ranges and FQDNs. With
  writes on, `n` / `e` / `x` create, edit and delete.
```

with

```markdown
- **Objects**: four tabs, labels, label groups, services and IP lists; `Tab` / `Shift-Tab`
  next / previous tab, `Enter` detail. The IP lists tab shows name, size (addresses after
  exclusions: `65.5K`, `16.8M` for a /8; `any` only for an included `0.0.0.0/0` or `::/0`;
  `5 FQDNs` for a list of names only), ranges, FQDNs, `used by` (rules of the loaded draft
  rulesets that name the list; `-` until Policy is loaded) and pending; `Enter` shows the size,
  those rules, then the first ranges and FQDNs. With writes on, `n` / `e` / `x` create, edit
  and delete labels, services and IP lists.
```

replace

```markdown
Workloads and to Events costs one request each; going back to them costs none. IP lists (`6`)
cost none within the cache TTL (15 minutes): the startup load already read them. Rule impact
```

with

```markdown
Workloads and to Events costs one request each; going back to them costs none. The IP lists
tab costs none within the cache TTL (15 minutes): the startup load already read them. Rule impact
```

and replace

```markdown
header shows `✎ writes` before the host and three screens take three keys: on Objects (labels,
services), IP lists and Workloads (unmanaged workloads only)
```

with

```markdown
header shows `✎ writes` before the host and two screens take three keys: on Objects (labels,
services, IP lists) and Workloads (unmanaged workloads only)
```

In `docs/architecture.md`, replace

```markdown
opening or closing an overlay. IP lists are their own screen
(`Screen::IpLists`, `IpListsState`): the list is the index's own (a cache
hit after startup), sizes are computed once per load, and `used_by` counts
```

with

```markdown
opening or closing an overlay. IP lists are the Objects screen's fourth tab
(`ObjectKind::IpLists`, `ObjectsState.ip_lists: IpListsState`, loaded by
`Cmd::LoadIpLists` on the tab's first visit): the list is the index's own (a cache
hit after startup), sizes are computed once per load, and `used_by` counts
```

In `docs/index.html`, replace

```html
        <li><kbd>3</kbd><strong>Objects.</strong> Labels, label groups and services, with cache age shown and per-type refresh.</li>
```

with

```html
        <li><kbd>3</kbd><strong>Objects.</strong> Labels, label groups, services and IP lists (each list with its size in addresses, exclusions taken out, its ranges and FQDNs, and the rules that use it), with cache age shown and per-type refresh.</li>
```

and delete the line

```html
        <li><kbd>6</kbd><strong>IP lists.</strong> Every IP list with its size in addresses (exclusions taken out), its ranges and FQDNs, and the rules that use it.</li>
```

- [ ] **Step 7: Gate and commit**

```bash
cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace
git add -A crates/illumio-tui README.md docs/architecture.md docs/index.html
git commit -m "feat(tui): IP lists become the fourth Objects tab; five screens again (issue #5)

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

Expected: 729 tests pass (728 before; one new unit test, the renamed ones unchanged in number).

---

### Task 2: `pce::traffic` grouping primitives and the rich fixture

**Files:**
- Create: `fixtures/traffic_flows_rich.json`, `fixtures/traffic_labels_rich.json` (from the generator below; the generator lives in the session scratchpad and is not committed)
- Modify: `crates/pce/src/traffic/resolved.rs` (`ResolvedFlow` ~line 76, `impl ResolvedFlow` ~92, `resolve_flows` ~103; tests ~131)
- Modify: `crates/pce/src/traffic/aggregate.rs` (rewritten: everything above `#[cfg(test)]`, and the `mod tests`)
- Modify: `crates/pce/src/traffic/mod.rs` (the `pub use aggregate::…` line)
- Modify: `crates/illumio-tui/src/tui/model.rs` (`TrafficState::rebuild_views` ~line 752), `crates/illumio-tui/src/cli/traffic.rs` (`query` ~line 369): the new `blocked_summary` argument
- Modify: `crates/illumio-tui/src/tui/test_support.rs` (new `fixture_flows_rich`, `model_with_rich_flows` before `model_with_flows`)
- Test: `crates/illumio-tui/tests/snapshots/tui_snapshots__traffic_blocked_80x24.snap` (re-recorded: the `sources` cells)

**Interfaces:**
- Consumes: `ResolvedEndpoint::{is_workload, label_value, display_name, label_text}`, `ResolvedLabel`, `PolicyDecision`, `crate::iplist_size::human` (`pub(crate)`), `models::Flow::{dst_bi, dst_bo}`.
- Produces:

```rust
// pce::traffic::resolved
pub struct ResolvedFlow { /* … */ pub bytes_in: Option<u64>, pub bytes_out: Option<u64> }
impl ResolvedFlow { pub fn bytes(&self) -> u64; pub fn has_bytes(&self) -> bool; }
// pce::traffic::aggregate (all re-exported from pce::traffic)
pub type Dominant = PolicyDecision;
pub fn dominant(allowed: u64, potentially_blocked: u64, blocked: u64) -> Dominant;
pub fn keys_or_default(keys: &[String]) -> Vec<String>;            // empty -> app,env
pub fn compact_count(n: u64) -> String;                             // 999, 1.2K, 13.6M
pub fn group_values(ep: &ResolvedEndpoint, keys: &[String]) -> Vec<String>;
pub fn group_text(ep: &ResolvedEndpoint, keys: &[String]) -> String;
pub fn group_key(ep: &ResolvedEndpoint, keys: &[String]) -> Vec<String>;
pub fn keys_seen(flows: &[ResolvedFlow], preferred: &[String]) -> Vec<(String, usize)>;
pub struct FlowGroup { pub src, pub dst: Vec<String>, pub flows, pub connections, pub bytes,
                       pub allowed, pub potentially_blocked, pub blocked: u64 }
impl FlowGroup { pub fn dominant(&self) -> Dominant; pub fn contains(&self, f: &ResolvedFlow, keys: &[String]) -> bool; }
pub const BLOCKED_NAMES: usize = 5;
pub struct BlockedGroup { pub dst: String, pub dst_labels: String, pub dst_group: Vec<String>,
                          pub dst_names: Vec<String>, pub dst_hosts: u64, pub port: Option<u16>,
                          pub proto: String, pub flows: u64, pub connections: u64,
                          pub sources: Vec<String>, pub source_count: u64, pub last_detected: Option<String> }
impl BlockedGroup { pub fn contains(&self, f: &ResolvedFlow, keys: &[String]) -> bool; }
pub fn group_by_labels(flows: &[ResolvedFlow], keys: &[String]) -> Vec<FlowGroup>;   // unchanged signature
pub fn blocked_summary(flows: &[ResolvedFlow], keys: &[String]) -> Vec<BlockedGroup>;
// pce::traffic::resolved::tests (pub(crate), for the mesh and treemap tests)
pub(crate) fn rich_index() -> NameIndex;
pub(crate) fn rich_flows() -> Vec<ResolvedFlow>;
// illumio_tui::tui::test_support
pub fn fixture_flows_rich() -> Vec<ResolvedFlow>;
pub fn model_with_rich_flows() -> Model;   // model_with_flows() with the 30 rich flows, views rebuilt
```

- [ ] **Step 1: Create the rich fixture**

Save as `$SCRATCH/gen_rich_fixture.py` (`$SCRATCH` is the executing session's scratchpad directory) and run it from the repository root:

```python
#!/usr/bin/env python3
"""Writes fixtures/traffic_flows_rich.json (30 raw Explorer flows) and
fixtures/traffic_labels_rich.json (the 7 labels they use beyond
fixtures/labels.json). Run once from the repository root; the output is
committed, this script is not."""
import json
import sys

out_dir = sys.argv[1] if len(sys.argv) > 1 else "fixtures"
LABELS = {  # id -> (key, value); 1-5 already exist in fixtures/labels.json
    101: ("app", "crm"), 102: ("app", "erp"), 103: ("app", "hr"), 104: ("env", "dev"),
    105: ("role", "api"), 106: ("loc", "us"), 107: ("datacenter", "fra1"),
}
# name -> (ip, label ids); 3 app=shop, 4 env=prod, 1 role=web, 2 role=db, 5 loc=eu
WORKLOADS = {
    "shop-web-p": ("10.0.1.11", [3, 4, 1, 5]),
    "shop-db-p": ("10.0.1.12", [3, 4, 2, 5]),
    "shop-web-d": ("10.0.9.11", [3, 104, 1, 5]),
    "shop-db-d": ("10.0.9.12", [3, 104, 2, 5]),
    "crm-web-p": ("10.0.2.11", [101, 4, 1, 106, 107]),
    "crm-db-p": ("10.0.2.12", [101, 4, 2, 106]),
    "crm-api-d": ("10.0.8.13", [101, 104, 105, 106]),
    "erp-api-p": ("10.0.3.13", [102, 4, 105, 5, 107]),
    "erp-db-p": ("10.0.3.12", [102, 4, 2, 5]),
    "erp-web-d": ("10.0.7.11", [102, 104, 1, 5]),
    "hr-web-p": ("10.0.4.11", [103, 4, 1, 106]),
    "hr-db-d": ("10.0.6.12", [103, 104, 2]),
}
CORP = {"ip": "10.1.4.9", "ip_lists": [{"href": "/orgs/5/sec_policy/draft/ip_lists/21", "name": "Corp"}]}
NET = {"ip": "203.0.113.7", "fqdn": "api.partner.example"}
PROTO = {"tcp": 6, "udp": 17, "icmp": 1}
# (src, dst, port, proto, process, user, direction, decision, conns, dst_bi, dst_bo, last seen)
FLOWS = [
    ("shop-web-p", "shop-db-p", 5432, "tcp", "postgres", "postgres", "inbound", "allowed", 420, 900000, 40000, "07:31:02"),
    ("shop-web-p", "shop-db-p", 5432, "tcp", "java", "app", "outbound", "allowed", 380, None, None, "07:30:40"),
    ("shop-web-p", "crm-web-p", 443, "tcp", "java", "app", "outbound", "allowed", 150, 52000, 9000, "07:29:10"),
    ("crm-web-p", "crm-db-p", 3306, "tcp", "mysqld", "mysql", "inbound", "allowed", 610, 2400000, 80000, "07:31:00"),
    ("crm-web-p", "crm-db-p", 3306, "tcp", "node", "www", "outbound", "allowed", 240, None, None, "07:28:00"),
    ("crm-web-p", "erp-api-p", 8443, "tcp", "node", "www", "outbound", "potentially_blocked", 35, 4100, 1200, "07:25:00"),
    ("crm-api-d", "crm-db-p", 3306, "tcp", "python3", "svc", "outbound", "blocked", 12, None, None, "07:20:00"),
    ("erp-api-p", "erp-db-p", 1521, "tcp", "oracle", "oracle", "inbound", "allowed", 300, 1500000, 60000, "07:31:05"),
    ("erp-api-p", "erp-db-p", 1521, "tcp", "java", "erp", "outbound", "allowed", 290, None, None, "07:30:05"),
    ("erp-web-d", "erp-api-p", 8443, "tcp", "nginx", "www-data", "outbound", "potentially_blocked", 44, None, None, "07:22:00"),
    ("erp-web-d", "erp-db-p", 1521, "tcp", "nginx", "www-data", "outbound", "blocked", 8, None, None, "07:21:00"),
    ("hr-web-p", "hr-db-d", 5432, "tcp", "php-fpm", "www", "outbound", "blocked", 20, None, None, "07:15:00"),
    ("hr-web-p", "shop-db-p", 5432, "tcp", "php-fpm", "www", "outbound", "potentially_blocked", 16, 3000, 800, "07:16:00"),
    ("hr-web-p", "crm-web-p", 443, "tcp", "curl", "root", "outbound", "allowed", 60, None, None, "07:18:00"),
    ("shop-web-d", "shop-db-d", 5432, "tcp", "postgres", "postgres", "inbound", "allowed", 90, 200000, 10000, "07:10:00"),
    ("shop-web-d", "shop-db-p", 5432, "tcp", "java", "app", "outbound", "blocked", 6, None, None, "07:12:00"),
    ("shop-web-d", "shop-db-d", 22, "tcp", "sshd", "root", "inbound", "potentially_blocked", 3, None, None, "07:05:00"),
    ("corp", "shop-web-p", 443, "tcp", "nginx", "www-data", "inbound", "allowed", 700, 8000000, 350000, "07:31:10"),
    ("corp", "crm-web-p", 443, "tcp", "node", "www", "inbound", "allowed", 520, None, None, "07:31:08"),
    ("corp", "hr-web-p", 443, "tcp", "httpd", "apache", "inbound", "potentially_blocked", 75, None, None, "07:26:00"),
    ("corp", "erp-api-p", 22, "tcp", "sshd", "root", "inbound", "blocked", 4, None, None, "07:02:00"),
    ("corp", "shop-db-p", 5432, "tcp", None, None, "inbound", "blocked", 2, None, None, "07:01:00"),
    ("shop-web-p", "net", 443, "tcp", "curl", "root", "outbound", "allowed", 33, 12000, 3000, "07:27:00"),
    ("crm-api-d", "net", 443, "tcp", "python3", "svc", "outbound", "potentially_blocked", 18, None, None, "07:19:00"),
    ("erp-api-p", "net", 443, "tcp", "java", "erp", "outbound", "allowed", 27, None, None, "07:24:00"),
    ("hr-db-d", "hr-web-p", None, "icmp", None, None, None, "blocked", 5, None, None, "07:00:30"),
    ("shop-web-p", "shop-db-p", 5432, "tcp", "java", "batch", "outbound", "allowed", 14, None, None, "07:08:00"),
    ("erp-db-p", "crm-db-p", 53, "udp", "named", "bind", "inbound", "unknown_decision", 9, None, None, "07:03:00"),
    ("crm-db-p", "erp-db-p", 1521, "tcp", "oracle", "oracle", "inbound", "potentially_blocked", 11, None, None, "07:04:00"),
    ("shop-web-p", "shop-db-p", 5432, "tcp", "python3", "app", "outbound", "allowed", 7, None, None, "07:06:00"),
]
assert len(FLOWS) == 30


def endpoint(name):
    if name == "corp":
        return CORP
    if name == "net":
        return NET
    ip, labels = WORKLOADS[name]
    return {"ip": ip, "workload": {
        "href": f"/orgs/5/workloads/rich-{name}", "hostname": f"{name}.example",
        "labels": [{"href": f"/orgs/5/labels/{i}"} for i in labels]}}


rows = []
for src, dst, port, proto, process, user, direction, decision, conns, bi, bo, last in FLOWS:
    service = {"proto": PROTO[proto]} if port is None else {"port": port, "proto": PROTO[proto]}
    if process:
        service["process_name"] = process
    if user:
        service["user_name"] = user
    flow = {"src": endpoint(src), "dst": endpoint(dst), "service": service,
            "policy_decision": decision, "num_connections": conns}
    if direction:
        flow["flow_direction"] = direction
    if bi is not None:
        flow["dst_bi"] = bi
        flow["dst_bo"] = bo
    flow["timestamp_range"] = {"first_detected": "2026-10-10T06:00:00Z",
                               "last_detected": f"2026-10-10T{last}Z"}
    rows.append(flow)


def write(name, items):
    with open(f"{out_dir}/{name}", "w") as fh:
        fh.write("[\n" + ",\n".join("  " + json.dumps(i, separators=(",", ":")) for i in items) + "\n]\n")


write("traffic_flows_rich.json", rows)
write("traffic_labels_rich.json", [
    {"href": f"/orgs/5/labels/{i}", "key": k, "value": v, "deleted": False}
    for i, (k, v) in sorted(LABELS.items())])
print("wrote", out_dir)
```

```bash
python3 -I "$SCRATCH/gen_rich_fixture.py" fixtures
shasum -a 256 fixtures/traffic_flows_rich.json fixtures/traffic_labels_rich.json
```

Expected:

```
718d85d6aedfa16a3216b1e85f295740be75f523525a22c4d26f742781144b31  fixtures/traffic_flows_rich.json
4ae8bf77d5b69a3c201055932529ac5613143e70c45d851f7e7918ea99a175e9  fixtures/traffic_labels_rich.json
```

What it holds (the tests below count on it): 12 workloads across `app` shop / crm / erp / hr and `env` prod / dev with `role` web / db / api; `loc` on every workload but `hr-db-d`; `datacenter=fra1` on `crm-web-p` and `erp-api-p` (11 flows carry it); the `Corp` IP list as a source (5 flows) and `api.partner.example` as a destination (3 flows); inbound and outbound flows, one ICMP flow without direction or process, one UDP flow with an unknown decision; `dst_bi` / `dst_bo` on 9 flows (13 625 100 bytes in all); 4 109 connections. Flows 1, 26 and 29 (0-based) are `shop-web-p -> shop-db-p 5432/tcp` outbound as `java`/`app`, `java`/`batch` and `python3`/`app`, so hiding the user or the process merges mesh rows.

- [ ] **Step 2: Write the failing tests**

In `crates/pce/src/traffic/resolved.rs`:

replace

```rust
    pub(crate) fn flows() -> Vec<ResolvedFlow> {
```

with

```rust
    /// `index()` plus the labels only the rich fixture uses (`app=crm`,
    /// `env=dev`, `role=api`, `loc=us`, `datacenter=fra1`, ...).
    pub(crate) fn rich_index() -> NameIndex {
        let mut labels: Vec<Label> =
            serde_json::from_str(include_str!("../../../../fixtures/labels.json")).unwrap();
        let rich: Vec<Label> = serde_json::from_str(include_str!(
            "../../../../fixtures/traffic_labels_rich.json"
        ))
        .unwrap();
        labels.extend(rich);
        let groups: Vec<LabelGroup> =
            serde_json::from_str(include_str!("../../../../fixtures/label_groups.json")).unwrap();
        let lists: Vec<IpList> =
            serde_json::from_str(include_str!("../../../../fixtures/ip_lists.json")).unwrap();
        let services: Vec<Service> =
            serde_json::from_str(include_str!("../../../../fixtures/services.json")).unwrap();
        NameIndex::new(&labels, &groups, &lists, &services)
    }

    /// The 30-flow fixture: 4 apps x 2 envs, roles, `loc`, `datacenter` on
    /// some workloads, the Corp IP list and an FQDN, users, processes, both
    /// directions, bytes on 9 flows.
    pub(crate) fn rich_flows() -> Vec<ResolvedFlow> {
        let raw: Vec<Flow> =
            serde_json::from_str(include_str!("../../../../fixtures/traffic_flows_rich.json"))
                .unwrap();
        resolve_flows(&raw, &rich_index())
    }

    pub(crate) fn flows() -> Vec<ResolvedFlow> {
```

replace

```rust
    #[test]
    fn icmp_flow_without_port_renders() {
```

with

```rust
    #[test]
    fn bytes_are_copied_from_dst_bi_and_dst_bo() {
        let f = flows();
        assert_eq!(
            (f[0].bytes_in, f[0].bytes_out),
            (Some(120_000), Some(4_000))
        );
        assert_eq!(f[0].bytes(), 124_000);
        assert!(f[0].has_bytes());
        assert_eq!((f[1].bytes_in, f[1].bytes_out), (None, None));
        assert_eq!(f[1].bytes(), 0);
        assert!(!f[1].has_bytes());
        let rich = rich_flows();
        assert_eq!(rich.len(), 30);
        assert_eq!(rich.iter().filter(|f| f.has_bytes()).count(), 9);
        assert_eq!(
            rich.iter().map(ResolvedFlow::bytes).sum::<u64>(),
            13_625_100
        );
        assert_eq!(rich[4].src.label_value("app"), Some("crm"));
        assert_eq!(rich[4].src.label_value("datacenter"), Some("fra1"));
    }

    #[test]
    fn icmp_flow_without_port_renders() {
```

In `crates/pce/src/traffic/aggregate.rs`, replace the whole `#[cfg(test)] mod tests { … }` block with:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::traffic::resolved::tests::{flows, rich_flows};

    fn keys(k: &[&str]) -> Vec<String> {
        k.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn groups_by_label_pair_with_fallbacks() {
        let groups = group_by_labels(&flows(), &keys(&["app", "env"]));
        assert_eq!(groups[0].src, vec!["shop", "prod"]);
        assert_eq!(groups[0].dst, vec!["shop", "prod"]);
        assert_eq!(groups[0].flows, 1);
        assert_eq!(groups[0].connections, 310);
        assert_eq!(groups[0].bytes, 124_000);
        assert_eq!(groups[0].allowed, 1);
        let corp = groups
            .iter()
            .find(|g| g.src == vec!["ipl:Corp", "-"])
            .expect("ip list source falls back to its name");
        assert_eq!(corp.potentially_blocked, 1);
        assert_eq!(corp.bytes, 0);
        let nas = groups
            .iter()
            .find(|g| g.src == vec!["unmanaged-nas", "-"])
            .expect("unlabeled workload falls back to its name");
        assert_eq!(nas.blocked, 1);
        let ext = groups
            .iter()
            .find(|g| g.dst == vec!["fqdn-1.example", "-"])
            .expect("fqdn destination");
        assert_eq!(ext.connections, 5);
        assert!(
            groups
                .windows(2)
                .all(|w| w[0].connections >= w[1].connections),
            "sorted by connections desc"
        );
    }

    #[test]
    fn group_values_dash_a_missing_key_and_every_slot_of_a_non_workload() {
        let f = flows();
        let k = keys(&["app", "env", "datacenter"]);
        assert_eq!(group_values(&f[0].src, &k), vec!["shop", "prod", "-"]);
        assert_eq!(group_text(&f[0].src, &k), "shop/prod/-");
        assert_eq!(group_values(&f[1].src, &k), vec!["-", "-", "-"], "IP list");
        assert_eq!(group_text(&f[3].dst, &keys(&["app"])), "-", "FQDN");
        assert_eq!(
            group_values(&f[2].src, &k),
            vec!["-", "-", "-"],
            "a workload without labels"
        );
        assert_eq!(
            group_values(&f[0].src, &keys(&["role", "app"])),
            vec!["web", "shop"],
            "in the order of the keys"
        );
        assert_eq!(group_key(&f[1].src, &k), vec!["ipl:Corp", "-", "-"]);
        assert_eq!(group_key(&f[0].src, &k), vec!["shop", "prod", "-"]);
        assert_eq!(group_text(&f[0].src, &[]), "", "no keys, no slots");
    }

    #[test]
    fn keys_seen_puts_the_preferred_keys_first_and_counts_flows_once() {
        let pref = keys(&["app", "env", "role", "loc"]);
        let seen = keys_seen(&rich_flows(), &pref);
        assert_eq!(
            seen,
            vec![
                ("app".to_string(), 30),
                ("env".to_string(), 30),
                ("role".to_string(), 30),
                ("loc".to_string(), 30),
                ("datacenter".to_string(), 11),
            ]
        );
        let seen = keys_seen(&rich_flows(), &keys(&["role"]));
        let order: Vec<&str> = seen.iter().map(|(k, _)| k.as_str()).collect();
        assert_eq!(order, vec!["role", "app", "datacenter", "env", "loc"]);
        let few = keys_seen(&flows(), &pref);
        assert_eq!(
            few,
            vec![
                ("app".to_string(), 4),
                ("env".to_string(), 4),
                ("role".to_string(), 4),
            ],
            "the 4-flow fixture: every flow has a labelled side; loc is on no workload"
        );
        assert!(keys_seen(&[], &pref).is_empty());
    }

    #[test]
    fn rich_groups_sum_bytes_and_count_unknown_only_in_flows() {
        let groups = group_by_labels(&rich_flows(), &keys(&["app", "env"]));
        assert_eq!(groups.len(), 21);
        let top: Vec<(String, String, u64, u64, u64)> = groups
            .iter()
            .take(3)
            .map(|g| {
                (
                    g.src.join("/"),
                    g.dst.join("/"),
                    g.flows,
                    g.connections,
                    g.bytes,
                )
            })
            .collect();
        assert_eq!(
            top,
            vec![
                ("crm/prod".into(), "crm/prod".into(), 2, 850, 2_480_000),
                ("shop/prod".into(), "shop/prod".into(), 4, 821, 940_000),
                ("ipl:Corp/-".into(), "shop/prod".into(), 2, 702, 8_350_000),
            ]
        );
        let unknown = groups
            .iter()
            .find(|g| g.src == vec!["erp", "prod"] && g.dst == vec!["crm", "prod"])
            .unwrap();
        assert_eq!(
            (unknown.flows, unknown.connections, unknown.allowed),
            (1, 9, 0)
        );
        assert_eq!(unknown.dominant(), PolicyDecision::Unknown);
        assert_eq!(groups[2].dominant(), PolicyDecision::Blocked);
        assert_eq!(
            groups.iter().map(|g| g.connections).sum::<u64>(),
            4_109,
            "every flow lands in exactly one group"
        );
        let f = rich_flows();
        assert!(groups[0].contains(&f[3], &keys(&["app", "env"])));
        assert!(!groups[0].contains(&f[0], &keys(&["app", "env"])));
    }

    #[test]
    fn blocked_summary_groups_by_destination_and_port() {
        let b = blocked_summary(&flows(), &keys(&["app", "env"]));
        assert_eq!(b.len(), 2);
        assert_eq!(b[0].dst, "host-1.example");
        assert_eq!(b[0].dst_group, vec!["shop", "prod"]);
        assert_eq!(b[0].port, Some(8443));
        assert_eq!(b[0].proto, "tcp");
        assert_eq!(b[0].sources, vec!["ipl:Corp/-"]);
        assert_eq!(b[0].last_detected.as_deref(), Some("2026-10-10T07:30:58Z"));
        assert_eq!(b[1].dst, "host-2.example");
        assert_eq!(b[1].dst_labels, "app=shop env=prod role=db");
        assert_eq!(b[1].proto, "icmp");
        assert_eq!(b[1].connections, 3);
        assert_eq!(b[1].sources, vec!["unmanaged-nas/-"]);
    }

    #[test]
    fn blocked_summary_groups_by_the_destination_group_and_caps_the_names() {
        let k = keys(&["app", "env"]);
        let b = blocked_summary(&rich_flows(), &k);
        assert_eq!(b.len(), 10);
        let shop = b
            .iter()
            .find(|r| r.dst_group == vec!["shop", "prod"])
            .unwrap();
        assert_eq!(
            (shop.port, shop.flows, shop.connections),
            (Some(5432), 3, 24)
        );
        assert_eq!(shop.dst_names, vec!["shop-db-p.example"]);
        assert_eq!(shop.dst_hosts, 1);
        assert_eq!(shop.sources, vec!["hr/prod", "shop/dev", "ipl:Corp/-"]);
        assert_eq!(shop.source_count, 3);
        let erp: Vec<(Option<u16>, u64)> = b
            .iter()
            .filter(|r| r.dst_group == vec!["erp", "prod"])
            .map(|r| (r.port, r.connections))
            .collect();
        assert_eq!(erp, vec![(Some(8443), 79), (Some(1521), 19), (Some(22), 4)]);
        assert_eq!(b[0].last_detected.as_deref(), Some("2026-10-10T07:26:00Z"));
        assert!(
            b.windows(2)
                .all(|w| w[0].last_detected >= w[1].last_detected),
            "newest first"
        );
        // By env alone, every prod destination on 5432/tcp is one row and
        // the names stop at five while the counts keep going.
        let mut many = rich_flows();
        let template = many[12].clone(); // hr/prod -> shop-db-p 5432 potentially blocked
        for i in 0..7 {
            let mut f = template.clone();
            f.dst.hostname = Some(format!("db-{i}.example"));
            f.src.hostname = Some(format!("src-{i}.example"));
            f.src.labels.retain(|l| l.key != "app");
            f.src.labels.push(crate::resolve::ResolvedLabel {
                href: format!("/orgs/5/labels/9{i}"),
                key: "app".into(),
                value: format!("app{i}"),
            });
            many.push(f);
        }
        let b = blocked_summary(&many, &k);
        let row = b
            .iter()
            .find(|r| r.dst_group == vec!["shop", "prod"])
            .unwrap();
        assert_eq!(row.dst_names.len(), 5);
        assert_eq!(row.dst_hosts, 8);
        assert_eq!(row.sources.len(), 5);
        assert_eq!(row.source_count, 10);
        let first = many
            .iter()
            .find(|f| row.contains(f, &k))
            .expect("a flow behind the row");
        assert_eq!(first.dst.display_name(), "shop-db-p.example");
        assert!(
            !row.contains(&many[0], &k),
            "an allowed flow is never behind a blocked row"
        );
    }

    #[test]
    fn empty_keys_fall_back_to_app_and_env() {
        assert_eq!(keys_or_default(&[]), keys(&["app", "env"]));
        assert_eq!(
            group_by_labels(&flows(), &[]),
            group_by_labels(&flows(), &keys(&["app", "env"]))
        );
        assert_eq!(
            blocked_summary(&flows(), &[]),
            blocked_summary(&flows(), &keys(&["app", "env"]))
        );
        assert_eq!(compact_count(999), "999");
        assert_eq!(compact_count(1_234), "1.2K");
        assert_eq!(compact_count(13_625_100), "13.6M");
        assert_eq!(dominant(0, 0, 0), PolicyDecision::Unknown);
        assert_eq!(dominant(3, 1, 0), PolicyDecision::PotentiallyBlocked);
    }
}
```

- [ ] **Step 3: Run to verify failure**

Run: `cargo test -p pce --lib traffic 2>&1 | tail -5`
Expected: FAIL to compile (`rich_flows`, `group_values`, `keys_seen`, `bytes`, `dst_group`, … do not exist; `blocked_summary` takes one argument).

- [ ] **Step 4: Implement**

In `crates/pce/src/traffic/aggregate.rs`, replace everything above `#[cfg(test)]` with:

```rust
//! Pure aggregations over resolved flows. The TUI's views and the CLI's
//! `--group` / `--blocked` / `groups` use these; no requests.
use super::resolved::{ResolvedEndpoint, ResolvedFlow};
use crate::models::PolicyDecision;
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};

/// The decision a group of flows is drawn in: blocked if any flow was
/// blocked, else potentially blocked if any was, else allowed if any was,
/// else unknown.
pub type Dominant = PolicyDecision;

pub fn dominant(allowed: u64, potentially_blocked: u64, blocked: u64) -> Dominant {
    if blocked > 0 {
        PolicyDecision::Blocked
    } else if potentially_blocked > 0 {
        PolicyDecision::PotentiallyBlocked
    } else if allowed > 0 {
        PolicyDecision::Allowed
    } else {
        PolicyDecision::Unknown
    }
}

/// `keys`, or `app,env` when empty.
pub fn keys_or_default(keys: &[String]) -> Vec<String> {
    if keys.is_empty() {
        vec!["app".into(), "env".into()]
    } else {
        keys.to_vec()
    }
}

/// `999`, `1.2K`, `3.4M`: at most six characters.
pub fn compact_count(n: u64) -> String {
    crate::iplist_size::human(u128::from(n))
}

/// The grouping values of an endpoint for `keys`: one entry per key, `-`
/// when missing or when the endpoint is not a workload.
pub fn group_values(ep: &ResolvedEndpoint, keys: &[String]) -> Vec<String> {
    if !ep.is_workload() {
        return vec!["-".to_string(); keys.len()];
    }
    keys.iter()
        .map(|k| ep.label_value(k).unwrap_or("-").to_string())
        .collect()
}

/// `shop/prod`; `-` for every slot of a non-workload.
pub fn group_text(ep: &ResolvedEndpoint, keys: &[String]) -> String {
    group_values(ep, keys).join("/")
}

/// The key an aggregation groups an endpoint under: its `group_values`
/// when it is a workload with labels; otherwise its name (or `ipl:<list>`)
/// in the first slot and `-` in the others, so IP lists, FQDNs and
/// unlabelled hosts stay apart and readable.
pub fn group_key(ep: &ResolvedEndpoint, keys: &[String]) -> Vec<String> {
    if ep.labels.is_empty() || !ep.is_workload() {
        let name = ep
            .ip_lists
            .first()
            .map(|l| format!("ipl:{l}"))
            .unwrap_or_else(|| ep.display_name());
        let mut k = vec![name];
        k.extend(std::iter::repeat_n(
            "-".to_string(),
            keys.len().saturating_sub(1),
        ));
        return k;
    }
    group_values(ep, keys)
}

/// Every label key seen in `flows`, with the number of flows carrying it
/// (on either side), sorted by the preferred order then name. Unresolved
/// labels (key `?`) are left out.
pub fn keys_seen(flows: &[ResolvedFlow], preferred: &[String]) -> Vec<(String, usize)> {
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    for f in flows {
        let keys: BTreeSet<&str> = f
            .src
            .labels
            .iter()
            .chain(&f.dst.labels)
            .map(|l| l.key.as_str())
            .filter(|k| *k != "?")
            .collect();
        for k in keys {
            *counts.entry(k.to_string()).or_default() += 1;
        }
    }
    let rank = |k: &str| {
        preferred
            .iter()
            .position(|p| p == k)
            .unwrap_or(preferred.len())
    };
    let mut out: Vec<(String, usize)> = counts.into_iter().collect();
    out.sort_by(|(a, _), (b, _)| rank(a).cmp(&rank(b)).then_with(|| a.cmp(b)));
    out
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FlowGroup {
    pub src: Vec<String>,
    pub dst: Vec<String>,
    pub flows: u64,
    pub connections: u64,
    /// Bytes in plus out over the group's flows; 0 when none reported any.
    pub bytes: u64,
    pub allowed: u64,
    pub potentially_blocked: u64,
    pub blocked: u64,
}

impl FlowGroup {
    pub fn dominant(&self) -> Dominant {
        dominant(self.allowed, self.potentially_blocked, self.blocked)
    }

    /// Whether `f` is one of this group's flows under `keys`.
    pub fn contains(&self, f: &ResolvedFlow, keys: &[String]) -> bool {
        let keys = keys_or_default(keys);
        group_key(&f.src, &keys) == self.src && group_key(&f.dst, &keys) == self.dst
    }
}

pub fn group_by_labels(flows: &[ResolvedFlow], keys: &[String]) -> Vec<FlowGroup> {
    let keys = keys_or_default(keys);
    let mut map: BTreeMap<(Vec<String>, Vec<String>), FlowGroup> = BTreeMap::new();
    for f in flows {
        let key = (group_key(&f.src, &keys), group_key(&f.dst, &keys));
        let g = map.entry(key.clone()).or_insert_with(|| FlowGroup {
            src: key.0.clone(),
            dst: key.1.clone(),
            flows: 0,
            connections: 0,
            bytes: 0,
            allowed: 0,
            potentially_blocked: 0,
            blocked: 0,
        });
        g.flows += 1;
        g.connections += f.num_connections;
        g.bytes += f.bytes();
        match f.policy_decision {
            PolicyDecision::Allowed => g.allowed += 1,
            PolicyDecision::PotentiallyBlocked => g.potentially_blocked += 1,
            PolicyDecision::Blocked => g.blocked += 1,
            PolicyDecision::Unknown => {}
        }
    }
    let mut out: Vec<FlowGroup> = map.into_values().collect();
    out.sort_by(|a, b| {
        b.connections
            .cmp(&a.connections)
            .then_with(|| a.src.cmp(&b.src))
    });
    out
}

/// At most this many names in `BlockedGroup::dst_names` and `sources`.
pub const BLOCKED_NAMES: usize = 5;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BlockedGroup {
    /// The first destination's display name (kept for existing consumers).
    pub dst: String,
    /// The first destination's labels, `app=shop env=prod`.
    pub dst_labels: String,
    /// The destination group the row stands for (`group_key` of the keys).
    pub dst_group: Vec<String>,
    /// Distinct destination display names, at most 5.
    pub dst_names: Vec<String>,
    /// Distinct destinations behind the row, uncapped.
    pub dst_hosts: u64,
    pub port: Option<u16>,
    pub proto: String,
    pub flows: u64,
    pub connections: u64,
    /// Distinct source group texts (`shop/prod`, `ipl:Corp/-`), at most 5.
    pub sources: Vec<String>,
    /// Distinct source groups behind the row, uncapped.
    pub source_count: u64,
    pub last_detected: Option<String>,
}

impl BlockedGroup {
    /// Whether `f` is one of this row's flows under `keys`.
    pub fn contains(&self, f: &ResolvedFlow, keys: &[String]) -> bool {
        is_blocked(f) && blocked_key(f, &keys_or_default(keys)) == self.key()
    }

    fn key(&self) -> BlockedKey {
        (self.dst_group.clone(), self.port, self.proto.clone())
    }
}

fn is_blocked(f: &ResolvedFlow) -> bool {
    matches!(
        f.policy_decision,
        PolicyDecision::Blocked | PolicyDecision::PotentiallyBlocked
    )
}

/// `(destination group, port, protocol)`: what a blocked row stands for.
type BlockedKey = (Vec<String>, Option<u16>, String);

fn blocked_key(f: &ResolvedFlow, keys: &[String]) -> BlockedKey {
    (group_key(&f.dst, keys), f.port, f.proto.clone())
}

/// Blocked and potentially blocked flows grouped by destination group,
/// port and protocol; newest first, then by destination group.
pub fn blocked_summary(flows: &[ResolvedFlow], keys: &[String]) -> Vec<BlockedGroup> {
    let keys = keys_or_default(keys);
    type Seen = (BTreeSet<String>, BTreeSet<String>);
    let mut map: BTreeMap<BlockedKey, (BlockedGroup, Seen)> = BTreeMap::new();
    for f in flows.iter().filter(|f| is_blocked(f)) {
        let key = blocked_key(f, &keys);
        let (g, (hosts, srcs)) = map.entry(key.clone()).or_insert_with(|| {
            (
                BlockedGroup {
                    dst: f.dst.display_name(),
                    dst_labels: f.dst.label_text(),
                    dst_group: key.0.clone(),
                    dst_names: Vec::new(),
                    dst_hosts: 0,
                    port: f.port,
                    proto: f.proto.clone(),
                    flows: 0,
                    connections: 0,
                    sources: Vec::new(),
                    source_count: 0,
                    last_detected: None,
                },
                (BTreeSet::new(), BTreeSet::new()),
            )
        });
        g.flows += 1;
        g.connections += f.num_connections;
        let name = f.dst.display_name();
        if !g.dst_names.contains(&name) && g.dst_names.len() < BLOCKED_NAMES {
            g.dst_names.push(name.clone());
        }
        hosts.insert(name);
        g.dst_hosts = hosts.len() as u64;
        let src = group_key(&f.src, &keys).join("/");
        if !g.sources.contains(&src) && g.sources.len() < BLOCKED_NAMES {
            g.sources.push(src.clone());
        }
        srcs.insert(src);
        g.source_count = srcs.len() as u64;
        if f.last_detected > g.last_detected {
            g.last_detected = f.last_detected.clone();
        }
    }
    let mut out: Vec<BlockedGroup> = map.into_values().map(|(g, _)| g).collect();
    out.sort_by(|a, b| {
        b.last_detected
            .cmp(&a.last_detected)
            .then_with(|| a.dst_group.cmp(&b.dst_group))
    });
    out
}
```

In `crates/pce/src/traffic/resolved.rs`:

replace

```rust
    pub last_detected: Option<String>,
    pub rules: Vec<String>,
}

impl ResolvedFlow {
```

with

```rust
    pub last_detected: Option<String>,
    pub rules: Vec<String>,
    /// Bytes the destination received (`dst_bi`), when the PCE reported it.
    pub bytes_in: Option<u64>,
    /// Bytes the destination sent (`dst_bo`), when the PCE reported it.
    pub bytes_out: Option<u64>,
}

impl ResolvedFlow {
    /// `bytes_in + bytes_out`, 0 when the PCE reported neither.
    pub fn bytes(&self) -> u64 {
        self.bytes_in.unwrap_or(0) + self.bytes_out.unwrap_or(0)
    }

    /// Whether the PCE reported a byte count for this flow.
    pub fn has_bytes(&self) -> bool {
        self.bytes_in.is_some() || self.bytes_out.is_some()
    }
```

replace

```rust
            rules: f.rules.iter().map(|r| r.href.clone()).collect(),
        })
```

with

```rust
            rules: f.rules.iter().map(|r| r.href.clone()).collect(),
            bytes_in: f.dst_bi,
            bytes_out: f.dst_bo,
        })
```

In `crates/illumio-tui/src/tui/test_support.rs`:

replace

```rust
/// A connected model with the fixture flows loaded as a finished query.
pub fn model_with_flows() -> Model {
```

with

```rust
/// The 30-flow fixture (4 apps x 2 envs, roles, `loc`, `datacenter` on
/// some workloads, the Corp IP list, an FQDN, users, processes, both
/// directions, bytes on 9 flows), resolved with its own extra labels.
pub fn fixture_flows_rich() -> Vec<ResolvedFlow> {
    let mut labels: Vec<Label> =
        serde_json::from_str(include_str!("../../../../fixtures/labels.json"))
            .expect("labels fixture");
    let rich: Vec<Label> =
        serde_json::from_str(include_str!("../../../../fixtures/traffic_labels_rich.json"))
            .expect("rich labels fixture");
    labels.extend(rich);
    let groups: Vec<LabelGroup> =
        serde_json::from_str(include_str!("../../../../fixtures/label_groups.json"))
            .expect("groups fixture");
    let lists: Vec<IpList> =
        serde_json::from_str(include_str!("../../../../fixtures/ip_lists.json"))
            .expect("ip lists fixture");
    let services: Vec<Service> =
        serde_json::from_str(include_str!("../../../../fixtures/services.json"))
            .expect("services fixture");
    let idx = NameIndex::new(&labels, &groups, &lists, &services);
    let raw: Vec<Flow> =
        serde_json::from_str(include_str!("../../../../fixtures/traffic_flows_rich.json"))
            .expect("rich flows fixture");
    resolve_flows(&raw, &idx)
}

/// A connected model with the rich fixture loaded as a finished query.
pub fn model_with_rich_flows() -> Model {
    let mut m = model_with_flows();
    let flows = fixture_flows_rich();
    if let Some(d) = m.traffic.data.as_mut() {
        d.matches_count = Some(flows.len() as u64);
        d.flows = flows;
    }
    m.traffic.rebuild_views();
    m
}

/// A connected model with the fixture flows loaded as a finished query.
pub fn model_with_flows() -> Model {
```

In `crates/pce/src/traffic/mod.rs`:

replace

```rust
pub use aggregate::{BlockedGroup, FlowGroup, blocked_summary, group_by_labels};
```

with

```rust
pub use aggregate::{
    BLOCKED_NAMES, BlockedGroup, Dominant, FlowGroup, blocked_summary, compact_count, dominant,
    group_by_labels, group_key, group_text, group_values, keys_or_default, keys_seen,
};
```

In `crates/illumio-tui/src/tui/model.rs`:

replace

```rust
        self.blocked.set_rows(blocked_summary(&flows));
```

with

```rust
        self.blocked
            .set_rows(blocked_summary(&flows, &self.group_keys));
```

In `crates/illumio-tui/src/cli/traffic.rs`:

replace

```rust
        ctx.emit(&blocked_summary(&flows), cache, result.truncated, window)
```

with

```rust
        ctx.emit(
            &blocked_summary(&flows, &ctx.config.scope_keys),
            cache,
            result.truncated,
            window,
        )
```

- [ ] **Step 5: Run the tests**

Run: `cargo fmt --all && cargo test -p pce --lib traffic`
Expected: PASS: `bytes_are_copied_from_dst_bi_and_dst_bo`, `groups_by_label_pair_with_fallbacks`, `group_values_dash_a_missing_key_and_every_slot_of_a_non_workload`, `keys_seen_puts_the_preferred_keys_first_and_counts_flows_once`, `rich_groups_sum_bytes_and_count_unknown_only_in_flows`, `blocked_summary_groups_by_destination_and_port`, `blocked_summary_groups_by_the_destination_group_and_caps_the_names`, `empty_keys_fall_back_to_app_and_env` and the existing ones.

- [ ] **Step 6: Re-record the one snapshot that changes**

```bash
INSTA_UPDATE=always cargo test -p illumio-tui --test tui_snapshots
find crates/illumio-tui/tests/snapshots -name '*.snap.new' -delete
git status --short crates/illumio-tui/tests/snapshots
git diff -U0 crates/illumio-tui/tests/snapshots | grep '^[-+][^-+]'
```

Expected: only `traffic_blocked_80x24.snap` modified, and only its two rows' `sources` cells: `10.1.4.9` becomes `ipl:Corp/-` and `unmanaged-nas` becomes `unmanaged-nas/-` (sources are source group texts now). The CLI tests stay green unchanged: `traffic query --blocked` keeps `dst` = `host-1.example`.

- [ ] **Step 7: Gate and commit**

```bash
cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace
git add fixtures/traffic_flows_rich.json fixtures/traffic_labels_rich.json crates/pce/src/traffic \
  crates/illumio-tui/src crates/illumio-tui/tests/snapshots/tui_snapshots__traffic_blocked_80x24.snap
git commit -m "feat(pce): group values, keys seen, bytes and key-aware blocked summary; rich traffic fixture

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

Expected: 735 tests pass.

---

### Task 3: `pce::traffic::mesh`

**Files:**
- Create: `crates/pce/src/traffic/mesh.rs`
- Modify: `crates/pce/src/traffic/mod.rs` (`pub mod mesh;` after `pub mod filter;`; re-export after `pub use filter::…`)

**Interfaces:**
- Consumes: `aggregate::{Dominant, dominant, group_key, keys_or_default}` (Task 2), `ResolvedFlow::{bytes, flow_direction, process_name, user_name, port, proto}`, test helpers `resolved::tests::{flows, rich_flows}`.
- Produces:

```rust
// pce::traffic::mesh (re-exported from pce::traffic)
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MeshRow {
    pub user: Option<String>, pub process: Option<String>, pub src: Vec<String>, pub dst: Vec<String>,
    pub port: Option<u16>, pub proto: String, pub dst_process: Option<String>,
    pub flows: u64, pub connections: u64, pub bytes: u64,
    pub allowed: u64, pub potentially_blocked: u64, pub blocked: u64,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct MeshColumns { pub user: bool, pub process: bool }
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct MeshKey { pub user: Option<String>, pub process: Option<String>, pub src: Vec<String>,
                     pub dst: Vec<String>, pub port: Option<u16>, pub proto: String,
                     pub dst_process: Option<String> }
impl MeshKey { pub fn service_text(&self) -> String; }
impl MeshRow {
    pub fn key(&self) -> MeshKey;
    pub fn service_text(&self) -> String;
    pub fn dominant(&self) -> Dominant;
    pub fn contains(&self, f: &ResolvedFlow, keys: &[String], cols: MeshColumns) -> bool;
}
pub fn process_sides(f: &ResolvedFlow) -> (Option<String>, Option<String>); // (source, destination)
pub fn mesh_key(f: &ResolvedFlow, keys: &[String], cols: MeshColumns) -> MeshKey;
pub fn mesh(flows: &[ResolvedFlow], keys: &[String], cols: MeshColumns) -> Vec<MeshRow>;
```

- [ ] **Step 1: Write the failing tests**

Create `crates/pce/src/traffic/mesh.rs` with only this module:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::traffic::resolved::tests::{flows, rich_flows};

    fn keys(k: &[&str]) -> Vec<String> {
        k.iter().map(|s| s.to_string()).collect()
    }

    const BOTH: MeshColumns = MeshColumns {
        user: true,
        process: true,
    };

    #[test]
    fn the_direction_places_the_process_on_its_side() {
        let rows = mesh(&flows(), &keys(&["app", "env"]), BOTH);
        assert_eq!(rows.len(), 4);
        let out = &rows[0]; // shop/prod -> shop/prod 5432/tcp, outbound postgres
        assert_eq!(out.connections, 310);
        assert_eq!(out.user.as_deref(), Some("postgres"));
        assert_eq!(out.process.as_deref(), Some("postgres"));
        assert_eq!(out.dst_process, None);
        assert_eq!(out.service_text(), "5432/tcp");
        assert_eq!(out.bytes, 124_000);
        let rich = mesh(&rich_flows(), &keys(&["app", "env"]), BOTH);
        let inbound = rich
            .iter()
            .find(|r| r.dst_process.as_deref() == Some("nginx"))
            .expect("the inbound Corp flow");
        assert_eq!(inbound.process, None, "seen on the destination");
        assert_eq!(inbound.src, vec!["ipl:Corp", "-"]);
        assert_eq!(inbound.user.as_deref(), Some("www-data"));
        let icmp = rich.iter().find(|r| r.proto == "icmp").unwrap();
        assert_eq!(
            (icmp.process.as_deref(), icmp.dst_process.as_deref()),
            (None, None),
            "no direction and no process"
        );
        assert_eq!(icmp.service_text(), "icmp");
    }

    #[test]
    fn hiding_the_user_or_the_process_merges_rows() {
        let f = rich_flows();
        let k = keys(&["app", "env"]);
        let none = MeshColumns::default();
        let no_user = MeshColumns {
            user: false,
            process: true,
        };
        let no_process = MeshColumns {
            user: true,
            process: false,
        };
        assert_eq!(mesh(&f, &k, BOTH).len(), 30);
        assert_eq!(mesh(&f, &k, no_user).len(), 29, "java as app and as batch");
        assert_eq!(
            mesh(&f, &k, no_process).len(),
            29,
            "java and python3 as app"
        );
        let merged = mesh(&f, &k, none);
        assert_eq!(merged.len(), 28);
        let shop = merged
            .iter()
            .find(|r| {
                r.src == vec!["shop", "prod"] && r.port == Some(5432) && r.dst_process.is_none()
            })
            .unwrap();
        assert_eq!((shop.flows, shop.connections), (3, 401));
        assert_eq!(
            (shop.user.as_deref(), shop.process.as_deref()),
            (None, None)
        );
        assert!(
            merged
                .iter()
                .all(|r| r.user.is_none() && r.process.is_none())
        );
        assert!(
            merged.iter().any(|r| r.dst_process.is_some()),
            "the destination process stays"
        );
    }

    #[test]
    fn rows_sort_by_connections_then_src_dst_and_service() {
        let rows = mesh(&rich_flows(), &keys(&["app", "env"]), BOTH);
        let top: Vec<u64> = rows.iter().take(4).map(|r| r.connections).collect();
        assert_eq!(top, vec![700, 610, 520, 420]);
        assert!(
            rows.windows(2)
                .all(|w| w[0].connections >= w[1].connections)
        );
        assert_eq!(
            rows.iter().map(|r| r.connections).sum::<u64>(),
            4_109,
            "every flow in exactly one row"
        );
        let unknown = rows.iter().find(|r| r.proto == "udp").unwrap();
        assert_eq!(
            (unknown.flows, unknown.allowed, unknown.dominant()),
            (1, 0, PolicyDecision::Unknown)
        );
        let by_role = mesh(&rich_flows(), &keys(&["role"]), BOTH);
        assert_eq!(by_role[0].src, vec!["ipl:Corp"]);
        assert_eq!(by_role[0].dst, vec!["web"]);
        assert_eq!(
            mesh(&flows(), &[], BOTH),
            mesh(&flows(), &keys(&["app", "env"]), BOTH)
        );
    }

    #[test]
    fn a_row_finds_its_flows_again() {
        let f = rich_flows();
        let k = keys(&["app", "env"]);
        let rows = mesh(&f, &k, MeshColumns::default());
        let shop = rows
            .iter()
            .find(|r| {
                r.src == vec!["shop", "prod"] && r.port == Some(5432) && r.dst_process.is_none()
            })
            .unwrap();
        let behind: Vec<usize> = f
            .iter()
            .enumerate()
            .filter(|(_, fl)| shop.contains(fl, &k, MeshColumns::default()))
            .map(|(i, _)| i)
            .collect();
        assert_eq!(behind, vec![1, 26, 29]);
        assert_eq!(shop.key(), mesh_key(&f[1], &k, MeshColumns::default()));
        assert_eq!(shop.key().service_text(), "5432/tcp");
        assert!(!shop.contains(&f[1], &k, BOTH), "other columns, other key");
    }
}
```

and in `crates/pce/src/traffic/mod.rs` replace `pub mod filter;` with

```rust
pub mod filter;
pub mod mesh;
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p pce --lib traffic::mesh 2>&1 | tail -5`
Expected: FAIL to compile (`mesh`, `MeshColumns`, `mesh_key` not found).

- [ ] **Step 3: Implement**

Put this above the tests module in `crates/pce/src/traffic/mesh.rs`:

```rust
//! The mesh: flows grouped by who talked to whom on which service, with
//! the user and the processes on each side, like the Illumio UI's mesh.
//! Pure; no requests.
use super::aggregate::{Dominant, dominant, group_key, keys_or_default};
use super::resolved::ResolvedFlow;
use crate::models::PolicyDecision;
use serde::Serialize;
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MeshRow {
    /// None when the column is hidden or the flow carried no user.
    pub user: Option<String>,
    /// The source's process (outbound flows), None when hidden or absent.
    pub process: Option<String>,
    /// Group values (`group_key`) of the source and the destination.
    pub src: Vec<String>,
    pub dst: Vec<String>,
    pub port: Option<u16>,
    pub proto: String,
    /// The destination's process (inbound flows).
    pub dst_process: Option<String>,
    pub flows: u64,
    pub connections: u64,
    pub bytes: u64,
    pub allowed: u64,
    pub potentially_blocked: u64,
    pub blocked: u64,
}

/// Which optional columns take part in the grouping.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct MeshColumns {
    pub user: bool,
    pub process: bool,
}

/// What a mesh row groups on; `MeshRow::key` gives it back, so a row can
/// find its flows again (the TUI's drill-down and `w`).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct MeshKey {
    pub user: Option<String>,
    pub process: Option<String>,
    pub src: Vec<String>,
    pub dst: Vec<String>,
    pub port: Option<u16>,
    pub proto: String,
    pub dst_process: Option<String>,
}

impl MeshKey {
    /// `5432/tcp`, or just `icmp` when there is no port.
    pub fn service_text(&self) -> String {
        service_text(self.port, &self.proto)
    }
}

fn service_text(port: Option<u16>, proto: &str) -> String {
    match port {
        Some(p) => format!("{p}/{proto}"),
        None => proto.to_string(),
    }
}

/// `(source process, destination process)`: the PCE reports one
/// `process_name`, observed on the reporting VEN. Inbound flows were seen on
/// the destination, outbound ones (and flows without a direction) on the
/// source.
pub fn process_sides(f: &ResolvedFlow) -> (Option<String>, Option<String>) {
    if f.flow_direction.as_deref() == Some("inbound") {
        (None, f.process_name.clone())
    } else {
        (f.process_name.clone(), None)
    }
}

/// The mesh key of one flow under `keys` (empty means `app,env`) and the
/// shown columns.
pub fn mesh_key(f: &ResolvedFlow, keys: &[String], cols: MeshColumns) -> MeshKey {
    let keys = keys_or_default(keys);
    let (process, dst_process) = process_sides(f);
    MeshKey {
        user: f.user_name.clone().filter(|_| cols.user),
        process: process.filter(|_| cols.process),
        src: group_key(&f.src, &keys),
        dst: group_key(&f.dst, &keys),
        port: f.port,
        proto: f.proto.clone(),
        dst_process,
    }
}

impl MeshRow {
    pub fn key(&self) -> MeshKey {
        MeshKey {
            user: self.user.clone(),
            process: self.process.clone(),
            src: self.src.clone(),
            dst: self.dst.clone(),
            port: self.port,
            proto: self.proto.clone(),
            dst_process: self.dst_process.clone(),
        }
    }

    pub fn service_text(&self) -> String {
        service_text(self.port, &self.proto)
    }

    pub fn dominant(&self) -> Dominant {
        dominant(self.allowed, self.potentially_blocked, self.blocked)
    }

    /// Whether `f` is one of this row's flows under `keys` and `cols`.
    pub fn contains(&self, f: &ResolvedFlow, keys: &[String], cols: MeshColumns) -> bool {
        mesh_key(f, keys, cols) == self.key()
    }
}

/// Rows grouped on `(user?, process?, src, dst, port, proto, dst_process)`,
/// user and process only when their column is shown. Sorted by connections
/// descending, then src, dst, service. Unknown decisions count in flows and
/// connections only.
pub fn mesh(flows: &[ResolvedFlow], keys: &[String], cols: MeshColumns) -> Vec<MeshRow> {
    let keys = keys_or_default(keys);
    let mut map: BTreeMap<MeshKey, MeshRow> = BTreeMap::new();
    for f in flows {
        let k = mesh_key(f, &keys, cols);
        let row = map.entry(k.clone()).or_insert_with(|| MeshRow {
            user: k.user,
            process: k.process,
            src: k.src,
            dst: k.dst,
            port: k.port,
            proto: k.proto,
            dst_process: k.dst_process,
            flows: 0,
            connections: 0,
            bytes: 0,
            allowed: 0,
            potentially_blocked: 0,
            blocked: 0,
        });
        row.flows += 1;
        row.connections += f.num_connections;
        row.bytes += f.bytes();
        match f.policy_decision {
            PolicyDecision::Allowed => row.allowed += 1,
            PolicyDecision::PotentiallyBlocked => row.potentially_blocked += 1,
            PolicyDecision::Blocked => row.blocked += 1,
            PolicyDecision::Unknown => {}
        }
    }
    let mut out: Vec<MeshRow> = map.into_values().collect();
    out.sort_by(|a, b| {
        b.connections
            .cmp(&a.connections)
            .then_with(|| a.src.cmp(&b.src))
            .then_with(|| a.dst.cmp(&b.dst))
            .then_with(|| a.port.cmp(&b.port))
            .then_with(|| a.proto.cmp(&b.proto))
    });
    out
}
```

In `crates/pce/src/traffic/mod.rs`, replace `pub use filter::{FilterExpr, resolve_actor};` with

```rust
pub use filter::{FilterExpr, resolve_actor};
pub use mesh::{MeshColumns, MeshKey, MeshRow, mesh, mesh_key, process_sides};
```

- [ ] **Step 4: Run the tests**

Run: `cargo fmt --all && cargo test -p pce --lib traffic::mesh`
Expected: PASS: `the_direction_places_the_process_on_its_side`, `hiding_the_user_or_the_process_merges_rows`, `rows_sort_by_connections_then_src_dst_and_service`, `a_row_finds_its_flows_again`.

- [ ] **Step 5: Gate and commit**

```bash
cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace
git add crates/pce/src/traffic/mesh.rs crates/pce/src/traffic/mod.rs
git commit -m "feat(pce): traffic mesh rows with user, processes by direction, service and decisions

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

Expected: 739 tests pass.

---

### Task 4: `pce::traffic::treemap`

**Files:**
- Create: `crates/pce/src/traffic/treemap.rs`
- Modify: `crates/pce/src/traffic/mod.rs` (`pub mod treemap;` after `pub mod resolved;`; the module is used by path, `pce::traffic::treemap::…`, so there is no re-export)

**Interfaces:**
- Consumes: `aggregate::{Dominant, FlowGroup, dominant, group_by_labels}` (Task 2), `FlowGroup::dominant`, `PolicyDecision::Unknown`, test helper `resolved::tests::rich_flows`.
- Produces:

```rust
// pce::traffic::treemap
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Rect { pub x: u16, pub y: u16, pub w: u16, pub h: u16 }
impl Rect {
    pub fn area(self) -> u32;
    pub fn is_empty(self) -> bool;
    pub fn inner(self) -> Rect;              // minus a one-cell border; empty below 3x3
    pub fn intersects(self, o: Rect) -> bool;
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Weight { #[default] Connections, Bytes }
impl Weight { pub fn of(self, g: &FlowGroup) -> u64; }
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tile { pub label: String, pub weight: u64, pub rect: Rect, pub decision: Dominant, pub children: Vec<Tile> }
impl Tile { pub fn is_other(&self) -> bool; }   // label starts with `other (`
pub fn layout(groups: &[FlowGroup], area: Rect, weight: Weight) -> Vec<Tile>;
pub fn layout_zoomed(groups: &[FlowGroup], src: &[String], area: Rect, weight: Weight) -> Vec<Tile>;
```

The algorithm, so a reviewer can check the code against it: items with zero weight are dropped; the rest sort by weight descending, then label (a stable sort, so equal keys keep their order). `squarify` works in visual units (x in cells, y in half cells, so a terminal cell of about 1:2 counts as square): rows go along the shorter side of what is left, an item joins the current row while the worst aspect ratio does not get worse, and the last row takes exactly what is left. Edges are rounded to cells with `floor(v + 0.5)`, every edge the same way wherever it appears, so tiles neither overlap nor leave gaps. If a named item gets no cell, it and every item after it fold into `other (N)` (last); if only `other` gets no cell, the smallest named item joins it; with nothing named left, `other` fills the area.

- [ ] **Step 1: Write the failing tests**

Create `crates/pce/src/traffic/treemap.rs` with only this module:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::traffic::aggregate::group_by_labels;
    use crate::traffic::resolved::tests::rich_flows;

    fn rect(x: u16, y: u16, w: u16, h: u16) -> Rect {
        Rect { x, y, w, h }
    }

    fn group(src: &str, dst: &str, connections: u64) -> FlowGroup {
        FlowGroup {
            src: vec![src.into()],
            dst: vec![dst.into()],
            flows: 1,
            connections,
            bytes: 0,
            allowed: 1,
            potentially_blocked: 0,
            blocked: 0,
        }
    }

    fn rich() -> Vec<FlowGroup> {
        group_by_labels(&rich_flows(), &["app".to_string(), "env".to_string()])
    }

    fn assert_partition(tiles: &[Tile], area: Rect) {
        let covered: u32 = tiles.iter().map(|t| t.rect.area()).sum();
        assert_eq!(covered, area.area(), "the tiles fill the area exactly");
        for (i, a) in tiles.iter().enumerate() {
            assert!(!a.rect.is_empty(), "{a:?}");
            assert!(
                a.rect.x >= area.x
                    && a.rect.y >= area.y
                    && a.rect.x + a.rect.w <= area.x + area.w
                    && a.rect.y + a.rect.h <= area.y + area.h,
                "{a:?} inside {area:?}"
            );
            for b in &tiles[i + 1..] {
                assert!(!a.rect.intersects(b.rect), "{a:?} overlaps {b:?}");
            }
        }
    }

    #[test]
    fn tiles_fill_the_area_without_overlap_at_both_snapshot_sizes() {
        for area in [rect(1, 9, 78, 13), rect(1, 9, 158, 37)] {
            let tiles = layout(&rich(), area, Weight::Connections);
            assert_partition(&tiles, area);
            assert_eq!(tiles[0].label, "ipl:Corp/-", "heaviest first");
            assert_eq!(tiles[0].weight, 1_301);
            assert_eq!(tiles[0].decision, PolicyDecision::Blocked);
            let total: u64 = tiles.iter().map(|t| t.weight).sum();
            assert_eq!(total, 4_109, "folding keeps every connection");
            for t in tiles.iter().filter(|t| !t.children.is_empty()) {
                assert_partition(&t.children, t.rect.inner());
                assert_eq!(t.children.iter().map(|c| c.weight).sum::<u64>(), t.weight);
            }
        }
        let wide = layout(&rich(), rect(1, 9, 158, 37), Weight::Connections);
        assert_eq!(wide.len(), 9, "every source gets a tile at 160x48");
        let shop = wide.iter().find(|t| t.label == "shop/prod").unwrap();
        let dsts: Vec<&str> = shop.children.iter().map(|c| c.label.as_str()).collect();
        assert_eq!(dsts, vec!["shop/prod", "crm/prod", "api.partner.example/-"]);
    }

    #[test]
    fn simple_weights_split_the_area_proportionally() {
        let two = [group("a", "x", 1), group("b", "y", 1)];
        let tiles = layout(&two, rect(0, 0, 10, 4), Weight::Connections);
        let rects: Vec<Rect> = tiles.iter().map(|t| t.rect).collect();
        assert_eq!(rects, vec![rect(0, 0, 5, 4), rect(5, 0, 5, 4)]);
        assert_eq!(
            tiles[0].children[0].rect,
            rect(1, 1, 3, 2),
            "inside the border"
        );
        let three_to_one = [group("b", "y", 1), group("a", "x", 3)];
        let tiles = layout(&three_to_one, rect(0, 0, 8, 2), Weight::Connections);
        let got: Vec<(&str, Rect)> = tiles.iter().map(|t| (t.label.as_str(), t.rect)).collect();
        assert_eq!(got, vec![("a", rect(0, 0, 6, 2)), ("b", rect(6, 0, 2, 2))]);
        assert!(
            tiles.iter().all(|t| t.children.is_empty()),
            "2 rows: no room for a border"
        );
        let one = layout(&[group("a", "x", 5)], rect(2, 3, 5, 4), Weight::Connections);
        assert_eq!(one[0].rect, rect(2, 3, 5, 4));
        assert_eq!(one[0].children[0].rect, rect(3, 4, 3, 2));
    }

    #[test]
    fn small_tiles_fold_into_other_and_a_single_cell_still_draws() {
        let g = rich();
        let tiny = layout(&g, rect(0, 0, 1, 1), Weight::Connections);
        assert_eq!(tiny.len(), 1);
        assert_eq!(tiny[0].label, "other (9)");
        assert_eq!(tiny[0].rect, rect(0, 0, 1, 1));
        assert_eq!(tiny[0].weight, 4_109);
        assert_eq!(tiny[0].decision, PolicyDecision::Unknown);
        assert!(tiny[0].is_other());
        let row = layout(&g, rect(0, 0, 3, 1), Weight::Connections);
        let got: Vec<(&str, Rect)> = row.iter().map(|t| (t.label.as_str(), t.rect)).collect();
        assert_eq!(
            got,
            vec![
                ("ipl:Corp/-", rect(0, 0, 1, 1)),
                ("other (8)", rect(1, 0, 2, 1))
            ]
        );
        let small = layout(&g, rect(0, 0, 10, 2), Weight::Connections);
        assert_partition(&small, rect(0, 0, 10, 2));
        assert_eq!(small.last().unwrap().label, "other (4)", "other comes last");
        assert!(!small[..small.len() - 1].iter().any(Tile::is_other));
    }

    #[test]
    fn zero_weight_and_empty_areas_give_no_tiles() {
        let g = rich();
        assert!(layout(&g, rect(0, 0, 0, 10), Weight::Connections).is_empty());
        assert!(layout(&g, rect(0, 0, 10, 0), Weight::Connections).is_empty());
        assert!(layout(&[], rect(0, 0, 10, 10), Weight::Connections).is_empty());
        assert!(
            layout(
                &[group("a", "x", 0)],
                rect(0, 0, 10, 10),
                Weight::Connections
            )
            .is_empty()
        );
        let by_bytes = layout(&g, rect(0, 0, 78, 13), Weight::Bytes);
        let labels: Vec<&str> = by_bytes.iter().map(|t| t.label.as_str()).collect();
        assert_eq!(
            labels,
            vec![
                "ipl:Corp/-",
                "crm/prod",
                "erp/prod",
                "shop/prod",
                "other (2)"
            ],
            "sources without bytes drop out; shop/dev and hr/prod are too small"
        );
        assert_partition(&by_bytes, rect(0, 0, 78, 13));
    }

    #[test]
    fn the_layout_is_deterministic_and_ignores_input_order() {
        let g = rich();
        let mut reversed = g.clone();
        reversed.reverse();
        let area = rect(1, 9, 78, 13);
        assert_eq!(
            layout(&g, area, Weight::Connections),
            layout(&g, area, Weight::Connections)
        );
        assert_eq!(
            layout(&g, area, Weight::Connections),
            layout(&reversed, area, Weight::Connections)
        );
        let ties = [group("b", "y", 2), group("a", "x", 2)];
        let tiles = layout(&ties, rect(0, 0, 10, 4), Weight::Connections);
        assert_eq!(tiles[0].label, "a", "equal weights sort by label");
    }

    #[test]
    fn zooming_lays_out_one_sources_destinations() {
        let g = rich();
        let area = rect(1, 9, 78, 13);
        let src = vec!["shop".to_string(), "prod".to_string()];
        let tiles = layout_zoomed(&g, &src, area, Weight::Connections);
        let got: Vec<(&str, u64)> = tiles.iter().map(|t| (t.label.as_str(), t.weight)).collect();
        assert_eq!(
            got,
            vec![
                ("shop/prod", 821),
                ("crm/prod", 150),
                ("api.partner.example/-", 33)
            ]
        );
        assert!(tiles.iter().all(|t| t.children.is_empty()));
        assert_partition(&tiles, area);
        assert!(layout_zoomed(&g, &["nope".to_string()], area, Weight::Connections).is_empty());
    }
}
```

and in `crates/pce/src/traffic/mod.rs` replace `pub mod resolved;` with

```rust
pub mod resolved;
pub mod treemap;
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p pce --lib traffic::treemap 2>&1 | tail -5`
Expected: FAIL to compile (`layout`, `Rect`, `Weight`, `Tile` not found).

- [ ] **Step 3: Implement**

Put this above the tests module in `crates/pce/src/traffic/treemap.rs`:

```rust
//! A squarified treemap of label-pair groups, in terminal cells: outer
//! tiles per source group, inner tiles per destination. Pure; no requests.
use super::aggregate::{Dominant, FlowGroup, dominant};
use crate::models::PolicyDecision;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Rect {
    pub x: u16,
    pub y: u16,
    pub w: u16,
    pub h: u16,
}

impl Rect {
    pub fn area(self) -> u32 {
        u32::from(self.w) * u32::from(self.h)
    }

    pub fn is_empty(self) -> bool {
        self.w == 0 || self.h == 0
    }

    /// `self` minus a one-cell border; empty when that leaves nothing.
    pub fn inner(self) -> Rect {
        if self.w < 3 || self.h < 3 {
            return Rect::default();
        }
        Rect {
            x: self.x + 1,
            y: self.y + 1,
            w: self.w - 2,
            h: self.h - 2,
        }
    }

    pub fn intersects(self, o: Rect) -> bool {
        self.x < o.x + o.w && o.x < self.x + self.w && self.y < o.y + o.h && o.y < self.y + self.h
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Weight {
    #[default]
    Connections,
    Bytes,
}

impl Weight {
    pub fn of(self, g: &FlowGroup) -> u64 {
        match self {
            Weight::Connections => g.connections,
            Weight::Bytes => g.bytes,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tile {
    /// Group text, `shop/prod`, or `other (N)` for N folded tiles.
    pub label: String,
    /// Connections or bytes.
    pub weight: u64,
    /// x, y, w, h in cells, in the coordinates of the `area` passed in.
    pub rect: Rect,
    pub decision: Dominant,
    /// Destinations, laid out inside `rect.inner()` (none when too small).
    pub children: Vec<Tile>,
}

impl Tile {
    /// Whether this tile stands for several folded ones.
    pub fn is_other(&self) -> bool {
        self.label.starts_with("other (")
    }
}

/// Squarified treemap of `groups` (from `group_by_labels`) into `area`:
/// outer tiles per src, inner per dst. Tiles narrower than 1 cell are
/// dropped and their weight is folded into an `other (N)` tile. Zero total
/// weight gives no tiles.
pub fn layout(groups: &[FlowGroup], area: Rect, weight: Weight) -> Vec<Tile> {
    let mut sources: Vec<(String, Vec<&FlowGroup>)> = Vec::new();
    for g in groups {
        let label = g.src.join("/");
        match sources.iter_mut().find(|(l, _)| *l == label) {
            Some((_, v)) => v.push(g),
            None => sources.push((label, vec![g])),
        }
    }
    let items: Vec<Item> = sources
        .into_iter()
        .map(|(label, gs)| {
            let (a, p, b) = gs.iter().fold((0, 0, 0), |(a, p, b), g| {
                (a + g.allowed, p + g.potentially_blocked, b + g.blocked)
            });
            Item {
                label,
                weight: gs.iter().map(|g| weight.of(g)).sum(),
                decision: dominant(a, p, b),
                children: gs.iter().map(|g| dst_item(g, weight)).collect(),
            }
        })
        .collect();
    place(items, area)
        .into_iter()
        .map(|(item, rect)| {
            let inner = rect.inner();
            let children = if item.children.is_empty() || inner.is_empty() {
                Vec::new()
            } else {
                place(item.children, inner)
                    .into_iter()
                    .map(|(c, r)| leaf(c, r))
                    .collect()
            };
            Tile {
                label: item.label,
                weight: item.weight,
                rect,
                decision: item.decision,
                children,
            }
        })
        .collect()
}

/// The destinations of the source group `src` as top-level tiles (no
/// children): the treemap zoomed in on one source.
pub fn layout_zoomed(
    groups: &[FlowGroup],
    src: &[String],
    area: Rect,
    weight: Weight,
) -> Vec<Tile> {
    let items: Vec<Item> = groups
        .iter()
        .filter(|g| g.src == src)
        .map(|g| dst_item(g, weight))
        .collect();
    place(items, area)
        .into_iter()
        .map(|(c, r)| leaf(c, r))
        .collect()
}

struct Item {
    label: String,
    weight: u64,
    decision: Dominant,
    children: Vec<Item>,
}

fn dst_item(g: &FlowGroup, weight: Weight) -> Item {
    Item {
        label: g.dst.join("/"),
        weight: weight.of(g),
        decision: g.dominant(),
        children: Vec::new(),
    }
}

fn leaf(item: Item, rect: Rect) -> Tile {
    Tile {
        label: item.label,
        weight: item.weight,
        rect,
        decision: item.decision,
        children: Vec::new(),
    }
}

/// Lays `items` out in `area`: zero weights dropped, heaviest first (then
/// by label; equal keys keep their order), and the tail that does not get
/// a whole cell folded into `other (N)`, last. When `other` itself gets no
/// cell, the smallest named item joins it, until everything fits (in the
/// end `other` alone fills the area).
fn place(items: Vec<Item>, area: Rect) -> Vec<(Item, Rect)> {
    if area.is_empty() {
        return Vec::new();
    }
    let mut items: Vec<Item> = items.into_iter().filter(|i| i.weight > 0).collect();
    if items.is_empty() {
        return Vec::new();
    }
    items.sort_by(|a, b| b.weight.cmp(&a.weight).then_with(|| a.label.cmp(&b.label)));
    let n = items.len();
    let mut kept = n;
    let rects = loop {
        let mut weights: Vec<u64> = items[..kept].iter().map(|i| i.weight).collect();
        if kept < n {
            weights.push(items[kept..].iter().map(|i| i.weight).sum());
        }
        if kept == 0 {
            break vec![area];
        }
        let rects: Vec<Rect> = squarify(&weights, f64::from(area.w), 2.0 * f64::from(area.h))
            .into_iter()
            .map(|e| cells(e, area))
            .collect();
        match rects.iter().position(|r| r.is_empty()) {
            None => break rects,
            Some(i) if i < kept => kept = i,
            Some(_) => kept -= 1,
        }
    };
    let rest: Vec<Item> = items.split_off(kept);
    let mut out: Vec<(Item, Rect)> = items.into_iter().zip(rects.iter().copied()).collect();
    if !rest.is_empty() {
        let other = Item {
            label: format!("other ({})", rest.len()),
            weight: rest.iter().map(|i| i.weight).sum(),
            decision: PolicyDecision::Unknown,
            children: Vec::new(),
        };
        out.push((other, rects[kept]));
    }
    out
}

/// `[x0, y0, x1, y1]` in visual units: x in cells, y in half cells, so a
/// cell (about 1:2) counts as square.
type Edges = [f64; 4];

/// Bruls, Huizing, van Wijk: rows along the shorter side of what is left;
/// a tile joins the current row while the worst aspect ratio does not get
/// worse. `weights` are positive and sorted descending; the last row takes
/// exactly what is left, and neighbouring tiles share their edge values.
fn squarify(weights: &[u64], w: f64, h: f64) -> Vec<Edges> {
    let total: u64 = weights.iter().sum();
    let scale = w * h / total as f64;
    let areas: Vec<f64> = weights.iter().map(|&x| x as f64 * scale).collect();
    let mut out = Vec::with_capacity(areas.len());
    let (mut rx0, mut ry0, rx1, ry1) = (0.0f64, 0.0f64, w, h);
    let mut i = 0;
    while i < areas.len() {
        let (rw, rh) = (rx1 - rx0, ry1 - ry0);
        let short = rw.min(rh);
        let mut j = i + 1;
        let mut sum = areas[i];
        while j < areas.len() {
            let next = sum + areas[j];
            if worst(&areas[i..=j], next, short) <= worst(&areas[i..j], sum, short) {
                sum = next;
                j += 1;
            } else {
                break;
            }
        }
        let last = j == areas.len();
        if rw >= rh {
            // A column on the left, tiles stacked top to bottom.
            let x_end = if last { rx1 } else { rx0 + sum / rh };
            let mut y = ry0;
            for (k, &a) in areas.iter().enumerate().take(j).skip(i) {
                let y_end = if k == j - 1 { ry1 } else { y + a / sum * rh };
                out.push([rx0, y, x_end, y_end]);
                y = y_end;
            }
            rx0 = x_end;
        } else {
            // A row on top, tiles left to right.
            let y_end = if last { ry1 } else { ry0 + sum / rw };
            let mut x = rx0;
            for (k, &a) in areas.iter().enumerate().take(j).skip(i) {
                let x_end = if k == j - 1 { rx1 } else { x + a / sum * rw };
                out.push([x, ry0, x_end, y_end]);
                x = x_end;
            }
            ry0 = y_end;
        }
        i = j;
    }
    out
}

fn worst(row: &[f64], sum: f64, short: f64) -> f64 {
    row.iter()
        .map(|&a| (short * short * a / (sum * sum)).max(sum * sum / (short * short * a)))
        .fold(0.0, f64::max)
}

/// Visual edges to cells: every edge rounds the same way wherever it
/// appears, so tiles neither overlap nor leave gaps.
fn cells(e: Edges, area: Rect) -> Rect {
    let round = |v: f64| (v + 0.5).floor() as u16;
    let (x0, x1) = (round(e[0]), round(e[2]));
    let (y0, y1) = (round(e[1] / 2.0), round(e[3] / 2.0));
    Rect {
        x: area.x + x0,
        y: area.y + y0,
        w: x1 - x0,
        h: y1 - y0,
    }
}
```

- [ ] **Step 4: Run the tests**

Run: `cargo fmt --all && cargo test -p pce --lib traffic::treemap`
Expected: PASS: `tiles_fill_the_area_without_overlap_at_both_snapshot_sizes`, `simple_weights_split_the_area_proportionally`, `small_tiles_fold_into_other_and_a_single_cell_still_draws`, `zero_weight_and_empty_areas_give_no_tiles`, `the_layout_is_deterministic_and_ignores_input_order`, `zooming_lays_out_one_sources_destinations`. The two snapshot areas, `(1, 9, 78, 13)` and `(1, 9, 158, 37)`, are exactly where Task 7 draws the treemap at 80x24 and 160x48.

- [ ] **Step 5: Gate and commit**

```bash
cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace
git add crates/pce/src/traffic/treemap.rs crates/pce/src/traffic/mod.rs
git commit -m "feat(pce): squarified treemap of label pairs in terminal cells with other folding

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

Expected: 745 tests pass. Clippy wants iterator loops in `squarify` (`for (k, &a) in areas.iter().enumerate().take(j).skip(i)`); the code above already uses them.

---

### Task 5: TUI display keys and the `g` picker

**Files:**
- Create: `crates/illumio-tui/src/tui/picker.rs`
- Modify: `crates/illumio-tui/src/tui/mod.rs` (`pub mod picker;` after `pub mod msg;`)
- Modify: `crates/illumio-tui/src/tui/model.rs` (imports ~lines 5-16; `Overlay::GroupKeys` ~178 and its `PartialEq` arm ~222; `flow_columns` ~320; `blocked_columns` ~522-610; `TrafficState` ~692-760; `Model::new_connecting` ~2133, `reset_data` ~2248, new `set_label_model` before `flow_prefill` ~2193; tests)
- Modify: `crates/illumio-tui/src/tui/update.rs` (imports ~lines 6-9; `Overlay::GroupKeys` arm ~793; `traffic_key` `g` and `Enter` arms ~1258-1272; new `open_key_picker`, `apply_group_keys` before `request_query`; `flow_rule_key` ~2250; tests)
- Modify: `crates/illumio-tui/src/tui/app.rs` (`run`, the two `model.limits` lines ~769)
- Modify: `crates/illumio-tui/src/tui/ui/mod.rs` (overlay dispatch ~line 44)
- Modify: `crates/illumio-tui/src/tui/ui/widgets/overlay.rs` (imports; `render_group_prompt` ~148 replaced by `render_key_picker`; `HELP_LINES` `Traffic  g`; `screen_hint` Traffic arms)
- Modify: `crates/illumio-tui/src/tui/ui/screens/traffic.rs` (imports; `layout` ~22; `render_flows` title ~148; `render_groups` ~169; `render_blocked` ~192; `endpoint_tags` ~203 replaced; `render_detail` lines ~240-280)
- Test: `crates/illumio-tui/tests/tui_snapshots.rs` (append)

**Interfaces:**
- Consumes: Task 2's `group_key`, `group_text`, `keys_or_default`, `keys_seen`, `blocked_summary(flows, keys)`, `BlockedGroup::{contains, dst_group, dst_names, dst_hosts, sources, source_count}`, `FlowGroup::dominant`, `ResolvedFlow::{has_bytes, bytes_in, bytes_out}`, `compact_count`; `test_support::{model_with_rich_flows, fixture_flows_rich}`; `Limits.{scope_keys, tier_key}`; `visible_columns` (`ui/widgets/table.rs`); `begin_edit`, `writes_refused`, `EditIntent::RuleFromFlow`.
- Produces:

```rust
// tui/picker.rs
#[derive(Debug, Clone, PartialEq, Eq)] pub struct PickerEntry { pub key: String, pub seen: Option<usize> }
#[derive(Debug, Clone, PartialEq, Eq)] pub struct KeyPicker { pub entries: Vec<PickerEntry>, pub cursor: usize, pub text: String, pub all: bool }
#[derive(Debug, Clone, PartialEq, Eq)] pub enum PickerOutcome { Open(KeyPicker), Close, Apply { keys: Vec<String>, all: bool } }
pub fn parse_keys(text: &str) -> Vec<String>;
impl KeyPicker {
    pub fn new(scope_keys: &[String], tier_key: &str, seen: &[(String, usize)], current: &[String], all: bool) -> Self;
    pub fn all_row(&self) -> usize;   pub fn text_row(&self) -> usize;
    pub fn chosen(&self) -> Vec<String>;   pub fn is_checked(&self, key: &str) -> bool;
    pub fn key(self, k: KeyEvent) -> PickerOutcome;
}
// tui/model.rs
Overlay::GroupKeys(KeyPicker)                                   // was GroupKeys { input }
#[derive(Debug, Clone)] pub struct FlowRow { pub flow: ResolvedFlow, pub src_group: String, pub dst_group: String }
impl FlowRow { pub fn new(flow: ResolvedFlow, keys: &[String]) -> Self; }  // + Deref<Target = ResolvedFlow>
pub fn flow_columns() -> Vec<Column<FlowRow>>;
pub const PREFERRED_KEYS: [&str; 4] = ["app", "env", "role", "loc"];
#[derive(Debug, Clone, PartialEq, Eq)] pub enum Drill { Pair(Vec<String>, Vec<String>) }   // Task 6 adds Mesh { .. }
impl Drill { pub fn matches(&self, f: &ResolvedFlow, keys: &[String]) -> bool; pub fn title(&self) -> String; }
pub struct TrafficState { /* … */ pub flows: TableView<FlowRow>, pub group_keys: Vec<String>, pub all_keys: bool, pub drill: Option<Drill>, .. }
impl TrafficState {
    pub fn new(window: &str, max_results: u32, auto_min_seconds: u64, keys: &[String]) -> Self;
    pub fn loaded_flows(&self) -> &[ResolvedFlow];
    pub fn effective_keys(&self) -> Vec<String>;
    pub fn has_bytes(&self) -> bool;
    pub fn first_flow(&self, pred: impl Fn(&ResolvedFlow) -> bool) -> Option<ResolvedFlow>;
    pub fn rebuild_views(&mut self);
}
impl Model { pub fn set_label_model(&mut self, scope_keys: Vec<String>, tier_key: String); }
// tui/ui/widgets/overlay.rs
pub fn render_key_picker(model: &Model, p: &KeyPicker, f: &mut Frame, area: Rect);  // replaces render_group_prompt
// tui/update.rs (private)
fn open_key_picker(model: &Model) -> KeyPicker;
fn apply_group_keys(model: &mut Model, keys: Vec<String>, all: bool);
```

- [ ] **Step 1: Write the failing tests**

Create `crates/illumio-tui/src/tui/picker.rs` with only this module, and add `pub mod picker;` after `pub mod msg;` in `crates/illumio-tui/src/tui/mod.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::KeyModifiers;

    fn kc(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn ch(c: char) -> KeyEvent {
        kc(KeyCode::Char(c))
    }

    fn keys(k: &[&str]) -> Vec<String> {
        k.iter().map(|s| s.to_string()).collect()
    }

    fn picker() -> KeyPicker {
        KeyPicker::new(
            &keys(&["app", "env"]),
            "role",
            &[
                ("app".into(), 30),
                ("env".into(), 30),
                ("role".into(), 30),
                ("loc".into(), 30),
                ("datacenter".into(), 11),
            ],
            &keys(&["app", "env"]),
            false,
        )
    }

    fn press(p: KeyPicker, k: KeyEvent) -> KeyPicker {
        match p.key(k) {
            PickerOutcome::Open(p) => p,
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn entries_list_the_profile_keys_then_loc_then_the_seen_keys_with_counts() {
        let p = picker();
        let listed: Vec<(&str, Option<usize>)> =
            p.entries.iter().map(|e| (e.key.as_str(), e.seen)).collect();
        assert_eq!(
            listed,
            vec![
                ("app", None),
                ("env", None),
                ("role", None),
                ("loc", None),
                ("datacenter", Some(11)),
            ]
        );
        assert!(p.is_checked("app") && p.is_checked("env") && !p.is_checked("role"));
        assert_eq!((p.all_row(), p.text_row()), (5, 6));
        // A profile key no flow carries, and a key typed earlier, are listed.
        let p = KeyPicker::new(
            &keys(&["site"]),
            "tier",
            &[],
            &keys(&["site", "zone"]),
            false,
        );
        let listed: Vec<&str> = p.entries.iter().map(|e| e.key.as_str()).collect();
        assert_eq!(listed, vec!["site", "tier", "loc", "zone"]);
    }

    #[test]
    fn space_toggles_keys_in_the_order_they_were_chosen() {
        let mut p = picker();
        p = press(p, ch('j')); // env
        p = press(p, ch(' '));
        assert_eq!(p.text, "app");
        p = press(p, kc(KeyCode::Down)); // role
        p = press(p, ch(' '));
        p = press(p, kc(KeyCode::Up)); // env
        p = press(p, ch(' '));
        assert_eq!(p.text, "app,role,env");
        assert_eq!(
            p.key(kc(KeyCode::Enter)),
            PickerOutcome::Apply {
                keys: keys(&["app", "role", "env"]),
                all: false
            }
        );
    }

    #[test]
    fn all_labels_unchecks_the_keys_and_a_key_clears_all_labels() {
        let mut p = picker();
        for _ in 0..5 {
            p = press(p, ch('j'));
        }
        assert_eq!(p.cursor, p.all_row());
        p = press(p, ch(' '));
        assert!(p.all && p.text.is_empty() && !p.is_checked("app"));
        assert_eq!(
            p.clone().key(kc(KeyCode::Enter)),
            PickerOutcome::Apply {
                keys: Vec::new(),
                all: true
            }
        );
        p = press(p, ch('k')); // datacenter
        p = press(p, ch(' '));
        assert!(!p.all);
        assert_eq!(p.text, "datacenter");
    }

    #[test]
    fn typing_edits_the_text_line_like_the_old_prompt() {
        let mut p = picker();
        for _ in 0..7 {
            p = press(p, kc(KeyCode::Backspace));
        }
        assert_eq!(p.cursor, p.text_row());
        assert_eq!(p.text, "");
        for c in "role, job ,".chars() {
            p = press(p, ch(c));
        }
        assert_eq!(p.text, "role,job,", "on the text line j and space are text");
        assert_eq!(
            p.key(kc(KeyCode::Enter)),
            PickerOutcome::Apply {
                keys: keys(&["role", "job"]),
                all: false
            }
        );
        let mut p = picker();
        for c in "zz".chars() {
            p = press(p, ch(c));
        }
        assert_eq!(
            p.text, "app,envzz",
            "a printable key jumps to the text line"
        );
        assert_eq!(picker().key(kc(KeyCode::Esc)), PickerOutcome::Close);
        assert_eq!(parse_keys(" app, ,env ,"), keys(&["app", "env"]));
    }
}
```

In `crates/illumio-tui/src/tui/update.rs`:

replace

```rust
        for view in [TrafficView::Aggregate, TrafficView::Blocked] {
            m.traffic.view = view;
            assert!(update(&mut m, key('w')).is_empty());
            assert_eq!(
                m.status.text,
                "select a flow in the Flows view (Enter on a pair)"
            );
            assert_eq!(m.overlay, Overlay::None);
        }
```

with

```rust
        m.traffic.view = TrafficView::Aggregate;
        assert!(update(&mut m, key('w')).is_empty());
        assert_eq!(
            m.status.text,
            "select a flow in the Flows view (Enter on a pair)"
        );
        assert_eq!(m.overlay, Overlay::None);
```

replace

```rust
    #[test]
    fn enter_in_aggregate_drills_into_the_pair_and_esc_clears() {
```

with

```rust
    #[test]
    fn g_opens_the_picker_and_enter_regroups_every_view_without_requests() {
        use crate::tui::test_support::model_with_rich_flows;
        let mut m = model_with_rich_flows();
        assert_eq!(m.traffic.group_keys, vec!["app", "env"], "the profile's scope keys");
        assert_eq!(m.traffic.flows.rows()[0].src_group, "shop/prod");
        assert!(update(&mut m, key('g')).is_empty());
        let Overlay::GroupKeys(p) = &m.overlay else {
            panic!("{:?}", m.overlay)
        };
        let listed: Vec<&str> = p.entries.iter().map(|e| e.key.as_str()).collect();
        assert_eq!(listed, vec!["app", "env", "role", "loc", "datacenter"]);
        assert_eq!(p.entries[4].seen, Some(11));
        for msg in [key('j'), key('j'), key(' '), enter()] {
            assert!(update(&mut m, msg).is_empty(), "the picker never requests");
        }
        assert_eq!(m.overlay, Overlay::None);
        assert_eq!(m.traffic.group_keys, vec!["app", "env", "role"]);
        assert_eq!(m.status.text, "grouped by app,env,role");
        assert_eq!(m.traffic.flows.rows()[0].src_group, "shop/prod/web");
        assert_eq!(m.traffic.flows.rows()[0].dst_group, "shop/prod/db");
        assert!(m.traffic.groups.rows().iter().all(|g| g.src.len() == 3));
        assert!(m.traffic.blocked.rows().iter().all(|b| b.dst_group.len() == 3));
        // A non-workload endpoint shows `-` in every slot.
        let corp = m
            .traffic
            .flows
            .rows()
            .iter()
            .find(|r| r.src.ip_lists == vec!["Corp"])
            .unwrap();
        assert_eq!(corp.src_group, "-/-/-");
    }

    #[test]
    fn all_labels_groups_by_every_seen_key_until_a_profile_switch() {
        use crate::tui::model::{Limits, ProfileInfo};
        use crate::tui::test_support::model_with_rich_flows;
        let mut m = model_with_rich_flows();
        update(&mut m, key('g'));
        for _ in 0..5 {
            update(&mut m, key('j'));
        }
        assert!(update(&mut m, key(' ')).is_empty());
        assert!(update(&mut m, enter()).is_empty());
        assert!(m.traffic.all_keys);
        assert_eq!(
            m.traffic.effective_keys(),
            vec!["app", "env", "role", "loc", "datacenter"]
        );
        assert!(m.traffic.groups.rows().iter().all(|g| g.src.len() == 5));
        assert_eq!(m.traffic.flows.rows()[4].src_group, "crm/prod/web/us/fra1");
        // Reopening shows `all labels` checked and no key.
        update(&mut m, key('g'));
        let Overlay::GroupKeys(p) = &m.overlay else {
            panic!("{:?}", m.overlay)
        };
        assert!(p.all && !p.is_checked("app"));
        update(&mut m, kc(KeyCode::Esc));
        update(
            &mut m,
            Msg::ProfileSwitched(Ok((
                ProfileInfo {
                    name: "prod".into(),
                    host: "prod.example".into(),
                    org_id: 9,
                },
                Limits {
                    auto_refresh_min_seconds: 60,
                    traffic_max_results: 500,
                    traffic_default_window: "1h".into(),
                    scope_keys: vec!["site".into()],
                    tier_key: "tier".into(),
                },
            ))),
        );
        assert_eq!(m.traffic.group_keys, vec!["site"], "the new profile's label model");
        assert!(!m.traffic.all_keys);
    }

    #[test]
    fn esc_keeps_the_keys_and_the_drill_and_applying_clears_the_drill() {
        use crate::tui::test_support::model_with_rich_flows;
        let mut m = model_with_rich_flows();
        update(&mut m, key('v'));
        update(&mut m, enter()); // crm/prod -> crm/prod
        assert_eq!(m.traffic.view, TrafficView::Flows);
        assert_eq!(m.traffic.flows.len(), 2);
        update(&mut m, key('g'));
        update(&mut m, key(' ')); // app off
        assert!(update(&mut m, kc(KeyCode::Esc)).is_empty());
        assert_eq!(m.traffic.group_keys, vec!["app", "env"]);
        assert!(m.traffic.drill.is_some());
        update(&mut m, key('g'));
        assert!(update(&mut m, enter()).is_empty());
        assert!(m.traffic.drill.is_none(), "applying drops the drill-down");
        assert_eq!(m.traffic.flows.len(), 30);
        // An empty text falls back to the profile's scope keys.
        update(&mut m, key('g'));
        for _ in 0..7 {
            update(&mut m, kc(KeyCode::Backspace));
        }
        assert!(update(&mut m, enter()).is_empty());
        assert_eq!(m.traffic.group_keys, vec!["app", "env"]);
        // Unknown keys are allowed and show `-`.
        update(&mut m, key('g'));
        for c in ",nope".chars() {
            update(&mut m, key(c));
        }
        update(&mut m, enter());
        assert_eq!(m.traffic.flows.rows()[0].src_group, "shop/prod/-");
    }

    #[test]
    fn w_on_a_blocked_row_drafts_from_the_first_flow_behind_it() {
        use crate::tui::test_support::model_with_rich_flows;
        let mut m = writes_on(model_with_rich_flows());
        m.traffic.view = TrafficView::Blocked;
        let row = m.traffic.blocked.selected_item().unwrap().clone();
        assert_eq!(row.dst_group, vec!["hr", "prod"], "newest first");
        assert!(update(&mut m, key('w')).is_empty(), "no request");
        let Overlay::Form(f) = &m.overlay else {
            panic!("{:?}", m.overlay)
        };
        assert_eq!(f.title, "Allow rule from flow");
        assert_eq!(f.value("Services"), "443/tcp");
        update(&mut m, kc(KeyCode::Esc));
        m.traffic.blocked.set_rows(Vec::new());
        assert!(update(&mut m, key('w')).is_empty());
        assert_eq!(m.status.text, "no row selected");
    }

    #[test]
    fn enter_in_aggregate_drills_into_the_pair_and_esc_clears() {
```

In `crates/illumio-tui/src/tui/model.rs`:

replace

```rust
    #[test]
    fn ip_list_rows_carry_their_size_and_how_many_loaded_rules_use_them() {
```

with

```rust
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
            vec!["src", "src group", "dst", "dst group", "service", "conns", "decision"]
        );
        assert!(row_width(&at80) <= 78, "inside the border at 80: {}", row_width(&at80));
        assert!(row_width(&visible_columns(&flows, 120)) <= 118);
        let at160 = visible_columns(&flows, 160);
        assert_eq!(flows[at160[0].0].title, "last seen");
        assert!(row_width(&at160) <= 158);
        let blocked = blocked_columns();
        let titles: Vec<&str> = blocked.iter().map(|c| c.title).collect();
        assert_eq!(
            titles,
            vec!["last seen", "dst", "dst name", "service", "flows", "conns", "sources"]
        );
    }

    #[test]
    fn blocked_cells_name_one_host_or_count_them_and_count_more_sources() {
        let flows = crate::tui::test_support::fixture_flows_rich();
        let keys = vec!["app".to_string(), "env".to_string()];
        let rows = pce::traffic::blocked_summary(&flows, &keys);
        let cols = blocked_columns();
        let shop = rows.iter().find(|r| r.dst_group == vec!["shop", "prod"]).unwrap();
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
    fn the_label_model_sets_the_group_keys_and_empty_falls_back_to_app_env() {
        let mut m = crate::tui::test_support::model_fixture();
        assert_eq!(m.traffic.group_keys, vec!["app", "env"]);
        m.set_label_model(vec!["site".into(), "zone".into()], "tier".into());
        assert_eq!(m.traffic.group_keys, vec!["site", "zone"]);
        assert_eq!(m.limits.tier_key, "tier");
        m.set_label_model(Vec::new(), "role".into());
        assert_eq!(m.traffic.group_keys, vec!["app", "env"]);
        assert!(m.limits.scope_keys.is_empty(), "the profile value itself is kept");
    }

    #[test]
    fn ip_list_rows_carry_their_size_and_how_many_loaded_rules_use_them() {
```

Append to `crates/illumio-tui/tests/tui_snapshots.rs`:

```rust
#[test]
fn traffic_group_columns_picker_and_blocked_by_group_80x24() {
    use illumio_tui::tui::model::{Overlay, TrafficView};
    use illumio_tui::tui::test_support::model_with_rich_flows;
    let mut m = model_with_rich_flows();
    let out = render_to_string(&m, 80, 24);
    assert!(
        out.contains("src group") && out.contains("dst group"),
        "{out}"
    );
    assert!(out.contains("shop/prod"), "{out}");
    assert!(out.contains("g group"), "the status hint names g: {out}");
    insta::assert_snapshot!("traffic_flows_groups_80x24", out);
    m.overlay = Overlay::GroupKeys(illumio_tui::tui::picker::KeyPicker::new(
        &m.limits.scope_keys,
        &m.limits.tier_key,
        &pce::traffic::keys_seen(
            m.traffic.loaded_flows(),
            &["app".into(), "env".into(), "role".into(), "loc".into()],
        ),
        &m.traffic.group_keys,
        false,
    ));
    let out = render_to_string(&m, 80, 24);
    assert!(out.contains("Group by"), "{out}");
    assert!(out.contains("[x] app") && out.contains("[ ] role"), "{out}");
    assert!(out.contains("(seen in 11 flows)"), "{out}");
    assert!(out.contains("keys: app,env"), "{out}");
    insta::assert_snapshot!("traffic_key_picker_80x24", out);
    m.overlay = Overlay::None;
    m.traffic.view = TrafficView::Blocked;
    let out = render_to_string(&m, 80, 24);
    assert!(out.contains("dst name") && out.contains("hr/prod"), "{out}");
    assert!(out.contains("by dst app,env"), "{out}");
    insta::assert_snapshot!("traffic_blocked_groups_80x24", out);
}

#[test]
fn flow_detail_shows_both_label_sets_and_the_bytes() {
    let mut m = model_with_flows();
    m.traffic.show_detail = true; // row 0: shop/prod -> shop/prod with bytes
    let out = render_to_string(&m, 160, 48);
    assert!(
        out.contains("src labels app=shop env=prod role=web"),
        "{out}"
    );
    assert!(
        out.contains("dst labels app=shop env=prod role=db"),
        "{out}"
    );
    assert!(out.contains("bytes in 120.0K out 4.0K"), "{out}");
    m.traffic.flows.select_next(); // Corp: an IP list, no labels, no bytes
    let out = render_to_string(&m, 160, 48);
    assert!(out.contains("src labels -"), "{out}");
    assert!(out.contains("ip lists: Corp"), "{out}");
    assert!(!out.contains("bytes in"), "{out}");
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p illumio-tui --lib 2>&1 | tail -5`
Expected: FAIL to compile (`KeyPicker`, `PickerOutcome`, `FlowRow`, `Drill`, `TrafficState::{all_keys, effective_keys, loaded_flows}`, `Model::set_label_model` and `Overlay::GroupKeys(_)` do not exist yet).

- [ ] **Step 3: Implement**

Put this above the tests module in `crates/illumio-tui/src/tui/picker.rs`:

```rust
//! The `g` overlay: which label keys every traffic view groups by. Pure
//! state; `update` applies the outcome and `ui` draws it.
use crossterm::event::{KeyCode, KeyEvent};

/// One key the picker offers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PickerEntry {
    pub key: String,
    /// Flows carrying the key, shown for keys outside the profile's label
    /// model (`None` for the profile keys and `loc`).
    pub seen: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyPicker {
    pub entries: Vec<PickerEntry>,
    /// The row under the cursor: an entry, then `all labels`
    /// (`all_row()`), then the `keys:` text line (`text_row()`).
    pub cursor: usize,
    /// The chosen keys as text, `app,env`. The toggles rewrite it; typing
    /// edits it.
    pub text: String,
    /// `all labels` is checked: every key seen in the loaded flows.
    pub all: bool,
}

/// What a key press leaves behind.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PickerOutcome {
    Open(KeyPicker),
    Close,
    /// Enter: these keys (possibly empty; the caller falls back to the
    /// profile's scope keys), or every seen key when `all`.
    Apply {
        keys: Vec<String>,
        all: bool,
    },
}

/// `app, env ,,role` -> `["app", "env", "role"]`.
pub fn parse_keys(text: &str) -> Vec<String> {
    text.split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(String::from)
        .collect()
}

impl KeyPicker {
    /// Entries: the profile's scope keys and tier key, then `loc`, then every
    /// other key in `seen` (with its count), then the current keys not listed
    /// yet (typed earlier, perhaps unknown). The text starts as `current`.
    pub fn new(
        scope_keys: &[String],
        tier_key: &str,
        seen: &[(String, usize)],
        current: &[String],
        all: bool,
    ) -> Self {
        let mut entries: Vec<PickerEntry> = Vec::new();
        let mut add = |key: &str, count: Option<usize>| {
            if !key.is_empty() && !entries.iter().any(|e| e.key == key) {
                entries.push(PickerEntry {
                    key: key.to_string(),
                    seen: count,
                });
            }
        };
        for k in scope_keys {
            add(k, None);
        }
        add(tier_key, None);
        add("loc", None);
        for (k, n) in seen {
            add(k, Some(*n));
        }
        for k in current {
            add(k, None);
        }
        Self {
            entries,
            cursor: 0,
            text: if all {
                String::new()
            } else {
                current.join(",")
            },
            all,
        }
    }

    pub fn all_row(&self) -> usize {
        self.entries.len()
    }

    pub fn text_row(&self) -> usize {
        self.entries.len() + 1
    }

    /// The keys the text names, in its order.
    pub fn chosen(&self) -> Vec<String> {
        parse_keys(&self.text)
    }

    pub fn is_checked(&self, key: &str) -> bool {
        !self.all && self.chosen().iter().any(|k| k == key)
    }

    /// `j`/`k` and arrows move (`j`/`k` are text on the text line), Space
    /// toggles the row, any other printable character or Backspace edits
    /// the text (moving the cursor to it), Enter applies, Esc closes.
    pub fn key(mut self, k: KeyEvent) -> PickerOutcome {
        let on_text = self.cursor == self.text_row();
        match k.code {
            KeyCode::Esc => return PickerOutcome::Close,
            KeyCode::Enter => {
                return PickerOutcome::Apply {
                    keys: if self.all { Vec::new() } else { self.chosen() },
                    all: self.all,
                };
            }
            KeyCode::Down => self.cursor = (self.cursor + 1).min(self.text_row()),
            KeyCode::Up => self.cursor = self.cursor.saturating_sub(1),
            KeyCode::Char('j') if !on_text => self.cursor = (self.cursor + 1).min(self.text_row()),
            KeyCode::Char('k') if !on_text => self.cursor = self.cursor.saturating_sub(1),
            KeyCode::Char(' ') if !on_text => self.toggle(),
            KeyCode::Char(' ') => {}
            KeyCode::Backspace => {
                self.cursor = self.text_row();
                self.all = false;
                self.text.pop();
            }
            KeyCode::Char(c) => {
                self.cursor = self.text_row();
                self.all = false;
                self.text.push(c);
            }
            _ => {}
        }
        PickerOutcome::Open(self)
    }

    fn toggle(&mut self) {
        if self.cursor == self.all_row() {
            self.all = !self.all;
            if self.all {
                self.text.clear();
            }
            return;
        }
        let Some(key) = self.entries.get(self.cursor).map(|e| e.key.clone()) else {
            return;
        };
        let mut chosen = if self.all { Vec::new() } else { self.chosen() };
        self.all = false;
        match chosen.iter().position(|k| *k == key) {
            Some(i) => {
                chosen.remove(i);
            }
            None => chosen.push(key),
        }
        self.text = chosen.join(",");
    }
}
```

In `crates/illumio-tui/src/tui/model.rs`:

replace

```rust
use pce::traffic::{BlockedGroup, FlowGroup, ResolvedFlow, blocked_summary, group_by_labels};
```

with

```rust
use crate::tui::picker::KeyPicker;
use pce::traffic::{
    BlockedGroup, FlowGroup, ResolvedFlow, blocked_summary, group_by_labels, group_key,
    group_text, keys_or_default, keys_seen,
};
```

replace

```rust
    /// Comma-separated label keys for the app-to-app view.
    GroupKeys {
        input: String,
    },
```

with

```rust
    /// `g`: the label keys every traffic view groups by.
    GroupKeys(KeyPicker),
```

replace

```rust
            (Overlay::Prompt { input: a }, Overlay::Prompt { input: b })
            | (Overlay::GroupKeys { input: a }, Overlay::GroupKeys { input: b })
            | (Overlay::Error(a), Overlay::Error(b)) => a == b,
```

with

```rust
            (Overlay::Prompt { input: a }, Overlay::Prompt { input: b })
            | (Overlay::Error(a), Overlay::Error(b)) => a == b,
            (Overlay::GroupKeys(a), Overlay::GroupKeys(b)) => a == b,
```

replace

```rust
pub fn flow_columns() -> Vec<Column<ResolvedFlow>> {
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
            hide_below: 0,
            narrow: Some(|f| {
                f.last_detected
                    .as_deref()
                    .map(short_time_narrow)
                    .unwrap_or_default()
            }),
            narrow_width: 11,
        },
        Column {
            title: "src",
            width: 22,
            cell: |f| f.src.display_name(),
            key: |f| SortKey::Text(f.src.display_name()),
            right: false,
            hide_below: 0,
            narrow: None,
            narrow_width: 16,
        },
        Column {
            title: "src labels",
            width: 0,
            cell: |f| f.src.label_text(),
            key: |f| SortKey::Text(f.src.label_text()),
            right: false,
            hide_below: 120,
            narrow: None,
            narrow_width: 0,
        },
        Column {
            title: "dst",
            width: 22,
            cell: |f| f.dst.display_name(),
            key: |f| SortKey::Text(f.dst.display_name()),
            right: false,
            hide_below: 0,
            narrow: None,
            narrow_width: 16,
        },
```

with

```rust
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
```

replace

```rust
        Column {
            title: "dst",
            width: 22,
            cell: |b| b.dst.clone(),
            key: |b| SortKey::Text(b.dst.clone()),
            right: false,
            hide_below: 0,
            narrow: None,
            narrow_width: 16,
        },
        Column {
            title: "dst labels",
            width: 0,
            cell: |b| b.dst_labels.clone(),
            key: |b| SortKey::Text(b.dst_labels.clone()),
            right: false,
            hide_below: 120,
            narrow: None,
            narrow_width: 0,
        },
```

with

```rust
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
```

replace

```rust
        Column {
            title: "sources",
            width: 30,
            cell: |b| b.sources.join(", "),
            key: |b| SortKey::Text(b.sources.join(", ")),
            right: false,
            hide_below: 0,
            narrow: None,
            narrow_width: 20,
        },
    ]
}
```

with

```rust
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
```

replace

```rust
    pub flows: TableView<ResolvedFlow>,
    pub show_detail: bool,
    pub view: TrafficView,
    pub group_keys: Vec<String>,
    pub groups: TableView<FlowGroup>,
    pub blocked: TableView<BlockedGroup>,
    /// Aggregate pair (src, dst group keys) the Flows view is narrowed to.
    pub drill: Option<(Vec<String>, Vec<String>)>,
```

with

```rust
    pub flows: TableView<FlowRow>,
    pub show_detail: bool,
    pub view: TrafficView,
    /// The label keys every view groups by: the profile's scope keys at
    /// startup and after `:profile`, then whatever `g` chose. Not persisted.
    pub group_keys: Vec<String>,
    /// `g` chose `all labels`: every key seen in the loaded flows.
    pub all_keys: bool,
    pub groups: TableView<FlowGroup>,
    pub blocked: TableView<BlockedGroup>,
    /// What the Flows view is narrowed to.
    pub drill: Option<Drill>,
```

replace

```rust
impl TrafficState {
    pub fn new(window: &str, max_results: u32, auto_min_seconds: u64) -> Self {
```

with

```rust
/// The keys `all labels` lists first when the flows carry them.
pub const PREFERRED_KEYS: [&str; 4] = ["app", "env", "role", "loc"];

/// What `Enter` narrowed the Flows view to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Drill {
    /// An App-to-app or treemap pair: source and destination group keys.
    Pair(Vec<String>, Vec<String>),
}

impl Drill {
    /// Whether `f` belongs to the drilled row under `keys`.
    pub fn matches(&self, f: &ResolvedFlow, keys: &[String]) -> bool {
        match self {
            Drill::Pair(src, dst) => {
                &group_key(&f.src, keys) == src && &group_key(&f.dst, keys) == dst
            }
        }
    }

    /// The Flows title after `Flows ▸ `.
    pub fn title(&self) -> String {
        match self {
            Drill::Pair(src, dst) => format!("{} → {}", src.join("/"), dst.join("/")),
        }
    }
}

impl TrafficState {
    pub fn new(window: &str, max_results: u32, auto_min_seconds: u64, keys: &[String]) -> Self {
```

replace

```rust
            view: TrafficView::Flows,
            group_keys: vec!["app".into(), "env".into()],
```

with

```rust
            view: TrafficView::Flows,
            group_keys: keys_or_default(keys),
            all_keys: false,
```

replace

```rust
    /// Recomputes every view from `data`. Pure; never requests.
    pub fn rebuild_views(&mut self) {
        let flows: Vec<ResolvedFlow> = self
            .data
            .as_ref()
            .map(|d| d.flows.clone())
            .unwrap_or_default();
        let shown: Vec<ResolvedFlow> = match &self.drill {
            Some((src, dst)) => flows
                .iter()
                .filter(|f| {
                    group_by_labels(std::slice::from_ref(f), &self.group_keys)
                        .first()
                        .is_some_and(|g| &g.src == src && &g.dst == dst)
                })
                .cloned()
                .collect(),
            None => flows.clone(),
        };
        self.flows.set_rows(shown);
        self.groups
            .set_rows(group_by_labels(&flows, &self.group_keys));
        self.blocked
            .set_rows(blocked_summary(&flows, &self.group_keys));
    }
```

with

```rust
    /// The loaded flows (none before the first query).
    pub fn loaded_flows(&self) -> &[ResolvedFlow] {
        self.data.as_ref().map_or(&[], |d| d.flows.as_slice())
    }

    /// The keys in force: every key seen in the loaded flows when `all_keys`
    /// (`app`, `env`, `role`, `loc` first), else `group_keys` (`app,env`
    /// when empty).
    pub fn effective_keys(&self) -> Vec<String> {
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
        let keys = self.effective_keys();
        let flows = self.loaded_flows();
        let shown: Vec<FlowRow> = flows
            .iter()
            .filter(|f| self.drill.as_ref().is_none_or(|d| d.matches(f, &keys)))
            .map(|f| FlowRow::new(f.clone(), &keys))
            .collect();
        let groups = group_by_labels(flows, &keys);
        let blocked = blocked_summary(flows, &keys);
        self.flows.set_rows(shown);
        self.groups.set_rows(groups);
        self.blocked.set_rows(blocked);
    }
```

replace

```rust
        let traffic = TrafficState::new(
            &traffic_default_window,
            traffic_max_results,
            auto_refresh_min_seconds,
        );
```

with

```rust
        let scope_keys: Vec<String> = crate::config::DEFAULT_SCOPE_KEYS.map(String::from).to_vec();
        let traffic = TrafficState::new(
            &traffic_default_window,
            traffic_max_results,
            auto_refresh_min_seconds,
            &scope_keys,
        );
```

replace

```rust
                scope_keys: crate::config::DEFAULT_SCOPE_KEYS.map(String::from).to_vec(),
                tier_key: crate::config::DEFAULT_TIER_KEY.into(),
```

with

```rust
                scope_keys,
                tier_key: crate::config::DEFAULT_TIER_KEY.into(),
```

replace

```rust
        let mut traffic = TrafficState::new(
            &self.limits.traffic_default_window,
            self.limits.traffic_max_results,
            min,
        );
```

with

```rust
        let mut traffic = TrafficState::new(
            &self.limits.traffic_default_window,
            self.limits.traffic_max_results,
            min,
            &self.limits.scope_keys,
        );
```

replace

```rust
    /// What a `w` form is pre-filled from:
```

with

```rust
    /// The profile's label model: the `w` form's dimensions, and the keys
    /// every traffic view groups by until `g` changes them.
    pub fn set_label_model(&mut self, scope_keys: Vec<String>, tier_key: String) {
        self.traffic.group_keys = keys_or_default(&scope_keys);
        self.traffic.all_keys = false;
        self.limits.scope_keys = scope_keys;
        self.limits.tier_key = tier_key;
    }

    /// What a `w` form is pre-filled from:
```

In `crates/illumio-tui/src/tui/update.rs`:

replace

```rust
        Overlay::GroupKeys { mut input } => match k.code {
            KeyCode::Esc => {
                model.overlay = Overlay::None;
                Vec::new()
            }
            KeyCode::Enter => {
                model.overlay = Overlay::None;
                let mut keys: Vec<String> = input
                    .split(',')
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(String::from)
                    .collect();
                if keys.is_empty() {
                    keys = vec!["app".into(), "env".into()];
                }
                model.traffic.group_keys = keys;
                model.traffic.drill = None;
                model.traffic.rebuild_views();
                Vec::new()
            }
            KeyCode::Backspace => {
                input.pop();
                model.overlay = Overlay::GroupKeys { input };
                Vec::new()
            }
            KeyCode::Char(c) => {
                input.push(c);
                model.overlay = Overlay::GroupKeys { input };
                Vec::new()
            }
            _ => Vec::new(),
        },
```

with

```rust
        Overlay::GroupKeys(picker) => {
            match picker.key(k) {
                PickerOutcome::Open(p) => model.overlay = Overlay::GroupKeys(p),
                PickerOutcome::Close => model.overlay = Overlay::None,
                PickerOutcome::Apply { keys, all } => {
                    model.overlay = Overlay::None;
                    apply_group_keys(model, keys, all);
                }
            }
            Vec::new()
        }
```

replace

```rust
        KeyCode::Char('g') => {
            model.overlay = Overlay::GroupKeys {
                input: model.traffic.group_keys.join(","),
            };
            return Vec::new();
        }
        KeyCode::Enter => match model.traffic.view {
            TrafficView::Aggregate => {
                if let Some(g) = model.traffic.groups.selected_item() {
                    model.traffic.drill = Some((g.src.clone(), g.dst.clone()));
```

with

```rust
        KeyCode::Char('g') => {
            model.overlay = Overlay::GroupKeys(open_key_picker(model));
            return Vec::new();
        }
        KeyCode::Enter => match model.traffic.view {
            TrafficView::Aggregate => {
                if let Some(g) = model.traffic.groups.selected_item() {
                    model.traffic.drill = Some(Drill::Pair(g.src.clone(), g.dst.clone()));
```

replace

```rust
/// Validated filter from the editor or a command: confirm if expensive, else apply.
```

with

```rust
/// The `g` picker over the profile's label model and the keys the loaded
/// flows carry.
fn open_key_picker(model: &Model) -> KeyPicker {
    let t = &model.traffic;
    let mut preferred = model.limits.scope_keys.clone();
    preferred.push(model.limits.tier_key.clone());
    preferred.push("loc".into());
    let seen = pce::traffic::keys_seen(t.loaded_flows(), &preferred);
    KeyPicker::new(
        &model.limits.scope_keys,
        &model.limits.tier_key,
        &seen,
        &t.group_keys,
        t.all_keys,
    )
}

/// Enter in the picker: the keys (the profile's scope keys when none),
/// every view rebuilt, the drill-down dropped. No request.
fn apply_group_keys(model: &mut Model, keys: Vec<String>, all: bool) {
    let t = &mut model.traffic;
    t.all_keys = all;
    if !all {
        t.group_keys = if keys.is_empty() {
            pce::traffic::keys_or_default(&model.limits.scope_keys)
        } else {
            keys
        };
    }
    t.drill = None;
    t.rebuild_views();
    model.status = StatusLine::info(format!("grouped by {}", t.effective_keys().join(",")));
}

/// Validated filter from the editor or a command: confirm if expensive, else apply.
```

replace

```rust
/// `w` on Traffic: a draft allow rule from the selected blocked or
/// potentially blocked flow (Flows view or a drilled pair). The aggregated
/// views carry names, not label hrefs, so they refuse.
fn flow_rule_key(model: &mut Model) -> Vec<Cmd> {
    if writes_refused(model) {
        return Vec::new();
    }
    if model.traffic.view != TrafficView::Flows {
        model.status = StatusLine::warn("select a flow in the Flows view (Enter on a pair)");
        return Vec::new();
    }
    let Some(flow) = model.traffic.flows.selected_item().cloned() else {
        model.status = StatusLine::warn("no row selected");
        return Vec::new();
    };
```

with

```rust
/// `w` on Traffic: a draft allow rule from the selected blocked or
/// potentially blocked flow (Flows view or a drilled pair), or from the
/// first such flow behind a Blocked row. App-to-app refuses: a pair holds
/// flows of every decision.
fn flow_rule_key(model: &mut Model) -> Vec<Cmd> {
    if writes_refused(model) {
        return Vec::new();
    }
    let t = &model.traffic;
    let flow = match t.view {
        TrafficView::Flows => t.flows.selected_item().map(|r| r.flow.clone()),
        TrafficView::Blocked => t.blocked.selected_item().and_then(|b| {
            let keys = t.effective_keys();
            t.first_flow(|f| b.contains(f, &keys))
        }),
        TrafficView::Aggregate => {
            model.status = StatusLine::warn("select a flow in the Flows view (Enter on a pair)");
            return Vec::new();
        }
    };
    let Some(flow) = flow else {
        model.status = StatusLine::warn("no row selected");
        return Vec::new();
    };
```

replace

```rust
use crate::tui::model::{
    Connection, EventsFilterEdit, Model, ObjectKind, Overlay, PendingAction, PolicyPane,
    PolicyState, QueryState, Screen, StatusLine, TrafficData, TrafficView, WritesState, rule_line,
};
```

with

```rust
use crate::tui::model::{
    Connection, Drill, EventsFilterEdit, Model, ObjectKind, Overlay, PendingAction, PolicyPane,
    PolicyState, QueryState, Screen, StatusLine, TrafficData, TrafficView, WritesState, rule_line,
};
use crate::tui::picker::{KeyPicker, PickerOutcome};
```

In `crates/illumio-tui/src/tui/app.rs`:

replace

```rust
    model.limits.scope_keys = cfg.scope_keys.clone();
    model.limits.tier_key = cfg.tier_key.clone();
```

with

```rust
    model.set_label_model(cfg.scope_keys.clone(), cfg.tier_key.clone());
```

In `crates/illumio-tui/src/tui/ui/mod.rs`:

replace

```rust
        Overlay::GroupKeys { input } => {
            widgets::overlay::render_group_prompt(model, input, f, area)
        }
```

with

```rust
        Overlay::GroupKeys(picker) => widgets::overlay::render_key_picker(model, picker, f, area),
```

In `crates/illumio-tui/src/tui/ui/widgets/overlay.rs`:

replace

```rust
pub fn render_group_prompt(model: &Model, input: &str, f: &mut Frame, area: Rect) {
    let t = &model.theme;
    let r = Rect::new(
        area.x,
        area.y + area.height.saturating_sub(1),
        area.width,
        1,
    );
    f.render_widget(Clear, r);
    let line = Line::from(vec![
        Span::styled("group keys: ", t.header_style()),
        Span::styled(input.to_string(), t.base()),
        Span::styled("▏", t.dim_style()),
    ]);
    f.render_widget(Paragraph::new(line).style(t.base()), r);
}
```

with

```rust
/// The `g` picker: a checkbox per key (the cursor row highlighted), `all
/// labels`, the editable `keys:` line and the key hints. The rows scroll
/// when the box is shorter than the list; the last three lines stay.
pub fn render_key_picker(model: &Model, p: &KeyPicker, f: &mut Frame, area: Rect) {
    let t = &model.theme;
    let rows = p.entries.len() as u16 + 1;
    let r = centered(area, 42, rows + 5);
    f.render_widget(Clear, r);
    let block = panel("Group by", "", t, true);
    let inner = block.inner(r);
    f.render_widget(block, r);
    let list_h = inner.height.saturating_sub(3) as usize;
    let first = p.cursor.min(p.all_row()).saturating_sub(list_h.saturating_sub(1));
    let row_style = |i: usize| {
        if i == p.cursor {
            t.selected_style()
        } else {
            t.base()
        }
    };
    let mut lines: Vec<Line> = p
        .entries
        .iter()
        .enumerate()
        .map(|(i, e)| {
            let mark = if p.is_checked(&e.key) { "[x]" } else { "[ ]" };
            let mut spans = vec![Span::styled(format!("{mark} {:<15}", e.key), row_style(i))];
            if let Some(n) = e.seen {
                let noun = if n == 1 { "flow" } else { "flows" };
                spans.push(Span::styled(format!(" (seen in {n} {noun})"), t.dim_style()));
            }
            Line::from(spans)
        })
        .collect();
    let all = if p.all { "[x]" } else { "[ ]" };
    lines.push(Line::from(Span::styled(
        format!("{all} all labels"),
        row_style(p.all_row()),
    )));
    let mut out: Vec<Line> = lines.into_iter().skip(first).take(list_h).collect();
    out.push(Line::from(""));
    let text = if p.all { "all labels".to_string() } else { p.text.clone() };
    let cursor = if p.cursor == p.text_row() { "▏" } else { "" };
    out.push(Line::from(vec![
        Span::styled("keys: ", t.header_style()),
        Span::styled(format!("{text}{cursor}"), row_style(p.text_row())),
    ]));
    out.push(Line::from(Span::styled(
        "Space toggle · Enter apply · Esc close",
        t.dim_style(),
    )));
    f.render_widget(Paragraph::new(out).style(t.base()), inner);
}
```

replace

```rust
use crate::tui::model::{Model, Screen, rule_kind_name};
```

with

```rust
use crate::tui::model::{Model, Screen, rule_kind_name};
use crate::tui::picker::KeyPicker;
```

replace

```rust
    ("Traffic  g", "change the aggregate label keys"),
```

with

```rust
    ("Traffic  g", "pick the label keys every view groups by"),
```

replace

```rust
        (Screen::Traffic, true) => "w:rule  f filter  r run  v view  / search",
        (Screen::Traffic, _) => "f filter  r run  a auto  v view  s sort  / search",
```

with

```rust
        (Screen::Traffic, true) => "w:rule  f filter  r run  v view  g group",
        (Screen::Traffic, _) => "f filter  r run  a auto  v view  g group  / search",
```

In `crates/illumio-tui/src/tui/ui/screens/traffic.rs`:

replace

```rust
/// Shared by `view` and `update` (for page sizes): filter bar 3, stats 3,
/// table takes the rest, detail 6 (0 when the terminal is short).
pub fn layout(body: Rect) -> TrafficLayout {
    let detail_h = if body.height >= 20 { 6 } else { 0 };
```

with

```rust
/// Shared by `view` and `update` (for page sizes): filter bar 3, stats 3,
/// table takes the rest, detail 8 (0 when the terminal is short).
pub fn layout(body: Rect) -> TrafficLayout {
    let detail_h = if body.height >= 20 { 8 } else { 0 };
```

replace

```rust
    let title = match &model.traffic.drill {
        Some((src, dst)) => {
            let badge = "Esc clears";
            meta = if meta.is_empty() {
                badge.to_string()
            } else {
                format!("{badge} · {meta}")
            };
            format!("Flows ▸ {} → {}", src.join("/"), dst.join("/"))
        }
```

with

```rust
    let title = match &model.traffic.drill {
        Some(drill) => {
            let badge = "Esc clears";
            meta = if meta.is_empty() {
                badge.to_string()
            } else {
                format!("{badge} · {meta}")
            };
            format!("Flows ▸ {}", drill.title())
        }
```

replace

```rust
    let meta = format!(
        "keys {} · Enter drills{}{}",
        model.traffic.group_keys.join(","),
        if meta.is_empty() { "" } else { " · " },
        meta
    );
    let block = panel(TrafficView::Aggregate.title(), &meta, t, true);
    let theme = t.clone();
    crate::tui::ui::widgets::table::render(f, area, &model.traffic.groups, t, block, &move |g| {
        let decision = if g.blocked > 0 {
            PolicyDecision::Blocked
        } else if g.potentially_blocked > 0 {
            PolicyDecision::PotentiallyBlocked
        } else {
            PolicyDecision::Allowed
        };
        theme.decision_style(decision)
    });
```

with

```rust
    let meta = format!(
        "keys {} · Enter drills{}{}",
        model.traffic.effective_keys().join(","),
        if meta.is_empty() { "" } else { " · " },
        meta
    );
    let block = panel(TrafficView::Aggregate.title(), &meta, t, true);
    let theme = t.clone();
    crate::tui::ui::widgets::table::render(f, area, &model.traffic.groups, t, block, &move |g| {
        theme.decision_style(g.dominant())
    });
```

replace

```rust
/// Every row here is blocked or potentially blocked; both read as `bad`.
fn render_blocked(model: &Model, f: &mut Frame, area: Rect) {
    let t = &model.theme;
    let meta = crate::tui::ui::widgets::table::header_meta(&model.traffic.blocked);
    let block = panel(TrafficView::Blocked.title(), &meta, t, true);
```

with

```rust
/// Every row here is blocked or potentially blocked; both read as `bad`.
fn render_blocked(model: &Model, f: &mut Frame, area: Rect) {
    let t = &model.theme;
    let meta = crate::tui::ui::widgets::table::header_meta(&model.traffic.blocked);
    let meta = format!(
        "by dst {}{}{}",
        model.traffic.effective_keys().join(","),
        if meta.is_empty() { "" } else { " · " },
        meta
    );
    let block = panel(TrafficView::Blocked.title(), &meta, t, true);
```

replace

```rust
/// Labels, then any IP lists the endpoint belongs to (an IP-list-only endpoint has no labels).
fn endpoint_tags(e: &ResolvedEndpoint) -> String {
    let mut s = e.label_text();
    if !e.ip_lists.is_empty() {
        if !s.is_empty() {
            s.push_str("  ");
        }
        s.push_str(&format!("ip lists: {}", e.ip_lists.join(", ")));
    }
    s
}
```

with

```rust
/// `name  ip`, then the IP lists the endpoint belongs to.
fn endpoint_line(e: &ResolvedEndpoint) -> String {
    let mut s = format!("{}  {}", e.display_name(), e.ip.clone().unwrap_or_default());
    if !e.ip_lists.is_empty() {
        s.push_str(&format!("  ip lists: {}", e.ip_lists.join(", ")));
    }
    s
}

/// `app=shop env=prod role=web`, or `-` without labels.
fn labels_line(e: &ResolvedEndpoint) -> String {
    let s = e.label_text();
    if s.is_empty() { "-".into() } else { s }
}

/// `  bytes in 117.2K out 3.9K` when the PCE reported either count.
fn bytes_text(fl: &ResolvedFlow) -> String {
    if !fl.has_bytes() {
        return String::new();
    }
    let n = |b: Option<u64>| b.map_or_else(|| "-".to_string(), compact_count);
    format!("  bytes in {} out {}", n(fl.bytes_in), n(fl.bytes_out))
}
```

replace

```rust
    let dash = || "-".to_string();
    let lines = vec![
        Line::from(vec![
            Span::styled("src  ", t.header_style()),
            Span::raw(format!(
                "{}  {}  {}",
                fl.src.display_name(),
                fl.src.ip.clone().unwrap_or_default(),
                endpoint_tags(&fl.src)
            )),
        ]),
        Line::from(vec![
            Span::styled("dst  ", t.header_style()),
            Span::raw(format!(
                "{}  {}  {}",
                fl.dst.display_name(),
                fl.dst.ip.clone().unwrap_or_default(),
                endpoint_tags(&fl.dst)
            )),
        ]),
        Line::from(vec![
            Span::styled("svc  ", t.header_style()),
            Span::raw(format!(
                "{}  process {}  user {}  {}",
                fl.service_text(),
                fl.process_name.clone().unwrap_or_else(dash),
                fl.user_name.clone().unwrap_or_else(dash),
                fl.policy_decision.as_str()
            )),
        ]),
        Line::from(vec![
            Span::styled("seen ", t.header_style()),
            Span::raw(format!(
                "first {}  last {}  rules {}",
                fl.first_detected.clone().unwrap_or_default(),
                fl.last_detected.clone().unwrap_or_default(),
                rules
            )),
        ]),
    ];
```

with

```rust
    let dash = || "-".to_string();
    let row = |head: &'static str, body: String| {
        Line::from(vec![
            Span::styled(format!("{head:<11}"), t.header_style()),
            Span::raw(body),
        ])
    };
    let lines = vec![
        row("src", endpoint_line(&fl.src)),
        row("src labels", labels_line(&fl.src)),
        row("dst", endpoint_line(&fl.dst)),
        row("dst labels", labels_line(&fl.dst)),
        row(
            "svc",
            format!(
                "{}  process {}  user {}  {}",
                fl.service_text(),
                fl.process_name.clone().unwrap_or_else(dash),
                fl.user_name.clone().unwrap_or_else(dash),
                fl.policy_decision.as_str()
            ),
        ),
        row(
            "seen",
            format!(
                "first {}  last {}{}  rules {}",
                fl.first_detected.clone().unwrap_or_default(),
                fl.last_detected.clone().unwrap_or_default(),
                bytes_text(fl),
                rules
            ),
        ),
    ];
```

replace

```rust
use pce::models::PolicyDecision;
use pce::traffic::ResolvedEndpoint;
```

with

```rust
use pce::models::PolicyDecision;
use pce::traffic::{ResolvedEndpoint, ResolvedFlow, compact_count};
```

- [ ] **Step 4: Run the unit tests**

Run: `cargo fmt --all && cargo test -p illumio-tui --lib`
Expected: PASS, including the picker tests, `g_opens_the_picker_and_enter_regroups_every_view_without_requests`, `all_labels_groups_by_every_seen_key_until_a_profile_switch`, `esc_keeps_the_keys_and_the_drill_and_applying_clears_the_drill`, `w_on_a_blocked_row_drafts_from_the_first_flow_behind_it`, `the_flow_and_blocked_columns_fit_80_columns_with_the_group_columns`, `blocked_cells_name_one_host_or_count_them_and_count_more_sources`, `the_label_model_sets_the_group_keys_and_empty_falls_back_to_app_env`, and the unchanged `v_cycles_views_and_g_sets_group_keys_without_requests` (Backspace and typing in the picker still edit the keys as text).

- [ ] **Step 5: Create and read the snapshots**

```bash
INSTA_UPDATE=always cargo test -p illumio-tui --test tui_snapshots
find crates/illumio-tui/tests/snapshots -name '*.snap.new' -delete
git status --short crates/illumio-tui/tests/snapshots | sed 's/.*tui_snapshots__//; s/\.snap$//' | sort
```

Expected, 3 new: `traffic_flows_groups_80x24`, `traffic_key_picker_80x24`, `traffic_blocked_groups_80x24`; and 22 modified: `chrome_dark_80x24`, `chrome_dark_160x48`, `chrome_dark_styled_80x24`, `chrome_light_styled_80x24`, `error_overlay_80x24`, `prompt_overlay_80x24`, `header_writes_dark_styled_80x24`, `header_writes_light_styled_80x24`, `form_rule_from_flow_80x24`, `form_rule_from_flow_light_styled_80x24`, `form_rule_from_flow_ctrl_n_80x24`, `form_rule_from_flow_ctrl_n_light_styled_80x24`, `traffic_filter_bar_80x24`, `traffic_filter_editor_80x24`, `traffic_flows_80x24`, `traffic_flows_160x48_detail`, `traffic_aggregate_80x24`, `traffic_blocked_80x24`, `light_traffic_160x48`, `help_overlay_80x24`, `help_overlay_end_80x24`, `help_overlay_end_light_styled_80x24`. Why they move: every Traffic snapshot's status hint now reads `f filter  r run  a auto  v view  g group  / search` (`w:rule  f filter  r run  v view  g group` with writes on) and its detail pane is 8 rows instead of 6 (two blank rows move); the flow tables show `src  src group  dst  dst group  service  conns  decision` at 80 columns (with `last seen` first at 160); Blocked shows `last seen  dst  dst name  service  flows  conns  sources` with meta `by dst app,env`; the help's `Traffic  g` line reads `pick the label keys every view groups by`.

Read the new ones: the flows table with `shop/prod` group cells and an IP list source as `-/-`; the picker box `Group by` with `[x] app`, `[x] env`, `[ ] role`, `[ ] loc`, `[ ] datacenter      (seen in 11 flows)`, `[ ] all labels`, `keys: app,env` and `Space toggle · Enter apply · Esc close`; Blocked with ten rows newest first (`hr/prod  hr-web-p.example  443/tcp … ipl:Corp/-`). In `traffic_flows_160x48_detail` the detail reads `src        10.1.4.9  10.1.4.9  ip lists: Corp`, `src labels -`, `dst        host-1.example  10.0.1.10`, `dst labels app=shop env=prod role=web`.

- [ ] **Step 6: Gate and commit**

```bash
cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace
git add crates/illumio-tui/src crates/illumio-tui/tests
git commit -m "feat(tui): label keys drive every traffic view; g opens a key picker; flows show group columns and bytes

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

Expected: 758 tests pass. If clippy flags the single-element loop in `w_opens_the_flow_form_only_for_a_blocked_flow`, the test edit above already replaces it with plain statements; Task 7 makes it a two-view loop again.

---

### Task 6: TUI Mesh view

**Files:**
- Modify: `crates/illumio-tui/src/tui/model.rs` (imports; `TrafficView` and a new `mesh_columns` after it; `TrafficState` fields, `Drill`, `MESH_ALL`, `TrafficState::new`, `rebuild_views`, new `toggle_mesh_column`, `is_searching`; tests)
- Modify: `crates/illumio-tui/src/tui/update.rs` (`traffic_key`: search arm, `u` / `p`, `Enter` on Mesh, table keys, `ensure_visible`; `scroll_focused`; `flow_rule_key`; tests)
- Modify: `crates/illumio-tui/src/tui/ui/screens/traffic.rs` (`render` dispatch, new `render_mesh`)
- Modify: `crates/illumio-tui/src/tui/ui/widgets/overlay.rs` (`HELP_LINES`: `Traffic  v`, `Traffic  Enter`, new `Mesh     u / p`, `Traffic  w`)
- Test: `crates/illumio-tui/tests/tui_snapshots.rs` (append)

**Interfaces:**
- Consumes: Task 3's `MeshColumns`, `MeshKey`, `MeshRow::{key, contains, dominant, service_text}`, `mesh`, `mesh_key`; Task 5's `Drill`, `TrafficState::{effective_keys, first_flow, rebuild_views}`, `FlowRow`, `model_with_rich_flows`, `visible_columns`.
- Produces:

```rust
// tui/model.rs
pub enum TrafficView { Flows, Aggregate, Mesh, Blocked }   // next(): Flows → Aggregate → Mesh → Blocked → Flows; title "Mesh"
pub const MESH_ALL: MeshColumns = MeshColumns { user: true, process: true };
pub fn mesh_columns(cols: MeshColumns) -> Vec<Column<MeshRow>>;
// user 12/8, process 14/10, src 18/14, dst 18/14, service 10/9, dst proc 12 (hide_below 120),
// conns 8/6 right, decision 11/8 (`pot.blocked` / narrow `pot.blk`); user and process only when shown
pub enum Drill { Pair(Vec<String>, Vec<String>), Mesh { key: MeshKey, cols: MeshColumns } }
pub struct TrafficState { /* … */ pub mesh: TableView<MeshRow>, pub mesh_cols: MeshColumns, .. }
impl TrafficState { pub fn toggle_mesh_column(&mut self, user: bool); }   // true: user, false: process
```

- [ ] **Step 1: Write the failing tests**

In `crates/illumio-tui/src/tui/model.rs`:

replace

```rust
    #[test]
    fn the_label_model_sets_the_group_keys_and_empty_falls_back_to_app_env() {
```

with

```rust
    #[test]
    fn mesh_columns_follow_the_toggles_and_fit_80_columns() {
        use crate::tui::ui::widgets::table::visible_columns;
        let titles = |cols: MeshColumns| -> Vec<&'static str> {
            mesh_columns(cols).iter().map(|c| c.title).collect()
        };
        assert_eq!(
            titles(MESH_ALL),
            vec!["user", "process", "src", "dst", "service", "dst proc", "conns", "decision"]
        );
        assert_eq!(
            titles(MeshColumns::default()),
            vec!["src", "dst", "service", "dst proc", "conns", "decision"]
        );
        let cols = mesh_columns(MESH_ALL);
        let at80 = visible_columns(&cols, 80);
        let width: u16 = at80.iter().map(|c| c.1).sum::<u16>() + at80.len() as u16 - 1 + 2;
        assert!(width <= 78, "{width}");
        assert!(!at80.iter().any(|c| cols[c.0].title == "dst proc"), "from 120 columns");
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
```

In `crates/illumio-tui/src/tui/update.rs`:

replace

```rust
    #[test]
    fn enter_in_aggregate_drills_into_the_pair_and_esc_clears() {
```

with

```rust
    #[test]
    fn mesh_sits_between_app_to_app_and_blocked_and_u_p_regroup_without_requests() {
        use crate::tui::test_support::model_with_rich_flows;
        let mut m = model_with_rich_flows();
        let mut seen = Vec::new();
        for _ in 0..4 {
            assert!(update(&mut m, key('v')).is_empty());
            seen.push(m.traffic.view);
        }
        assert_eq!(
            seen,
            vec![
                TrafficView::Aggregate,
                TrafficView::Mesh,
                TrafficView::Blocked,
                TrafficView::Flows
            ]
        );
        update(&mut m, key('v'));
        update(&mut m, key('v'));
        assert_eq!(m.traffic.view, TrafficView::Mesh);
        assert_eq!(m.traffic.mesh.len(), 30);
        update(&mut m, key('j'));
        update(&mut m, key('s'));
        assert!(update(&mut m, key('u')).is_empty(), "no request");
        assert!(!m.traffic.mesh_cols.user);
        assert_eq!(m.traffic.mesh.len(), 29);
        assert_eq!(m.traffic.mesh.selected_pos(), 0, "back on the first row");
        assert_eq!(m.traffic.mesh.sort(), None, "a new table");
        assert!(update(&mut m, key('p')).is_empty());
        assert_eq!(m.traffic.mesh.len(), 28);
        assert_eq!(m.traffic.mesh.columns[0].title, "src");
        update(&mut m, key('u'));
        update(&mut m, key('p'));
        assert_eq!(m.traffic.mesh.len(), 30);
        // u and p mean nothing on the other views.
        update(&mut m, key('v'));
        assert!(update(&mut m, key('u')).is_empty());
        assert!(m.traffic.mesh_cols.user);
    }

    #[test]
    fn enter_on_a_mesh_row_drills_to_its_flows_and_esc_clears() {
        use crate::tui::test_support::model_with_rich_flows;
        let mut m = model_with_rich_flows();
        m.traffic.view = TrafficView::Mesh;
        update(&mut m, key('u'));
        update(&mut m, key('p'));
        let row = m
            .traffic
            .mesh
            .rows()
            .iter()
            .position(|r| r.flows == 3)
            .unwrap();
        for _ in 0..row {
            update(&mut m, key('j'));
        }
        assert!(update(&mut m, enter()).is_empty());
        assert_eq!(m.traffic.view, TrafficView::Flows);
        assert_eq!(m.traffic.flows.len(), 3, "java, batch and python3 to shop-db-p");
        assert!(m.traffic.flows.rows().iter().all(|r| r.port == Some(5432)));
        assert!(matches!(m.traffic.drill, Some(Drill::Mesh { .. })));
        assert_eq!(
            m.traffic.drill.as_ref().unwrap().title(),
            "shop/prod → shop/prod 5432/tcp"
        );
        assert!(update(&mut m, kc(KeyCode::Esc)).is_empty());
        assert!(m.traffic.drill.is_none());
        assert_eq!(m.traffic.flows.len(), 30);
    }

    #[test]
    fn w_on_a_mesh_row_drafts_from_its_first_blocked_flow() {
        use crate::tui::test_support::model_with_rich_flows;
        let mut m = writes_on(model_with_rich_flows());
        m.traffic.view = TrafficView::Mesh;
        assert!(update(&mut m, key('w')).is_empty());
        assert_eq!(m.status.text, "this flow is allowed; nothing to add");
        let pos = m
            .traffic
            .mesh
            .rows()
            .iter()
            .position(|r| r.blocked > 0)
            .unwrap();
        for _ in 0..pos {
            update(&mut m, key('j'));
        }
        assert!(update(&mut m, key('w')).is_empty(), "no request");
        let Overlay::Form(f) = &m.overlay else {
            panic!("{:?}", m.overlay)
        };
        assert_eq!(f.title, "Allow rule from flow");
    }

    #[test]
    fn enter_in_aggregate_drills_into_the_pair_and_esc_clears() {
```

replace

```rust
        assert!(update(&mut m, key('v')).is_empty());
        assert_eq!(m.traffic.view, TrafficView::Blocked);
        assert_eq!(m.traffic.blocked.len(), 2);
```

with

```rust
        assert!(update(&mut m, key('v')).is_empty());
        assert_eq!(m.traffic.view, TrafficView::Mesh);
        assert!(update(&mut m, key('v')).is_empty());
        assert_eq!(m.traffic.view, TrafficView::Blocked);
        assert_eq!(m.traffic.blocked.len(), 2);
```

replace

```rust
    fn search_captures_keys_on_aggregate_and_blocked_views() {
        use crate::tui::test_support::model_with_flows;
        for presses in [1, 2] {
```

with

```rust
    fn search_captures_keys_on_aggregate_mesh_and_blocked_views() {
        use crate::tui::test_support::model_with_flows;
        for presses in [1, 2, 3] {
```

replace

```rust
                TrafficView::Aggregate => m.traffic.groups.search_text().to_string(),
                TrafficView::Blocked => m.traffic.blocked.search_text().to_string(),
```

with

```rust
                TrafficView::Aggregate => m.traffic.groups.search_text().to_string(),
                TrafficView::Mesh => m.traffic.mesh.search_text().to_string(),
                TrafficView::Blocked => m.traffic.blocked.search_text().to_string(),
```

replace

```rust
        assert_eq!(HELP_LINES.len(), 32);
```

with

```rust
        assert_eq!(HELP_LINES.len(), 33);
        assert!(HELP_LINES.iter().any(|(k, _)| *k == "Mesh     u / p"));
```

replace

```rust
        assert_eq!(last, 10, "32 lines, 22 rows inside the box");
```

with

```rust
        assert_eq!(last, 11, "33 lines, 22 rows inside the box");
```

Append to `crates/illumio-tui/tests/tui_snapshots.rs`:

```rust
#[test]
fn traffic_mesh_80x24_160x48_and_light() {
    use illumio_tui::tui::model::TrafficView;
    use illumio_tui::tui::test_support::model_with_rich_flows;
    let mut m = model_with_rich_flows();
    m.traffic.view = TrafficView::Mesh;
    let out = render_to_string(&m, 80, 24);
    assert!(out.contains("Mesh"), "{out}");
    assert!(out.contains("u user · p process · Enter drills"), "{out}");
    assert!(
        out.contains("www-data") && out.contains("ipl:Corp/-"),
        "{out}"
    );
    assert!(!out.contains("dst proc"), "from 120 columns: {out}");
    insta::assert_snapshot!("traffic_mesh_80x24", out);
    let out = render_to_string(&m, 160, 48);
    assert!(out.contains("dst proc") && out.contains("nginx"), "{out}");
    assert!(out.contains("pot.blocked"), "{out}");
    insta::assert_snapshot!("traffic_mesh_160x48", out);
    m.set_theme("light");
    insta::assert_snapshot!("light_traffic_mesh_160x48", render_styled(&m, 160, 48));
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p illumio-tui --lib 2>&1 | tail -5`
Expected: FAIL to compile (`TrafficView::Mesh`, `mesh_columns`, `MESH_ALL`, `Drill::Mesh`, `mesh_cols` do not exist).

- [ ] **Step 3: Implement**

In `crates/illumio-tui/src/tui/model.rs`:

replace

```rust
use pce::traffic::{
    BlockedGroup, FlowGroup, ResolvedFlow, blocked_summary, group_by_labels, group_key, group_text,
    keys_or_default, keys_seen,
};
```

with

```rust
use pce::traffic::{
    BlockedGroup, FlowGroup, MeshColumns, MeshKey, MeshRow, ResolvedFlow, blocked_summary,
    group_by_labels, group_key, group_text, keys_or_default, keys_seen, mesh, mesh_key,
};
```

replace

```rust
pub enum TrafficView {
    Flows,
    Aggregate,
    Blocked,
}

impl TrafficView {
    pub fn next(self) -> Self {
        match self {
            TrafficView::Flows => TrafficView::Aggregate,
            TrafficView::Aggregate => TrafficView::Blocked,
            TrafficView::Blocked => TrafficView::Flows,
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            TrafficView::Flows => "Flows",
            TrafficView::Aggregate => "App-to-app",
            TrafficView::Blocked => "Blocked",
        }
    }
}
```

with

```rust
pub enum TrafficView {
    Flows,
    Aggregate,
    Mesh,
    Blocked,
}

impl TrafficView {
    /// `v`: Flows, App-to-app, Mesh, Blocked, then Flows again.
    pub fn next(self) -> Self {
        match self {
            TrafficView::Flows => TrafficView::Aggregate,
            TrafficView::Aggregate => TrafficView::Mesh,
            TrafficView::Mesh => TrafficView::Blocked,
            TrafficView::Blocked => TrafficView::Flows,
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            TrafficView::Flows => "Flows",
            TrafficView::Aggregate => "App-to-app",
            TrafficView::Mesh => "Mesh",
            TrafficView::Blocked => "Blocked",
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
```

replace

```rust
    pub groups: TableView<FlowGroup>,
    pub blocked: TableView<BlockedGroup>,
    /// What the Flows view is narrowed to.
```

with

```rust
    pub groups: TableView<FlowGroup>,
    pub mesh: TableView<MeshRow>,
    /// The Mesh view's optional columns (`u`, `p`); both shown at first.
    pub mesh_cols: MeshColumns,
    pub blocked: TableView<BlockedGroup>,
    /// What the Flows view is narrowed to.
```

replace

```rust
    /// An App-to-app or treemap pair: source and destination group keys.
    Pair(Vec<String>, Vec<String>),
}
```

with

```rust
    /// An App-to-app or treemap pair: source and destination group keys.
    Pair(Vec<String>, Vec<String>),
    /// A Mesh row, under the columns it was grouped with.
    Mesh { key: MeshKey, cols: MeshColumns },
}
```

replace

```rust
            Drill::Pair(src, dst) => {
                &group_key(&f.src, keys) == src && &group_key(&f.dst, keys) == dst
            }
        }
```

with

```rust
            Drill::Pair(src, dst) => {
                &group_key(&f.src, keys) == src && &group_key(&f.dst, keys) == dst
            }
            Drill::Mesh { key, cols } => &mesh_key(f, keys, *cols) == key,
        }
```

replace

```rust
            Drill::Pair(src, dst) => format!("{} → {}", src.join("/"), dst.join("/")),
        }
```

with

```rust
            Drill::Pair(src, dst) => format!("{} → {}", src.join("/"), dst.join("/")),
            Drill::Mesh { key, .. } => format!(
                "{} → {} {}",
                key.src.join("/"),
                key.dst.join("/"),
                key.service_text()
            ),
        }
```

replace

```rust
            groups: TableView::new(group_columns()),
            blocked: TableView::new(blocked_columns()),
```

with

```rust
            groups: TableView::new(group_columns()),
            mesh: TableView::new(mesh_columns(MESH_ALL)),
            mesh_cols: MESH_ALL,
            blocked: TableView::new(blocked_columns()),
```

replace

```rust
/// The keys `all labels` lists first when the flows carry them.
```

with

```rust
/// Both optional Mesh columns shown.
pub const MESH_ALL: MeshColumns = MeshColumns {
    user: true,
    process: true,
};

/// The keys `all labels` lists first when the flows carry them.
```

replace

```rust
        let groups = group_by_labels(flows, &keys);
        let blocked = blocked_summary(flows, &keys);
        self.flows.set_rows(shown);
        self.groups.set_rows(groups);
        self.blocked.set_rows(blocked);
    }
```

with

```rust
        let groups = group_by_labels(flows, &keys);
        let mesh_rows = mesh(flows, &keys, self.mesh_cols);
        let blocked = blocked_summary(flows, &keys);
        self.flows.set_rows(shown);
        self.groups.set_rows(groups);
        self.mesh.set_rows(mesh_rows);
        self.blocked.set_rows(blocked);
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
```

replace

```rust
            TrafficView::Aggregate => self.groups.is_searching(),
            TrafficView::Blocked => self.blocked.is_searching(),
```

with

```rust
            TrafficView::Aggregate => self.groups.is_searching(),
            TrafficView::Mesh => self.mesh.is_searching(),
            TrafficView::Blocked => self.blocked.is_searching(),
```

In `crates/illumio-tui/src/tui/update.rs`:

replace

```rust
            TrafficView::Aggregate => {
                search_key(&mut t.groups, k);
                t.groups.ensure_visible(page);
            }
```

with

```rust
            TrafficView::Aggregate => {
                search_key(&mut t.groups, k);
                t.groups.ensure_visible(page);
            }
            TrafficView::Mesh => {
                search_key(&mut t.mesh, k);
                t.mesh.ensure_visible(page);
            }
```

replace

```rust
        KeyCode::Enter => match model.traffic.view {
            TrafficView::Aggregate => {
                if let Some(g) = model.traffic.groups.selected_item() {
                    model.traffic.drill = Some(Drill::Pair(g.src.clone(), g.dst.clone()));
                    model.traffic.view = TrafficView::Flows;
                    model.traffic.rebuild_views();
                }
            }
```

with

```rust
        KeyCode::Char(c @ ('u' | 'p')) if model.traffic.view == TrafficView::Mesh => {
            model.traffic.toggle_mesh_column(c == 'u');
        }
        KeyCode::Enter => match model.traffic.view {
            TrafficView::Aggregate => {
                if let Some(g) = model.traffic.groups.selected_item() {
                    model.traffic.drill = Some(Drill::Pair(g.src.clone(), g.dst.clone()));
                    model.traffic.view = TrafficView::Flows;
                    model.traffic.rebuild_views();
                }
            }
            TrafficView::Mesh => {
                let t = &mut model.traffic;
                if let Some(r) = t.mesh.selected_item() {
                    t.drill = Some(Drill::Mesh {
                        key: r.key(),
                        cols: t.mesh_cols,
                    });
                    t.view = TrafficView::Flows;
                    t.rebuild_views();
                }
            }
```

replace

```rust
                TrafficView::Flows => table_key(&mut t.flows, k, page),
                TrafficView::Aggregate => table_key(&mut t.groups, k, page),
                TrafficView::Blocked => table_key(&mut t.blocked, k, page),
            }
        }
    }
    let t = &mut model.traffic;
    match t.view {
        TrafficView::Flows => t.flows.ensure_visible(page),
        TrafficView::Aggregate => t.groups.ensure_visible(page),
        TrafficView::Blocked => t.blocked.ensure_visible(page),
    }
```

with

```rust
                TrafficView::Flows => table_key(&mut t.flows, k, page),
                TrafficView::Aggregate => table_key(&mut t.groups, k, page),
                TrafficView::Mesh => table_key(&mut t.mesh, k, page),
                TrafficView::Blocked => table_key(&mut t.blocked, k, page),
            }
        }
    }
    let t = &mut model.traffic;
    match t.view {
        TrafficView::Flows => t.flows.ensure_visible(page),
        TrafficView::Aggregate => t.groups.ensure_visible(page),
        TrafficView::Mesh => t.mesh.ensure_visible(page),
        TrafficView::Blocked => t.blocked.ensure_visible(page),
    }
```

replace

```rust
                TrafficView::Flows => step(&mut t.flows, down, page),
                TrafficView::Aggregate => step(&mut t.groups, down, page),
                TrafficView::Blocked => step(&mut t.blocked, down, page),
```

with

```rust
                TrafficView::Flows => step(&mut t.flows, down, page),
                TrafficView::Aggregate => step(&mut t.groups, down, page),
                TrafficView::Mesh => step(&mut t.mesh, down, page),
                TrafficView::Blocked => step(&mut t.blocked, down, page),
```

replace

```rust
        TrafficView::Blocked => t.blocked.selected_item().and_then(|b| {
            let keys = t.effective_keys();
            t.first_flow(|f| b.contains(f, &keys))
        }),
```

with

```rust
        TrafficView::Blocked => t.blocked.selected_item().and_then(|b| {
            let keys = t.effective_keys();
            t.first_flow(|f| b.contains(f, &keys))
        }),
        TrafficView::Mesh => t.mesh.selected_item().and_then(|r| {
            let keys = t.effective_keys();
            t.first_flow(|f| r.contains(f, &keys, t.mesh_cols))
        }),
```

replace

```rust
/// `w` on Traffic: a draft allow rule from the selected blocked or
/// potentially blocked flow (Flows view or a drilled pair), or from the
/// first such flow behind a Blocked row. App-to-app refuses: a pair holds
/// flows of every decision.
```

with

```rust
/// `w` on Traffic: a draft allow rule from the selected blocked or
/// potentially blocked flow (Flows view or a drilled pair), or from the
/// first such flow behind a Blocked or Mesh row. App-to-app refuses: a
/// pair holds flows of every decision.
```

In `crates/illumio-tui/src/tui/ui/screens/traffic.rs`:

replace

```rust
        TrafficView::Aggregate => render_groups(model, f, l.table.union(l.detail)),
```

with

```rust
        TrafficView::Aggregate => render_groups(model, f, l.table.union(l.detail)),
        TrafficView::Mesh => render_mesh(model, f, l.table.union(l.detail)),
```

replace

```rust
/// Every row here is blocked or potentially blocked; both read as `bad`.
```

with

```rust
/// Rows are styled by their dominant decision, like App-to-app.
fn render_mesh(model: &Model, f: &mut Frame, area: Rect) {
    let t = &model.theme;
    let meta = crate::tui::ui::widgets::table::header_meta(&model.traffic.mesh);
    let meta = format!(
        "keys {} · u user · p process · Enter drills{}{}",
        model.traffic.effective_keys().join(","),
        if meta.is_empty() { "" } else { " · " },
        meta
    );
    let block = panel(TrafficView::Mesh.title(), &meta, t, true);
    let theme = t.clone();
    crate::tui::ui::widgets::table::render(f, area, &model.traffic.mesh, t, block, &move |r| {
        theme.decision_style(r.dominant())
    });
}

/// Every row here is blocked or potentially blocked; both read as `bad`.
```

In `crates/illumio-tui/src/tui/ui/widgets/overlay.rs`:

replace

```rust
    ("Traffic  v", "cycle Flows · App-to-app · Blocked"),
```

with

```rust
    ("Traffic  v", "cycle Flows · App-to-app · Mesh · Blocked"),
```

replace

```rust
    (
        "Traffic  Enter",
        "detail (flows) · drill into a pair (aggregate)",
    ),
```

with

```rust
    (
        "Traffic  Enter",
        "detail (flows) · drill into a row (app-to-app, mesh)",
    ),
    ("Mesh     u / p", "show or hide the user · process columns"),
```

replace

```rust
        "Traffic  w",
        "draft allow rule from a blocked flow (writes)",
```

with

```rust
        "Traffic  w",
        "draft allow rule from a blocked flow or row (writes)",
```

- [ ] **Step 4: Run the unit tests**

Run: `cargo fmt --all && cargo test -p illumio-tui --lib`
Expected: PASS, including `mesh_sits_between_app_to_app_and_blocked_and_u_p_regroup_without_requests`, `enter_on_a_mesh_row_drills_to_its_flows_and_esc_clears`, `w_on_a_mesh_row_drafts_from_its_first_blocked_flow`, `mesh_columns_follow_the_toggles_and_fit_80_columns`, the updated `v_cycles_views_and_g_sets_group_keys_without_requests`, `search_captures_keys_on_aggregate_mesh_and_blocked_views` and `help_scrolls_to_its_last_line_and_reopens_at_the_top` (33 lines).

- [ ] **Step 5: Create and read the snapshots**

```bash
INSTA_UPDATE=always cargo test -p illumio-tui --test tui_snapshots
find crates/illumio-tui/tests/snapshots -name '*.snap.new' -delete
git status --short crates/illumio-tui/tests/snapshots
```

Expected: 3 new (`traffic_mesh_80x24`, `traffic_mesh_160x48`, `light_traffic_mesh_160x48`) and the 3 help overlays modified (the `v` line `cycle Flows · App-to-app · Mesh · Blocked`, the new `Mesh     u / p` line, the scroll range `of 33`). Read the mesh ones: panel `Mesh` with meta `keys app,env · u user · p process · Enter drills`; at 80 columns the header `user     process    src            dst            service   conns  decision` and the first row `www-data -          ipl:Corp/-     shop/prod      443/tcp      700 allowed`, no `dst proc`; at 160 the `dst proc` column (`nginx`, `mysqld`, `node`, `postgres`, …) and `pot.blocked`; the styled light snapshot colours rows by their dominant decision.

- [ ] **Step 6: Gate and commit**

```bash
cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace
git add crates/illumio-tui/src crates/illumio-tui/tests
git commit -m "feat(tui): Mesh view (user, process, source, destination, service, destination process) with u/p toggles

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

Expected: 763 tests pass.

---

### Task 7: TUI Treemap view

**Files:**
- Modify: `crates/illumio-tui/src/tui/model.rs` (imports; `TrafficView::Treemap`; new `TreemapTarget`, `Dir`, `TreemapState` after `TrafficView`; `TrafficState.treemap`, `new`, `rebuild_views`, new `relayout_treemap`, `is_searching`; `Model::new_connecting`, `reset_data`)
- Modify: `crates/illumio-tui/src/tui/update.rs` (imports; `Msg::Resize`; `Msg::QueryDone` drops the zoom; `traffic_key`: search arm, `Enter`, `Esc`, `b`, direction keys, `ensure_visible`; new `treemap_enter`, `treemap_out`; `scroll_focused`; `flow_rule_key`; `apply_group_keys`; tests)
- Modify: `crates/illumio-tui/src/tui/ui/screens/traffic.rs` (imports; new `treemap_area`; `render` dispatch; new `render_treemap`, `cells`, `clip_text`, `draw_fill`)
- Modify: `crates/illumio-tui/src/tui/ui/widgets/overlay.rs` (`HELP_LINES`: `Traffic  v`, new `Treemap  hjkl Enter b`)
- Test: `crates/illumio-tui/tests/tui_snapshots.rs` (append)

**Interfaces:**
- Consumes: Task 4's `treemap::{Rect, Tile, Weight, layout, layout_zoomed}`, `Tile::is_other`, `Rect::{area, inner}`; Task 5's `TrafficState::{has_bytes, rebuild_views}`, `Drill::Pair`, `apply_group_keys`; Task 6's `TrafficView` cycle; `compact_count`; `Theme::decision_style`; `ui::layout::chrome`, `screens::traffic::layout`.
- Produces:

```rust
// tui/model.rs
pub enum TrafficView { Flows, Aggregate, Mesh, Blocked, Treemap }   // next(): … Blocked → Treemap → Flows; title "Treemap"
#[derive(Debug, Clone, Copy, PartialEq, Eq)] pub struct TreemapTarget { pub outer: usize, pub inner: Option<usize>, pub rect: TreemapRect }
#[derive(Debug, Clone, Copy, PartialEq, Eq)] pub enum Dir { Left, Right, Up, Down }
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TreemapState { pub weight: Weight, pub zoom: Option<Vec<String>>, pub cursor: usize, pub tiles: Vec<Tile>, pub area: TreemapRect }
impl TreemapState {
    pub fn new(area: TreemapRect) -> Self;
    pub fn targets(&self) -> Vec<TreemapTarget>;      // each outer tile, then its destinations
    pub fn current(&self) -> Option<TreemapTarget>;
    pub fn tile(&self, t: TreemapTarget) -> &Tile;
    pub fn move_cursor(&mut self, dir: Dir);
}
pub struct TrafficState { /* … */ pub treemap: TreemapState, .. }
impl TrafficState { pub fn relayout_treemap(&mut self); }   // called by rebuild_views and on resize
// (`TreemapRect` is `pce::traffic::treemap::Rect`, imported under that name in model.rs and traffic.rs)
// tui/ui/screens/traffic.rs
pub fn treemap_area(size: (u16, u16)) -> TreemapRect;   // (1, 9, 78, 13) at 80x24, (1, 9, 158, 37) at 160x48
// tui/update.rs (private)
fn treemap_enter(model: &mut Model);
fn treemap_out(model: &mut Model);
```

- [ ] **Step 1: Write the failing tests**

In `crates/illumio-tui/src/tui/update.rs`:

replace

```rust
        assert_eq!(HELP_LINES.len(), 33);
```

with

```rust
        assert_eq!(HELP_LINES.len(), 34);
        assert!(HELP_LINES.iter().any(|(k, _)| *k == "Treemap  hjkl Enter b"));
```

replace

```rust
        assert_eq!(last, 11, "33 lines, 22 rows inside the box");
```

with

```rust
        assert_eq!(last, 12, "34 lines, 22 rows inside the box");
```

replace

```rust
        m.traffic.view = TrafficView::Aggregate;
        assert!(update(&mut m, key('w')).is_empty());
        assert_eq!(
            m.status.text,
            "select a flow in the Flows view (Enter on a pair)"
        );
        assert_eq!(m.overlay, Overlay::None);
```

with

```rust
        for view in [TrafficView::Aggregate, TrafficView::Treemap] {
            m.traffic.view = view;
            assert!(update(&mut m, key('w')).is_empty());
            assert_eq!(
                m.status.text,
                "select a flow in the Flows view (Enter on a pair)"
            );
            assert_eq!(m.overlay, Overlay::None);
        }
```

replace

```rust
    #[test]
    fn enter_in_aggregate_drills_into_the_pair_and_esc_clears() {
```

with

```rust
    fn on_treemap(w: u16, h: u16) -> Model {
        let mut m = crate::tui::test_support::model_with_rich_flows();
        assert!(update(&mut m, Msg::Resize(w, h)).is_empty());
        for _ in 0..4 {
            assert!(update(&mut m, key('v')).is_empty());
        }
        assert_eq!(m.traffic.view, TrafficView::Treemap);
        m
    }

    #[test]
    fn the_treemap_follows_the_terminal_size_and_v_comes_back_to_flows() {
        use pce::traffic::treemap::Rect;
        let mut m = on_treemap(80, 24);
        let tm = &m.traffic.treemap;
        assert_eq!(tm.area, Rect { x: 1, y: 9, w: 78, h: 13 });
        assert_eq!(tm.tiles.len(), 9);
        assert_eq!(tm.tiles[0].label, "ipl:Corp/-");
        assert_eq!(tm.cursor, 0);
        assert!(update(&mut m, Msg::Resize(160, 48)).is_empty());
        assert_eq!(
            m.traffic.treemap.area,
            Rect { x: 1, y: 9, w: 158, h: 37 }
        );
        let covered: u32 = m.traffic.treemap.tiles.iter().map(|t| t.rect.area()).sum();
        assert_eq!(covered, 158 * 37);
        assert!(update(&mut m, key('v')).is_empty());
        assert_eq!(m.traffic.view, TrafficView::Flows);
        // No flows: no tiles, and the keys still cost nothing.
        let mut empty = model_fixture();
        empty.traffic.view = TrafficView::Treemap;
        for msg in [key('l'), enter(), kc(KeyCode::Esc), key('b')] {
            assert!(update(&mut empty, msg).is_empty());
        }
        assert!(empty.traffic.treemap.tiles.is_empty());
    }

    #[test]
    fn the_treemap_cursor_moves_towards_the_key_and_stays_at_an_edge() {
        let mut m = on_treemap(80, 24);
        let centre_x = |m: &Model| {
            let r = m.traffic.treemap.current().unwrap().rect;
            2 * u32::from(r.x) + u32::from(r.w)
        };
        let start = centre_x(&m);
        assert!(update(&mut m, key('l')).is_empty());
        let right = centre_x(&m);
        assert!(right > start, "{start} -> {right}");
        assert!(update(&mut m, kc(KeyCode::Left)).is_empty());
        assert!(centre_x(&m) < right);
        for _ in 0..40 {
            update(&mut m, key('h'));
        }
        let leftmost = m.traffic.treemap.cursor;
        update(&mut m, key('h'));
        assert_eq!(m.traffic.treemap.cursor, leftmost, "nothing further left");
        for _ in 0..40 {
            update(&mut m, key('j'));
        }
        let r = m.traffic.treemap.current().unwrap().rect;
        assert_eq!(r.y + r.h, 22, "the bottom row of the area");
    }

    #[test]
    fn enter_zooms_into_a_source_then_drills_and_esc_zooms_out() {
        let mut m = on_treemap(80, 24);
        let shop = m
            .traffic
            .treemap
            .targets()
            .iter()
            .position(|t| t.inner.is_none() && m.traffic.treemap.tiles[t.outer].label == "shop/prod")
            .unwrap();
        m.traffic.treemap.cursor = shop;
        assert!(update(&mut m, enter()).is_empty());
        assert_eq!(
            m.traffic.treemap.zoom,
            Some(vec!["shop".to_string(), "prod".to_string()])
        );
        let labels: Vec<&str> = m.traffic.treemap.tiles.iter().map(|t| t.label.as_str()).collect();
        assert_eq!(labels, vec!["shop/prod", "crm/prod", "api.partner.example/-"]);
        assert!(update(&mut m, kc(KeyCode::Esc)).is_empty());
        assert_eq!(m.traffic.treemap.zoom, None);
        assert_eq!(m.traffic.treemap.cursor, shop, "back on the source");
        update(&mut m, enter());
        assert!(update(&mut m, enter()).is_empty(), "the largest destination");
        assert_eq!(m.traffic.view, TrafficView::Flows);
        assert_eq!(
            m.traffic.drill,
            Some(Drill::Pair(
                vec!["shop".into(), "prod".into()],
                vec!["shop".into(), "prod".into()]
            ))
        );
        assert_eq!(m.traffic.flows.len(), 4);
        // An inner tile drills straight away; an `other` tile only explains.
        let mut m = on_treemap(80, 24);
        let tm = &m.traffic.treemap;
        let crm = tm
            .targets()
            .iter()
            .position(|t| {
                t.inner.is_some()
                    && tm.tiles[t.outer].label == "ipl:Corp/-"
                    && tm.tile(*t).label == "crm/prod"
            })
            .unwrap();
        let other = tm
            .targets()
            .iter()
            .position(|t| t.inner.is_some() && tm.tile(*t).is_other())
            .unwrap();
        m.traffic.treemap.cursor = other;
        assert!(update(&mut m, enter()).is_empty());
        assert!(m.status.text.starts_with("other (2): too small"), "{}", m.status.text);
        assert_eq!(m.traffic.view, TrafficView::Treemap);
        m.traffic.treemap.cursor = crm;
        update(&mut m, enter());
        assert_eq!(m.traffic.view, TrafficView::Flows);
        assert_eq!(m.traffic.flows.len(), 1, "Corp to crm/prod");
    }

    #[test]
    fn b_weighs_by_bytes_or_says_there_are_none() {
        use pce::traffic::treemap::Weight;
        let mut m = on_treemap(80, 24);
        assert!(update(&mut m, key('b')).is_empty());
        assert_eq!(m.traffic.treemap.weight, Weight::Bytes);
        assert_eq!(m.traffic.treemap.tiles[0].weight, 8_350_000);
        assert!(update(&mut m, key('b')).is_empty());
        assert_eq!(m.traffic.treemap.weight, Weight::Connections);
        if let Some(d) = m.traffic.data.as_mut() {
            for f in &mut d.flows {
                f.bytes_in = None;
                f.bytes_out = None;
            }
        }
        m.traffic.rebuild_views();
        assert!(update(&mut m, key('b')).is_empty());
        assert_eq!(m.status.text, "no byte counts in this result");
        assert_eq!(m.traffic.treemap.weight, Weight::Connections);
    }

    #[test]
    fn the_picked_keys_reach_the_mesh_and_the_treemap() {
        let mut m = on_treemap(80, 24);
        for msg in [key('g'), key('j'), key('j'), key(' '), enter()] {
            assert!(update(&mut m, msg).is_empty());
        }
        assert_eq!(m.traffic.group_keys, vec!["app", "env", "role"]);
        assert!(m.traffic.mesh.rows().iter().all(|r| r.src.len() == 3));
        assert_eq!(m.traffic.treemap.tiles[0].label, "ipl:Corp/-/-");
        assert!(
            m.traffic
                .treemap
                .tiles
                .iter()
                .any(|t| t.label == "shop/prod/web"),
            "sources by app, env and role"
        );
    }

    #[test]
    fn new_keys_and_a_new_result_drop_the_zoom() {
        let mut m = on_treemap(80, 24);
        update(&mut m, enter());
        assert!(m.traffic.treemap.zoom.is_some());
        update(&mut m, key('g'));
        update(&mut m, enter());
        assert_eq!(m.traffic.treemap.zoom, None, "applying keys zooms out");
        update(&mut m, enter());
        assert!(m.traffic.treemap.zoom.is_some());
        let mut keys_role = m.traffic.group_keys.clone();
        keys_role.push("role".into());
        m.traffic.group_keys = keys_role;
        m.traffic.rebuild_views();
        assert_eq!(m.traffic.treemap.zoom, None, "the zoomed source no longer exists");
    }

    #[test]
    fn enter_in_aggregate_drills_into_the_pair_and_esc_clears() {
```

replace

```rust
                TrafficView::Flows => unreachable!("v moved off Flows"),
```

with

```rust
                TrafficView::Flows | TrafficView::Treemap => unreachable!("v moved off Flows"),
```

replace

```rust
        let mut seen = Vec::new();
        for _ in 0..4 {
            assert!(update(&mut m, key('v')).is_empty());
            seen.push(m.traffic.view);
        }
        assert_eq!(
            seen,
            vec![
                TrafficView::Aggregate,
                TrafficView::Mesh,
                TrafficView::Blocked,
                TrafficView::Flows
            ]
        );
```

with

```rust
        let mut seen = Vec::new();
        for _ in 0..5 {
            assert!(update(&mut m, key('v')).is_empty());
            seen.push(m.traffic.view);
        }
        assert_eq!(
            seen,
            vec![
                TrafficView::Aggregate,
                TrafficView::Mesh,
                TrafficView::Blocked,
                TrafficView::Treemap,
                TrafficView::Flows
            ]
        );
```

replace

```rust
        assert_eq!(m.traffic.blocked.len(), 2);
        assert!(update(&mut m, key('v')).is_empty());
        assert_eq!(m.traffic.view, TrafficView::Flows);
```

with

```rust
        assert_eq!(m.traffic.blocked.len(), 2);
        assert!(update(&mut m, key('v')).is_empty());
        assert_eq!(m.traffic.view, TrafficView::Treemap);
        assert!(update(&mut m, key('v')).is_empty());
        assert_eq!(m.traffic.view, TrafficView::Flows);
```

Append to `crates/illumio-tui/tests/tui_snapshots.rs`:

```rust
#[test]
fn traffic_treemap_80x24_160x48_zoomed_and_light() {
    use illumio_tui::tui::model::TrafficView;
    use illumio_tui::tui::msg::Msg;
    use illumio_tui::tui::test_support::model_with_rich_flows;
    use illumio_tui::tui::update;
    let mut m = model_with_rich_flows();
    m.traffic.view = TrafficView::Treemap;
    assert!(update(&mut m, Msg::Resize(80, 24)).is_empty());
    let out = render_to_string(&m, 80, 24);
    assert!(out.contains("Treemap"), "{out}");
    assert!(
        out.contains("by conns · b bytes · Enter zoom · Esc out"),
        "{out}"
    );
    assert!(out.contains("ipl:Corp/-") && out.contains("▒"), "{out}");
    insta::assert_snapshot!("traffic_treemap_80x24", out);
    assert!(update(&mut m, Msg::Resize(160, 48)).is_empty());
    let out = render_to_string(&m, 160, 48);
    assert!(
        out.contains("702") && out.contains("821"),
        "destination weights: {out}"
    );
    insta::assert_snapshot!("traffic_treemap_160x48", out);
    m.set_theme("light");
    insta::assert_snapshot!("light_traffic_treemap_160x48", render_styled(&m, 160, 48));
    m.set_theme("dark");
    assert!(update(&mut m, Msg::Resize(80, 24)).is_empty());
    m.traffic.treemap.zoom = Some(vec!["shop".into(), "prod".into()]);
    m.traffic.relayout_treemap();
    let out = render_to_string(&m, 80, 24);
    assert!(out.contains("Treemap ▸ shop/prod"), "{out}");
    assert!(
        out.contains("▒crm/prod▒") && out.contains("▒33▒"),
        "the destinations: {out}"
    );
    insta::assert_snapshot!("traffic_treemap_zoomed_80x24", out);
}

#[test]
fn an_empty_result_draws_the_treemap_like_an_empty_table() {
    use illumio_tui::tui::model::TrafficView;
    let mut m = model_fixture();
    m.traffic.view = TrafficView::Treemap;
    let out = render_to_string(&m, 80, 24);
    assert!(out.contains("Treemap") && out.contains("no rows"), "{out}");
}

#[test]
fn mesh_and_treemap_keep_the_truncated_marker() {
    use illumio_tui::tui::model::TrafficView;
    use illumio_tui::tui::test_support::model_with_rich_flows;
    let mut m = model_with_rich_flows();
    m.traffic.data.as_mut().unwrap().truncated = true;
    for view in [TrafficView::Mesh, TrafficView::Treemap] {
        m.traffic.view = view;
        let out = render_to_string(&m, 160, 48);
        assert!(out.contains("⚠ truncated"), "{view:?}: {out}");
    }
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p illumio-tui --lib 2>&1 | tail -5`
Expected: FAIL to compile (`TrafficView::Treemap`, `TreemapState`, `treemap`, `Dir`, `relayout_treemap` do not exist).

- [ ] **Step 3: Implement**

In `crates/illumio-tui/src/tui/model.rs`:

replace

```rust
use pce::traffic::{
    BlockedGroup, FlowGroup, MeshColumns, MeshKey, MeshRow, ResolvedFlow, blocked_summary,
    group_by_labels, group_key, group_text, keys_or_default, keys_seen, mesh, mesh_key,
};
```

with

```rust
use pce::traffic::treemap::{Rect as TreemapRect, Tile, Weight, layout, layout_zoomed};
use pce::traffic::{
    BlockedGroup, FlowGroup, MeshColumns, MeshKey, MeshRow, ResolvedFlow, blocked_summary,
    group_by_labels, group_key, group_text, keys_or_default, keys_seen, mesh, mesh_key,
};
```

replace

```rust
    Mesh,
    Blocked,
}

impl TrafficView {
    /// `v`: Flows, App-to-app, Mesh, Blocked, then Flows again.
    pub fn next(self) -> Self {
        match self {
            TrafficView::Flows => TrafficView::Aggregate,
            TrafficView::Aggregate => TrafficView::Mesh,
            TrafficView::Mesh => TrafficView::Blocked,
            TrafficView::Blocked => TrafficView::Flows,
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            TrafficView::Flows => "Flows",
            TrafficView::Aggregate => "App-to-app",
            TrafficView::Mesh => "Mesh",
            TrafficView::Blocked => "Blocked",
        }
    }
}
```

with

```rust
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
```

replace

```rust
    pub mesh_cols: MeshColumns,
    pub blocked: TableView<BlockedGroup>,
```

with

```rust
    pub mesh_cols: MeshColumns,
    pub blocked: TableView<BlockedGroup>,
    pub treemap: TreemapState,
```

replace

```rust
            blocked: TableView::new(blocked_columns()),
            drill: None,
```

with

```rust
            blocked: TableView::new(blocked_columns()),
            treemap: TreemapState::new(TreemapRect::default()),
            drill: None,
```

replace

```rust
        self.flows.set_rows(shown);
        self.groups.set_rows(groups);
        self.mesh.set_rows(mesh_rows);
        self.blocked.set_rows(blocked);
    }
```

with

```rust
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
        if !self.has_bytes() {
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
```

replace

```rust
            TrafficView::Mesh => self.mesh.is_searching(),
            TrafficView::Blocked => self.blocked.is_searching(),
        }
```

with

```rust
            TrafficView::Mesh => self.mesh.is_searching(),
            TrafficView::Blocked => self.blocked.is_searching(),
            TrafficView::Treemap => false,
        }
```

replace

```rust
        let traffic = TrafficState::new(
            &traffic_default_window,
            traffic_max_results,
            auto_refresh_min_seconds,
            &scope_keys,
        );
```

with

```rust
        let size = (80, 24);
        let mut traffic = TrafficState::new(
            &traffic_default_window,
            traffic_max_results,
            auto_refresh_min_seconds,
            &scope_keys,
        );
        traffic.treemap.area = crate::tui::ui::screens::traffic::treemap_area(size);
```

replace

```rust
            size: (80, 24),
```

with

```rust
            size,
```

replace

```rust
        traffic.seq = self.traffic.seq + 1; // a late result of the old query is stale
```

with

```rust
        traffic.seq = self.traffic.seq + 1; // a late result of the old query is stale
        traffic.treemap.area = crate::tui::ui::screens::traffic::treemap_area(self.size);
```

In `crates/illumio-tui/src/tui/update.rs`:

replace

```rust
            TrafficView::Blocked => {
                search_key(&mut t.blocked, k);
                t.blocked.ensure_visible(page);
            }
        }
        return Vec::new();
```

with

```rust
            TrafficView::Blocked => {
                search_key(&mut t.blocked, k);
                t.blocked.ensure_visible(page);
            }
            // `is_searching` is false on the treemap: it has no table.
            TrafficView::Treemap => {}
        }
        return Vec::new();
```

replace

```rust
        Msg::Resize(w, h) => {
            model.size = (w, h);
            Vec::new()
        }
```

with

```rust
        Msg::Resize(w, h) => {
            model.size = (w, h);
            let area = crate::tui::ui::screens::traffic::treemap_area(model.size);
            if area != model.traffic.treemap.area {
                model.traffic.treemap.area = area;
                model.traffic.relayout_treemap();
            }
            Vec::new()
        }
```

replace

```rust
                    model.traffic.drill = None;
                    model.traffic.rebuild_views();
                    model.traffic.show_detail = false;
```

with

```rust
                    model.traffic.drill = None;
                    model.traffic.treemap.zoom = None;
                    model.traffic.rebuild_views();
                    model.traffic.show_detail = false;
```

replace

```rust
            TrafficView::Flows => model.traffic.show_detail = !model.traffic.show_detail,
            TrafficView::Blocked => {}
        },
        KeyCode::Esc | KeyCode::Backspace
            if model.traffic.view == TrafficView::Flows && model.traffic.drill.is_some() =>
        {
            model.traffic.drill = None;
            model.traffic.rebuild_views();
        }
        _ => {
            let t = &mut model.traffic;
            match t.view {
                TrafficView::Flows => table_key(&mut t.flows, k, page),
                TrafficView::Aggregate => table_key(&mut t.groups, k, page),
                TrafficView::Mesh => table_key(&mut t.mesh, k, page),
                TrafficView::Blocked => table_key(&mut t.blocked, k, page),
            }
        }
    }
    let t = &mut model.traffic;
    match t.view {
        TrafficView::Flows => t.flows.ensure_visible(page),
        TrafficView::Aggregate => t.groups.ensure_visible(page),
        TrafficView::Mesh => t.mesh.ensure_visible(page),
        TrafficView::Blocked => t.blocked.ensure_visible(page),
    }
    Vec::new()
}
```

with

```rust
            TrafficView::Flows => model.traffic.show_detail = !model.traffic.show_detail,
            TrafficView::Blocked => {}
            TrafficView::Treemap => treemap_enter(model),
        },
        KeyCode::Esc | KeyCode::Backspace
            if model.traffic.view == TrafficView::Flows && model.traffic.drill.is_some() =>
        {
            model.traffic.drill = None;
            model.traffic.rebuild_views();
        }
        KeyCode::Esc if model.traffic.view == TrafficView::Treemap => treemap_out(model),
        KeyCode::Char('b') if model.traffic.view == TrafficView::Treemap => {
            let t = &mut model.traffic;
            if !t.has_bytes() {
                model.status = StatusLine::info("no byte counts in this result");
            } else {
                t.treemap.weight = match t.treemap.weight {
                    Weight::Connections => Weight::Bytes,
                    Weight::Bytes => Weight::Connections,
                };
                t.relayout_treemap();
            }
        }
        _ => {
            let t = &mut model.traffic;
            match t.view {
                TrafficView::Flows => table_key(&mut t.flows, k, page),
                TrafficView::Aggregate => table_key(&mut t.groups, k, page),
                TrafficView::Mesh => table_key(&mut t.mesh, k, page),
                TrafficView::Blocked => table_key(&mut t.blocked, k, page),
                TrafficView::Treemap => {
                    let dir = match k.code {
                        KeyCode::Left | KeyCode::Char('h') => Some(Dir::Left),
                        KeyCode::Right | KeyCode::Char('l') => Some(Dir::Right),
                        KeyCode::Up | KeyCode::Char('k') => Some(Dir::Up),
                        KeyCode::Down | KeyCode::Char('j') => Some(Dir::Down),
                        _ => None,
                    };
                    if let Some(d) = dir {
                        t.treemap.move_cursor(d);
                    }
                }
            }
        }
    }
    let t = &mut model.traffic;
    match t.view {
        TrafficView::Flows => t.flows.ensure_visible(page),
        TrafficView::Aggregate => t.groups.ensure_visible(page),
        TrafficView::Mesh => t.mesh.ensure_visible(page),
        TrafficView::Blocked => t.blocked.ensure_visible(page),
        TrafficView::Treemap => {}
    }
    Vec::new()
}

/// Enter on the Treemap: an outer tile zooms into its source; a
/// destination (or any tile while zoomed) opens the Flows view drilled to
/// that pair. An `other (N)` tile only explains itself.
fn treemap_enter(model: &mut Model) {
    let t = &mut model.traffic;
    let Some(target) = t.treemap.current() else {
        return;
    };
    let tile = t.treemap.tile(target).clone();
    if tile.is_other() {
        model.status = StatusLine::info(format!(
            "{}: too small to draw one by one; zoom in or widen the terminal",
            tile.label
        ));
        return;
    }
    let groups = t.groups.rows();
    let src_text = match (&t.treemap.zoom, target.inner) {
        (None, None) => {
            let src = groups
                .iter()
                .find(|g| g.src.join("/") == tile.label)
                .map(|g| g.src.clone());
            if let Some(src) = src {
                t.treemap.zoom = Some(src);
                t.treemap.cursor = 0;
                t.relayout_treemap();
            }
            return;
        }
        (Some(z), _) => z.join("/"),
        (None, Some(_)) => t.treemap.tiles[target.outer].label.clone(),
    };
    let pair = groups
        .iter()
        .find(|g| g.src.join("/") == src_text && g.dst.join("/") == tile.label)
        .map(|g| (g.src.clone(), g.dst.clone()));
    if let Some((src, dst)) = pair {
        t.drill = Some(Drill::Pair(src, dst));
        t.view = TrafficView::Flows;
        t.rebuild_views();
    }
}

/// Esc on the Treemap: out of a zoom, the cursor back on that source.
fn treemap_out(model: &mut Model) {
    let tm = &mut model.traffic.treemap;
    let Some(z) = tm.zoom.take() else {
        return;
    };
    model.traffic.relayout_treemap();
    let tm = &mut model.traffic.treemap;
    let label = z.join("/");
    if let Some(i) = tm
        .targets()
        .iter()
        .position(|t| t.inner.is_none() && tm.tiles[t.outer].label == label)
    {
        tm.cursor = i;
    }
}
```

replace

```rust
use crate::tui::model::{
    Connection, Drill, EventsFilterEdit, Model, ObjectKind, Overlay, PendingAction, PolicyPane,
    PolicyState, QueryState, Screen, StatusLine, TrafficData, TrafficView, WritesState, rule_line,
};
```

with

```rust
use crate::tui::model::{
    Connection, Dir, Drill, EventsFilterEdit, Model, ObjectKind, Overlay, PendingAction,
    PolicyPane, PolicyState, QueryState, Screen, StatusLine, TrafficData, TrafficView, WritesState,
    rule_line,
};
use pce::traffic::treemap::Weight;
```

replace

```rust
                TrafficView::Mesh => step(&mut t.mesh, down, page),
                TrafficView::Blocked => step(&mut t.blocked, down, page),
```

with

```rust
                TrafficView::Mesh => step(&mut t.mesh, down, page),
                TrafficView::Blocked => step(&mut t.blocked, down, page),
                // The wheel moves table rows; the treemap has none.
                TrafficView::Treemap => {}
```

replace

```rust
        TrafficView::Aggregate => {
            model.status = StatusLine::warn("select a flow in the Flows view (Enter on a pair)");
            return Vec::new();
        }
    };
```

with

```rust
        TrafficView::Aggregate | TrafficView::Treemap => {
            model.status = StatusLine::warn("select a flow in the Flows view (Enter on a pair)");
            return Vec::new();
        }
    };
```

replace

```rust
    t.drill = None;
    t.rebuild_views();
    model.status = StatusLine::info(format!("grouped by {}", t.effective_keys().join(",")));
```

with

```rust
    t.drill = None;
    t.treemap.zoom = None;
    t.rebuild_views();
    model.status = StatusLine::info(format!("grouped by {}", t.effective_keys().join(",")));
```

In `crates/illumio-tui/src/tui/ui/screens/traffic.rs`:

replace

```rust
use crate::tui::model::{Model, QueryState, TrafficView};
```

with

```rust
use crate::tui::model::{Model, QueryState, TrafficView};
use crate::tui::ui::theme::Theme;
```

replace

```rust
use pce::traffic::{ResolvedEndpoint, ResolvedFlow, compact_count};
```

with

```rust
use pce::traffic::treemap::{Rect as TreemapRect, Tile, Weight};
use pce::traffic::{ResolvedEndpoint, ResolvedFlow, compact_count};
```

replace

```rust
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Wrap};
```

with

```rust
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Paragraph, Wrap};
```

replace

```rust
/// Rows of data visible in the flow table (minus border and header).
```

with

```rust
/// The cells the treemap fills on a terminal of `size`: the table and
/// detail area of `layout` minus the panel border, exactly where
/// `render_treemap` draws.
pub fn treemap_area(size: (u16, u16)) -> TreemapRect {
    let Some(c) = crate::tui::ui::layout::chrome(Rect::new(0, 0, size.0, size.1)) else {
        return TreemapRect::default();
    };
    let l = layout(c.body);
    let r = l.table.union(l.detail);
    TreemapRect {
        x: r.x + 1,
        y: r.y + 1,
        w: r.width.saturating_sub(2),
        h: r.height.saturating_sub(2),
    }
}

/// Rows of data visible in the flow table (minus border and header).
```

replace

```rust
        TrafficView::Mesh => render_mesh(model, f, l.table.union(l.detail)),
```

with

```rust
        TrafficView::Mesh => render_mesh(model, f, l.table.union(l.detail)),
        TrafficView::Treemap => render_treemap(model, f, l.table.union(l.detail)),
```

replace

```rust
/// Every row here is blocked or potentially blocked; both read as `bad`.
```

with

```rust
/// The tiles the model holds: each source a plain-bordered block titled
/// with its group, its destinations filled with `▒` in their decision's
/// colour, label and weight written over the fill where they fit; the
/// cursor tile reversed.
fn render_treemap(model: &Model, f: &mut Frame, area: Rect) {
    let t = &model.theme;
    let tm = &model.traffic.treemap;
    let title = match &tm.zoom {
        Some(z) => format!("{} ▸ {}", TrafficView::Treemap.title(), z.join("/")),
        None => TrafficView::Treemap.title().to_string(),
    };
    let meta = match tm.weight {
        Weight::Connections => "by conns · b bytes · Enter zoom · Esc out",
        Weight::Bytes => "by bytes · b conns · Enter zoom · Esc out",
    };
    let block = panel(&title, meta, t, true);
    let inner = block.inner(area);
    f.render_widget(block, area);
    if tm.tiles.is_empty() {
        f.render_widget(
            Paragraph::new(Line::from(Span::styled("no rows", t.dim_style()))),
            inner,
        );
        return;
    }
    let cur = tm.current();
    let is_cur = |outer: usize, inner_i: Option<usize>| {
        cur.is_some_and(|c| c.outer == outer && c.inner == inner_i)
    };
    for (i, tile) in tm.tiles.iter().enumerate() {
        if tile.children.is_empty() {
            draw_fill(f, inner, tile, t, is_cur(i, None));
            continue;
        }
        let r = cells(tile.rect).intersection(inner);
        if r.is_empty() {
            continue;
        }
        let mut style = t.dim_style();
        if is_cur(i, None) {
            style = style.add_modifier(Modifier::REVERSED);
        }
        let label = clip_text(&tile.label, r.width.saturating_sub(2) as usize);
        f.render_widget(
            Block::bordered()
                .border_style(style)
                .title(Span::styled(label, style)),
            r,
        );
        for (j, c) in tile.children.iter().enumerate() {
            draw_fill(f, inner, c, t, is_cur(i, Some(j)));
        }
    }
}

fn cells(r: TreemapRect) -> Rect {
    Rect::new(r.x, r.y, r.w, r.h)
}

fn clip_text(s: &str, n: usize) -> String {
    s.chars().take(n).collect()
}

/// A filled tile: `▒` in the decision colour, the label on the first row
/// when it fits with a cell either side, the weight on the second.
fn draw_fill(f: &mut Frame, clip: Rect, tile: &Tile, t: &Theme, cursor: bool) {
    let r = cells(tile.rect).intersection(clip);
    if r.is_empty() {
        return;
    }
    let mut fill = t.decision_style(tile.decision);
    let mut text = Style::new().fg(t.fg).add_modifier(Modifier::BOLD);
    if cursor {
        fill = fill.add_modifier(Modifier::REVERSED);
        text = text.add_modifier(Modifier::REVERSED);
    }
    let w = r.width as usize;
    let row = |label: Option<String>| -> Line<'static> {
        match label {
            Some(l) if l.chars().count() + 2 <= w => {
                let n = l.chars().count();
                Line::from(vec![
                    Span::styled("▒", fill),
                    Span::styled(l, text),
                    Span::styled("▒".repeat(w - 1 - n), fill),
                ])
            }
            _ => Line::from(Span::styled("▒".repeat(w), fill)),
        }
    };
    let mut lines = vec![row(Some(tile.label.clone()))];
    if r.height >= 2 {
        lines.push(row(Some(compact_count(tile.weight))));
    }
    while lines.len() < r.height as usize {
        lines.push(row(None));
    }
    f.render_widget(Paragraph::new(lines), r);
}

/// Every row here is blocked or potentially blocked; both read as `bad`.
```

In `crates/illumio-tui/src/tui/ui/widgets/overlay.rs`:

replace

```rust
    ("Traffic  v", "cycle Flows · App-to-app · Mesh · Blocked"),
```

with

```rust
    ("Traffic  v", "cycle Flows · App-to-app · Mesh · Blocked · Treemap"),
```

replace

```rust
    ("Mesh     u / p", "show or hide the user · process columns"),
```

with

```rust
    ("Mesh     u / p", "show or hide the user · process columns"),
    (
        "Treemap  hjkl Enter b",
        "move · zoom, then drill · conns/bytes · Esc out",
    ),
```

- [ ] **Step 4: Run the unit tests**

Run: `cargo fmt --all && cargo test -p illumio-tui --lib`
Expected: PASS, including `the_treemap_follows_the_terminal_size_and_v_comes_back_to_flows`, `the_treemap_cursor_moves_towards_the_key_and_stays_at_an_edge`, `enter_zooms_into_a_source_then_drills_and_esc_zooms_out`, `b_weighs_by_bytes_or_says_there_are_none`, `new_keys_and_a_new_result_drop_the_zoom`, `the_picked_keys_reach_the_mesh_and_the_treemap`, the updated `w_opens_the_flow_form_only_for_a_blocked_flow` (Aggregate and Treemap refuse), the five-view cycles and `help_scrolls_to_its_last_line_and_reopens_at_the_top` (34 lines).

- [ ] **Step 5: Create and read the snapshots**

```bash
INSTA_UPDATE=always cargo test -p illumio-tui --test tui_snapshots
find crates/illumio-tui/tests/snapshots -name '*.snap.new' -delete
git status --short crates/illumio-tui/tests/snapshots
```

The test file also gains `an_empty_result_draws_the_treemap_like_an_empty_table` (spec 8: the empty-result text) and `mesh_and_treemap_keep_the_truncated_marker` (spec 8, at 160 columns: at 80 the stats line clips the marker once the counts have two digits, as it already does on Flows), which record nothing.

Expected: 4 new (`traffic_treemap_80x24`, `traffic_treemap_160x48`, `traffic_treemap_zoomed_80x24`, `light_traffic_treemap_160x48`) and the 3 help overlays modified (the `v` line with `· Treemap`, the new `Treemap  hjkl Enter b` line, `of 34`). Read them: at 80x24 nine source boxes (`┌ipl:Corp/-──…┐`, `┌shop/prod…`, `┌crm/prod…`, `┌erp/prod…`, then small ones), destinations filled with `▒` carrying `shop/prod` / `702`, `crm/prod` / `520` inside Corp, a too-small source drawn as a filled tile; the zoomed one titled `Treemap ▸ shop/prod` with `shop/prod` / `821`, `crm/prod` / `150` and a narrow `33` tile; the styled light one colours fills by decision (`ok`, `warn`, `bad`, dim for `other`).

- [ ] **Step 6: Gate and commit**

```bash
cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace
git add crates/illumio-tui/src crates/illumio-tui/tests
git commit -m "feat(tui): Treemap view of sources and destinations with cursor, zoom, drill and conns/bytes weight

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

Expected: 772 tests pass.

---

### Task 8: CLI `traffic groups` and `traffic mesh`

**Files:**
- Modify: `crates/illumio-tui/src/cli/traffic.rs` (module doc; imports; `TrafficCmd` and `QueryArgs` ~lines 22-80 split into `QueryArgs`, `GroupsArgs`, `MeshArgs`, `FilterArgs`; `build_query` signature ~134; `TableRows for Vec<FlowGroup>` ~272 gains `bytes`; new `TableRows for Vec<MeshRow>`; `run` / `query` ~339-380 replaced by `run`, `fetch`, `keys_or_profile`, `query`, `groups`, `mesh_cmd`)
- Test: `crates/illumio-tui/tests/cli.rs` (`every_flag_has_help_text` ~line 46; append three tests)

**Interfaces:**
- Consumes: Task 2's `group_by_labels`, `blocked_summary(flows, keys)`, `FlowGroup.bytes`; Task 3's `mesh`, `MeshColumns`, `MeshRow::{service_text, dominant}`; `Context::{emit, config.scope_keys, pce, cache_mode}`, `cache_label`, `Window`, `DryRun`, `BuiltQuery`; test helpers `mount_traffic`, `cmd`, `stdout_json` (`tests/cli.rs`; the mounted download is `fixtures/traffic_flows.json`, the 4-flow fixture).
- Produces:

```rust
// cli/traffic.rs
pub enum TrafficCmd { Query(QueryArgs), Groups(GroupsArgs), Mesh(MeshArgs) }
#[derive(Args, Debug, Clone, Default)] pub struct FilterArgs { window, from, to, src, dst, exclude_src, exclude_dst, port, proto, process, decision, max_results, dry_run, yes }
#[derive(Args, Debug, Clone, Default)] pub struct QueryArgs { #[command(flatten)] pub filter: FilterArgs, pub group: Vec<String>, pub blocked: bool }
#[derive(Args, Debug, Clone, Default)] pub struct GroupsArgs { #[command(flatten)] pub filter: FilterArgs, pub keys: Vec<String> }
#[derive(Args, Debug, Clone, Default)] pub struct MeshArgs { #[command(flatten)] pub filter: FilterArgs, pub keys: Vec<String>, pub no_user: bool, pub no_process: bool }
pub async fn build_query(ctx: &Context, args: &FilterArgs) -> Result<BuiltQuery, CliError>;   // was &QueryArgs
impl TableRows for Vec<MeshRow>;   // user, process, src, dst, service, dst process, flows, conns, bytes, decision
// Vec<FlowGroup> table headers: src, dst, flows, conns, bytes, allowed, pot.blocked, blocked
```

```
illumio-tui traffic groups [query flags] [--keys app,env]
illumio-tui traffic mesh   [query flags] [--keys app,env] [--no-user] [--no-process]
```

Both print the `traffic query` envelope (`data`, `meta` with cache, truncated, window); `groups` emits `Vec<FlowGroup>`, `mesh` emits `Vec<MeshRow>`. `--dry-run`, the cost guard (`--yes`) and the cache behave exactly as in `traffic query`, because all three go through the same `fetch`.

- [ ] **Step 1: Write the failing tests**

In `crates/illumio-tui/tests/cli.rs`:

replace

```rust
        vec!["traffic", "query", "--help"],
```

with

```rust
        vec!["traffic", "query", "--help"],
        vec!["traffic", "groups", "--help"],
        vec!["traffic", "mesh", "--help"],
```

Append to `crates/illumio-tui/tests/cli.rs`:

```rust
#[tokio::test(flavor = "multi_thread")]
async fn traffic_groups_sum_bytes_and_honour_keys() {
    let server = MockServer::start().await;
    mount_traffic(&server, 2).await; // two processes, no shared cache
    let out = stdout_json(
        cmd(&server)
            .args(["traffic", "groups", "--window", "1h"])
            .assert()
            .success(),
    );
    let data = out["data"].as_array().unwrap();
    assert_eq!(data.len(), 4, "the profile's app,env pairs");
    assert_eq!(data[0]["src"], json!(["shop", "prod"]));
    assert_eq!(data[0]["dst"], json!(["shop", "prod"]));
    assert_eq!(data[0]["connections"], 310);
    assert_eq!(data[0]["bytes"], 124_000);
    assert_eq!(data[1]["src"], json!(["ipl:Corp", "-"]));
    assert_eq!(data[1]["bytes"], 0, "no byte counts: 0 in JSON");
    assert_eq!(out["meta"]["cache"], "miss");
    assert_eq!(out["meta"]["truncated"], false);
    let table = cmd(&server)
        .args(["traffic", "groups", "--window", "1h", "--keys", "role", "--table"])
        .assert()
        .success();
    let text = String::from_utf8_lossy(&table.get_output().stdout).to_string();
    let header = text.lines().next().unwrap();
    for h in [
        "src", "dst", "flows", "conns", "bytes", "allowed", "pot.blocked", "blocked",
    ] {
        assert!(header.contains(h), "{header}");
    }
    assert!(text.contains("web") && text.contains("124000"), "{text}");
    cmd(&server)
        .args(["traffic", "groups", "--blocked"])
        .assert()
        .code(2);
}

#[tokio::test(flavor = "multi_thread")]
async fn traffic_mesh_places_processes_by_direction_and_drops_hidden_columns() {
    let server = MockServer::start().await;
    mount_traffic(&server, 3).await;
    let out = stdout_json(
        cmd(&server)
            .args(["traffic", "mesh", "--window", "1h"])
            .assert()
            .success(),
    );
    let rows = out["data"].as_array().unwrap();
    assert_eq!(rows.len(), 4);
    assert_eq!(rows[0]["user"], "postgres");
    assert_eq!(rows[0]["process"], "postgres", "outbound: the source's process");
    assert!(rows[0]["dst_process"].is_null());
    assert_eq!(rows[0]["port"], 5432);
    assert_eq!(rows[0]["proto"], "tcp");
    assert_eq!(rows[0]["bytes"], 124_000);
    assert_eq!(rows[0]["allowed"], 1);
    let corp = rows.iter().find(|r| r["src"] == json!(["ipl:Corp", "-"])).unwrap();
    assert!(corp["user"].is_null() && corp["process"].is_null());
    let out = stdout_json(
        cmd(&server)
            .args(["traffic", "mesh", "--window", "1h", "--no-user", "--keys", "role"])
            .assert()
            .success(),
    );
    assert!(out["data"][0]["user"].is_null());
    assert_eq!(out["data"][0]["process"], "postgres");
    assert_eq!(out["data"][0]["src"], json!(["web"]));
    let table = cmd(&server)
        .args(["traffic", "mesh", "--window", "1h", "--no-process", "--table"])
        .assert()
        .success();
    let text = String::from_utf8_lossy(&table.get_output().stdout).to_string();
    let header = text.lines().next().unwrap();
    for h in [
        "user", "process", "src", "dst", "service", "dst process", "flows", "conns", "bytes",
        "decision",
    ] {
        assert!(header.contains(h), "{header}");
    }
    assert!(text.contains("potentially_blocked"), "{text}");
}

#[tokio::test(flavor = "multi_thread")]
async fn traffic_groups_and_mesh_dry_runs_send_nothing() {
    let server = MockServer::start().await;
    mount_traffic(&server, 0).await;
    for sub in ["groups", "mesh"] {
        let out = stdout_json(
            cmd(&server)
                .args(["traffic", sub, "--window", "1h", "--port", "443", "--dry-run"])
                .assert()
                .success(),
        );
        assert_eq!(out["data"]["services"]["include"], json!([{"port": 443}]));
        assert_eq!(out["data"]["max_results"], 1000);
        assert_eq!(out["meta"]["cache"], "none");
    }
}
```

The numbers come from `fixtures/traffic_flows.json` resolved with the mocked object lists: four app,env pairs, `shop/prod -> shop/prod` first with 310 connections and 120 000 + 4 000 bytes, the Corp flow without bytes.

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p illumio-tui --test cli traffic_ 2>&1 | tail -15`
Expected: FAIL: the three new tests exit 2 (`unrecognized subcommand 'groups'` / `'mesh'`).

- [ ] **Step 3: Implement**

In `crates/illumio-tui/src/cli/traffic.rs`:

replace

```rust
//! `traffic query`: build an Explorer query from human filters, guard its
//! cost, run it through the cached facade, and print flows, label-pair
//! groups, or the blocked summary.
```

with

```rust
//! `traffic query|groups|mesh`: build an Explorer query from human filters,
//! guard its cost, run it through the cached facade, and print flows,
//! label-pair groups, the blocked summary or the mesh.
```

replace

```rust
use pce::traffic::{
    BlockedGroup, FilterExpr, FlowGroup, ResolvedFlow, TrafficQueryBuilder, blocked_summary,
    group_by_labels, parse_window, resolve_actor, resolve_flows,
};
```

with

```rust
use pce::traffic::{
    BlockedGroup, FilterExpr, FlowGroup, MeshColumns, MeshRow, ResolvedFlow, TrafficQueryBuilder,
    blocked_summary, group_by_labels, mesh, parse_window, resolve_actor, resolve_flows,
};
```

replace

```rust
#[derive(Subcommand, Debug)]
pub enum TrafficCmd {
    /// Run an Explorer traffic query
    Query(QueryArgs),
}

#[derive(Args, Debug, Clone, Default)]
pub struct QueryArgs {
```

with

```rust
#[derive(Subcommand, Debug)]
pub enum TrafficCmd {
    /// Run an Explorer traffic query
    Query(QueryArgs),
    /// Label-pair groups of the query's flows (connections, bytes, decisions)
    Groups(GroupsArgs),
    /// The mesh: user, process, source, destination, service, destination process
    Mesh(MeshArgs),
}

/// `traffic query`: the filter, then how to print the flows.
#[derive(Args, Debug, Clone, Default)]
pub struct QueryArgs {
    #[command(flatten)]
    pub filter: FilterArgs,
    /// Group by label keys, e.g. app,env
    #[arg(long, value_delimiter = ',', conflicts_with = "blocked")]
    pub group: Vec<String>,
    /// Only blocked and potentially blocked flows, grouped by destination group and port
    #[arg(long)]
    pub blocked: bool,
}

#[derive(Args, Debug, Clone, Default)]
pub struct GroupsArgs {
    #[command(flatten)]
    pub filter: FilterArgs,
    /// Label keys to group by, e.g. app,env,role (default: the profile's scope keys)
    #[arg(long, value_delimiter = ',')]
    pub keys: Vec<String>,
}

#[derive(Args, Debug, Clone, Default)]
pub struct MeshArgs {
    #[command(flatten)]
    pub filter: FilterArgs,
    /// Label keys to group sources and destinations by (default: the profile's scope keys)
    #[arg(long, value_delimiter = ',')]
    pub keys: Vec<String>,
    /// Do not split rows by user
    #[arg(long)]
    pub no_user: bool,
    /// Do not split rows by the source process
    #[arg(long)]
    pub no_process: bool,
}

/// The Explorer filter every `traffic` command shares.
#[derive(Args, Debug, Clone, Default)]
pub struct FilterArgs {
```

replace

```rust
    /// Row cap (default from profile, 1000; > 10000 needs --yes)
    #[arg(long)]
    pub max_results: Option<u32>,
    /// Group by label keys, e.g. app,env
    #[arg(long, value_delimiter = ',', conflicts_with = "blocked")]
    pub group: Vec<String>,
    /// Only blocked and potentially blocked flows, grouped by destination and port
    #[arg(long)]
    pub blocked: bool,
    /// Print the request body instead of sending it
```

with

```rust
    /// Row cap (default from profile, 1000; > 10000 needs --yes)
    #[arg(long)]
    pub max_results: Option<u32>,
    /// Print the request body instead of sending it
```

replace

```rust
pub async fn build_query(ctx: &Context, args: &QueryArgs) -> Result<BuiltQuery, CliError> {
```

with

```rust
pub async fn build_query(ctx: &Context, args: &FilterArgs) -> Result<BuiltQuery, CliError> {
```

replace

```rust
impl TableRows for Vec<FlowGroup> {
    fn headers(&self) -> Vec<&'static str> {
        vec![
            "src",
            "dst",
            "flows",
            "conns",
            "allowed",
            "pot.blocked",
            "blocked",
        ]
    }
    fn rows(&self) -> Vec<Vec<String>> {
        self.iter()
            .map(|g| {
                vec![
                    g.src.join("/"),
                    g.dst.join("/"),
                    g.flows.to_string(),
                    g.connections.to_string(),
                    g.allowed.to_string(),
                    g.potentially_blocked.to_string(),
                    g.blocked.to_string(),
                ]
            })
            .collect()
    }
}
```

with

```rust
/// A bytes cell: `-` for every row when no row carried bytes.
fn bytes_cell(bytes: u64, any: bool) -> String {
    if any { bytes.to_string() } else { "-".into() }
}

impl TableRows for Vec<FlowGroup> {
    fn headers(&self) -> Vec<&'static str> {
        vec![
            "src",
            "dst",
            "flows",
            "conns",
            "bytes",
            "allowed",
            "pot.blocked",
            "blocked",
        ]
    }
    fn rows(&self) -> Vec<Vec<String>> {
        let any = self.iter().any(|g| g.bytes > 0);
        self.iter()
            .map(|g| {
                vec![
                    g.src.join("/"),
                    g.dst.join("/"),
                    g.flows.to_string(),
                    g.connections.to_string(),
                    bytes_cell(g.bytes, any),
                    g.allowed.to_string(),
                    g.potentially_blocked.to_string(),
                    g.blocked.to_string(),
                ]
            })
            .collect()
    }
}

impl TableRows for Vec<MeshRow> {
    fn headers(&self) -> Vec<&'static str> {
        vec![
            "user",
            "process",
            "src",
            "dst",
            "service",
            "dst process",
            "flows",
            "conns",
            "bytes",
            "decision",
        ]
    }
    fn rows(&self) -> Vec<Vec<String>> {
        let any = self.iter().any(|r| r.bytes > 0);
        let or_dash = |v: &Option<String>| v.clone().unwrap_or_else(|| "-".into());
        self.iter()
            .map(|r| {
                vec![
                    or_dash(&r.user),
                    or_dash(&r.process),
                    r.src.join("/"),
                    r.dst.join("/"),
                    r.service_text(),
                    or_dash(&r.dst_process),
                    r.flows.to_string(),
                    r.connections.to_string(),
                    bytes_cell(r.bytes, any),
                    r.dominant().as_str().to_string(),
                ]
            })
            .collect()
    }
}
```

replace

```rust
pub async fn run(ctx: &Context, cmd: &TrafficCmd) -> Result<(), CliError> {
    match cmd {
        TrafficCmd::Query(a) => query(ctx, a).await,
    }
}

async fn query(ctx: &Context, args: &QueryArgs) -> Result<(), CliError> {
    let BuiltQuery { query: q, index } = build_query(ctx, args).await?;
    let window = Some(Window {
        start: q.start_date.clone(),
        end: q.end_date.clone(),
    });
    if args.dry_run {
        return ctx.emit(&DryRun(q), "none", false, window);
    }
    let idx = match index {
        Some(idx) => idx,
        None => ctx.pce.name_index(ctx.cache_mode).await?,
    };
    let result = ctx
        .pce
        .traffic(&q, ctx.cache_mode, &CancellationToken::new(), None)
        .await?;
    let cache = cache_label(result.hit, ctx.cache_mode);
    let flows = resolve_flows(&result.value.flows, &idx);
    if args.blocked {
        ctx.emit(
            &blocked_summary(&flows, &ctx.config.scope_keys),
            cache,
            result.truncated,
            window,
        )
    } else if !args.group.is_empty() {
        ctx.emit(
            &group_by_labels(&flows, &args.group),
            cache,
            result.truncated,
            window,
        )
    } else {
        ctx.emit(&flows, cache, result.truncated, window)
    }
}
```

with

```rust
pub async fn run(ctx: &Context, cmd: &TrafficCmd) -> Result<(), CliError> {
    match cmd {
        TrafficCmd::Query(a) => query(ctx, a).await,
        TrafficCmd::Groups(a) => groups(ctx, a).await,
        TrafficCmd::Mesh(a) => mesh_cmd(ctx, a).await,
    }
}

/// One query's flows, resolved, and what the envelope reports about them.
struct Fetched {
    flows: Vec<ResolvedFlow>,
    cache: &'static str,
    truncated: bool,
    window: Option<Window>,
}

/// Builds and runs the query (cost guard, cache, one name index). A dry
/// run prints the request body and returns `None`.
async fn fetch(ctx: &Context, args: &FilterArgs) -> Result<Option<Fetched>, CliError> {
    let BuiltQuery { query: q, index } = build_query(ctx, args).await?;
    let window = Some(Window {
        start: q.start_date.clone(),
        end: q.end_date.clone(),
    });
    if args.dry_run {
        ctx.emit(&DryRun(q), "none", false, window)?;
        return Ok(None);
    }
    let idx = match index {
        Some(idx) => idx,
        None => ctx.pce.name_index(ctx.cache_mode).await?,
    };
    let result = ctx
        .pce
        .traffic(&q, ctx.cache_mode, &CancellationToken::new(), None)
        .await?;
    Ok(Some(Fetched {
        flows: resolve_flows(&result.value.flows, &idx),
        cache: cache_label(result.hit, ctx.cache_mode),
        truncated: result.truncated,
        window,
    }))
}

/// `--keys`, else the profile's scope keys.
fn keys_or_profile(ctx: &Context, keys: &[String]) -> Vec<String> {
    if keys.is_empty() {
        ctx.config.scope_keys.clone()
    } else {
        keys.to_vec()
    }
}

async fn query(ctx: &Context, args: &QueryArgs) -> Result<(), CliError> {
    let Some(f) = fetch(ctx, &args.filter).await? else {
        return Ok(());
    };
    if args.blocked {
        let keys = keys_or_profile(ctx, &[]);
        ctx.emit(
            &blocked_summary(&f.flows, &keys),
            f.cache,
            f.truncated,
            f.window,
        )
    } else if !args.group.is_empty() {
        ctx.emit(
            &group_by_labels(&f.flows, &args.group),
            f.cache,
            f.truncated,
            f.window,
        )
    } else {
        ctx.emit(&f.flows, f.cache, f.truncated, f.window)
    }
}

async fn groups(ctx: &Context, args: &GroupsArgs) -> Result<(), CliError> {
    let Some(f) = fetch(ctx, &args.filter).await? else {
        return Ok(());
    };
    let keys = keys_or_profile(ctx, &args.keys);
    ctx.emit(
        &group_by_labels(&f.flows, &keys),
        f.cache,
        f.truncated,
        f.window,
    )
}

async fn mesh_cmd(ctx: &Context, args: &MeshArgs) -> Result<(), CliError> {
    let Some(f) = fetch(ctx, &args.filter).await? else {
        return Ok(());
    };
    let keys = keys_or_profile(ctx, &args.keys);
    let cols = MeshColumns {
        user: !args.no_user,
        process: !args.no_process,
    };
    ctx.emit(&mesh(&f.flows, &keys, cols), f.cache, f.truncated, f.window)
}
```

Append to `crates/illumio-tui/src/cli/traffic.rs` (its first unit tests; they pin spec 8's `-` for absent bytes, which the recorded flows cannot show because one of them carries bytes):

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn group(bytes: u64) -> FlowGroup {
        FlowGroup {
            src: vec!["a".into()],
            dst: vec!["b".into()],
            flows: 1,
            connections: 2,
            bytes,
            allowed: 1,
            potentially_blocked: 0,
            blocked: 0,
        }
    }

    #[test]
    fn bytes_cells_are_a_dash_only_when_no_row_has_bytes() {
        assert_eq!(vec![group(0), group(0)].rows()[0][4], "-");
        let rows = vec![group(5), group(0)].rows();
        assert_eq!((rows[0][4].as_str(), rows[1][4].as_str()), ("5", "0"));
        let row = MeshRow {
            user: None,
            process: Some("java".into()),
            src: vec!["a".into()],
            dst: vec!["b".into()],
            port: None,
            proto: "icmp".into(),
            dst_process: None,
            flows: 1,
            connections: 3,
            bytes: 0,
            allowed: 0,
            potentially_blocked: 1,
            blocked: 0,
        };
        assert_eq!(
            vec![row].rows()[0],
            vec!["-", "java", "a", "b", "icmp", "-", "1", "3", "-", "potentially_blocked"]
        );
    }
}
```

- [ ] **Step 4: Run the CLI tests**

Run: `cargo fmt --all && cargo test -p illumio-tui --test cli && cargo test -p illumio-tui --lib cli::traffic`
Expected: PASS, including `bytes_cells_are_a_dash_only_when_no_row_has_bytes`, `traffic_groups_sum_bytes_and_honour_keys`, `traffic_mesh_places_processes_by_direction_and_drops_hidden_columns`, `traffic_groups_and_mesh_dry_runs_send_nothing`, and the existing `traffic_query_flows_json`, `traffic_query_group_and_blocked_views`, `traffic_dry_run_prints_body_without_submitting`, `traffic_guards_exit_2` (the flags moved into `FilterArgs` but read the same).

- [ ] **Step 5: Gate and commit**

```bash
cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace
git add crates/illumio-tui/src/cli/traffic.rs crates/illumio-tui/tests/cli.rs
git commit -m "feat(cli): traffic groups and traffic mesh for agents, with --keys, --no-user, --no-process

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

Expected: 776 tests pass.

---

### Task 9: Docs, tracking, reads-only smoke, final gate

**Files:**
- Modify: `README.md` (Traffic bullet ~88-91; `## CLI` examples ~141-144 and a paragraph before the `--impact` one; `### Label model` last sentence ~402; the `w` paragraph ~371)
- Modify: `docs/architecture.md` (TUI module table ~42-55; "One message" step 4 ~62; new `### Traffic views` before `## Testing`)
- Modify: `docs/index.html` (command list ~79-83; Explorer paragraph ~105; Screens `<kbd>1</kbd>` item ~197)
- Modify: `tasks/todo.md` (append the checklist and the review)
- `tasks/lessons.md`: no change unless the execution produced a correction (then append a `## Traffic views (issue #6)` section).
- Create (scratchpad only, never committed): `$SCRATCH/traffic_views_smoke.py`

**Interfaces:**
- Consumes: the built binary `target/debug/illumio-tui`; `../illumio-mcp-server/.env` (credentials of the read-only test PCE demo100, read by `--env-file`, never copied, printed or committed); a scratch venv with `pyte`.
- Produces: documentation only; no code changes. (Every Review Focus test lives in its owning task; this task adds none.)

- [ ] **Step 1: README**

Replace

```markdown
- **Traffic**: `f` edit the filter, `r` run (bypasses the cache), `v` cycle Flows / App-to-app /
  Blocked, `g` change the group keys, `s` sort, `/` search, `Enter` detail (flows) or drill into
  a pair (app-to-app), `a` auto-refresh. A filter that would be expensive asks for
  confirmation first. With writes on, `w` turns a blocked flow into a draft allow rule.
```

with

```markdown
- **Traffic**: `f` edit the filter, `r` run (bypasses the cache), `a` auto-refresh, `s` sort,
  `/` search; a filter that would be expensive asks for confirmation first. `v` cycles five
  views of the one result, all grouped by the same label keys: the profile's scope keys
  (`app,env` by default) until `g` opens the picker (`Space` toggles a key, `all labels` takes
  every key the flows carry, typing edits the list as text, `Enter` applies, for this session).
  **Flows** shows each side's group (`shop/prod`; `-` for an IP list or FQDN, whose name is in
  the name column) and, on `Enter`, a detail with both label sets and the bytes. **App-to-app**
  pairs the groups; `Enter` drills into a pair's flows. **Mesh** lists user, process, source,
  destination, service and destination process like the Illumio UI's mesh (`u` / `p` hide the
  user / process column and merge rows, `Enter` drills). **Blocked** groups blocked and
  potentially blocked flows by destination group and service. **Treemap** draws each source as
  a box and its destinations as tiles sized by connections (`b` bytes) and coloured by
  decision: `h` / `j` / `k` / `l` move, `Enter` zooms into a source and then drills into a
  destination, `Esc` zooms out. None of this sends a request. With writes on, `w` turns a
  blocked flow (or the first one behind a Blocked or Mesh row) into a draft allow rule.
```

In `## CLI`, after `    illumio-tui traffic query --window 1h --blocked` add

```
    illumio-tui traffic groups --window 1h --keys app,env,role
    illumio-tui traffic mesh --window 1h --no-user --json
```

and before the paragraph that starts with `` `--impact` on `policy rules <ruleset>` `` add

```markdown
`traffic groups` and `traffic mesh` take the `traffic query` filter flags (and its dry run,
cost guard and cache) and print the same envelope. `groups` gives one row per source and
destination group (`src`, `dst`, `flows`, `connections`, `bytes`, `allowed`,
`potentially_blocked`, `blocked`); `mesh` one row per user, source process, source group,
destination group, port, protocol and destination process. The PCE reports one process per
flow, seen where the flow was reported: an inbound flow's process is the destination's,
anything else the source's. `--keys` picks the label keys (default: the profile's scope keys),
`--no-user` / `--no-process` merge rows across users or processes. An endpoint that is not a
labelled workload is grouped under its name or `ipl:<list>`. `bytes` is `0` in JSON and `-`
in tables when the PCE reported none.
```

Replace `Only the TUI's `w` form (pre-fill and `Ctrl-N`) uses them.` (end of `### Label model`) with

```markdown
The TUI's `w` form (pre-fill and `Ctrl-N`) uses them, the traffic views group by the scope keys
until `g` picks others (for the session), and `traffic groups`, `traffic mesh` and
`traffic query --blocked` default to them.
```

and in the `w` paragraph replace

```markdown
On Traffic, `w` on a blocked or potentially blocked flow (Flows view or a
drilled pair) opens an allow-rule form pre-filled from the flow in the label
```

with

```markdown
On Traffic, `w` on a blocked or potentially blocked flow (Flows view or a
drilled pair; on a Blocked or Mesh row, the first such flow behind it) opens an
allow-rule form pre-filled from the flow in the label
```

- [ ] **Step 2: Architecture** (`docs/architecture.md`)

In the TUI module table, after the `impact` row add

```markdown
| `picker` | the `g` overlay: which label keys the traffic views group by |
```

Replace

```markdown
4. `update` stores the result and calls `rebuild_views`, which derives the
   Flows, App-to-app and Blocked tables from the one result.
```

with

```markdown
4. `update` stores the result and calls `rebuild_views`, which derives the
   Flows, App-to-app, Mesh and Blocked tables and the treemap's tiles from the
   one result, grouped by the label keys in force.
```

Insert before `## Testing`:

```markdown
### Traffic views

Every view is a projection of the flows the last query returned; none
requests anything. `TrafficState.group_keys` starts as the profile's
`scope_keys` (`Model::set_label_model`, at startup and after `:profile`);
`g` opens `tui/picker.rs` (`KeyPicker`), and `all_keys` stands for every key
the flows carry (`pce::traffic::keys_seen`, `app`, `env`, `role`, `loc`
first). `rebuild_views` resolves the keys once (`effective_keys`) and
rebuilds every view with them. Grouping lives in `pce::traffic`: `group_text`
(`-` in every slot of a non-workload) labels the Flows rows, which are
`FlowRow`s because a column cell is a plain `fn` that cannot see the keys;
the aggregations (`group_by_labels`, `blocked_summary`, `mesh`) group by
`group_key`, which keeps IP lists, FQDNs and unlabelled hosts apart under
their name. `pce::traffic::mesh` places the one process the PCE reports per
flow on the destination for inbound flows and on the source otherwise.
`pce::traffic::treemap::layout` is a squarified treemap in terminal cells (a
row counts as two columns), rounded edge by edge so tiles neither overlap
nor leave gaps, with the tail that gets no cell folded into `other (N)`.
The TUI stores the tiles in `TreemapState` and recomputes them when the
groups, the keys, the zoom, the weight or the terminal size change
(`Msg::Resize`, `treemap_area`), never while drawing. `Drill` narrows the
Flows view to an App-to-app or treemap pair or a Mesh row; new keys and a
new result drop it and the zoom. The CLI's `traffic groups` and
`traffic mesh` run the same functions over one `fetch`.
```

- [ ] **Step 3: Website** (`docs/index.html`)

After

```html
<span class="p">$ </span>illumio-tui traffic query --window 1h --blocked
```

add

```html
<span class="p">$ </span>illumio-tui traffic groups --window 1h --keys app,env,role
<span class="p">$ </span>illumio-tui traffic mesh --window 1h --json
```

Replace `Both run on the downloaded result, with zero extra requests.` with

```html
<code>traffic groups</code> and <code>traffic mesh</code> print the label pairs and the mesh (user, process, source, destination, service, destination process) for agents, with <code>--keys</code> for your label model. All of them run on the downloaded result, with zero extra requests.
```

and replace the Traffic screen item

```html
        <li><kbd>1</kbd><strong>Traffic.</strong> One filter, one query, three views on <code>v</code>: the flow table, the app-to-app aggregate with a drill-down on <code>enter</code>, and a blocked focus grouped by destination and port.</li>
```

with

```html
        <li><kbd>1</kbd><strong>Traffic.</strong> One filter, one query, five views on <code>v</code>, all grouped by the label keys you pick with <code>g</code>: the flow table, app-to-app pairs with a drill-down on <code>enter</code>, a mesh of user, process, source, destination and service, a blocked focus grouped by destination group and port, and a treemap of sources and their destinations.</li>
```

- [ ] **Step 4: Tracking** (append to `tasks/todo.md`)

```markdown
## Traffic views (issue #6; folds in issue #5)
Spec: docs/superpowers/specs/2026-10-11-traffic-views-design.md · Plan: docs/superpowers/plans/2026-10-11-traffic-views.md
- [x] Task 1 IP lists as the fourth Objects tab
- [x] Task 2 pce::traffic grouping primitives and the rich fixture
- [x] Task 3 pce::traffic::mesh
- [x] Task 4 pce::traffic::treemap
- [x] Task 5 TUI display keys and the g picker
- [x] Task 6 TUI Mesh view
- [x] Task 7 TUI Treemap view
- [x] Task 8 CLI traffic groups and traffic mesh
- [x] Task 9 Docs, tracking, reads-only smoke, final gate
```

Tasks 1-8 have landed when this task runs; Task 9 is ticked in the same commit as this file (Step 8).

- [ ] **Step 5: Final gate**

Run: `cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace`
Expected: PASS, 776 tests; note the count for the review.

Run: `git diff 34b4fa9 HEAD -- Cargo.toml crates/*/Cargo.toml Cargo.lock`
Expected: empty (no new dependency).

Run: `git diff --stat 34b4fa9 HEAD -- crates/pce`
Expected: `crates/pce/src/traffic/{aggregate,resolved,mod}.rs` changed, `mesh.rs` and `treemap.rs` new; nothing else in `crates/pce` (no request path changed).

- [ ] **Step 6: Reads-only smoke against demo100**

No write may be sent to demo100: the TUI runs without `--allow-writes`, the CLI commands are reads, `PCE_LIVE_WRITES` stays unset, and every run uses a scratch `XDG_STATE_HOME`. The smoke costs one Explorer query in the TUI (24h, cap 1000) and one in the CLI (1h, cap 200); nothing else queries traffic.

```bash
cargo build -p illumio-tui
python3 -m venv "$SCRATCH/venv" && "$SCRATCH/venv/bin/pip" install pyte
mkdir -p "$SCRATCH/state-tv"
```

Create `$SCRATCH/traffic_views_smoke.py`:

```python
#!/usr/bin/env python3
"""Traffic views smoke against the read-only test PCE: one query, then every
view, the picker, the treemap zoom and the bytes weight; the request counter
must not move after the query. Reads only: no --allow-writes, no 'y'."""
import fcntl, json, os, pty, re, select, struct, subprocess, sys, termios, time
import pyte

BIN, ENV_FILE, STATE = sys.argv[1], sys.argv[2], sys.argv[3]
COLS, ROWS = 120, 36
AUDIT = os.path.join(STATE, "illumio-tui", "writes.jsonl")


def child_env():
    env = {k: v for k, v in os.environ.items() if k not in ("ILLUMIO_WRITES", "PCE_LIVE_WRITES")}
    env["XDG_STATE_HOME"] = STATE
    return env


pid, fd = pty.fork()
if pid == 0:
    os.execve(BIN, [BIN, "--env-file", ENV_FILE], child_env())
fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", ROWS, COLS, 0, 0))
screen = pyte.Screen(COLS, ROWS)
stream = pyte.ByteStream(screen)


def pump(seconds):
    end = time.time() + seconds
    while time.time() < end:
        ready, _, _ = select.select([fd], [], [], 0.05)
        if ready:
            try:
                stream.feed(os.read(fd, 65536))
            except OSError:
                return


def text():
    return "\n".join(screen.display)


def calls():
    m = re.search(r"api (\d+) calls", screen.display[0])
    return int(m.group(1)) if m else -1


def press(keys, wait=0.4):
    for k in keys:
        assert k not in "yY", "the smoke never presses y"
        os.write(fd, k.encode())
        pump(0.15)
    pump(wait)
    return text()


pump(6)
before = calls()
press("r")
deadline = time.time() + 120
while time.time() < deadline and not re.search(r"\d+ flows · ", text()) and "failed" not in text():
    pump(1)
assert "failed" not in text(), text()
after_query = calls()
assert after_query > before, ("the query made no request?", before, after_query)
flows = int(re.search(r"(\d+) flows · ", text()).group(1))
print(f"query: {flows} flows, api calls {before} -> {after_query}")
assert "src group" in text() and "dst group" in text(), text()

seen = {}
seen["app-to-app"] = press("v")
seen["mesh"] = press("v")
assert "u user · p process" in seen["mesh"], seen["mesh"]
seen["mesh -u -p"] = press("up")
seen["blocked"] = press("v")
assert "by dst" in seen["blocked"], seen["blocked"]
seen["treemap"] = press("v")
assert "Treemap" in seen["treemap"], seen["treemap"]
if flows:
    seen["zoom"] = press("\r")
    assert "Treemap ▸" in seen["zoom"] or "too small" in seen["zoom"] or "Flows ▸" in seen["zoom"], seen["zoom"]
    seen["zoom out"] = press("\x1b")
seen["bytes"] = press("b")
seen["picker"] = press("g")
assert "Group by" in seen["picker"] and "all labels" in seen["picker"], seen["picker"]
seen["regrouped"] = press("jj \r")
assert "grouped by" in seen["regrouped"], seen["regrouped"].splitlines()[-1]
seen["flows"] = press("v")
for name, shot in seen.items():
    print(f"--- {name}\n" + "\n".join(shot.splitlines()[8:20]))
assert calls() == after_query, ("a view, key or zoom made a request", after_query, calls())
print("api calls after the query:", calls(), "(unchanged)")
for _ in range(3):
    os.write(fd, b"\x1b")
    pump(0.3)
os.write(fd, b"q")
pump(1)
os.waitpid(pid, 0)

out = subprocess.run([BIN, "--env-file", ENV_FILE, "--json", "traffic", "mesh", "--window", "1h",
                      "--max-results", "200"], env=child_env(), capture_output=True, text=True, timeout=180)
assert out.returncode == 0, (out.returncode, out.stderr[:500])
mesh = json.loads(out.stdout)
rows = mesh["data"]
assert all({"user", "process", "src", "dst", "dst_process", "bytes"} <= set(r) for r in rows)
print("traffic mesh: rows", len(rows), "api_calls", mesh["meta"]["api_calls"], "truncated", mesh["meta"]["truncated"])
dry = subprocess.run([BIN, "--env-file", ENV_FILE, "--json", "traffic", "groups", "--window", "1h", "--dry-run"],
                     env=child_env(), capture_output=True, text=True, timeout=60)
assert dry.returncode == 0 and json.loads(dry.stdout)["meta"]["cache"] == "none", dry.stderr[:300]
print("traffic groups --dry-run: request body only")
assert not os.path.exists(AUDIT), "an audit file exists: something was sent"
print("audit log: none")
```

Run it:

```bash
"$SCRATCH/venv/bin/python" -I "$SCRATCH/traffic_views_smoke.py" target/debug/illumio-tui ../illumio-mcp-server/.env "$SCRATCH/state-tv"
```

Expected: the query costs requests once; the five views, `u` / `p`, the picker, the zoom and `b` leave the API call counter unchanged; `traffic mesh` returns rows with the mesh fields; the dry run reports `cache: none`; no audit file. Record the flow count, the call counts and the CLI `api_calls`. If an assertion fails, stop and debug (systematic-debugging); do not retry against the PCE in a loop.

- [ ] **Step 7: Review section** (append to `tasks/todo.md` under the checklist)

Write `## Review (Traffic views)` with: what shipped (one line each: the IP lists tab and five screens; the keys and the `g` picker; Flows group columns, detail labels and bytes; Blocked by destination group and `w` on its rows; Mesh with `u` / `p`, drill and `w`; Treemap with cursor, zoom, drill and bytes; `traffic groups`, `traffic mesh`), the final gate's test count from Step 5, the `crates/pce` diff summary and the empty dependency diff, the smoke's flow count, call counts and CLI `api_calls`, the statement that no write was sent to any PCE, and every deviation reported by a task.

- [ ] **Step 8: Commit**

```bash
cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace
git add README.md docs/architecture.md docs/index.html tasks/todo.md
git commit -m "docs: traffic views, the g picker, mesh, treemap and the CLI groups/mesh; tracking and review

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

## Self-review

Checked against the spec with the plan finished, and against a replay: every task was applied in order to a scratch worktree at `34b4fa9`, with the gate green after each.

**1. Spec coverage**

| Spec | Requirement | Task (test) |
| --- | --- | --- |
| §1 | every view a projection of the loaded flows, no new request | 5, 6, 7 (every key test asserts an empty `Vec<Cmd>`); 9 smoke (API counter unchanged after the query) |
| §2 | edition, no new dependency, no request from views/keys/zoom/columns, Elm style, label model, trailer, read-only smoke | Global Constraints; 9 Step 5 dependency diff; every commit block |
| §3 | IP lists the fourth Objects tab; five screens; `:iplists` / `:ip`; tab row, help, hints, docs; snapshots re-recorded; IP list snapshots move to Objects; no behaviour change in the table, detail, sizes, writes | 1 (`the_ip_lists_tab_loads_once_and_r_bypasses_the_cache_once`, `ip_lists_wait_for_the_name_index_and_the_command_opens_their_tab`, `the_tab_row_shortens_only_when_the_full_row_does_not_fit`, runtime `opening_the_ip_lists_tab_costs_nothing_after_startup_and_r_bypasses_once`, `update_ip_list_puts_only_the_changed_ranges`, `create_ip_list_on_the_objects_ip_lists_tab_posts_once_and_reloads_once`; 65 re-recorded + 5 moved snapshots) |
| §4.1 | `group_keys` from `limits.scope_keys` at load and on `:profile`; session only; `all_keys`; `effective_keys` once per rebuild | 5 (`the_label_model_sets_the_group_keys_and_empty_falls_back_to_app_env`, `all_labels_groups_by_every_seen_key_until_a_profile_switch`) |
| §4.2 | the `g` picker: entries and counts, `j`/`k`/arrows, Space, Enter, Esc, all labels, editable `keys:` line, empty text falls back, unknown keys allowed, applying clears drill and zoom, `g group` hint | 5 (picker tests, `g_opens_the_picker_and_enter_regroups_every_view_without_requests`, `esc_keeps_the_keys_and_the_drill_and_applying_clears_the_drill`, snapshot `traffic_key_picker_80x24`); 7 (`new_keys_and_a_new_result_drop_the_zoom`) |
| §4.3 | Flows group columns at 80, `-` for non-workloads, detail labels and bytes; App-to-app by the keys; Blocked by destination group with its columns; `w` behind a Blocked row | 5 (`the_flow_and_blocked_columns_fit_80_columns_with_the_group_columns`, `blocked_cells_name_one_host_or_count_them_and_count_more_sources`, `w_on_a_blocked_row_drafts_from_the_first_flow_behind_it`, `flow_detail_shows_both_label_sets_and_the_bytes`, snapshots `traffic_flows_groups_80x24`, `traffic_blocked_groups_80x24`) |
| §4.4 | `group_values`, `group_text`, `keys_seen`; `group_by_labels` keeps its signature; `blocked_summary(flows, keys)` with `dst_group`, `dst_names`, group-text `sources`; bytes on `ResolvedFlow` and `FlowGroup` | 2 (all eight aggregate/resolved tests) |
| §5.1 | `MeshRow`, `MeshColumns`, `mesh`; grouping with optional user/process; sort; decisions like `FlowGroup`; process placed by direction, none means source | 3 (four mesh tests) |
| §5.2 | `TrafficView::Mesh` between App-to-app and Blocked; `mesh_cols` both on; columns and widths; `u`/`p` regroup and reset the cursor; `Enter` drills with a mesh key, title, Esc; `s`, `/`, `w`; meta | 6 (three update tests, `mesh_columns_follow_the_toggles_and_fit_80_columns`, `search_captures_keys_on_aggregate_mesh_and_blocked_views`, mesh snapshots) |
| §6.1 | `Tile`, `Rect`, `Weight`, `layout`: squarified, cell aspect, `other (N)`, zero weight, determinism | 4 (six treemap tests) |
| §6.2 | `TrafficView::Treemap`, `TreemapState`, recompute on rebuild and resize, rendering, cursor, zoom, Enter to Flows, Esc, `b` with refusal, meta, zoomed title, clamped cursor | 7 (six update tests, treemap snapshots incl. zoomed and light) |
| §6.3 | no animation, no extra legend, no treemap in the CLI | 7, 8 (nothing added) |
| §7 | `traffic groups`, `traffic mesh`, shared query flags without `--group` / `--blocked`, `--keys` default, envelope, table headers, dry run / cost guard / cache as `traffic query` | 8 (three CLI tests, `every_flag_has_help_text`) |
| §8 | unknown keys show `-`; empty treemap text; no bytes: `b` refuses, `-` in tables, `0` in JSON; truncated marker on mesh and treemap | 5 (`esc_keeps_…`: `nope` shows `-`); 7 (`an_empty_result_draws_the_treemap_like_an_empty_table`, `b_weighs_by_bytes_or_says_there_are_none`, `mesh_and_treemap_keep_the_truncated_marker`); 8 (`bytes_cells_are_a_dash_only_when_no_row_has_bytes`, `traffic_groups_sum_bytes_and_honour_keys`: `bytes` 0) |
| §9 | pce unit tests, rich fixture, TUI update tests (five-view cycle, picker, keys reach flows/blocked/mesh/treemap, `u`/`p`, mesh drill, treemap cursor/zoom/drill/`b`, no `Cmd`), the listed snapshots, CLI tests, headless smoke | 2-4; 2 Step 1; 5-7 (`the_picked_keys_reach_the_mesh_and_the_treemap` covers mesh and treemap rows); 5-7 snapshots; 8; 9 Step 6 |

Gaps found and fixed while reviewing: three §8/§9 requirements had no test in the first draft (bytes `-` in CLI tables, the truncated marker on Mesh/Treemap, the picked keys reaching the Mesh and Treemap rows); Task 8 and Task 7 now carry them, and the replay counts above include them.

**2. Placeholder scan.** No `TBD`, `TODO`, "implement later", "similar to Task N", "add appropriate error handling" or test-less test steps. Every code step carries the code that was replayed; inside code blocks `…` appears only in the Interfaces summaries (`/* … */` for unchanged fields) and in one existing UI string (`"loading objects…"`), never as elided code. The fixture is produced by a script given in full, with checksums.

**3. Type consistency.** The names later tasks consume are the ones earlier tasks produce, and the replay compiled them in order: `group_key` / `group_text` / `keys_or_default` / `keys_seen` / `compact_count` / `Dominant` (Task 2) in Tasks 3-8; `MeshColumns`, `MeshKey`, `MeshRow::{key, contains, dominant, service_text}`, `mesh`, `mesh_key` (Task 3) in Tasks 6 and 8; `treemap::{Rect, Tile, Weight, layout, layout_zoomed}` (Task 4) in Task 7, imported as `TreemapRect` in `model.rs` and `traffic.rs`; `FlowRow`, `Drill`, `TrafficState::{effective_keys, loaded_flows, has_bytes, first_flow, rebuild_views}`, `KeyPicker`, `apply_group_keys` (Task 5) in Tasks 6-7; `TrafficView` grows `Mesh` (Task 6) then `Treemap` (Task 7), and each task updates every exhaustive `match` on it (`is_searching`, the search, table, `ensure_visible` and wheel arms, `flow_rule_key`, the test closures). The spec's binding signatures are kept verbatim (`group_values`, `group_text`, `keys_seen`, `MeshRow`, `MeshColumns`, `mesh`, `Tile`, `Rect`, `Weight`, `layout`, `blocked_summary(flows, keys)`); `Rect` and `Weight` also derive `Default`, and `TreemapState` adds `area`.

**4. Review Focus.** The five lines above each name the test that pins them and the task that owns it: Task 1 (IP lists tab state), Tasks 2 and 5 (non-workload endpoints), Tasks 4 and 7 (treemap sizes), Tasks 5 and 7 (stale drill and zoom), Tasks 2 and 5 (more than five names). Request discipline itself is spec-covered (§2) and asserted in every TUI key test.
