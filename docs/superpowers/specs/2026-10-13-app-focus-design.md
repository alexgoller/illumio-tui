# App focus and the app overlay — design

Date: 2026-10-13. Status: approved in conversation, awaiting spec review.

## Goal

An Illumio admin wants to look at one application at a time. Two features cover that:

1. **App focus.** Narrow every traffic view (Flows, App-to-app, Mesh, Treemap,
   Blocked) to one app's inbound, outbound, or both directions of traffic.
2. **App overlay.** `A` opens one screen with everything about that app:
   inbound and outbound peers, workloads with enforcement modes and VEN
   health, matching policy, and blocked flows over time with spikes.

Hard rule, unchanged: every PCE request is caused by the user, throttled,
cached, counted and visible. Nothing refreshes on its own.

## Definitions

- **App:** a group under the current `g` keys. The default keys are the
  profile scope keys (`app,env`). With those keys the app is written
  `shop/prod`; with `g` set to `app` alone it is `shop`. It is the same
  thing as one row label in App-to-app.
- **Direction:** `in` means flows whose destination is in the app. `out`
  means flows whose source is in the app. `both` is the union of the two.
  A flow from the app to itself counts once in `both`, and in each of
  `in` and `out` on its own.
- **Focus:** an app plus a direction. `Focus { app: GroupKey, keys: Vec<String>, direction: Direction }`.

## Approach

**Chosen: focus is a filter on the model, applied before every view's aggregation.**
- It is a single predicate, `Focus::keeps(&Flow) -> bool`, in
  `pce::traffic` next to `group`/`group_by_service`.
- The TUI applies it where the views take their flows today, in the same
  place `Decisions::kept` runs. The `d` filter, `o`, search and sort then
  work on top of it unchanged.
- The CLI uses the same predicate when it narrows a loaded result, and the
  same label set when it builds a query.

**Rejected:**
- Per-view focus logic, implemented once per view: five copies that would
  drift apart.
- A sixth "App" screen: Alex chose an overlay launched from Traffic.

## Part 1: app focus across every traffic view

### TUI

- **Setting the focus.**
  - `F` on a row whose app is unambiguous sets the focus to that row's app.
    That works on an App-to-app pair or section, a Flows row, a Mesh row, a
    Treemap tile and a Blocked row.
  - On a pair or flow row, `F` takes the destination app when the view is
    showing `in` sections. Otherwise it takes the source app first, and a
    second `F` switches to the destination app.
  - `F` on a row with no clear app, or `:focus <app>`, opens a picker with
    completion over the apps in the loaded result.
- **Direction.** `X` cycles in → out → both. A new focus starts at `both`.
- **Effect.** All five views show only the flows that `Focus::keeps`
  accepts. App-to-app keeps its pairs, by-service (`o`) and in/out/both
  section modes.
- **Display.** Every traffic view's header shows `focus shop/prod · in`.
- **Clearing.** `Esc` clears the focus; `F` with the focus set and no row
  under the cursor also clears it.
  - The overlay's `Esc` takes precedence while the overlay is open.
  - Today's `Esc` behaviour in Traffic still applies once no focus is set.
- **Changing `g`.** Changing the `g` keys clears the focus, because the app
  is no longer defined under the new keys. The status line says so.

### Requests

- Setting or changing the focus sends nothing: it only narrows the loaded
  result.
- **Truncated result.** If the loaded result was truncated (it hit the cap),
  the header shows `focus is partial: r re-queries shop/prod`, and the
  status line asks once. `r` then runs a focused query:
  - **`in`:** one Explorer query with the app's labels as destination
    includes (one AND group made of the app's label hrefs).
  - **`out`:** the same, with the app's labels as source includes.
  - **`both`:** two queries (in, then out), merged with duplicates removed.
    Explorer combines sources and destinations with AND, so one query
    cannot express "either side".
  - Everything else the user set stays the same: window, other filters,
    cap, confirmation (the confirm prompt states the number of queries),
    caching, backfill.
- **Clearing.** Clearing the focus shows the cached unfocused result again
  and sends no request.

### CLI

- `traffic query|groups|mesh --app shop/prod [--direction in|out|both]`
  - The app's labels go into the query, so the cap is spent on this app.
  - `--direction` defaults to `both`.
- **Compatibility.** Today `traffic groups --by in|out|both --app X` keeps
  one app's sections from the loaded flows. That keeps working, and the
  query is now also focused, so the sections come out the same.
  - `--app` without `--by` stops being an error: it now gives the pairs
    for the focused flows.
  - `--direction` conflicts with `--by`, because the sections already
    choose a direction.
- **Unknown app.** If the app does not match any label values under the
  keys, the command exits 2 and lists the closest matches. It sends no
  traffic request; it does fetch labels, which are cached.
- `meta` reports the request count, including 2 queries for `both`.

## Part 2: the app overlay (`A`)

- `A` on any row whose app is unambiguous (same rule as `F`) opens a
  full-screen overlay for that app.
  - It takes the current focus direction, or `both` if no focus is set.
  - `X` cycles the direction inside the overlay.
  - `Esc` returns to the view the overlay was opened from.
  - `:app <app>` opens it by name.
- Panels, in btop style:

```
┌ shop/prod · in+out · last 1d ─────────────── 3 req · cached ┐
│ Workloads 14   enforced 9 · selective 2 · visibility 3      │
│ VENs  13 active · 1 stale                                   │
├ Inbound ──────────────────┬ Outbound ─────────────────────┤
│ web/prod  tcp/443  ✓ 812  │ db/prod   tcp/5432  ✓ 1.2k    │
│ ops/prod  tcp/22   ✗  41  │ dns       udp/53    ✓  640    │
├ Blocked over time ────────┴───────────────────────────────┤
│ ▁▁▂▁▁▁█▂▁▁▁▁▁▃▁▁  spike 09:00 (x6 median)   t: exact       │
├ Policy ────────────────────┬ Workloads ────────────────────┤
│ RS shop-prod  12 rules     │ shop-web-01  web  enforced  ●  │
│ RS core-svc    3 rules     │ shop-db-01   db   visibility ● │
└────────────────────────────┴───────────────────────────────┘
```

### Panels and what they cost

| Panel | Source | Requests |
|---|---|---|
| Inbound / Outbound | loaded result, or the focused re-query; rows are peer group × service × decision, built with `group_by_service` | 0 |
| Rough timeline | buckets of the loaded flows' last-detected timestamps | 0 |
| Exact timeline (`t`) | one blocked-only query per bucket: 24×1h for 1d, 28×6h for 1w, 30×1d for 30d (other windows: about 24 equal buckets) | N, confirmed with the count, run one at a time, Esc cancels, cached |
| Workloads, enforcement, VENs | `PceClient::workloads` with `WorkloadFilter.labels` set to the app's labels | 1 (paged), cached |
| Policy | rulesets whose scope covers the app's labels, plus rules that name the app's labels as consumer or provider. Draft or active follows the Policy screen setting | 0 if the Policy cache holds them, otherwise 1, cached |

- **Opening the overlay** sends at most two requests: workloads, and
  rulesets if they are not cached. Both are counted in the header.
- **Reload.** `r` reloads the panels and asks for confirmation first.
- **Rough vs exact timeline.** The rough timeline is labelled `approx`. It
  counts flow records by their last-detected time, not connections over
  time. The exact timeline replaces it once it has been loaded.
- **Spikes.** A spike is a bucket at least 3× the median of the non-zero
  buckets. The worst spike is named with its time and its multiple. If
  there are fewer than 4 non-zero buckets, no spike is flagged.
- **Navigation.**
  - `Tab` moves between panels.
  - `Enter` on a peer closes the overlay and sets the focus to that pair in
    Flows.
  - `Enter` on a workload opens its detail on the Workloads screen;
    `Enter` on a ruleset opens it on the Policy screen.
  - `w` on a blocked peer row drafts an allow rule, as it does today (needs
    writes to be armed).
- **Workloads panel.** One row per workload: name, role label, enforcement
  mode, VEN status. The header line shows the count of each enforcement
  mode and the active/stale VEN counts.

### CLI

`illumio-tui app <app> [--direction in|out|both] [--timeline] [--yes]`

- Prints one envelope with these sections: `workloads` (with the
  enforcement breakdown), `inbound`, `outbound`, `policy`, `timeline`.
  `--table` and `--csv` work per section, as `traffic groups` does today.
- **Timeline.** Without `--timeline`, the timeline is the approximate one
  (0 requests). `--timeline` adds the per-bucket queries; if the bucket
  count is above the confirm threshold, the command needs `--yes`.
- `meta` reports requests, bytes and cache hits for the whole envelope.

## Errors and edge cases

- **Unknown app** in `:focus` or `:app`: the status line says so and
  offers the closest match. Nothing is sent.
- **App with no flows in the window:** the panels show "no flows in the
  window". The workloads and policy panels still load.
- **Workloads request fails:** that panel shows the error. The other
  panels still render. `r` retries.
- **Exact timeline cancelled part-way:** the buckets loaded so far are
  kept and marked partial. Pressing `t` again resumes from the cache.
- **Missing label key:** if the app's keys include a key that a workload or
  flow lacks, that workload or flow is not in the app. This matches how
  `group` treats it today.

## Testing

- **Unit tests in `pce`:**
  - `Focus::keeps` for in, out and both, including self-traffic.
  - The `both` merge removes duplicates.
  - Bucket plans for each window.
  - Spike detection: median, 3× threshold, fewer than 4 non-zero buckets.
  - Ruleset scope coverage.
- **TUI tests:**
  - `F`, `X` and `Esc` on each view, narrowing the flows each view shows.
  - The header badge.
  - `g` clears the focus.
  - The truncated-result prompt.
  - The overlay renders, as insta snapshots.
  - The overlay never sends a request it did not count.
- **CLI tests:**
  - `--app`/`--direction` builds the right query bodies (2 for `both`).
  - Unknown app exits 2.
  - The `app` envelope shape.
  - `--timeline` refuses without `--yes` above the threshold.
- **Read-only smoke on demo100** (no `--yes`, no `y`):
  - `traffic groups --app <app> --direction in`
  - `app <app>` without `--timeline`

## Delivery

Three GitHub issues, each with its own worktree, review and merge:

1. Focus (pce predicate, TUI `F`/`X`/badge/re-query, CLI `--app --direction`).
2. App overlay without the exact timeline, plus the `app` CLI.
3. Exact bucketed timeline and spike annotation (TUI `t`, CLI `--timeline`).

## Related

- alexgoller/illumio-mcp-skills#1 proposes an "app profile" skill covering
  the same content for agents.
