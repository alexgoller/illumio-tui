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
