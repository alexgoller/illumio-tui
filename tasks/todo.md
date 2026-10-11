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
- Unverified against a live PCE (no write-approved PCE yet): `workloads update --add-ip/--remove-ip` (the interfaces PUT, with `network`, `loopback`, `href` and null extras dropped before sending) and FQDN `description`/unknown fields carried through an IP list update.

## Plan 4: TUI editing (labels, IP lists, services, unmanaged workloads; pending diff)
Spec: docs/superpowers/specs/2026-10-11-tui-editing-design.md · Plan: docs/superpowers/plans/2026-10-11-tui-editing.md
- [x] Task 1 Library: active IP lists and services
- [x] Task 2 Writes state, runtime guard and header marker
- [x] Task 3 Form overlay
- [x] Task 4 Edit intents and form adapters
- [x] Task 5 Editing keys, scrollable help, hints
- [x] Task 6 Plan and confirm
- [x] Task 7 Execute, report and reload
- [x] Task 8 Pending diff
- [x] Task 9 Docs, tracking, final gate and request-free smoke

## Review (Plan 4)
- Shipped: `· ✎ writes` header marker while profile and `--allow-writes` both allow writes; `n`/`e`/`x` on Labels & objects and unmanaged Workloads, with refusals that send nothing; a form overlay with CLI-grammar list fields; a confirm overlay that shows summary, `METHOD path` and body before `y` sends once; status line report and one reload after a write; a navigable pending list on Policy with `Enter` diffing a draft IP list or service against active (one cached GET).
- Final gate: fmt, clippy `-D warnings`, `cargo test --workspace` pass, 457 tests, 0 failed; the CLI sources and tests are untouched by Plan 4 (empty diff stat).
- Smoke against demo100 (headless, `--env-file`, scratch `XDG_STATE_HOME`, `PCE_LIVE_WRITES` unset, script never presses `y`): A (flag missing) 5 calls before and after `n`; B (profile not enabled) 5 and 5; C (form opened and cancelled, label planned and its confirm overlay cancelled with `n`) 5 calls up to Policy, 7 after loading Policy and the pending list; the first pending entry was a rule set (`rule diffs arrive with Plan 5`). Extra run D: selecting a pending service and `Enter` cost one GET (7 to 8 calls).
- No write was sent to any PCE; the audit log in the scratch state dir had 0 lines. The TUI write path is proven by the wiremock runtime tests in `tests/tui_runtime.rs` only.
- Deviations reported by tasks: Task 8 had two accepted deviations (`Ref::new(&diff.href)` bound to a local in `render_diff` for a borrow fix; `std::slice::from_ref(&c)` instead of `&[c.clone()]` in the diff tests on clippy's advice). Task 5 wrote its tests after the implementation (no RED run).
- Final whole-branch review (opus): 0 Critical, 5 Important, 7 Minor. One fix wave (5 commits, a9c43f8..417e935), re-reviewed clean: one write in flight at a time and a late plan never lands on another overlay (typed-ahead `y`); reloaded labels rebuild the name index at no extra request (and the README now says what `r` does); workload addresses match the shown text first and a prefix removal is refused instead of reported as "no changes"; an unanswered write whose audit failed still reloads; the confirm overlay wraps its body; the `✎ writes` marker sits before the host so a long hostname cannot push it off screen; the stale "loading the active version of ..." status is replaced. Gate after the wave: 467 tests, 0 failed.
- Parked (documented, not fixed): wrapping counts characters not display cells; the confirm overlay's summary and `METHOD path` lines still clip when very long; a typed non-canonical IPv6 token against the PCE's spelling still shows as remove plus add (untouched forms are fine); `k` in the confirm overlay does not re-clamp after a resize; removing a duplicated entry fails closed with a planner error; in-field cursor movement is not in the form (spec 4.3), so editing a long list means Ctrl-U and retyping.

## Plan 5: policy writes (rulesets, rules, deny rules, provisioning, managed labels)
Spec: docs/superpowers/specs/2026-10-11-policy-writes-design.md · Plan: docs/superpowers/plans/2026-10-11-policy-writes.md
- [x] Task 1 Models and kinds
- [x] Task 2 Actors and services
- [x] Task 3 Rulesets
- [x] Task 4 Rules and deny rules
- [x] Task 5 Provisioning
- [x] Task 6 Managed workload labels and the label-group lookup
- [x] Task 7 CLI token grammar and resolution
- [x] Task 8 CLI rulesets, rules and deny rules
- [x] Task 9 CLI provisioning, managed labels and the guard table
- [x] Task 10 Docs, tracking, opt-in live test, dry-run smoke, final gate

## Review (Plan 5)
- Shipped: `policy rulesets add|update|delete`, `policy rules add|update|delete`, `policy deny-rules add|update|delete` (draft only, compact actor and service token grammar, names resolved with at most one list GET per kind); `policy provision --all|NAMES|HREFS` (one pending GET, explicit `change_subset` grouped by type, `counts` per type); `workloads update --managed-labels` (labels-only PUT, other flags exit 2); 23-entry guard table.
- Gate: `cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace` green, 555 tests, 0 failed (illumio-tui lib 182, cli 24, cli_writes 67, tui_runtime 30, tui_snapshots 33; pce lib 92, client 9, facade 10, live 1, live_writes 2 (skipping), policy_writes 29, recorded 2, traffic_poll 8, write_contract 64; xtask 2).
- TUI diff vs main: one file, `crates/illumio-tui/src/tui/update.rs`, +5 lines (the `RuleSet | Rule | DenyRule | Provision` arm in `reload_after_write`).
- Smoke against demo100, dry runs only, `--yes` never passed, `PCE_LIVE_WRITES` unset, scratch `XDG_STATE_HOME`: `policy provision --all --dry-run` api_calls 1 (POST /orgs/5636114/sec_policy, counts rule_sets=2 services=2); `policy rules add "Quarantine Policy: Strict" ... --dry-run` api_calls 1 (POST .../rule_sets/24206847997116627/sec_rules); `policy rulesets add smoke-test --dry-run` api_calls 0 (POST .../rule_sets, scopes [[]]). Audit file `<scratch>/state/illumio-tui/writes.jsonl` is 0 bytes (0 lines). No write was sent.
- `crates/pce/tests/live_writes.rs` was extended with `live_ruleset_and_rule_lifecycle` and NOT run with `PCE_LIVE_WRITES=1`; without it both tests print the skip line and pass.
- Deviations: see the per-task reports in `.superpowers/sdd/2026-10-11-policy-writes/`. README: the brief's sentence that a pending object of another type makes `--all` refuse was replaced, because the code groups every pending type generically and only refuses an href it cannot parse.
- Parked: see the final whole-branch review of Plan 5 for the items it names.
