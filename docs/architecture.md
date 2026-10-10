# Architecture notes (Plan 1: library and CLI)

## Crates

- `crates/pce`: the library. No terminal code, no process-global state.
- `crates/illumio-tui`: the binary. `cli/` is the non-interactive mode;
  the TUI (Plan 2) will sit next to it and reuse every library type.
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

## Testing

Offline tests use `fixtures/*.json` (hand-written, deterministic) and
`wiremock` for HTTP. `fixtures/recorded/` holds anonymised real responses
and is only shape-checked. The recorded fixtures were captured from a real
PCE (demo100); an object type that PCE legitimately had none of at record
time (e.g. `label_groups.json`) is committed as an empty list, and the
recorded-shape test treats an empty recorded list as a pass (it prints a
note rather than failing), since an empty list is a valid shape, not a
recording error. `PCE_LIVE=1` enables the single live test.
