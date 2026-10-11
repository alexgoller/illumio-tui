# Architecture notes

## Crates

- `crates/pce`: the library. No terminal code, no process-global state.
- `crates/illumio-tui`: the binary. `cli/` is the non-interactive mode;
  the TUI (`tui/`) sits next to it and reuses every library type.
- `crates/xtask`: developer tooling (`cargo xtask record`).

## One request, top to bottom

`illumio-tui traffic query --window 1h --src app=web`

1. `cli/traffic.rs::build_query` parses `app=web` into a `FilterExpr`.
2. `Pce::resolve_filter_groups` asks `Pce::name_index` for the label
   table. The index is built from four cached lists (labels, label groups,
   IP lists, services), each one request with `max_results=10000`.
   `app=web` becomes `{"label": {"href": "/orgs/1/labels/7"}}`. An unknown
   value fails here, before the PCE sees anything.
3. `TrafficQueryBuilder::last(1h)` produces the request body with full UTC
   timestamps and `[[]]` for the empty destination include.
4. `TrafficQuery::needs_confirmation` checks the window and row cap.
5. `Pce::traffic` looks the exact body up in the cache, then calls
   `traffic::poll::run_traffic_query`: POST the query, sleep 1 s, poll,
   back off ×1.5 to 10 s, download the result, flag truncation.
6. `traffic::resolved::resolve_flows` replaces label hrefs with names.
7. `Context::emit` prints the JSON envelope or a table; `meta` carries the
   call and byte counters from `PceClient::stats`.

Connection settings (host, port, org id) are resolved before step 1, in
`cli::context::Context::new`, from the global flags (`--host`,
`--pce-port`, `--org-id`, ...), then environment variables, then the
selected profile. The global override flag is `--pce-port`, not `--port`:
`--port` on `traffic query` is the service-port filter in step 1-3 above,
a different thing entirely, so the connection override was named
`--pce-port` to keep the two unambiguous on the same command line.

## TUI

Module map under `crates/illumio-tui/src/tui/`:

| Module | Role |
|---|---|
| `model` | all state: screens, tables, overlays, counters, theme |
| `msg` | the `Msg` enum (keys, mouse, ticks, query and fetch results) |
| `update` | pure `update(&mut Model, Msg) -> Vec<Cmd>`; no I/O |
| `app` | the runtime: terminal setup, event loop, spawns tasks for `Cmd`s |
| `commands` | parser for the `:` prompt |
| `filter` | the filter editor and its conversion to a traffic query |
| `table` | sortable, searchable table state shared by every screen |
| `form` | the generic edit form (fields, keys, horizontal scroll) |
| `edit` | edit intents, pre-filled forms, form to library draft or patch |
| `diff` | draft against active for a pending IP list, service or ruleset |
| `ui/` | theme, layout, per-screen drawing and widgets (panels, overlays) |

One message, top to bottom: pressing `r` on Traffic.

1. `app` reads a crossterm event and turns it into `Msg::Key`.
2. `update` handles it, marks the screen busy and returns `Cmd::RunQuery`.
3. `app` spawns a task that runs `filter.to_query` and `pce.traffic`,
   sending `Msg::QueryProgress` while it polls and `Msg::QueryDone` at the end.
4. `update` stores the result and calls `rebuild_views`, which derives the
   Flows, App-to-app and Blocked tables from the one result.
5. `app` draws the model once, after draining whatever else is queued.

Tick policy: no timer at all while idle; 250 ms while a request is in
flight (spinner, progress); 1 s when any screen has auto-refresh on. Auto-refresh
is opt-in per screen, at least 60 s apart, and turns itself off after a failure.
There is one in-flight task per resource, so the TUI never issues concurrent
misses for the same data. The first visit to Workloads and to Events fetches
once; later visits use the cache.

Testing: `update` is a pure function, so most behaviour is tested by feeding
messages to a `Model` and asserting on the model and the returned commands.
Screens are checked with ratatui's `TestBackend` and `insta` snapshots at
80x24 and 160x48 in the dark theme, plus styled snapshots that include colours
for dark and light (`tests/tui_snapshots.rs`). The runtime is tested with an
injectable event stream and a mock PCE (`tests/tui_runtime.rs`), so no
terminal is needed.

## PCE protection, where it lives

| Rule | Code |
|---|---|
| 2 req/s, burst 5 | `pce::limiter::RateLimiter`, used in `PceClient::send` |
| back off on 429/503, never retry POST | `PceClient::send` |
| count every call and byte | `PceClient::stats` |
| cache lists 15/5 min | `Pce::cached_list` with `CacheTtls` |
| one query per filter | `Pce::traffic` cache key is the request body |
| confirm big queries | `TrafficQuery::needs_confirmation` |
| show the request without sending | `traffic query --dry-run` |

## Writes

`pce::write` is the only path that mutates the PCE. A `Writer` exists only
through `Pce::writer(&WriteGuard)`, and a `WriteGuard` cannot be built unless
the profile allows writes, the caller passed `--allow-writes`, and the audit
log opens for append. Writes are two-phase: `plan_create_*`, `plan_update_*`
and `plan_delete_*` build a `Plan` (the exact `WriteRequest`, or a `Noop` when
`--if-missing` found an exact match) without sending anything; `execute` sends
it once through `PceClient::write` (no retry, 400/406/409/422 -> `Rejected`),
appends the audit entry in a single write, invalidates the cache prefixes of the
kind (`objects:`, `workloads`, `policy:`, or both `policy:` and `objects:`
after a provision), and re-fetches the object after a 204. A
timeout, connection loss, gateway status or undecodable 2xx body after the
request left is reported as `write_unanswered` ("may have applied") and
audited with `phase: "sent"`; a failed re-fetch, a POST without an href or an
audit append failure is success with `warnings`. An update sends only the
keys that change; a changed list (`ip_ranges`, `fqdns`, `service_ports`,
workload `labels` or `interfaces`) is computed locally and sent whole because
the PCE replaces those arrays wholesale, and list entries are carried through
with their unknown fields. A service update that changes ports is refused when
the service has fields this tool cannot preserve (`windows_services`, ICMP
type/code, a port without a protocol). Workloads are written only
when the PCE reports `managed: false` and no VEN. The CLI layer
(`cli/write.rs`) owns `--dry-run`/`--yes`, exact name resolution and the
envelope (`meta.write`).

Policy writes (Plan 5) add `write/actors.rs` (`ActorRef`, `ServiceRef`: the
typed sides of a rule and the members of a scope), `write/rule_sets.rs`,
`write/rules.rs` and `write/provision.rs`. Rules and deny rules live under
their ruleset's href (`checked_nested_href`); an update reads the rule once
as JSON and applies the patch to that JSON, so untouched providers,
consumers and services go back exactly as the PCE returned them, unknown
keys included; the policy models carry an `extra` map for the same reason.
`plan_provision` reads `sec_policy/pending` once (bypassing the cache) and
sends an explicit `change_subset`; a pending rule maps to its ruleset.
The token grammar (`ActorToken`, `ServiceToken`, scopes and the strict href
helpers) lives in `crates/illumio-tui/src/tokens.rs`, shared by the CLI and
the TUI. The CLI parses tokens before any request and resolves names against
lists it fetches at most once per command (`Lists`).

### TUI editing

The model knows only `WritesState` (enabled, reason); the runtime owns
`Option<Arc<WriteGuard>>`, built in `run` and again on every `:profile`
switch from the same inputs as the CLI (profile `writes`, `--allow-writes`,
the JSONL audit log) with `source: "tui"`. A switch to a profile without
writes drops the guard and the header marker.

One edit, top to bottom: `e` on an IP list.

1. `update` checks `model.writes`, clones the selected list into
   `EditIntent::UpdateIpList` and opens `Overlay::Form` pre-filled by
   `edit::start`.
2. `Enter`: `edit::build_intent` diffs the fields against the original into
   an `IpListPatch` (`Ok(None)` when nothing changed) and returns
   `Cmd::PlanWrite`.
3. `app` spawns a task: `pce.writer(&guard).plan_update_ip_list` fetches the
   list once and returns the `Plan`; `Msg::Planned` opens
   `Overlay::ConfirmWrite` with summary, `METHOD path` and the pretty body.
4. `y`: `Cmd::ExecuteWrite`; the task calls `Writer::execute` once (audit,
   cache prefix invalidated) and drops `policy:pending`; `Msg::WriteDone`
   sets the status line and reloads the IP lists once with `Bypass`.

`n`/`Esc` on the confirm overlay or the form sends nothing. The pending
overlay keeps a selection; `Enter` sends `Cmd::LoadDiff` with the
`PendingChange`; the task reads the draft list and
`Pce::active_ip_lists`/`active_services` through the cache and
`tui/diff.rs` computes `+`/`-`/same lines, so a second diff of the same kind
within five minutes costs nothing.

### TUI policy editing

Policy writes in the TUI reuse the editing path above. `n`/`e`/`x` on the
Policy panes (and `N` on the rules pane) build `EditIntent`s from the
selected ruleset or rule, found by href in `PolicyState.raw` or the
deny-rule fallback; they refuse in the active scope. `tui/edit.rs` renders a
rule's actors and services back to tokens through the `NameIndex`
(`actor_tokens`, `service_tokens`) and resolves typed tokens through it
(`resolve_actor_tokens`, `resolve_service_tokens`) without a request. An
edit compares the field's tokens with the shown ones first, so untouched
entries stay exactly as the PCE returned them, and a field holding an entry
the grammar cannot express may not change.

`app::plan_intent` maps the intents to the policy planners. A `w` form names
its ruleset: `RulesetTarget::Name` costs one draft ruleset read through the
cache before `plan_create_rule`; when the plan fails, the form comes back
with the error (`TrafficState.rule_form`). `P` in the pending overlay plans a
`ProvisionRequest` (the marked hrefs, or all), which reads the pending list
once more past the cache. After `WriteDone` a policy write reloads the draft
rulesets once with `Bypass` (`PolicyState.quiet_reload` keeps the write's
report in the status line); when the selected ruleset's inline deny list is
empty, that reload may cost one more GET for its deny rules (the on-demand
fallback). A provision also reloads the pending list and reports
`provisioned <version href>: <counts>`, the counts taken from the body when
`y` was pressed. A plan with nothing to send (`Plan::Noop`, such as an update
whose fields already match) is an info status, never a request.

The `w` pre-fill follows the profile's label model (`Limits.scope_keys`,
`Limits.tier_key`, from `[profiles.<name>.labels]` or `ILLUMIO_SCOPE_KEYS` /
`ILLUMIO_TIER_KEY`): `edit::start` takes a `FlowPrefill` (those keys and the
loaded draft rulesets, `Model::flow_prefill`) and fills providers with the
destination's tier, consumers with the source's tier (same scope) or its
scope and tier (unscoped), and the ruleset with the one loaded draft ruleset
whose scope is the destination's (by label href); nothing is requested.
`Form.alt` holds every attached label for `Ctrl-A`. When the plan says the
typed name is unknown and the destination has a scope, the form offers
`Ctrl-N` (`Form.offers_ruleset`, a `RulesetOffer` of the checked name and the
scope, shown in the Ruleset hint row and the panel keys while the field still
holds that name): it plans `CreateRuleSet` scoped like the destination and
keeps the rule and the form in `PolicyState.rule_after_ruleset`; the
ruleset's `WriteDone(Ok)` plans `CreateRule` into the new href (a second
confirm, the form back in `TrafficState.rule_form` so a failed plan reopens
it), and any failure, cancel or profile switch drops it. Both writes
reload the draft rulesets.

`Enter` on a pending ruleset reads the draft rulesets (cached) and the active
ones (one GET, five minutes); `diff::rule_set_diff_for` lists name,
description, state, scopes and one line per rule, matched by href. Each
side's deny rules are a `DenySide`: its inline list; else the draft fallback
the Policy screen fetched; else none when that side's list payload inlines
deny rules for any ruleset (the PCE then inlines them for all, so empty
means zero); else unfetched, in which case deny rules are not compared and a
note says which side was not fetched. A diff that arrives while another
overlay than the pending list is open does not replace it.

## Testing

Offline tests use `fixtures/*.json` (hand-written, deterministic) and
`wiremock` for HTTP. `fixtures/recorded/` holds anonymised real responses
and is only shape-checked. The recorded fixtures were captured from a real
PCE (demo100); an object type that PCE legitimately had none of at record
time (e.g. `label_groups.json`) is committed as an empty list, and the
recorded-shape test treats an empty recorded list as a pass (it prints a
note rather than failing), since an empty list is a valid shape, not a
recording error. `PCE_LIVE=1` enables the single live test.

`crates/pce/tests/write_contract.rs` pins every write body with wiremock;
`crates/illumio-tui/tests/cli_writes.rs` covers the guard, exit codes 2/4/6, dry
runs and the audit file. `crates/pce/tests/policy_writes.rs` pins every policy
body (rulesets, rules, deny rules, provisioning, managed labels) with wiremock.
`live_writes.rs` is opt-in (`PCE_LIVE_WRITES=1`); it creates and deletes one
label, and runs a draft ruleset and rule lifecycle that plans but never sends a
provision.
