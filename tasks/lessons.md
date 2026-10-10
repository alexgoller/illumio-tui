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
