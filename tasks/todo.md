# illumio-tui tasks

## Plan 1: pce library and CLI
- [x] Task 1 scaffold
- [x] Task 2 error + auth
- [x] Task 3 limiter
- [x] Task 4 client
- [x] Task 5 cache + facade
- [x] Task 6 object models
- [x] Task 7 workloads
- [x] Task 8 name index + filter expressions
- [x] Task 9 policy models + views
- [x] Task 10 traffic query builder
- [x] Task 11 traffic poller
- [x] Task 12 resolved flows + aggregation
- [x] Task 13 config
- [x] Task 14 CLI skeleton + health
- [x] Task 15 objects commands
- [x] Task 16 workloads command
- [x] Task 17 policy commands
- [x] Task 18 traffic command
- [x] Task 19 profile commands
- [x] Task 20 fixture recorder + live smoke test
- [x] Task 21 CI, cargo-deny, docs

## Review (Plan 1)
- Library: throttled client, TTL cache, typed models, async Explorer poller, name resolution, aggregation.
- CLI: health, labels, label-groups, iplists, services, workloads, policy (rulesets, rules, pending), traffic query (flows, --group, --blocked, --dry-run), profile (list, use).
- Verified: `cargo test --workspace`, clippy clean, live smoke test against demo100 (50 flows in 9 API calls, 278 KB).
- Deferred to Plan 2: the interactive TUI, auto-refresh, `:profile` switching at runtime.

## Plan 2: TUI
- [x] Task 1 TUI core: model, messages, pure update, theme, chrome view, snapshots
- [x] Task 2 Runtime: terminal, event loop, command executor, startup
- [x] Task 3 Table state (sort, search, selection, scrolling) and its renderer
- [x] Task 4 Command prompt, help overlay, status errors
- [x] Task 5 Traffic filter model, filter bar, editor, cost hint, confirmation
- [x] Task 6 Traffic query execution: run, progress, cancel, results
- [x] Task 7 Flow table view, detail pane, header stats (decision bar, sparkline, age)
- [x] Task 8 App-to-app aggregate and blocked views, view cycling, drill-down
- [x] Task 9 Opt-in auto-refresh with countdown, focus and running guards
- [x] Task 10 Policy screen: rulesets, rules (allow + deny), draft/active, detail, pending
- [x] Task 11 Labels & objects screen
- [x] Task 12 Workloads screen and the jump to Traffic
- [x] Task 13 Profile switching, draft/active commands, disconnected recovery, error overlay
- [x] Task 14 Mouse support, light-theme snapshots for every screen, key-map polish
- [x] Task 15 VEN health: derived agent status, CLI `--unhealthy`, Workloads screen health
- [x] Task 16 Events: library model and filter, facade, CLI command, recorder
- [x] Task 17 Events screen
- [x] Task 18 Docs, README, task review, final gate

## Review (Plan 2)
- Interactive TUI: five screens (Traffic with Flows / App-to-app / Blocked, Policy, Labels & objects, Workloads with VEN health, Events), `:` commands, help overlay, filter editor with cost guard, opt-in auto-refresh that switches itself off on failure, profile switching, error overlay, mouse (wheel, tab click), dark and light themes.
- Final gate green: `cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace`.
- Events fixture recorded once from the test PCE (the recorder records every set, so 15 read calls, not one) into a scratch directory; the anonymiser was extended for `pce_fqdn`, `src_ip` and `api_key_id`, an `xtask anonymise` subcommand added, the file re-processed, audited per key and committed as `fixtures/recorded/events.json`. `cargo test -p pce --test recorded` green.
- Manual run against the test PCE at 100x30 through a pseudo-terminal: connect 5 calls; `r` 9 calls (1000 flows, truncation warning visible); `v` `v` no calls; `2` 10; `3` 10 (cache hits); `4` 11 (206 workloads, health summary shown); `h` no calls; `5` 12; `?` overlay; `q` exit 0. Final header: 12 API calls. Alternate screen left and mouse reporting disabled on exit.

## Plan 3: writes (labels, IP lists, services, unmanaged workloads)
Spec: docs/superpowers/specs/2026-10-10-pce-writes-design.md · Plan: docs/superpowers/plans/2026-10-10-pce-writes.md
- [x] Task 1 PceClient::write, PceError::Rejected
- [x] Task 2 Audit sink (JSONL 0600, memory, null)
- [x] Task 3 WriteGuard, Plan/Written/Outcome, Writer::execute
- [x] Task 4 Spec parsers (IP ranges, ports, interfaces)
- [x] Task 5 Labels planners
- [x] Task 6 IP list planners and patch
- [x] Task 7 Service planners and patch
- [x] Task 8 Unmanaged workload planners, lookup, patch
- [x] Task 9 CLI plumbing: writes setting, --allow-writes, exit 6, envelope, write.rs
- [x] Task 10 labels add/update/delete
- [x] Task 11 iplists add/update/delete
- [x] Task 12 services add/update/delete
- [x] Task 13 workloads add/update/delete
- [x] Task 14 Live opt-in smoke, docs, review, final gate

## Review (Plan 3)
- Shipped: `pce::write` (`WriteGuard`, `Writer`, planners for labels, IP lists, services and unmanaged workloads, JSONL audit) and CLI `labels|iplists|services|workloads add|update|delete` with `--allow-writes`, `--dry-run`, `--yes`, `--if-missing`, exit code 6, and `meta.write` in the envelope. Docs: README "Writes", architecture "Writes", website section 1.
- Safety model: profile opt-in plus per-run flag, audit log must open before anything is sent, no retries, ambiguous or managed targets refused locally, updates that cannot preserve unknown fields refused locally.
- Final gate: `cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace` (see commit for counts: 391 tests passing).
- The live write smoke (`PCE_LIVE_WRITES=1 cargo test -p pce --test live_writes`) was NOT run against any PCE: the shared demo PCE is approved for reads only. Without the env it prints the skip line and passes. Writes are unproven against a real PCE until it is run on a designated non-production one.
- Request counts (final-review fix wave, `--no-cache`, asserted in `tests/cli_writes.rs`): `workloads update <href> --set-labels a=1,b=2 --yes` sends 4 requests (one `/labels` GET, the workload GET, the PUT, the re-fetch); `iplists update Corp --add-fqdn x --yes` sends 4 (one `/ip_lists` GET, the href GET, the PUT, the re-fetch). Name resolution reads only the one list it needs; before the fix every resolved name built the four-list name index (four GETs uncached, more when a command resolved several names).
