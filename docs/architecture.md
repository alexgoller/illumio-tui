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
appends the audit entry in a single write, invalidates the cache prefix of the
kind (`objects:` or `workloads`), and re-fetches the object after a 204. A
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

## Testing

Offline tests use `fixtures/*.json` (hand-written, deterministic) and
`wiremock` for HTTP. `fixtures/recorded/` holds anonymised real responses
and is only shape-checked. The recorded fixtures were captured from a real
PCE (demo100); an object type that PCE legitimately had none of at record
time (e.g. `label_groups.json`) is committed as an empty list, and the
recorded-shape test treats an empty recorded list as a pass (it prints a
note rather than failing), since an empty list is a valid shape, not a
recording error. `PCE_LIVE=1` enables the single live test.

`crates/pce/tests/write_contract.rs` pins every write body with wiremock; `crates/illumio-tui/tests/cli_writes.rs` covers the guard, exit codes 2/4/6, dry runs and the audit file. `live_writes.rs` is opt-in (`PCE_LIVE_WRITES=1`) and only ever creates and deletes one label.
