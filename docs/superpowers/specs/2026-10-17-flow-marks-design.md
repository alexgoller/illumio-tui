# Flow marks: design

Date: 2026-10-17. Status: approved by Alex in conversation on 2026-10-16 (points 1-9 below), awaiting spec review.

## Goal

Let the admin mark traffic they have judged: hide known noise, flag what matters, and record what was reviewed or accepted as blocked. The marks live in a local file, are re-applied to every result, and change nothing on the PCE.

Hard rules, unchanged:

- Marks never cause a PCE request. They are matched against loaded flows only, and their form completes from the loaded name index only.
- No new dependencies. `Cargo.lock` has `toml` 1.x (`toml_parser`, `toml_writer`) and no `toml_edit`. See File writes.
- A broken marks file never crashes the TUI or the CLI. It shows a warning and turns marks off.
- Marks are not policy. The policy generator ignores squelch.

## Definitions

- **Mark:** one `[[mark]]` entry: an action, a match (`src`, `dst`, `service`, `decision`), an optional PCE scope, an optional expiry and a comment: why it was marked.
- **Match:** directional, source → destination, on a `ResolvedFlow`. Every field given must match; an absent field matches anything.
- **Active:** not expired (`now < expires`), and in scope for the current PCE.
- **Hit:** the winning active mark of one flow, as `(action, mark id)`.
- **Highlight actions:** `critical`, `important`, `revisit`. **Quiet actions:** `accept-blocked`, `reviewed`. **Squelch:** `squelch`.

## The file

`marks.toml` next to `config.toml`: `$XDG_CONFIG_HOME/illumio-tui/marks.toml`, else `~/.config/illumio-tui/marks.toml` (the same rule as `config::default_config_path`, so tests point it at a temp dir with `XDG_CONFIG_HOME`). A missing file means no marks. TOML only; JSON goes through `marks export` / `marks import`.

```toml
# illumio-tui marks. The tool rewrites this file: comments are not kept, put text in `comment`.

[[mark]]
id = "m-7k2q9x"
action = "squelch"
src = { labels = { app = "monitoring" } }
dst = "any"
service = "5666/tcp"
expires = "2027-01-15T23:59:59Z"
comment = "Nagios NRPE, known"
created = "2026-10-17T09:12:00Z"
created_on = "pce.example.com"

[[mark]]
id = "m-c41fz0"
action = "accept-blocked"
src = { ip_list = "rfc1918" }
dst = { labels = { app = "shop", env = ["prod", "dr"] } }
service = "8000-8010/tcp"
decision = "blocked"
pce = "pce.example.com"
comment = "legacy batch, blocked on purpose (CHG-1182)"
created = "2026-10-17T09:20:00Z"
created_on = "pce.example.com"

[[mark]]
id = "m-q9e2aa"
action = "critical"
src = "any"
dst = { workload = "db-01" }
pce = "pce.example.com"
org = 1
created = "2026-10-17T09:31:00Z"
created_on = "pce.example.com"
```

### Fields

| Field | Set by | Values |
|---|---|---|
| `action` | user, required | `critical`, `important`, `revisit`, `accept-blocked`, `reviewed`, `squelch` |
| `src`, `dst` | user, required | a side (below) |
| `service` | user | `5666/tcp`, `8000-8010/tcp`, `udp` (any port of the protocol), a protocol name or number `proto_number` knows (`icmp`, `47`), or a list of these (OR). Absent: any service. |
| `decision` | user | `allowed`, `potentially_blocked`, `blocked`, or a list (OR). Absent: any. Matched against the reported `policy_decision`, never the draft decision. |
| `draft_decision` | user | Same values as `decision`, matched against the draft decision (`D`, #29). A mark with `draft_decision` applies only while draft decisions are loaded for the result on screen; otherwise it is inactive for that result and the views say `N marks wait for the draft decision (D)`. The CLI applies it only with `--draft`. Example: `decision = "blocked"` + `draft_decision = "allowed"` marks the flows the pending policy fixes. |
| `pce` | user | the PCE host the mark is limited to (case-insensitive); absent: portable |
| `org` | user | optional org id, only with `pce` |
| `expires` | user | RFC 3339 UTC timestamp; absent: never |
| `comment` | user | **required**: why the flows were marked (free text, e.g. a ticket or the owner's answer). A file entry without one loads with a warning (`m-… has no comment`) and shows `(no comment)`. |
| `id` | tool | `m-` + 6 Crockford base32 characters |
| `created` | tool | RFC 3339 UTC |
| `created_on` | tool | the PCE host the mark was created on (information only; it never scopes) |

**Sides.** Exactly one of:

- `"any"`;
- `{ labels = { key = "value" | ["v1", "v2"] } }`: the endpoint is a workload and has a label under every listed key, with one of the listed values (a list is OR). Keys are compared exactly, values exactly (label values are case-sensitive on the PCE). Unresolved labels (key `?`) never match;
- `{ workload = "web-01" }`: the endpoint's hostname, else its workload name (`ResolvedEndpoint::hostname`, `name`), case-insensitive. Requires `pce`;
- `{ ip = "10.0.0.0/8" }`: an address or CIDR, v4 or v6, containing the endpoint's `ip` (workloads included);
- `{ ip_list = "rfc1918" }`: the name is one of the endpoint's `ip_lists` as Explorer reported them (case-insensitive). No range evaluation, no request.

FQDN, process and user are not matchable in v1.

### Validation

`MarkFile::parse` returns every problem at once, each with its mark's position (`mark 3 (m-c41fz0): workload needs pce`). Any problem makes the whole file invalid (marks off; see Errors). Rules:

- a side with zero or several kinds, an unknown kind, an empty `labels` table or an empty value list;
- a bad CIDR (`pce::write::specs::parse_cidr` is made `pub(crate)` and reused), a bad port, a range with `from > to`, an unknown protocol, a port on a protocol without ports;
- `workload` without `pce`; `org` without `pce`;
- `accept-blocked` without `decision`, or with `allowed` in it;
- an unknown action or decision; a bad timestamp;
- two marks with one `id`.

Unknown fields, in a mark or at the top level, are not errors: they are kept (see File writes).

## Matching and precedence

### One flow

`ActiveMarks::classify(&ResolvedFlow) -> Option<Hit>`:

1. Take every active mark that matches.
2. **The strongest action wins**: critical > important > revisit > accept-blocked > reviewed > squelch. So a highlight always beats squelch, and so does a quiet action: a critical flow is never hidden by a broad squelch.
3. **Same action, several marks:** the action is the same, so only the reported id (and comment) depends on the choice. The most specific mark wins, by a score summed over its fields:
   - side: workload 5, ip /32 or /128 4, labels 2 + one per key, ip_list 2, a wider CIDR 1, any 0;
   - service: one port 3, a range 2, a protocol 1, absent 0;
   - `decision` given 1; `pce` given 1.
   - Ties: the earlier `created`, then the lower `id`.

A more specific mark of a weaker action never overrides a broader stronger one. That keeps the rule one line long, and it errs toward showing.

### Aggregated rows: filter the flows, then aggregate

Marks are applied to flow records **before** any view aggregates them:

- **Squelch removes flow records, not rows.** Every view aggregates the unsquelched flows, through the existing aggregators unchanged (`group_by_labels`, `group_by_service`, `mesh`, `app_sections`, `app_sections_both`, `app_peers`, `treemap::layout`). A row disappears exactly when every flow in it is squelched. A partly squelched row stays, and its counts are those of the flows it still shows, so the visible rows always add up to the visible flows.
- **Highlights: any.** A row shows the strongest highlight any of its visible flows has. A critical flow must surface wherever it is aggregated.
- **Quiet actions: all.** A row is dimmed as reviewed (or accepted) only when every visible flow in it is reviewed or accepted, and no flow has a highlight. A row that is only partly reviewed is not dimmed: dimming it would hide the unreviewed part.
- **Per-row counts without duplicating key logic.** The marked flows are partitioned by action, and each subset goes through **the same aggregator** as the view. The row summaries are joined to the view's rows by the row's own key: `FlowGroup` (`src`, `dst`, `service`), `MeshRow::key()`, `PeerRow` (section app, direction, `peer`, `service`), `AppPeer` (`peer`, `service`, `decision`). The subsets partition the flows, so this costs at most one more aggregation.

With `M` on, the views aggregate every flow, and squelched flows are counted in the summary like any action.

### Counts per view

`squelched {rows, flows}` for the view on screen:

- **flows:** the squelched flow records in the view's own input, after the focus, the drill and the view's decision filter (App-to-app `d`, Blocked's blocked and potentially blocked). A record counts once, even in `both`, where it sits in two sections.
- **rows:** the row keys of the squelched subset's aggregation that the unsquelched aggregation does not have, which are the rows that disappeared. Flows view: rows = flows. Sections and Blocked: peer rows (a section header that loses every row disappears and is not counted). Mesh: mesh rows. Treemap: flows only (tiles are layout, not rows). App overlay: per panel, peers.

Invariant, tested for every view: unsquelched aggregation's flows + squelched flows = all flows of the view's input.

## Scope and expiry

- **PCE scope.** A mark without `pce` is portable: label names, CIDRs, IP list names and ports mean the same on any PCE. A mark with `pce` is active only when the profile's host equals it (case-insensitive) and, when `org` is given, the profile's `org_id` equals it. `workload` marks are always scoped. `:profile` re-evaluates which marks are active; no reload is needed.
- **Defaults.** Marks made in the TUI default to the current PCE (`pce = host`, no `org`), and the form can widen them to any PCE. Marks made with `marks add` are portable unless `--pce` is given, because scripts usually write them for several PCEs.
- **Expiry: UTC.** `expires` is stored as an RFC 3339 UTC instant. The inputs: `1d`, `7d`, `30d`, `90d` (now + N × 24 h), a date `YYYY-MM-DD` (23:59:59Z of that day), an RFC 3339 instant, or `never` (no field). UTC, because:
  - it is what the CLI's date-only `--to` already means (`cli::traffic::parse_time`);
  - Explorer's timestamps are UTC;
  - a mark file shared between admins in different time zones means the same instant for all of them.
  - The TUI shows `expires 2027-01-15 (UTC)` and `12d left`.
- **Form defaults per action:** `squelch` 90d, `accept-blocked` 90d, `reviewed` 30d, `revisit` 7d, `critical` and `important` never. A hide or a forgiveness should lapse on its own; a flag should not.
- **Expired marks** stay in the file. They are inactive and greyed in `:marks` and `marks list` (`active: false`). Nothing deletes them except `x`, `marks rm` or `marks rm --expired`. At startup, marks expiring within 3 days add one status line: `2 marks expire within 3 days · :marks`.

## File I/O: reads, writes, concurrency

### Reading and reload

- `MarksStore` (`illumio-tui`) holds the path, the last stamp (mtime, length) and the parsed result (`Result<MarkSet, MarksError>`).
- `MarksStore::refresh()` costs one `stat`. It parses again only when the stamp changed. It is called at the start of `TrafficState::rebuild_views`, when the app overlay rebuilds, when the generator opens, when `:marks` opens and before every write. There is no watcher. An edit in another editor or in another instance shows on the next rebuild, or at once with `:marks`.
- Rebuilding is pure except for this `stat`: a local file, never a request. The doc comment on `rebuild_views` says so.

### File writes

**Ruling: rewrite the file through serde, keep unknown fields, do not keep comments, and back up once.**

- `toml_edit` is not in `Cargo.lock`, and adding it is not allowed. `toml` 1.x serializes a document but drops comments and formatting.
- **Unknown fields are kept.** Each mark and the top level carry `#[serde(flatten)] extra: toml::Table`, as `config::ConfigFile` and `Profile` do. A newer version's fields survive a rewrite by this one.
- **Order is kept.** Marks are written in file order, and new marks are appended. Known fields are written in the order of the table above.
- **Comments.** Every write starts with the one-line header shown above. The first write that would drop a comment line (a line whose trimmed text starts with `#` and is not the header) first copies the file to `marks.toml.bak` (mode 0600, replacing an older backup), and the status says `marks.toml had comments: kept in marks.toml.bak`.
- **An invalid file is never written.** Writing it would lose the marks that did not parse. `m`, `:marks` edits and the CLI's `add`, `rm` and `import` refuse with the parse errors. The file must be fixed by hand.

### Atomic writes and permissions

1. The directory is created if needed (0700 on Unix).
2. The text is written to `marks.toml.tmp-<pid>` in the same directory, created with mode 0600, then `sync_all`.
3. The temp file is renamed over `marks.toml`.
4. The new stamp is recorded, so this instance does not parse its own write again.

**Mode 0600.** Marks name internal hosts, networks and apps, and `config.toml` is already 0600 (`profile use`). An existing file with wider permissions keeps working; the next write sets 0600.

### Several instances

A second TUI, the CLI or a hand edit may change the file between this instance's read and its write. Every write is an **operation**, not a whole state:

- add a mark;
- replace mark `id` with new fields;
- delete mark `id`;
- set `expires` of mark `id`;
- import a list.

The write runs under a lock:

1. **Lock.** The instance creates `marks.toml.lock` with `create_new` (O_EXCL; std only). It retries every 25 ms for up to 1 s. A lock file older than 10 s is stale (a crashed writer) and is removed once. If the lock cannot be taken, the write fails with `marks.toml is locked by another illumio-tui (marks.toml.lock): try again`.
   - `File::lock` would be simpler, but it needs Rust 1.89 and the workspace's `rust-version` is 1.88.
2. **Reload.** The file is read and parsed again, whatever its stamp says.
3. **Apply.** The operation is applied to the fresh marks. For an edit, a delete or an extend whose `id` is gone, the write stops with `mark m-… was deleted elsewhere`. Otherwise the last writer wins per mark: two edits of one mark keep the later one.
4. **Write, unlock.** The file is written atomically, then the lock file is removed.

So two instances that each add a mark both keep their mark, and nothing is lost to a whole-file overwrite. The only lost update is two edits of the same mark, and that is the intended "last writer wins".

### Ids

- **New marks.** `m-` + 6 Crockford base32 characters (30 bits) from an FNV-1a hash of (the creation time in ns, the pid, the mark's canonical text). FNV-1a is about ten lines in `pce::marks`; the lockfile has `rand`, but only as an indirect dependency. If the id is already in the file, the hash is taken again with a counter appended.
- **Hand-written marks without an id** get a derived id, `h-` + 6 characters of the FNV-1a hash of their canonical text. The id is stable across loads and machines, so `marks rm h-…` works. The file is not rewritten only to add ids; the next write stores them.

## Data model

### `pce::marks` (new, pure: no I/O, no clock)

```rust
pub enum Action { Critical, Important, Revisit, AcceptBlocked, Reviewed, Squelch } // Ord: strongest first
pub enum Side { Any, Labels(BTreeMap<String, Vec<String>>), Workload(String),
                Ip { addr: IpAddr, bits: u8 }, IpList(String) }
pub enum ServiceSpec { Proto(u8), Ports { proto: u8, from: u16, to: u16 } }
pub struct PceScope { pub host: String, pub org: Option<u64> }
pub struct PceId { pub host: String, pub org: u64 }                // the profile in use
pub struct Mark { pub id: String, pub action: Action, pub src: Side, pub dst: Side,
                  pub services: Vec<ServiceSpec>,                  // empty: any
                  pub decisions: Vec<PolicyDecision>,              // empty: any
                  pub scope: Option<PceScope>, pub expires: Option<Timestamp>,
                  pub comment: String, pub created: Option<Timestamp>, pub created_on: Option<String> }
pub struct MarkSet { pub marks: Vec<Mark> }                         // validated, file order
pub struct Hit { pub action: Action, pub mark: usize }              // index into MarkSet::marks
pub struct ActiveMarks<'a> { .. }                                   // in scope, unexpired, by action
pub struct RowMarks { pub by_action: [u64; 6] }                     // flow records per winning action
pub enum Marker { Critical, Important, Revisit, Accepted, Reviewed, Squelched, PartlySquelched(u64) }

impl Mark { pub fn matches(&self, f: &ResolvedFlow) -> bool; pub fn specificity(&self) -> u32; }
impl MarkSet {
    pub fn active(&self, pce: &PceId, now: Timestamp) -> ActiveMarks<'_>;
    pub fn from_file(file: &MarkFile) -> Result<MarkSet, Vec<MarkProblem>>;
}
impl ActiveMarks<'_> {
    pub fn is_empty(&self) -> bool;
    pub fn classify(&self, f: &ResolvedFlow) -> Option<Hit>;
}
pub struct Partition { pub kept: Vec<ResolvedFlow>, pub by_action: [Vec<ResolvedFlow>; 6],
                       pub hits: usize }
pub fn partition(flows: &[ResolvedFlow], m: &ActiveMarks, keep_squelched: bool) -> Partition;
pub fn marker(row_flows: u64, rm: &RowMarks, squelched_hidden: u64) -> Option<Marker>;
pub fn side_text(s: &Side) -> String;        // the side grammar below
pub fn parse_side(s: &str) -> Result<Side, String>;
```

- `MarkFile` / `MarkEntry` are the serde mirror of the TOML, with `extra: toml::Table` in `illumio-tui`. They hold strings and are converted by `MarkSet::from_file`, so `pce` gains no `toml` dependency. The conversion lives in `illumio-tui::marks` and calls `pce::marks` parsers (`parse_side`, `ServiceSpec::parse`).
- **Speed.** `ActiveMarks` groups the marks by action, strongest first, so `classify` stops at the first action with a match. 200 000 flows against 200 marks is the budget. A release-mode test (`#[ignore]`, run in CI) asserts under 300 ms.
- **Copies.** `partition` copies the flows only when something matches. `kept` is a copy only when at least one flow is squelched, and the subsets copy only the marked flows. At the 200k cap that is at most one more copy, the same order as `refresh_focused`.
- **Annotations.** `FlowGroup`, `MeshRow`, `PeerRow` and `AppPeer` gain `#[serde(skip_serializing_if = "Option::is_none")] pub marks: Option<RowMarks>`, filled after aggregation by `pce::marks::annotate_*`, one per row type. They join by key from the per-action aggregations, so the aggregators keep their signatures.

### Side grammar (text)

The TUI form, the CLI flags and `:marks` all use one text form. It follows the prefixes of the traffic filter (`FieldKind::TrafficActors`):

- `any`;
- `app=shop env=prod|dr`: labels. Tokens are separated by spaces or commas, and `|` within a value is OR. Quote values with spaces;
- `wl:web-01`;
- `ip:10.0.0.0/8`;
- `ipl:rfc1918`.

`|` means OR inside one value, while the traffic filter uses it between groups. A side has no groups, so the two never meet in one field. The hint says `| = or`.

### `illumio-tui`

- `marks.rs` (new):
  - `MarkFile`, `MarkEntry` (serde, `extra`);
  - `MarksStore` (`path`, `stamp`, `refresh`, `apply(op)` with the lock, reload, atomic write and `.bak`);
  - `MarkOp`;
  - `MarksError` (`Parse(Vec<MarkProblem>)`, `Io`, `Locked`, `Gone(id)`);
  - `default_marks_path()`.
- `TrafficState` gains:
  - `marks: MarksStore`;
  - `show_squelched: bool` (`M`);
  - `marked: Option<MarkedView>`, computed in `rebuild_views` next to `focused`: the `Partition` of `view_flows()`, the per-view squelched counts and the hit per flow for `FlowRow`;
  - `FlowRow` gains `mark: Option<Hit>`.
- `TableView` gains `marker: Option<fn(&T) -> Option<Marker>>`. The table widget (`ui/widgets/table.rs`) draws it as a 2-cell gutter before the first column. It is **not a column**, so column indices, `s`'s cycle, the default sort, search and `:export` are unchanged.

## TUI

### Keys

`m` and `M` are free on the Traffic screen and in the app overlay. `traffic_key` binds neither. The only `m` in the TUI is the Workloads filter (`workloads_key`: `next_managed`), on another screen. `M` is unused everywhere. Both stay `m` and `M`.

- `m` works in Flows, App-to-app (pairs, by service, section peer and header rows), Mesh, Blocked, and the app overlay's Inbound and Outbound panels. While a table search is being typed, keys go to the search as today.
  - On the treemap: `m: Enter drills to the flows, mark there`. A tile is a layout cell, and drilling gives the exact pair.
  - On the app overlay's Policy and Workloads panels: `m marks traffic rows (Inbound, Outbound)`.
- `M` toggles `show_squelched` for every traffic view and the app overlay. It is session-only and rebuilds the views. It sends no request.
- `:marks [action]` opens the marks overlay, filtered to one action when one is given (`:marks revisit`).

### Display

| Hit (row marker) | Gutter | Row |
|---|---|---|
| critical | `●` in `theme.bad`, bold | the group or peer cell in `bad` |
| important | `●` in `theme.warn` | the group or peer cell in `warn` |
| revisit | `◆` in `theme.accent` | unchanged |
| accept-blocked (all flows) | `≈` dim | whole row dim |
| reviewed (all flows) | `✓` dim | whole row dim |
| squelched, `M` on (all flows) | `~` dim | whole row dim |
| partly squelched, `M` off | `~` dim | the flows cell gets a dim `~12` suffix (the hidden records); its sort key stays the visible count |

- **Colours.** Decision columns keep their decision colours. The highlight colours sit only on the gutter and the name cell, so a red critical marker never reads as a decision. No theme token is added; both palettes already have `bad`, `warn`, `accent` and `dim`.
- **Status meta.** The view's panel meta (where the row count is) appends `37 rows (412 flows) squelched · M shows them`, or with `M` on `37 rows (412 flows) squelched, shown ~ · M hides them`. With no squelch hit nothing is added.
- **Blocked.**
  - Flows whose hit is `accept-blocked` count as **accepted**, not as blocked problems. The section header reads `41 blocked · 12 accepted`, and the view meta reads `41 blocked flows · 12 accepted`.
  - The header's `Totals` text is computed with the accepted records taken out of `blocked` / `potentially_blocked`. `Totals` is not changed in `pce`: the TUI builds the header text from `Totals` and the section's `RowMarks`.
  - Fully accepted rows are dimmed in place; the sort is unchanged.
- **Flows detail** (`Enter`). One line: `mark: squelch m-7k2q9x · Nagios NRPE, known · expires 2027-01-15 (UTC)`, plus `also matches: m-… (reviewed)` when weaker marks match too.
- **Treemap.** It lays out the unsquelched groups (or all of them with `M`). A tile whose group has a highlight shows its gutter glyph before the label. The meta gets `412 flows squelched`.
- **App overlay.** The panels use `app_peers` over the partitioned `flows_for` and show the same gutter. The panel title says `3 peers (41 flows) squelched`.
- **Generator.** See below.

### The mark form (`m`)

- **Overlay.** `Overlay::MarkForm(Box<MarkForm>)`. `MarkForm` wraps the generic `Form` for its fields, keys, completion and rendering (`render_form`), plus `MarkCtx`:
  - the prefill;
  - the editing id, if any;
  - the view it came from (to return to the app overlay as `form_from_app` does).
- **Submit.** `FormEvent::Submit` on a `MarkForm` goes to `marks_submit`, never to `edit::build_intent` or `Cmd::PlanWrite`. It is not gated by `:writes`, because no PCE write exists.
  - `Form::new` needs an intent, so `EditIntent` gains a unit variant `Mark`. `build_intent` returns `Err("internal: mark form")` for it, and a test asserts that it is never reached.

```
┌ New mark · App-to-app in · shop/prod ← monitoring/prod 5666/tcp ────────────┐
│ Action       squelch                      Ctrl-N: critical important revisit… │
│ Source       app=monitoring env=prod      Ctrl-A all labels · Ctrl-T kind      │
│ Destination  app=shop env=prod                                                │
│ Service      5666/tcp                     empty: any · 8000-8010/tcp · udp    │
│ Decision                                  empty: any · blocked …              │
│ Expires      90d                          1d 7d 30d 90d YYYY-MM-DD never      │
│ Scope        this PCE (pce.example.com)   Ctrl-N: any PCE                      │
│ Comment      why: CHG-1182, owner confirmed (required)                        │
├───────────────────────────────────────────────────────────────────────────────┤
│ matches 3 rows · 41 flows in this result · hides them (M shows)               │
│ 5 of them stay shown: critical m-q9e2aa wins                                  │
│ Enter saves to marks.toml · Esc cancels                                       │
└───────────────────────────────────────────────────────────────────────────────┘
```

- **Fields.**
  - Action, Expires, Scope and Decision complete from fixed lists. They use new `FieldKind`s: `MarkAction`, `MarkExpiry`, `MarkScope`, and the existing `Decisions`.
  - Source and Destination use a new `FieldKind::MarkSide`. It completes label `key=value` from the index, `ipl:` names from the index, and `wl:` from the loaded workloads, with no request.
  - Service is `FieldKind::Text`, with the hint shown.
- **Prefill from the row.**
  - **Workload endpoint (Flows).** The labels under the keys in force (`TrafficState::keys`) that the endpoint has. With none: `wl:<hostname>`.
  - **Non-workload endpoint.** `ipl:<first reported list>`, else `ip:<ip>`. An FQDN-only endpoint becomes `any`, with the hint `fqdn cannot be marked`.
  - **Group key** (pairs, sections, mesh, app overlay). Each non-`-` value under its key. `ipl:X` becomes `ipl:X`. A display-name group (an IP, or an unlabelled workload) becomes `ip:` when it parses as an address, else `wl:`. A `-` value (the endpoint lacks that key) is dropped, because a mark cannot require a missing key.
  - **Section and app overlay rows.** Direction-aware: in `in`, the peer is the source and the app is the destination. A section header fills only the app side; the peer side is `any`.
  - **Service.** The row's service. It is empty for plain pairs and headers.
  - **Decision.** The row's decision filter or decision. Blocked's filter is `blocked|potentially_blocked`, an App-to-app `d` filter fills it the same way, and an app overlay peer gives its own decision. Otherwise it is empty.
  - **Action.** In Blocked the default is `accept-blocked`, elsewhere `reviewed`. Expires follows the action's default until it is edited.
  - **Scope.** `this PCE`.
- **Widening.** The admin widens by deleting tokens or fields:
  - `Ctrl-U` clears a field;
  - `Ctrl-A` swaps a side between the keys in force and every label of the row's endpoint (the existing alternates, as in the `w` form);
  - `Ctrl-T` cycles a side's kind among those the row allows: labels, then `wl:`, `ip:`, `ipl:`, then `any`.
- **Live preview.** It is computed on every key, locally. It classifies the loaded result (`loaded_flows`, all of it, not only the focus) against the active marks plus the draft mark. It shows:
  - `matches N rows · M flows in this result`, where rows are the current view's rows that hold a matching flow;
  - for squelch, `hides them (M shows)`;
  - `K of them stay shown: <action> <id> wins` when stronger marks take some flows;
  - `matches nothing in this result` (allowed: marks are for the future too).
  - At the 200k cap one pass is under 10 ms per mark, so there is no debounce.
- **Validation** is the file's validation for this one mark; the first error shows in red under the fields. For example, choosing `any PCE` with a `wl:` side gives `workload marks are PCE-scoped`.
- **Save.**
  - The comment is required: `Enter` with an empty comment keeps the form open, puts the cursor on Comment and says `comment required: why are these flows marked?`. The CLI `marks add` refuses without `--comment` (exit 2). Editing a mark in `:marks` keeps its comment and offers it for change; `e` (extend) asks for the reason again, prefilled.
  - `Enter` runs `MarksStore::apply(Add | Replace)`: lock, reload, write.
  - Then the views rebuild, the form closes to where it came from, and the status says `mark m-7k2q9x saved: squelch, 41 flows · :marks lists marks`.
  - A failure (locked, invalid file, I/O) keeps the form open with the error.

### `:marks` overlay

- **Overlay.** `Overlay::Marks(MarksView)`. The view is a `TableView<MarkRow>`, so `/` search and `s` sort come for free.

```
┌ Marks · 14 active · 3 expired · marks.toml ───────────── :marks revisit filters ┐
│   action          match                                     scope     expires   hits  comment
│ ● critical        any → wl:db-01                            this PCE  never       12  
│ ◆ revisit         app=crm → app=shop env=prod 443/tcp       any PCE   5d left     40  check with crm team
│ ~ squelch         app=monitoring → any 5666/tcp             any PCE   91d left   412  Nagios NRPE, known
│ ≈ accept-blocked  ipl:rfc1918 → app=shop env=prod|dr …      this PCE  never        3  CHG-1182
│   reviewed        app=web → app=shop 8443/tcp               other PCE 2026-10-01   –  (expired)
│ Enter edit · x delete · e +30d · Esc back                                        │
└──────────────────────────────────────────────────────────────────────────────┘
```

- **Rows.** Active marks come first, in file order, then out-of-scope and expired ones, greyed. `hits` counts the flows of the loaded result the mark matches, with `· N to stronger` when a stronger mark wins some of them; it is `–` for inactive marks.
- **Keys.**
  - `j`/`k` move;
  - `Enter` opens the mark form on the mark (`Replace`);
  - `x` asks `delete m-… (squelch, 412 flows)? y/n` through `Overlay::Confirm` with `PendingAction::DeleteMark(id)`;
  - `e` extends by 30 days from the later of now and the current expiry. It also revives an expired mark. On a `never` mark it does nothing and says so;
  - `Esc` / `q` back.
  - Every change is one `MarkOp` through the lock.
- **Filter.** `:marks <action>` filters by action. An unknown action gives `unknown action: critical important revisit accept-blocked reviewed squelch`.
- **Invalid file.** The overlay shows the parse problems instead of rows, and the path.

### Help

New `HELP_LINES` after the `Traffic  w` line:

- `("Traffic  m", "mark the row: squelch · reviewed · critical … (marks.toml)")`
- `("Traffic  M", "show or hide squelched rows (~)")`
- `("", ":marks [action]  the marks: Enter edit · x delete · e +30d")`

Plus the `:` line gets `:marks`. The app overlay's `Tab j/k X Enter` line gains `m`.

## CLI

### `illumio-tui marks`

`marks` reads and writes only the local file. It makes no request, needs no profile for `list`, `export` and `rm`, and uses the profile's host for `--pce` without a value.

```
illumio-tui marks list   [--action A] [--active | --expired]
illumio-tui marks add    --action A --src SIDE --dst SIDE [--service S] [--decision D]
                         [--expires 30d|YYYY-MM-DD|never] [--pce [HOST]] [--org N] --comment TEXT
illumio-tui marks rm     <id>... | --expired
illumio-tui marks export [--format json|toml]          # stdout; JSON by default
illumio-tui marks import <file> [--replace]            # JSON or TOML by extension
```

- **`list`.** JSON by default when piped: `{"data": [{"id", "action", "src", "dst", "service", "decision", "pce", "org", "expires", "active", "comment", "created", "created_on", ...extra}], "meta": {...}}`. `src` and `dst` are in the side grammar. `--table` and `--csv` print the same columns. The meta has `api_calls: 0`.
- **`add`.** It prints the new id. Validation errors exit 2, naming the field.
- **`rm`.** An unknown id exits 2 and nothing is removed. Several ids are one operation.
- **`import`.** It validates every incoming mark before writing anything. An incoming mark whose id exists is skipped (`--replace` replaces it), and one without an id gets a new one. The result is `{"added": n, "replaced": n, "skipped": [ids]}`. The JSON shape is `{"marks": [...]}` with the TOML field names, and it is what `export` writes.
- **Exit codes.** An invalid marks file makes `add`, `rm` and `import` exit 2 with the problems and write nothing. A held lock exits 3.

### Traffic commands apply marks

`traffic query`, `traffic groups` (every `--by` and `--services`), `traffic mesh` and `app`:

- **Applying.** Marks are applied as in the TUI: classify, then aggregate the unsquelched records. Squelched rows are omitted.
- **`meta.squelched: {rows, flows}`.** The counts follow the per-view rules: `query` rows = flows, `groups --by in` peer rows, and so on. It is present whenever a valid marks file has at least one active mark, zeros included.
- **`meta.marks: {active, file}`.**
- **`mark` on JSON rows.**
  - Flows: `"mark": {"action": "critical", "id": "m-q9e2aa"}`.
  - Aggregated rows: `"mark": {"action": "critical", "id": "m-q9e2aa", "flows": 3}` for the row marker's action and its strongest mark. Rows without a marker carry no `mark`. Partly squelched rows (without `--show-squelched`) add `"squelched": 12`.
  - The full per-action counts are in `"marks": {"critical": 3, "reviewed": 38}` (`RowMarks`).
  - The query rows are wrapped as `MarkedFlow { #[serde(flatten)] flow, mark }`, so `ResolvedFlow` is unchanged.
- **Tables and CSV** get a last `mark` column (the action) when marks are active.
- **`--show-squelched`.** Squelched rows are included, with `mark.action = "squelch"`. `meta.squelched` still counts them.
- **`--no-marks`.** No marks are read: no `mark` fields, no omission, no `meta.squelched`.
- **An invalid file** gives a warning on stderr and in `meta.warnings` (`marks.toml invalid: 2 problems, marks off (illumio-tui marks list shows them)`). The command runs without marks and exits 0.

## Policy generator

**Ruling: the generator ignores marks for its rules.** A squelched flow is still traffic that needs a rule once the app is enforced. It only reports them:

- `rows`, `propose` and coverage get every flow, `flows_for` as today.
- **The TUI header** gets `· 52 squelched (rules include them)` when some of the app's flows are squelched.
- **The CLI.** `gen` JSON `meta.squelched: {"flows": 52}` and the same note in table mode. There is no `rows` count, because the generator's rows are not hidden.
- **Highlights** do not affect rules or defaults either. A critical flow under an unchecked row stays unchecked. Showing marks on generator rows is out of scope for v1.
- **Test.** With marks that squelch half of an app's flows, `gen` JSON is byte-identical to the run with `--no-marks`, except `meta`.

## Errors

- **Invalid `marks.toml` (TUI).**
  - At load or reload, a status line: `marks.toml invalid (2 problems): marks off · :marks shows them`.
  - The traffic panel meta keeps a dim `marks off: marks.toml invalid` while it lasts.
  - `m` opens the `:marks` overlay with the problems instead of the form.
  - Fixing the file is picked up on the next rebuild.
- **An unreadable file** (permissions, a directory) behaves the same, with the I/O error.
- **A write fails** (locked, I/O, `Gone`, invalid): the form or overlay stays open with the error, and nothing is half-written, because of the temp file and rename.
- **Never a panic.** Every parse and I/O path returns `MarksError`. A fuzz-style test feeds truncated and garbage files.

## Testing

- **`pce::marks` units:**
  - each side kind, both directions (a mark `a → b` never matches `b → a`);
  - labels: every key required, a list as OR, a missing key, `?` labels, a non-workload with labels;
  - CIDR v4/v6, a host address, an address of the other family;
  - `ip_list` case-insensitively against `ip_lists`;
  - services: a port, a range's edges, protocol only, icmp, a list;
  - decisions;
  - precedence across all six actions: a highlight beats squelch, quiet beats squelch, a specific weaker mark does not override a broad stronger one;
  - same-action tie-breaks (specificity, then `created`, then id);
  - expiry at the boundary (`now == expires` is expired);
  - scope by host, by host and org, portable;
  - every validation rule;
  - FNV ids stable for the same canonical text, and `h-` ids stable across loads.
- **Aggregation** (`fixtures/recorded/` flows plus synthetic):
  - a row is hidden iff all its flows are squelched;
  - partial rows keep the visible counts plus `~N`;
  - highlight-any and quiet-all;
  - Blocked's accepted counts;
  - the invariant visible + squelched = input, per view (pairs, by service, in, out, both, mesh, Blocked, app peers);
  - the squelched row counts per view, with `both` counting a record once in flows.
- **Store** (tempdir):
  - a round trip keeps unknown fields and order;
  - a file with comments → `.bak` once, then the header;
  - the temp file plus rename, mode 0600 (Unix);
  - an invalid file is never written;
  - mtime reload, and no reparse after its own write;
  - two stores adding to one file keep both marks;
  - a stale lock is removed, a live lock gives `Locked`;
  - `Gone` for an edit of a deleted id.
- **TUI** (`test_support`, insta):
  - `m` in each view and panel opens the prefilled form (a snapshot per view);
  - `m` on the treemap and on the Policy panel gives the status texts;
  - `Ctrl-A`, `Ctrl-T` and deleting tokens change the preview;
  - Enter writes the file and rebuilds;
  - **no `Cmd` is returned by any marks key** (no request);
  - `M` toggles the gutter and the meta text;
  - `:marks` with `Enter`, `x` + `y`, `e`, and `:marks revisit`;
  - an invalid file: the status line, no crash, `m` opens the problems;
  - the help lines are present.
- **CLI** (`assert_cmd`, `XDG_CONFIG_HOME` in a tempdir, wiremock with recorded traffic):
  - `marks add`, `list`, `rm`, `export`, `import` round trip, `import` refused atomically on one bad mark;
  - `traffic query` and `groups --by in` with marks: omitted rows, `meta.squelched`, `mark` fields;
  - `--show-squelched`, `--no-marks`;
  - an invalid file: a warning and exit 0;
  - `gen` identical apart from `meta`.
- **Performance.** 200k flows × 200 marks under the budget (release, `#[ignore]`, in CI).
- **Manual smoke on demo100.** Read-only, with marks in a temp `XDG_CONFIG_HOME`: `traffic groups --by in` with one squelch and one critical mark. Report `meta.squelched`. Only the traffic query itself is sent; marks add no request.

## Out of scope (v1)

- Syncing marks to the PCE (labels, rule descriptions, Explorer saved filters) or sharing beyond `export`/`import`.
- Matching on FQDN, process, user or transmission.
- Negation, wildcards or regex in sides; time-of-day marks.
- `m` on the treemap, the Policy, Objects, Workloads and Events screens, and generator rows.
- A `mark` column in `:export`.
- Keeping comments on rewrite (needs `toml_edit`).
- A file watcher.
- Auto-deleting expired marks.

## Review focus

1. **Honest counts.** Filter-then-aggregate, the per-action aggregations joined by row key (no copied key logic), and the invariant test across every view, including `both` and Blocked's accepted counts.
2. **Precedence.** Strongest action first, so no squelch ever hides a highlight. Specificity only picks the reported id.
3. **File safety.**
   - the lock, then reload, then apply the operation, then the atomic write;
   - never writing an invalid file;
   - the `.bak` before comments are dropped;
   - mode 0600;
   - the stale lock rule.
4. **No requests.** No marks path returns a `Cmd` or calls the client; the form's completion uses loaded data only.
5. **The generator is unchanged by marks**, apart from its counts.
6. **Keys.** `m`/`M` reach no other handler in the traffic views and the app overlay, and a search being typed still takes them.

## Delivery

Three GitHub issues, each with its own worktree, review and merge:

1. **Model, file and CLI.** `pce::marks` (types, matching, precedence, partition, annotations, the side grammar), `illumio-tui::marks` (`MarkFile`, `MarksStore`: lock, reload, atomic write, `.bak`), and `illumio-tui marks list|add|rm|export|import`. Traffic commands and `app` apply marks (`meta.squelched`, `mark` fields, `--show-squelched`, `--no-marks`), and `gen` reports `meta.squelched`. This reviews matching and counting without any UI.
2. **TUI display.**
   - `TrafficState` marks: refresh at rebuild, partition, `FlowRow::mark`, `RowMarks` on rows;
   - the `TableView::marker` gutter;
   - `M`, the meta texts, Blocked's accepted counts, the treemap glyphs, the app overlay panels and the generator header;
   - invalid-file warnings, the startup expiry line, help.
3. **TUI creation and management.** The `m` form (prefill per view, widening, live preview, save), the `:marks` overlay (edit, delete, extend, filter), and docs: README, `docs/index.html`, in-TUI help.

## Decisions (Alex, 2026-10-17)

1. **Default expiries per action** stay: squelch 90d, accept-blocked 90d, reviewed 30d, revisit 7d, critical and important never.
2. **Draft decision is in**, as its own field `draft_decision` (see Fields), so a mark can match the reported decision, the draft one, or both.
3. **Why a mark exists** is the required `comment` field; hand-written `#` comments in the file are not kept (one `.bak`, a header says so).
