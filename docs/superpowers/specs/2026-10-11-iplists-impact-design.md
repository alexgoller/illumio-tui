# illumio-tui — Design Spec (Plan 7: IP lists screen and rule impact)

Date: 2026-10-11
Status: approved in conversation (sections 1–5 of the chat design)
Author: Alex Goller with Claude
Tracks: GitHub issue #4 (rule impact, IP list sizes); Alex's request for IP lists as their own pane
Builds on: Plans 4–6 (TUI editing, policy writes, TUI policy editing), issue #3 (label model)

## 1. Purpose and scope

Two deliverables:

- **IP lists screen.** IP lists leave the Objects tabs and get a sixth top-level
  screen (`6`, `:iplists`) with a size column, a "used by" detail, and the
  Plan 4 editing keys.
- **Rule impact.** For an existing rule, a rule being written, or a `w` form,
  the TUI and the CLI report how many workloads the rule touches on the
  provider and consumer side, evaluated over the cached workload list with
  ruleset scopes, intra/extra-scope consumers, label groups, exclusions,
  `all` and workload actors; IP-list actors are reported with a size, never
  expanded into peers. The CLI exposes the same numbers in JSON for agents,
  including on dry runs.

Everything stays user-caused: the workload list is read only on `i` in the
TUI or `--impact` in the CLI, and only when it is not already cached.

Success:

- `6` shows IP lists with name, size, ranges, FQDNs, used-by and pending
  columns; `Enter` opens the detail with ranges, FQDNs and the rules that
  reference the list; `n`/`e`/`x` edit as on Objects.
- `Enter` on a rule shows `impact: 12 providers · 3 consumers · Corp: 65.5K
  addresses` when the workload cache is warm, or `impact: press i to load
  workloads (1 request)` otherwise.
- `illumio-tui policy rules add Shop-Prod --provider role=web --consumer
  iplist:Corp --service 443/tcp --dry-run --impact --json` prints the request
  and `data.impact` with the counts, without sending.

## 2. IP lists screen

### 2.1 Screen and tabs

`Screen::IpLists` (title `IP lists`, key `6`, command `:iplists`) is added to
`Screen::ALL` as the last entry, so every existing number key keeps its
screen (Events stays `5`) and the tab row shows `6 IP lists` last. The Objects
screen title becomes `Objects` and `ObjectKind::IpLists` is removed from its
tabs (`ObjectKind::ALL` has three entries; `ages/loaded_at/loading` shrink to
three). The tab row uses each screen's `short_title()` (`Traf`, `Pol`,
`Obj`, `Wl`, `Ev`, `IPs`) when the full row would exceed the width; the
header snapshot at 80 columns shows the short form only if the full row no
longer fits (compute, do not assume).

### 2.2 State and loading

```rust
pub struct IpListsState {
    pub table: TableView<IpListRow>,   // IpListRow { list: IpList, size: IpListSize, used_by: usize }
    pub age_secs: u64, pub loaded_at: Option<Instant>, pub loading: bool,
    pub show_detail: bool, pub auto: AutoRefresh,
}
```

`Cmd::LoadIpLists(mode)` → `Pce::ip_lists(mode)` → `Msg::IpListsLoaded(Result<(Vec<IpList>, u64), String>)`
(added to the runtime's `task_result` list). First visit loads once; `r`
bypasses; `a` auto-refresh as other screens. `used_by` is computed from
`PolicyState.raw` when rulesets are loaded (draft scope), else `-`, and is
recomputed on `RuleSetsLoaded`; it costs no request. Columns: `name` (28),
`size` (10, right-aligned, sort by address count), `ranges` (6), `fqdns` (5),
`used by` (7), `pending` (7, hidden below 70 columns). Detail pane: name,
description, size line, one line per range (`!` for exclusions, description),
FQDNs, then `used by:` with one line per referencing rule (`<ruleset> ·
allow/deny · <one-line summary>`), or `used by: load Policy (2) to see
rules`.

Keys: `j/k`, `/`, `s`, `Enter` detail toggle, `r`, `a`, `n`/`e`/`x`
through `EditIntent::{CreateIpList, UpdateIpList, DeleteIpList}` (unchanged
adapters; `reload_after_write` for `ObjectKind::IpList` now reloads
`LoadIpLists(Bypass)`). The Objects screen's IP list editing keys are gone
with the tab. `screen_hint`, `HELP_LINES` (`6` in the number line; `IP lists`
line), the `:iplists` command and the help snapshots change deliberately.

### 2.3 Size (`crates/pce/src/iplist_size.rs`)

```rust
pub struct IpListSize { pub v4: u128, pub v6: u128, pub unbounded: Option<String>, pub ranges: usize, pub fqdns: usize }
pub fn ip_list_size(list: &IpList) -> IpListSize;
impl IpListSize { pub fn display(&self) -> String; } // "254", "65.5K", "1.0M", "any (0.0.0.0/0)", "3 FQDNs" when no ranges
```

- A CIDR `A/N` contributes `2^(bits-N)`; a range `A-B` contributes `B-A+1`
  (inclusive, per family); a single address `1`; parse errors contribute `0`
  and are counted in `ranges` only.
- Exclusions subtract the overlap with the included ranges of the same
  family (intervals: merge included ranges first, then subtract excluded
  intervals; clamp at zero).
- `unbounded` is `Some("0.0.0.0/0")`/`Some("::/0")` when such a range is
  included, or `Some("≥16M addresses")` when `v4 + v6 >= 2^24`; display is
  `any (<reason>)` and sorting treats it as the largest.
- Human units: `< 1000` → as is; `K` with one decimal below `1M`; `M`/`G`
  above; the raw numbers go to JSON (`v4`, `v6` as strings to avoid JSON
  number limits, `unbounded`).
- Shown in the IP lists table and detail, in the rule detail next to IP-list
  actors, in the `iplists` CLI listing (`size` column; JSON `size` object).

## 3. Rule impact (`crates/pce/src/impact.rs`)

### 3.1 Inputs and output

```rust
pub struct ImpactInputs<'a> {
    pub workloads: &'a [Workload],
    pub label_groups: &'a [LabelGroup],
    pub ip_lists: &'a [IpList],
    pub truncated: bool,
}
pub struct Impact {
    pub providers: usize, pub consumers: usize,
    pub per_scope: Vec<ScopeImpact>,   // ScopeImpact { scope: Vec<String> (tokens), providers, consumers }
    pub ip_lists: Vec<IpListRef>,      // IpListRef { name, href, size: IpListSize, side: Side }
    pub unsupported: Vec<String>,      // virtual_service / virtual_server hrefs seen
    pub truncated: bool,
}
pub fn rule_impact(scopes: &[Vec<Actor>], providers: &[Actor], consumers: &[Actor], unscoped_consumers: bool, inputs: &ImpactInputs) -> Impact;
```

`Rule`, `DenyRule`, `RuleDraft`/`RulePatch`-applied bodies and `DenyRuleDraft`
feed the same function (the TUI/CLI convert drafts' `ActorRef`s to `Actor`s
with `to_actor`).

### 3.2 Semantics

- `in_scope(w, scope)`: every scope actor matches `w`: a label actor matches
  when `w` carries that label href; a label-group actor matches when `w`
  carries any member label of the group (members and sub-groups expanded
  recursively, cycle-safe, through `label_groups`); an empty scope matches
  all. A ruleset with an empty `scopes` list is one empty scope.
- `match(w, actors)`: actors are grouped by kind. Labels and label groups are
  grouped by key (a group's key is its `key` field): across keys AND, within
  a key OR. `all` matches every workload. A `workload` actor matches by
  href (`canon`, active folded to draft). IP lists and virtual services never
  match a workload. Exclusion actors (`exclusion == Some(true)`) are applied
  last: a workload carrying an excluded label or group member is removed.
  A list with no positive workload-matching actor (only IP lists) matches
  nothing.
- Per scope `Si`: `P_i = {w | in_scope(w, Si) ∧ match(w, providers)}`;
  `C_i = {w | match(w, consumers)}` intersected with `in_scope(w, Si)` unless
  `unscoped_consumers`. `providers = |∪ P_i|`, `consumers = |∪ C_i|`.
- Workloads use `labels[].href`; `managed`/unmanaged both count.
- `truncated` is copied from the workload read; the display shows `≥`.

### 3.3 Where it shows

- **Rule detail overlay** (`Enter` on a rule): grows by two rows:
  `impact: 12 providers · 3 consumers` and `Corp: 65.5K addresses` (one IP
  list per line, at most three, then `+N more`), or `impact: press i to load
  workloads (1 request)`, or `impact: ≥12 providers (workload list truncated)`.
  `i` inside the overlay (and on the Policy screen) issues
  `Cmd::LoadWorkloads(Use)`; the overlay re-renders on `WorkloadsLoaded`.
- **Rules pane**: a `P/C` column (`12/3`, width 7) when the workload cache
  is warm, `-` otherwise; recomputed on `WorkloadsLoaded` and
  `RuleSetsLoaded`.
- **Confirm overlay** for rule create/update (Policy `n`/`N`/`e`, and `w`):
  one line under the summary: `impact: 12 providers · 3 consumers` from the
  planned body (actors converted back from the request's `providers`/
  `consumers` JSON) or the "press i" text; the overlay keeps `y`/`n`.
- `Msg::WorkloadsLoaded` carries `truncated: bool`; `WorkloadsState.truncated`.

### 3.4 CLI

- `policy rules <ruleset> --impact`: each rule gains `impact` (JSON object as
  3.1 with `per_scope`, `ip_lists`, `unsupported`, `truncated`); table mode
  adds a `P/C` column. Costs one workloads read (cache) plus the usual.
- `policy rules impact <rule-href>`: one rule, same shape, exit 2 for a bad
  href, 4 for not found.
- `policy rules add|update … --dry-run --impact` and `deny-rules` likewise:
  `data.impact` next to the request; without `--dry-run`, `--impact` adds
  `impact` to the written output computed before the send.
- `iplists` listing: `size` column and JSON `size`.
- `meta.truncated` is OR'd with the workload read's flag; `--impact` without a
  cached list makes exactly one `workloads` GET (`max_results` 10000).

## 4. Errors

| condition | behaviour |
|-----------|-----------|
| workloads not cached (TUI) | "press i to load" line; no request |
| workloads read fails | status error; impact lines read `impact: unavailable (<err>)` |
| truncated workload list | counts shown with `≥` and the note; JSON `truncated: true` |
| actor the evaluator cannot match (virtual service) | listed under `unsupported`, counts exclude it |
| IP list href unknown to the index | size `?`, name = href |
| `i` while loading | "workloads already loading" |

## 5. Testing

- `pce` unit tests: `ip_list_size` (CIDR v4/v6, from-to, single, exclusion
  overlap and non-overlap, `0.0.0.0/0`, `::/0`, ≥16M, FQDN-only, parse error);
  `rule_impact` on a fixture with two scopes, a label group with a sub-group,
  an exclusion, `all`, a workload actor, an IP-list consumer, unscoped
  consumers, empty scopes, truncated flag; property: providers ≤ workloads.
- TUI unit tests: `used_by` counts, `P/C` column rendering, overlay lines in
  the three states, `i` emits `LoadWorkloads` once.
- Snapshots (both themes): IP lists screen 80x24 and 160x48 with detail,
  header tab row at 80 and 160, rule detail with impact, confirm with impact,
  help pages.
- Runtime (wiremock): `6` loads IP lists once; `i` on a rule detail loads
  workloads once and a second `i` costs nothing; `n` on IP lists creates and
  reloads once; CLI `policy rules --impact` one workloads GET, `--dry-run
  --impact` zero writes, `iplists` size column.
- Smoke on demo100 (reads only): `6`, detail, `2` → `Enter` → `i` → impact
  line; CLI `policy rules <first> --impact --json` and `iplists --json`.

## 6. Out of scope

Expanding FQDNs or IP lists into peers; virtual services as peers; simulating
policy decisions; auto-fetching workloads; per-rule provisioning; impact for
rulesets as a whole (sum is misleading).

## 7. File map

| file | change |
|------|--------|
| `crates/pce/src/iplist_size.rs` (new), `lib.rs` | size |
| `crates/pce/src/impact.rs` (new), `lib.rs` | impact |
| `crates/pce/src/resolve.rs` | `label_group(href)`, `ip_list(href)` accessors |
| `crates/illumio-tui/src/tui/model.rs` | `Screen::IpLists`, `IpListsState`, `ObjectKind` shrink, `WorkloadsState.truncated`, rules `P/C` column |
| `crates/illumio-tui/src/tui/msg.rs` | `Cmd::LoadIpLists`, `Msg::IpListsLoaded`, `WorkloadsLoaded` truncated |
| `crates/illumio-tui/src/tui/update.rs` | keys `6`, `i`, IP lists screen keys, reloads, used-by/impact recompute |
| `crates/illumio-tui/src/tui/app.rs` | tasks, `task_result` |
| `crates/illumio-tui/src/tui/ui/screens/iplists.rs` (new), `objects.rs`, `policy.rs`, `ui/mod.rs`, `widgets/overlay.rs` | rendering, tabs, hints, help |
| `crates/illumio-tui/src/tui/commands.rs` | `:iplists` |
| `crates/illumio-tui/src/cli/policy.rs`, `objects.rs`, `output.rs` | `--impact`, `impact` subcommand, size column |
| tests as in section 5; `README.md`, `docs/architecture.md`, `docs/index.html` | docs |
