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
- Deviations reported by tasks:
  - Task 5: pending hrefs are grouped generically by object type (controller ruling over the spec's fixed list); a small `NOT_POLICY` deny-list (`workloads`, `labels`) keeps non-policy segments refused; after a fix round, id-less singleton hrefs such as `/orgs/N/sec_policy/draft/firewall_settings` are accepted and grouped (test `id_less_singletons_are_grouped_and_accepted`).
  - Task 7 (fix round): prefixed hrefs (`label:`, `group:`, `iplist:`, `workload:`, `service:` + `/orgs/...`) resolve through the strict library checkers with no list GET.
  - Task 8: the `policy rules [RULESET]` list positional became optional so `add|update|delete` sit beside it; a ruleset literally named `add`, `update` or `delete` is addressed by href.
  - Task 3: the no-op reason is `rule_set <name> already matches` (name included).
  - Task 10: README says `--all` groups every pending type generically, and unknown-name errors list up to five similar names.
- Parked: wrapping/`who` duplication cosmetic items; ASCII-only case folding in name matches; `workload:` lookups are not cached per token; actors with two kind keys parse by precedence (not a PCE shape); an identical relabel sends a redundant PUT; the singleton parser accepts collection hrefs (explicit targets must be pending); `resolve_labels_as` is compared order-sensitively (redundant PUT, no loss).
- Final whole-branch review: 0 Critical, 3 Important fixed in one wave (guard table profile-off proof, resolve_labels_as default, provisioning a ruleset pending deletion by name).

## Plan 6: TUI policy editing (rulesets, rules, deny rules, provisioning, ruleset diff, blocked flow to rule)
Spec: docs/superpowers/specs/2026-10-11-tui-policy-editing-design.md · Plan: docs/superpowers/plans/2026-10-11-tui-policy-editing.md
- [x] Task 1 Shared token module and TUI token resolution
- [x] Task 2 Ruleset editing
- [x] Task 3 Rule and deny-rule editing
- [x] Task 4 Pending marks and provisioning
- [x] Task 5 Ruleset pending diff
- [x] Task 6 Blocked flow to rule (w)
- [x] Task 7 Help lines and writes-off coverage across screens
- [x] Task 8 Docs, tracking, final gate and request-free smoke

## Review (Plan 6)
- Shipped: `n` / `e` / `x` on the Policy rulesets pane (draft only; the active scope says `switch to draft to edit policy`); `n` / `N` / `e` / `x` for allow and deny rules with the CLI's token grammar, resolved through the loaded index without a request; `Space` marks and `P` provisions from the pending overlay, with the exact `change_subset` on the confirm and `provisioned <version href>: <counts>` afterwards; `Enter` on a pending ruleset shows its changes against active; `w` on Traffic turns a blocked or potentially blocked flow into a draft allow rule; the token grammar moved to `crates/illumio-tui/src/tokens.rs`, shared by CLI and TUI.
- Gate: `cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace` green, 611 tests, 0 failed (illumio-tui lib 216, cli 24, cli_writes 71, tui_runtime 40, tui_snapshots 38; pce lib 95, client 9, facade 10, live 1, live_writes 2 (skipping), policy_writes 29, recorded 2, traffic_poll 8, write_contract 64; xtask 2).
- CLI diff vs `0e4ebad` (`src/cli`, `tests/cli.rs`, `tests/cli_writes.rs`): one file, `crates/illumio-tui/src/cli/write.rs` (+12 -426: imports, wrappers, the moved items and their tests); no message, exit code or envelope changed. `crates/pce` diff: one file, `crates/pce/src/resolve.rs` (+36, `service_href` and its test).
- Smoke against demo100 (headless, `--env-file`, scratch `XDG_STATE_HOME`, `PCE_LIVE_WRITES` unset, the script never presses `y`): A (`--allow-writes`, profile off) `n e x N P w` all say `writes are not enabled`, api calls 5, 6, 6, 6, 7, 7, 7 (only the screen loads and the `p` pending GET cost requests, no editing key did). C (writes on): rule form opened and cancelled (`cancelled, nothing sent`, no request), pending overlay with `Space` mark (`1/4 · 1 marked`), `P` planned one pending GET (7 to 8) and showed the provision confirm, cancelled with `n`; Traffic `r` ran one Explorer query (8 to 12, a read), `w` could not be exercised: demo100's 24h window returned 1000 flows (cap), all allowed, 0 blocked, 0 potentially blocked, so `/blocked` matched no row and `w` said `no row selected`. The `w` form is covered by the unit and runtime tests only. A diagnostic rerun of the Traffic query alone (9 calls) confirmed the counts. No write was sent to any PCE; the TUI write path is proven by the wiremock runtime tests in `tests/tui_runtime.rs` only. The audit file `writes.jsonl` in each scratch state dir (`state6/illumio-tui/` and `state6b/illumio-tui/`) is 0 bytes (0 lines).
- Deviations reported by tasks:
  - Task 1: `NameIndex::service_href` is an exact-name match (pending deletions excluded, `None` when ambiguous), not case-insensitive.
  - Task 2 (fix round): `quiet_reload` is taken before the scope check, so a scope switch cannot leak it into a later load; a dead arm and a delete-summary assertion were fixed.
  - Task 3: `render_form` shortens the footer hint to `Enter review · Esc cancel` when title plus hint exceed the 70-cell box.
  - Task 4: the rulesets reload after a provision follows the scope shown, not always draft.
  - Task 5 (fix round): the ruleset diff carries the model's fallback deny rules for the draft side and says `deny rules: not fetched for <side>` when a side has none inline and no fallback.
  - Task 6: `EditIntent::RuleFromFlow(Box<ResolvedFlow>)` (clippy `large_enum_variant`).
  - Task 7: the help key column widened from 24 to 25 cells; the writes-off runtime test counts requests server-side per session.
- Final whole-branch review: 0 Critical, 2 Important fixed in one wave (ruleset diff treats inline-empty deny lists as authoritative when the PCE inlines deny rules; update Noop shown as info), plus the rules-pane scope title Alex asked for. The same wave guarded a late `DiffLoaded`, named the rule in a rule delete confirm, fixed the `provisioned <version href>` wording and the help spacing; report in `.superpowers/sdd/2026-10-11-tui-policy-editing/final-fix-report.md`.
- Parked: allow/deny duplication (~50 lines); ambiguous/pending-delete names render but cannot resolve back; late DiffLoaded guard added; `w` not exercised on demo100 (no blocked flows in the window); help column 25; `Box<ResolvedFlow>`. Per-task minor lists in `.superpowers/sdd/2026-10-11-tui-policy-editing/progress.md`.

## Issue #3: label-model rule writing
Brief: .superpowers/sdd/issue-3/brief.md · Report: .superpowers/sdd/issue-3/report.md
- [x] 1 Config: `[profiles.<name>.labels]` scope_keys/tier_key, `ILLUMIO_SCOPE_KEYS` / `ILLUMIO_TIER_KEY`, carried in `Limits`
- [x] 2 `w` pre-fill in the label model (tiers, scopes, the matching loaded draft ruleset or a scope hint)
- [x] 3 `Ctrl-A` swaps in every attached label and back
- [x] 4 `Ctrl-N` on an unknown ruleset name: create the scoped ruleset, then the rule (two confirms, two POSTs)
- [x] 5 Docs (README Traffic `w` + Label model, architecture paragraph)

## Review (Issue #3)
- Shipped: the `w` form pre-fills providers with the destination's tier and consumers with the source's tier (same scope) or its scope and tier (unscoped); `loc` and custom keys are never pre-filled; the ruleset is the one loaded draft ruleset scoped like the destination, else empty with `scope app=…, env=…` as the hint. `Ctrl-A` toggles every attached label. An unknown ruleset name with a scoped destination offers `Ctrl-N`, which creates the ruleset scoped like the destination and then plans the rule into it for a second confirm. Dimension names come from `[profiles.<name>.labels]` or `ILLUMIO_SCOPE_KEYS` / `ILLUMIO_TIER_KEY`.
- Gate: `cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace` green, 631 tests, 0 failed (illumio-tui lib 234, cli 24, cli_writes 71, tui_runtime 42, tui_snapshots 38; pce unchanged).
- Unchanged: `crates/pce`, `src/cli`, the CLI tests, dependencies. Snapshots changed: `form_rule_from_flow_80x24`, `form_rule_from_flow_light_styled_80x24`.
- Fix round 1 (a1cae19): the Ctrl-N offer moved to the Ruleset hint row and the panel keys (the error row keeps the planner's text), disarms when the name changes, a failed chained rule plan reopens the form, `w form Ctrl-A Ctrl-N` is in the help; 636 tests. Not smoke-tested against a PCE.

## Plan 7: IP lists screen and rule impact
Spec: docs/superpowers/specs/2026-10-11-iplists-impact-design.md · Plan: docs/superpowers/plans/2026-10-11-iplists-impact.md
- [x] Task 1 pce::iplist_size
- [x] Task 2 pce::impact
- [x] Task 3 IP lists screen (read side)
- [x] Task 4 IP lists screen (write side, used-by recompute, CLI size)
- [x] Task 5 Workload cache plumbing and rule detail impact
- [x] Task 6 Confirm overlay impact line
- [x] Task 7 CLI --impact for existing rules
- [x] Task 8 CLI --impact on rule writes
- [x] Task 9 Docs, tracking, reads-only smoke, final gate

## Review (Plan 7)
- Shipped: the IP lists screen (`6`, `:iplists` / `:ip`) with size, ranges, FQDNs, used-by and pending, a detail with the size and the rules that use the list, and `n` / `e` / `x` with writes on; sizes in the CLI (`iplists` carries `size`; table column); `i` on Policy reads the workloads once and fills the rule detail's `impact` and `ip lists` rows and the rules pane's `P/C` column; the same impact line on a rule's confirm overlay; `policy rules <ruleset> --impact`; `policy rules impact <href>`; `--impact` on `policy rules|deny-rules add|update` in dry runs and results.
- Gate: `cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace` green, 704 tests, 0 failed (illumio-tui lib 264, cli 26, cli_writes 78, tui_runtime 46, tui_snapshots 43; pce lib 117, client 9, facade 10, live 1, live_writes 2 (skipping), policy_writes 32, recorded 2, traffic_poll 8, write_contract 64; xtask 2).
- `crates/pce` diff vs `c0aa870`: new `iplist_size.rs` and `impact.rs`, `resolve.rs` (`canon` visibility, accessors, tests), `lib.rs` (modules, re-exports), plus the merged hotfix (`write/mod.rs`, `write/rules.rs`, `models/labels.rs`, `tests/policy_writes.rs`: inline rule services are TCP/UDP, `icmp` resolves to the ICMP service object). The brief expected an empty `crates/pce/src/write` diff; it is not empty only because of the hotfix. No Cargo.toml or Cargo.lock change (empty diff, no new dependency).
- Smoke against demo100 (headless TUI without `--allow-writes`, `--env-file`, scratch `XDG_STATE_HOME`, `PCE_LIVE_WRITES` unset, the script never presses `y`): api calls per step 5 (startup), 5 after `6` (no request), 5 after Enter on the first list (detail: `size any (0.0.0.0/0)`, `used by load Policy (2) to see rules`), 6 after `2` (the draft rulesets), 6 after Tab, 6 after Enter on a rule (detail with `impact      press i to load workloads (1 request)`), 7 after `i` (one workloads read, 206 workloads; detail `impact      0 providers · 0 consumers`, `P/C` column `0/0`), 7 after a second `i` (no request). CLI: `policy rules 'Quarantine Policy: Strict' --impact --json` 3 rules, api_calls 6, truncated false, all `(0, 0)`; `iplists --json` api_calls 1, 12 lists, each with `size`. A rule detail was exercised (the first ruleset has rules); the zero counts are real (its quarantine labels match no workload). No write was sent to any PCE. The audit file `state7/illumio-tui/writes.jsonl` in the scratch directory was never created (absent).
- Deviations reported by tasks:
  - Task 3: Objects has three tabs now that IP lists left it, so editing keys moved: one unit test asserts 3 commands instead of 4, two runtime key streams lost one `Tab`, the writes-off runtime test now lands on Services (sends nothing). 61 existing snapshots re-recorded for the one reason that every tab row changed (six screens, short titles on a narrow terminal). Fix round: four overlay snapshots re-recorded on the IP lists background.
  - Task 5: `a auto` was replaced by `i impact` in the writes-off Policy hint (80 columns were full; `a` stays in the help). The impact is stored on `RuleRow.impact` (computed once per `rebuild_rules`), never per draw; `pce::{workload_labels, rule_impact_with}` added so label sets are built once per load; P/C sorts numerically with `-` rows last; 21 snapshots re-recorded (the status-line hint).
  - Task 6: the confirm overlay's impact line is stored in `PolicyState.confirm_impact` (refreshed on `Planned`, `i`, `WorkloadsLoaded`, `RuleSetsLoaded`), not computed while drawing; two new snapshots, none changed.
  - Task 8 (fix round): an update whose rule is missing from the cached rulesets is exit 4 before the workloads read, rather than computing on empty sides.
  - Hotfix: main (a97b3ca, inline rule services must be TCP/UDP, `icmp` resolves to the ICMP service object) was merged into the branch (2e44198) with one import conflict in `cli/write.rs` resolved by keeping both imports.
- Parked (per-task minor lists in `.superpowers/sdd/2026-10-11-iplists-impact/progress.md`): `iplists.rs` duplicates part of the Objects meta/detail code; `used_by` compares hrefs exactly; no `--yes` impact tests for update / deny; a recount keeps the selection by index; `-` first on a descending P/C sort; the confirm check order; CLI impact with `--active`; the index-keyed union; per-keypress re-evaluation.
- Final whole-branch review: 1 Critical (P/C clipping), 4 Important fixed in one wave (stale-index fallback and notes, resolve_labels_as, used-by placement, any only for /0); the Minor items in the same wave (notes in the rule detail and confirm line, CLI `--impact` builds label sets once, `human()` rounds before the unit, README cache TTL, this review). Report: `.superpowers/sdd/2026-10-11-iplists-impact/final-fix-report.md`.

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

## Review (Traffic views)
- Shipped: the IP lists tab (fourth on Objects) and five screens (`1` Traffic, `2` Policy, `3` Objects, `4` Workloads, `5` Events; `:iplists` / `:ip` open the tab); the display keys (profile scope keys, session-wide) and the `g` picker; Flows with `src group` / `dst group` columns and a detail with both label sets and bytes; Blocked by destination group and service, with `w` on its rows; Mesh (user, process, source, destination, service, destination process) with `u` / `p`, drill and `w`; Treemap with cursor, zoom, drill and bytes weight, laid out in `update` and re-laid on resize; CLI `traffic groups` and `traffic mesh` with `--keys`, `--no-user`, `--no-process`.
- Gate: `cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace` green, 783 tests passed, 0 failed (the plan expected 776).
- `crates/pce` diff vs `34b4fa9`: `traffic/{aggregate,resolved,mod}.rs` changed, `traffic/mesh.rs` and `traffic/treemap.rs` new, plus one comment in `iplist_size.rs` (Task 1: "screen" became "tab"); no request path changed. `git diff 34b4fa9 HEAD -- Cargo.toml crates/*/Cargo.toml Cargo.lock` is empty: no new dependency.
- Smoke against demo100: not run (described only: one Explorer query in the TUI, then every view, the picker, the zoom and the bytes weight with the API call counter asserted unchanged, then the two CLI reads; no `--allow-writes`, never `y`). Nothing in the views, keys, zoom or columns emits a `Cmd`: every TUI key test asserts an empty `Vec<Cmd>`.
- No write was sent to any PCE.
- Deviations reported by tasks: see `.superpowers/sdd/2026-10-11-traffic-views/` (per-task reports).
