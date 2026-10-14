# Draft policy decision: design

Date: 2026-10-14. Status: approved by Alex in conversation on 2026-10-14.

## Goal

Show, next to each flow's reported decision, the decision the **draft** policy would make, so an admin can check whether their draft rules fix the blocked flows before provisioning.

## What the PCE offers

Verified against the PCE 24.5 northbound schema and workloader (brian1917/illumioapi `traffic.go`).

- After an async traffic query has completed, `PUT <query href>/update_rules?label_based_rules=false&offset=0&limit=<cap>` (empty body) asks the PCE to re-evaluate the result against draft policy.
- The query's status then gains `rules`. Poll `GET <query href>` until `rules == "completed"`.
- The download then carries `draft_policy_decision` (a string) per flow, plus `rules` (draft allow rules), `enforcement_boundaries` and `override_deny_rules` (also draft).
- Model `draft_policy_decision` as a string-backed enum. Known values are allowed, potentially_blocked and blocked; keep any other value verbatim (`Other(String)`) so new PCE values don't break parsing.

## Behaviour

- **Opt-in, per query.** The CLI uses `--draft` on `traffic query|groups|mesh` and `app`. The TUI uses `D` in Traffic, which re-evaluates the result on screen.
- **Cost.** Draft re-evaluation costs 1 PUT plus status polls plus 1 download. When the async query is still known (our own run, same session), there is no new POST.
  - The CLI runs the query with `--draft` in one go: POST, polls, PUT, polls, download.
  - The TUI's `D` needs the query href. Keep the href of the result on screen in its data. If it is missing (the result came from the cache, or the PCE dropped the query), `D` runs the query again with draft, as one confirmable action that states its cost.
  - Every request is counted. The confirmation rules are the same as for a query (above 10k rows, or a window above 7d).
- **Caching.** The draft result is cached under its own slot, keyed by the query body. Clearing it returns to the plain result with no request.
- **Seq guard.** A late draft answer must not land on a result that changed in the meantime (new query, focus change, cap step).

## Display

- **Flows view:** a `draft` column. Rows whose draft decision differs from the reported one are highlighted, for example `blocked → allowed`.
- **App-to-app pairs, by service and sections:** a `would allow` count per row (blocked or potentially blocked now, allowed in draft), plus `would block` if any. In the JSON and CSV, add `draft_allowed` and `draft_blocked` counts per row, with the flows unit stated.
- **App overlay:** in the in/out peers, the `would allow` mark per row, and the header shows "draft: N blocked flows fixed".
- **Decision filter `d`:** add a step "fixed by draft" (blocked or potentially blocked now, allowed in draft), active only while draft data is loaded.
- **CLI JSON:** every flow carries `draft_policy_decision`. `meta.draft` holds `{evaluated: true, fixed: N, newly_blocked: M}`. The CLI table gets a `draft` column.
- **Header:** the TUI header shows `draft ✓` while draft decisions are loaded.

## Errors

- A PCE that rejects `update_rules` (old version) produces a clear error. The plain result stays on screen.
- A status that never reaches `rules == completed` uses the same poll timeout as the query, and Esc cancels it.

## Testing

- **Model:** parse unknown draft values.
- **Poll loop with wiremock:**
  - The PUT path and query string.
  - Polling until rules are completed.
  - The download carries the draft fields.
  - Request counts.
- **TUI:**
  - `D` with a known href sends one PUT and no POST.
  - `D` without an href asks, then re-runs the query.
  - The seq guard drops late answers.
  - The `fixed by draft` filter step.
  - Snapshots of the draft column and marks.
- **CLI:**
  - `--draft` JSON fields and `meta.draft`.
  - The CSV columns.
  - The `--draft` request sequence.
- **Read-only smoke on demo100** (no `--yes`):
  - `traffic query --window 1h --max-results 200 --draft --json` once. Report the draft decision counts.
  - Note that `update_rules` is a PUT on our own query resource. It changes no policy, but say so in the docs.

## Docs

README, docs/index.html and the in-TUI help. Say that the draft re-evaluation is the PCE's own Explorer feature and that it does not modify policy.
