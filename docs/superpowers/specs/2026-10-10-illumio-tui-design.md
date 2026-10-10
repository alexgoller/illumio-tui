# illumio-tui — Design Spec (Step 1: read-only traffic and policy)

Date: 2026-10-10
Status: approved in conversation, pending written review
Author: Alex Goller with Claude

## 1. Purpose and audience

A terminal user interface for the Illumio Core Policy Compute Engine (PCE),
written in Rust. Step 1 is **read-only**: browse and monitor traffic and policy.
Writes (rules, rulesets, provisioning, labels) are step 2.

Primary users: hardcore PCE administrators, Illumio Professional Services, and
Illumio engineering. Secondary users: AI agents and scripts, via a
non-interactive CLI mode of the same binary.

Success for step 1:

- An admin can open the tool against a PCE and, within a minute, see current
  traffic for a chosen app, group it by label pair, and isolate blocked flows.
- The same admin can browse rulesets and rules with every href resolved to a
  readable name, in draft or active.
- An agent can run `illumio-tui traffic query --json` and get a stable JSON
  document with resolved labels and cost metadata.
- The tool is demonstrably gentle on the PCE: every request is user-caused,
  throttled, cached, and counted on screen.

## 2. Constraints

- **PCE load.** The PCE is a shared, expensive resource. The tool never issues a
  request the user did not cause or cannot see. See §5.
- **Language.** Rust, stable toolchain. The owner does not write Rust, so code
  must be self-explanatory and behaviourally verifiable via tests, the CLI, and
  a live smoke test.
- **API.** Illumio Core PCE REST API v2, HTTP Basic auth with API key and secret,
  org-scoped URLs. Works for SaaS and on-prem PCEs. Illumio Cloud / FQS APIs are
  out of scope.
- **Look.** btop/bottom lineage: boxed panels with titled borders, dense stats,
  bars and sparklines, one dark and one light palette.
- **Test PCE.** demo100.illum.io, org 5636114. Credentials are read from the
  sibling project's `.env` at test time and never copied into this repo.

## 3. Architecture

Cargo workspace with two crates.

### 3.1 `crates/pce` (library)

Pure library, no terminal code. Reusable by future tools (MCP server, scripts).

- `client.rs` — `PceClient`: reqwest + tokio, HTTP Basic, base URL
  `{scheme}://{host}:{port}/api/v2/orgs/{org}/`. `GET /api/v2/health` is the only
  org-less call. Sends `Accept: application/json`. 30 s request timeout.
- `auth.rs` — credential struct; redacts secrets in `Debug`.
- `limiter.rs` — token-bucket throttle shared by all requests (§5.1).
- `cache.rs` — in-memory TTL cache keyed by endpoint + canonicalised params (§5.3).
- `error.rs` — `PceError` enum: `Auth`, `NotFound`, `RateLimited { retry_after }`,
  `Timeout`, `Network`, `InvalidFilter(String)`, `Server { status, body }`,
  `QueryFailed(String)`.
- `models/` — serde types with real-fixture coverage:
  - `traffic.rs` — `TrafficQuery` request body, `AsyncQueryStatus`, `Flow` row
    (`src`/`dst` endpoint with `ip`, `fqdn`, `workload{href,hostname,name,labels}`,
    `ip_lists[{href,name}]`; `service{port,proto,process_name,windows_service_name,user_name}`;
    `policy_decision`, `flow_direction`, `num_connections`, `dst_bi`, `dst_bo`,
    `state`, `timestamp_range{first_detected,last_detected}`; optional
    `draft_policy_decision`, `rules[{href}]`). Unknown fields preserved via
    `#[serde(flatten)] extra: Map`.
  - `policy.rs` — `RuleSet{href,name,description,enabled,scopes:Vec<Vec<Actor>>,
    rules,update_type,created_at,updated_at}`, `Rule{href,enabled,description,
    providers,consumers,ingress_services,resolve_labels_as,unscoped_consumers,
    sec_connect,stateless,machine_auth,network_type,update_type}`,
    `DenyRule{…,override_:bool}`, `Actor{actors:Option<"ams">,label,label_group,
    workload,virtual_service,virtual_server,ip_list}` (each `Option<Href>`),
    `PendingChange{href,change_type,name}`.
  - `labels.rs` — `Label{href,key,value,deleted}`, `LabelGroup`, `IpList{href,
    name,ip_ranges,fqdns}`, `Service{href,name,service_ports[{port,to_port,proto}],
    windows_egress_services}`.
  - `workloads.rs` — `Workload{href,name,hostname,interfaces[{name,address}],
    labels[{href}],enforcement_mode,visibility_level,online,managed,os_type,
    agent{status}}`.
- `traffic/query.rs` — builder that enforces PCE filter rules: include is a list
  of AND-groups (outer OR), empty include serialises as `[[]]`, exclude is flat,
  dates are full RFC 3339 UTC timestamps, backwards windows rejected client-side.
- `traffic/poll.rs` — the async query state machine: POST
  `traffic_flows/async_queries` with `Prefer: respond-async`; poll the returned
  href with 1 s initial delay, ×1.5 growth, 10 s cap; `failed` → `QueryFailed`;
  `completed` without `result` is retried for a 30 s grace window; then GET the
  download href. Reads `matches_count` and marks the result `truncated` when
  rows == `max_results` or `matches_count` > rows. Cancellable via
  `CancellationToken`.
- `resolve.rs` — `NameResolver`: maps label/ip_list/service/workload hrefs to
  display names from the cache; never issues per-row requests.

Policy reads default to `sec_policy/draft`; the caller selects `Draft | Active`.
Deny rules are fetched per ruleset from `{ruleset.href}/deny_rules`.
Collection listing sends an explicit `max_results` (default 10 000) and reads
`X-Total-Count` to detect truncation; no implicit two-request pattern.

### 3.2 `crates/illumio-tui` (binary)

Elm-style architecture:

- `model.rs` — one `Model` struct: active screen, per-screen state, caches
  view, query status, API counters, profile, errors.
- `msg.rs` — one `Msg` enum: terminal events (key, mouse, resize, tick, focus),
  user intents (`RunQuery`, `SetView`, `ToggleDraftActive`, …), and task results
  (`QueryStarted`, `QueryPolling{elapsed}`, `QueryDone(Result<…>)`,
  `LabelsLoaded`, …).
- `update.rs` — pure `fn update(&mut Model, Msg) -> Vec<Cmd>`. `Cmd` is a
  side-effect description (`SpawnQuery(TrafficQuery)`, `Fetch(Resource)`,
  `CancelQuery`, `Quit`). Testable without a terminal or network.
- `app.rs` — the runtime: crossterm event stream + `tokio::mpsc<Msg>` from
  background tasks, executes `Cmd`s by spawning tasks on the `PceClient`.
- `ui/` — `theme.rs` (dark + light tokens), `layout.rs`, `widgets/` (titled
  panel, bar, sparkline, table with sort/search, filter bar, status line, help
  overlay, command prompt), `screens/{traffic,policy,objects,workloads}.rs`.
  Views are pure functions of `&Model`.
- `cli/` — clap subcommands (§6), each rendering through shared formatters.
- `config.rs` — profile loading and value resolution (§7).

Rendering is on-demand: redraw on event or on a 250 ms tick only while a task is
in flight (for the elapsed-time indicator). Idle UI consumes no CPU.

## 4. Screens

Global chrome: top header (profile host, org, draft/active, API call count and
bytes), screen tabs `1–4`, `:` command prompt, `?` help overlay, bottom status
line with query state (idle / running Ns / done / truncated / error).

Commands: `:traffic`, `:policy`, `:objects`, `:workloads`, `:filter <expr>`,
`:window <dur>`, `:profile <name>`, `:draft`, `:active`, `:q`.

### 4.1 Traffic (default)

One filter bar, one query, one in-memory result set, three views toggled by `v`:

- **Flow table** — one row per flow: last seen, src (name + key labels), dst,
  port, proto, connections, decision. Sortable (`s`), searchable (`/`). Detail
  pane: resolved labels of both ends, process/user, first/last seen, and the
  allowing rule when the PCE reports `rules`.
- **App-to-app aggregate** — group by a label-key pair (default `app,env`,
  changeable with `g`). Columns: src group, dst group, flows, connections,
  decision breakdown as a stacked bar. `enter` drills into that pair's flows.
- **Blocked focus** — rows limited to `blocked` and `potentially_blocked`,
  grouped by destination + port, sorted by last seen.

Header stats for the result set: total rows, decision breakdown bar, truncation
warning, result age.

Filter bar fields: window (relative `1h/24h/7d` or absolute), src, dst (label
shorthand `key=value`, IP, ip list name, workload hostname), service
(port/proto), decision set. `f` edits, `r` runs, `a` toggles auto-refresh.

### 4.2 Policy

Left: rulesets (name, enabled, scopes, rule count, pending badge from
`update_type`). Right: rules of the selected ruleset with consumers, providers,
services resolved to names; deny rules in a separate section with override
marked. `d` toggles draft/active. `enter` opens a rule detail pane. `p` opens
the pending-changes panel (from `sec_policy/pending`).

### 4.3 Labels and objects

Tabs: labels (key/value, usage), label groups, IP lists (ranges, fqdns),
services (ports). Table + detail. Shows cache age; `r` refreshes that object
type only.

### 4.4 Workloads

Table: hostname, name, labels, enforcement mode, visibility, online, VEN status.
Detail: interfaces, OS. `t` jumps to Traffic with the workload preset as source
(`T` as destination).

## 5. PCE protection

### 5.1 Throttle and accounting

One token bucket in `PceClient`: default 2 req/s, burst 5, per profile. Every
request increments a call counter and byte counter exposed to the UI header and
CLI `meta`. HTTP 429 and 503 pause the whole client with exponential backoff
plus jitter (base 2 s, cap 60 s), honouring `Retry-After`. POSTs are never
retried automatically.

### 5.2 No unsolicited requests

Nothing refreshes on its own. Auto-refresh is opt-in per screen (`a`), minimum
60 s (profile-configurable upward, can be disabled), shows a countdown in the
panel title, and skips a tick while a query is running or the terminal is
unfocused. Startup performs exactly: health check, then one request each for
labels, label groups, IP lists, services.

### 5.3 Cache

| Data | TTL | Invalidation |
|---|---|---|
| labels, label groups, ip lists, services | 15 min | `r` on objects screen |
| rulesets + rules (per draft/active) | 5 min | `r` on policy screen |
| workloads | 5 min | `r` on workloads screen |
| traffic result set | until filter changes | `r` on traffic screen |

View switches, sorting, searching, grouping, drill-downs never re-query.
Panel titles show cache age. `:profile` switch clears everything.

### 5.4 Explorer query guards

Defaults: window 24 h, `max_results` 1 000. Before running, the filter bar shows
a cost hint (window length, cap, "unbounded" if no src/dst/service filter).
Confirmation required when window > 7 d or cap > 10 000. Only one query runs at
a time; editing the filter cancels the in-flight one. Truncation is always
shown.

## 6. CLI mode

`illumio-tui` with no subcommand opens the TUI. Subcommands:

```
traffic query [--window 24h] [--from/--to <rfc3339>] [--src <expr>]... [--dst <expr>]...
              [--port N --proto tcp|udp] [--decision allowed,potentially_blocked,blocked]
              [--max-results N] [--group key1,key2] [--dry-run] [--yes]
policy rulesets [--draft|--active] [--name <substr>]
policy rules <ruleset name or href> [--draft|--active]
policy pending
labels [--key K] [--value V]
label-groups | iplists | services
workloads [--hostname H] [--label key=value]...
health
profile list | profile use <name>
```

Global flags: `--profile`, `--env-file`, `--json` (default when stdout is not a
TTY), `--table`, `--no-cache`.

JSON envelope on stdout:

```json
{"data": [...],
 "meta": {"profile":"demo100","elapsed_ms":1840,"api_calls":3,"bytes":21344,
          "cache":"miss","truncated":false,"window":{"start":"…","end":"…"}}}
```

Hrefs are resolved to names with the href kept (`{"href":"/orgs/1/labels/7",
"key":"app","value":"web"}`). Errors: JSON on stderr `{"error":{"kind":…,
"message":…}}`; exit codes 1 generic, 2 arguments, 3 auth, 4 PCE error, 5 rate
limited. `--table` renders with the TUI's table widget. `--dry-run` prints the
request body without sending. `--max-results` > 10 000 requires `--yes`.

## 7. Configuration and auth

File `~/.config/illumio-tui/config.toml` (mode 0600), kubeconfig-style profiles:

```toml
default_profile = "demo100"
[profiles.demo100]
host = "demo100.illum.io"; port = 443; org_id = 5636114; tls_verify = true
api_key = "…"                 # optional
api_secret_cmd = "op read …"  # optional, run to obtain the secret
[profiles.demo100.limits]
requests_per_second = 2; auto_refresh_min_seconds = 60
traffic_max_results = 1000; traffic_default_window = "24h"
```

Resolution per value: CLI flag → env (`PCE_HOST`, `PCE_PORT`, `PCE_ORG_ID`,
`API_KEY`, `API_SECRET`, `PCE_TLS_VERIFY`, same names as illumio-mcp-server) →
profile → default. `--env-file` loads a dotenv file. Secrets are never written
by the tool, never logged, never rendered. Startup health failure opens the TUI
in a disconnected state with the error visible.

## 8. Testing and verification

Offline suite (runs in CI, no PCE):

1. Model round-trips over anonymised fixtures recorded from demo100 by
   `cargo xtask record` (hostnames/IPs replaced, label keys kept).
2. `wiremock` client tests: async query dance incl. the completed-without-result
   race, 429 backoff, truncation, `X-Total-Count`, `[[]]` include encoding,
   draft/active routing, deny_rules per ruleset. Limiter and cache property tests.
3. `update()` tests: filter edit cancels query; view switch emits no `Cmd`;
   auto-refresh respects minimum interval and running state.
4. `insta` snapshots via Ratatui `TestBackend` for each screen at 80×24 and
   160×48, dark and light.
5. `assert_cmd` CLI tests: envelope, exit codes, `--dry-run`.

Live smoke test, opt-in with `PCE_LIVE=1`: health, label load, one 1-hour
traffic query with `max_results=50`; asserts on shape only. ≤ 6 API calls.

Toolchain: stable Rust, `cargo clippy -D warnings`, `rustfmt`, `cargo deny`,
GitHub Actions on macOS + Linux.

## 9. Out of scope for step 1

All writes and provisioning; events and audit; VEN/agent health; container
clusters; Illumio Cloud / FQS; plugins; themes beyond dark and light; mouse
drag; multi-PCE simultaneous views.

## 10. Reader aids

Every module opens with a plain-language doc comment. `docs/architecture.md`
walks one `Msg` from keypress to rendered flow table. The CLI `--table` output
and the live smoke test are the owner's behavioural review tools.
