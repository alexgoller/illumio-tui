# Traffic accuracy and speed: design

Date: 2026-10-14.
Status: the decisions below were taken by Alex on 2026-10-14.
Issues: #22, #23, #26, #27, #28.

Evidence: an external comparison with workloader (lab PCE), plus an investigation on demo100 (7 queries). The key facts follow.

## What Explorer returns

- An async traffic query returns every stored **aggregate record** whose lifetime overlaps the window.
- Each record carries its own `first_detected` and `last_detected`. `last_detected` can be after the window end.
- Each record's `num_connections` (and bytes) is a **lifetime** count, not a count for the window.
- A narrower window returns a subset of the *same* records with the same counters.
  - On demo100, 1 hour of the day held 99 % of the day's connections.
  - One blocked-only hourly bucket returned exactly the day's blocked records that overlap that hour.
- The PCE can return several versions of one aggregate (the same `first_detected`, an earlier `last_detected`, a lower count). The latest version supersedes the others.
- `flows_count` in the completed status is the number of rows returned after the cap. `matches_count` is a bounded raw count, **not** a total: 400 at cap 200, and 4,631 for 4,625 rows at cap 10k. A supercluster reports per-region counts in `regions[]`.

What this means for us:
- Summing records across overlapping windows (backfill slices, timeline buckets) is wrong. That was the 12–23× inflation.
- A record must be identified by its flow tuple **plus `first_detected`**. Duplicates are resolved by keeping the latest version.
- Truncation is `flows_count >= cap` (per region, when there are regions), never `matches_count > rows`.

## Decisions

1. **One merge rule (#22).** `pce::traffic::merge` dedupes by record key and never sums.
   - The record key is src (href + IP), dst (href + IP), port, proto, process, user, decision, direction and `first_detected`.
   - For the same key, keep the copy with the latest `last_detected` (tie: the larger `num_connections`).
   - `focus::union_flows` uses the same function.
   - `flow_key` stays for display grouping only.
2. **Backfill raises the cap (#22). Time slices are gone.**
   - `B` (TUI) and `--backfill` (CLI) re-run the same window at the next `CAP_STEPS` rung: 10k → 25k → 50k → 100k → 200k.
   - Each rung is one query, and the result replaces the previous one.
   - Any rung above `CONFIRM_MAX_RESULTS` (10k) asks first: `y` in the TUI, `--yes` in the CLI. This holds even when the previous result proved the data is larger, because Alex wants confirmation above 10k.
   - The CLI's `--backfill` climbs rungs until the result is complete or the confirmed ceiling is reached. The ceiling is 200k with `--yes`, otherwise 10k. `meta.cap_steps` lists the caps it tried.
   - The TUI's `B` climbs one rung per confirmation.
   - Retire `--max-slices` and `meta.slices`. Passing `--max-slices` exits 2 with "time slices were removed: --backfill raises the cap".
   - `traffic_auto_backfill` becomes "raise the cap up to 10k without asking".
   - Keys that were slice-specific (`+ - ] [` acting as slices) go back to plain window and cap changes. If a key only existed for slices, remove it, and its help line too.
3. **Coverage goes (#22).**
   - Remove the `last_detected`-span coverage estimate (`coverage`, `slice_for`, `plan_backfill`, `truncated_coverage`, the TUI `coverage()` and `low_coverage`).
   - A complete result covers its window.
   - A truncated result says "N flows, capped at C; B raises the cap" and claims no time span.
   - `meta.covered` is the window when the result is complete, and `null` when it is truncated.
4. **Reliable truncation (#23).**
   - `AsyncQueryStatus` gains `regions`.
   - `truncated` = any region's `flows_count >= cap` when there are regions, otherwise `flows_count >= cap`. When `flows_count` is missing, fall back to `rows >= cap`.
   - A region that did not respond sets a separate `incomplete` flag, shown in the header and in `meta`.
   - `matches_count` is never shown as a total and never summed.
5. **The timeline is dropped (#22; Alex, 2026-10-14).** Explorer returns lifetime aggregates, so no honest view of blocked flows over time is possible.
   - Remove from the app overlay: the timeline panel (approx and exact), `t`, and spikes. The overlay keeps its other panels, and the space goes to the inbound and outbound peers.
   - Remove from the CLI: `app --timeline`, the `timeline` section, `TIMELINE_CONFIRM_QUERIES`, and the timeline cache slots.
   - Remove `pce::traffic::timeline` and `Pce::timeline_bucket`, plus the docs and help that describe them.

6. **Count units (#26).**
   - Every traffic table header and `--help` states the unit: `flows` counts flow records; `conns` and `bytes` are lifetime counts of those records.
   - `traffic groups` (pairs, `--services`, sections) gains per-decision connection columns, `allowed_conns`, `pot_blocked_conns` and `blocked_conns`, in JSON and CSV. In table mode, show them only when space allows.
   - The README notes that connection counts are lifetime counts of the aggregate records, as in the PCE UI.
7. **Transmission filter (#27).**
   - `--transmission unicast|broadcast|multicast` (repeatable). The TUI filter gets the same field, plus a `:unicast` toggle.
   - It maps to Explorer's `exclude`/`include` transmission filter. Check the exact query field against the API docs and our fixtures. If Explorer can't filter it, filter locally and say so.
   - The default stays "all", as today. The docs explain the difference from workloader's unicast-only default.
8. **Speed (#28).** No extra load.
   - Enable reqwest's `gzip` feature. This is the one approved change to Cargo.toml and Cargo.lock.
   - Lower the poll cap from 10 s to 3 s (start at 1 s, ×1.5).
   - CLI `fetch` submits the query concurrently with the name-index loads.
   - The result download gets its own timeout (300 s), so that 200k-row results don't hit the 30 s client timeout.

## Compatibility

These are breaking changes for v0.1.9:
- `--max-slices` and `meta.slices` are removed.
- The meaning of `meta.covered` changes.
- The app timeline is removed: `t`, `app --timeline` and the `timeline` section.
- `--backfill` now means raising the cap.

The release notes must spell all of this out.

## Testing

- **Unit tests in `pce`:**
  - Merge dedupe: versions, tie, never summed.
  - Truncation from statuses: 4631/4625 at cap 10k is complete; 400/200 at cap 200 is truncated; a regions case; missing `flows_count`.
  - Cap-raise steps.
- **TUI:**
  - `B` asks above 10k and replaces the result.
  - No time-slice code paths remain.
  - The header for truncated vs complete results.
  - Late answers are guarded.
- **CLI:**
  - `--backfill` with mocks: 10k truncated → asks for `--yes` → climbs.
  - `--max-slices` exits 2.
  - `covered` is null when truncated.
  - `--transmission`.
  - The per-decision connection columns.
  - gzip: the client sends `Accept-Encoding: gzip`, and a gzipped mock body decodes.
- **Read-only smoke on demo100** (no `--yes`, no `y`):
  - A 1-day query reports `truncated: false`.

## Delivery

Two issues, in order:
- **A.** `pce` core: merge, truncation, coverage removal, backfill as cap raise (CLI and TUI), and speed. Closes #22 (backfill part), #23 and #28.
- **B.** Remove the timeline (#22, timeline part), count units (#26) and the transmission filter (#27).
