# Lessons

Captured after corrections and review findings, so the same mistake is not repeated.

## 2026-10-10 — Plan 1 (pce library + CLI), subagent-driven execution

- **Count requests under every cache mode.** `--no-cache` quintupled object-list fetches on `traffic query` because each filter group rebuilt the name index. Any code path that takes a `CacheMode` must have a request-counting test for `Bypass`, not only for `Use`.
- **Guards before network.** Validate windows, caps, and `--yes` from local data before resolving names against the PCE. The review found four GETs issued before a guard failed.
- **One process per `assert_cmd` call.** Wiremock `.expect(n)` counts per spawned process; there is no shared cache across CLI invocations in a test.
- **Global clap flags collide silently with subcommand flags of the same name.** Prefix global flags (`--pce-port`).
- **Edition 2024 clippy wants let-chains.** Plan code with nested `if cond { if let ... }` will be rewritten; tell implementers to list that as a deviation.
- **Reviewer "knowledge" is not evidence.** Two Critical findings (serde_json's `zmij` dependency, cargo-deny's `crate =` exception key) were wrong for the installed tool versions. Run the one-line tooling check (`cargo tree`, `cargo deny check`) before opening a fix round.
- **Real PCE data revises assumptions.** Rulesets carry `deny_rules` inline (the per-ruleset endpoint is a fallback), active-policy hrefs differ from draft hrefs only by path segment, recorded lists can be legitimately empty, and `process_name`/`user_name` carry account names that need anonymising.
- **Error `Display` can leak input.** `toml::de::Error` echoes the offending source line; config errors must use `message()` so a malformed secret line never reaches stderr.
- **JSON for agents must keep hrefs.** Display strings alone (`ipl:Corp`) are not a stable contract; resolved objects carry `{kind, href, name}`.

## Plan 2 (TUI), 2026-10-10

- Carry small review rulings into the next task's brief instead of opening a fix round; every fix round in this plan closed in one re-review.
- `dotenvy::Error::LineParse` prints the raw line (possibly a secret); map it to the line index only. Fixed in `config.rs`; benefits the CLI too.
- The recorder records every set (~15 read calls). Record into a scratch directory, re-process with `cargo xtask anonymise`, and audit every string value per key before committing; the first events recording leaked `pce_fqdn`, `src_ip` and `api_key_id`.
- Headless TUI smoke: drive the binary through a Python `pty.fork` with `TIOCSWINSZ`, reconstruct screens with `pyte`. Grepping the raw byte stream is wrong: ratatui rewrites only changed cells, so first-frame text persists.
- Mouse events must stay request-free; when that hides data (deny rules for a wheel-selected ruleset), say so in the panel meta rather than fetching.
- Auto-refresh that fails switches itself off (`auto off · failed`), so a struggling PCE is never polled unattended.

## Plan 3 (writes), 2026-10-10

- Every "full list PUT" round trip (IP ranges, services, interfaces) first dropped fields the model did not carry; reviewers caught three silent-data-loss paths. Either carry unknown fields through (`extra` maps) or refuse the update locally.
- Managed checks must fail closed: a missing `managed` flag or a present VEN means refuse, never write.
- `--if-missing` must propagate lookup errors instead of treating them as "absent", or it creates duplicates.
- An audit line must go out in a single `write` syscall, so concurrent appends cannot interleave.
- hrefs are parsed strictly by component, never by substring; writes disable redirects. A substring check let `/orgs/5/labels/../workloads/x` pass as a label href, and URL normalisation then sent the write to another object; a followed 302 turned a PUT into a GET reported as success.
- An update sends only the keys it changes. Re-sending unchanged lists is how fields get lost.

## Plan 4 (TUI editing), 2026-10-11

- A confirm overlay that opens asynchronously needs an in-flight guard: keys typed while the planner's GET runs land on whatever opens next, so a `y` meant for a prompt can confirm a write. Refuse new edits while one is in flight and discard a plan that lands over another overlay.
- Pre-filled list fields must match the shown text first for every field type, not only the one the previous plan burned us on (ranges). The address field compared a parsed token with the raw PCE string, which turned an untouched form into an interface rewrite.
- A doc sentence about refresh behaviour ("press `r`") is a claim about a code path; check which path the key actually runs before writing it.
- When a screen is the safety gate (the dry-run overlay), clipping its content is not polish: treat it as Important in task reviews, not as a deferred minor.
- Task reviewers rate per task; the whole-branch review is where "minor" findings on safety paths get promoted. Keep the deferred-minor list in the ledger and hand it to the final reviewer explicitly.
- In zsh, backticks inside `echo "…"` run as commands; write ledger lines with a quoted heredoc.

## Plan 5 (policy writes), 2026-10-11

- Check real recorded data before encoding an "empty" shape: the spec said `scopes: []` for an unscoped ruleset; every unscoped ruleset the PCE returns is `[[]]`. The plan writer caught it by reading `fixtures/recorded/`.
- Fixed allow-lists for what can be provisioned fail closed on the first PCE that has something else pending (virtual services, firewall settings). Group by the href's type segment and let the PCE refuse what it will not accept.
- Id-less singleton hrefs (`sec_policy/draft/firewall_settings`) are a real PCE shape; a strict `<type>/<id>` parser must have a sibling for them or `--all` can never run.
- Apply list patches to the fetched JSON, not to the typed model: that is what keeps unknown actor and service keys alive through a round trip, independent of how the typed conversion treats `extra`.
- A grammar with prefixes and hrefs needs the "prefix + href" case decided up front; otherwise the prefix wins and the href is treated as a name.
- A field the PCE always returns (`resolve_labels_as`) is probably one it wants on create; default it explicitly rather than hoping the server does.
- Name lookups used for provisioning must include objects pending deletion; the pending list is the real guard.

## Plan 6 (TUI policy editing), 2026-10-11

- "Empty list" and "not fetched" are different facts; decide which one an empty payload means from the surrounding data (the PCE inlines deny rules everywhere or nowhere) before rendering "unchanged" on a pre-provision screen.
- A planner's `Noop` is an answer, not a failure; every result path that can carry it needs an info rendering.
- Any late async result (`DiffLoaded`, `Planned`, `PendingLoaded`) must check which overlay is open before replacing it; the typed-ahead `y` guard from Plan 4 generalises to "never swap a modal under the user".
- Writes-off runtime tests must count requests at the mock server and assert the status after each key; a client-side counter and a final status can both lie.
- Form-to-draft adapters get reused by new entry points (`w` from a flow) only if the form shape, not the key handler, is the unit; keep the pre-fill as data.

## Hotfix 2026-10-11

- The first live rule write found that the PCE refuses inline ICMP: the `sec_rule_ingress_services` schema takes `proto` in [6, 17] only (port optional, meaning all ports), and ICMP is the `ilo_defaults`/`ICMP` service object. The opt-in live test had never run; a schema assumption needs one real write per shape.

## Plan 7 (IP lists and rule impact), 2026-10-11

- A fix that widens a value's range needs the formatter re-checked: removing the `≥16M → any` cap exposed `human()` topping out at G, so IPv6 lists clipped in the column. When a ruling changes a bound, grep for every place that assumed the old bound and add a width test against the real column.
- Compact cells need a width test against the real column width, not a prose budget: the 7-cell P/C cell clipped `≥1234/≥5678` until the test read `rule_columns()`.
- Impact computed per draw is a hidden hot loop; store it on the row and recompute only when the inputs (workloads, index) change.
- The name index must be rebuilt when any object it covers reloads (labels, IP lists); a stale index silently zeros a side or drops a note.
- Honour `resolve_labels_as` per side; the virtual-services case counts nothing from labels and says so in a note.
- Unknown scope groups are skipped with a note rather than zeroing the side: over-stating blast radius is the safer direction for an allow rule.

## Plan 8 (traffic views: label grouping, mesh, treemap), 2026-10-12

- A plan writer that replays its own edits on a scratch worktree before publishing turns implementation into transcription: nine tasks, two one-round fix loops, no interface mismatch. Worth the planning cost on a 9-task plan.
- Fold rules need the dominant case stated: "smallest named tile joins other" inverted the treemap's purpose when one source dwarfed the tail. Give the remainder tile a floor instead of taking from the named ones.
- A model that only learns its size from resize events is wrong until the first resize; read the terminal size once at startup through the normal update path and test it with the test backend.
- A per-side field needs a per-side rule: the PCE reports user_name and process_name from the same VEN, so both follow flow_direction. Placing one and copying the other showed an IP list with a user.
- Aggregations that change their grouping must change every renderer of that group: the CLI blocked table kept printing the first host after the TUI grouped by destination group.
- Subagents in this harness carry an Opus attribution reminder; state the trailer in every dispatch and verify with git log before each review; rewrite once with filter-branch before the final review, not per task.
- Store resolved keys at rebuild time, never derive them per draw; the `quiet()` test helper makes "no request from this key" a one-liner.
