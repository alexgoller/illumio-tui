# App to enforcement: design

Date: 2026-10-19. Status: written by the controller while Alex was off keyboard ("go without many interruptions"); every decision below is a ruling he can overturn.

## Goal

Take one application from "observed traffic, no policy" to "enforced", safely, in one terminal: see whether it is ready, close the gaps with the tools that already exist (generator, rule form, provision), move its workloads to enforcement, watch what gets blocked afterwards, and roll back with one key.

Every step already exists in the app somewhere (app focus, app overlay, draft decision, policy generator, pending/provision, bulk enforcement, flow marks). This feature adds the *readiness* judgement that ties them together, a guided overlay, and a change record that makes rollback exact.

## Terms

- **App**: the scope label values of the profile's scope keys (default `app`, `env`), as in app focus (`:focus shop/prod`).
- **App flows**: unicast flows of the current traffic result with the app on either side, inbound (destination in the app) and outbound (source in the app). Flows inside the app count once, as inbound.
- **Would block**: an app flow whose draft decision (after `update_rules`) is `potentially_blocked` or `blocked` (or another non-allowed value). These are the flows that enforcement would block under the draft policy, once provisioned.
- **Accepted**: an app flow with a live flow mark `squelch` or `accept-blocked` (#40–#42). Accepted flows never count as gaps.

## Readiness

Computed locally from data that is loaded or loadable with requests the app already makes:

| Input | Source | Cost |
|---|---|---|
| Workloads in the app | workloads list (cached) | 0–1 GET |
| Traffic and draft decision | the current traffic query + `update_rules` (existing `D` path) | the query + 1 PUT + polls + 1 download |
| Pending changes | `sec_policy/pending` (cached) | 0–1 GET |
| Policy and rulesets | loaded draft policy (cached) | 0–1 GET |
| Marks | `marks.toml` (local) | 0 |

Per app, readiness reports:

- **Workloads**: managed / unmanaged count; enforcement mix (`idle`, `visibility_only`, `selective`, `full`); offline VENs; managed workloads missing the tier key (`role`).
- **Evidence**: window length and the number of app flows and connections; *little evidence* when the window is under 7 days or under 20 app flows.
- **Coverage**: connections in and out, allowed by draft vs would-block, as counts and a percentage; accepted connections shown separately.
- **Gaps**: the would-block flows grouped as policy tuples (peer group, service), the same grouping the generator uses, largest first.
- **Pending**: pending changes in rulesets whose scope covers the app, and pending objects those rules use. Enforcement uses *active* policy, so unprovisioned rules are a blocker even when the draft covers everything.

**Verdict** (first that applies):

1. `not evaluated`: no draft decision loaded.
2. `needs rules`: would-block connections > 0.
3. `needs provision`: pending changes for the app.
4. `ready`: no blockers left. *Warnings* (no effect on the verdict): little evidence, workloads without a tier label, offline VENs, unmanaged workloads (they are never moved).

Already in full enforcement: `enforced`, with the same numbers as a check.

## Surfaces

### CLI

- `illumio-tui readiness [--window 7d] [--app shop/prod]... [--json|--table|--csv]`
  - One traffic query plus the draft evaluation, then the per-app table: app, workloads, enforcement mix, flows, connections in/out, would-block, coverage %, pending, verdict, warnings.
  - With `--app`, the query is narrowed to those apps (include filters) and the output adds the gaps list.
  - The usual cost rules apply (window above 7d or cap above 10k needs `--yes`), and the cost is in `meta`.
- `illumio-tui enforce <app> --to full|selective [--visibility LEVEL] [--window 7d] --dry-run|--yes [--confirm-count N] [--force]`
  - Runs readiness for the app first. Refuses with exit 6, `write_refused`, when the verdict is not `ready` (or `enforced`), unless `--force`; the message lists the blockers.
  - Then the existing bulk enforcement on the app's managed workloads, with all its guards.
  - Writes a **change record** (below).
- `illumio-tui enforce <app> --watch [--since <record|time>]`
  - One traffic query from the change time (default: the latest record for the app) to now. Lists flows with the reported decision `blocked` involving the app, grouped as tuples.
  - Exit 0 when there are none. Exit 7 (new, `blocked_since_enforcement`) when there are, so scripts can alert.
- `illumio-tui enforce <app> --rollback [--record <path>] --dry-run|--yes`
  - Restores each workload's previous enforcement mode and visibility level from the record, in bulk requests grouped by `(mode, visibility)`.

### TUI

- `:readiness`: an overlay listing every app in the current traffic result with its verdict and numbers, sortable (`s`) and searchable (`/`).
  - When the draft decision isn't loaded, it shows `not evaluated` everywhere and offers `D` (the existing confirm).
  - `Enter` on an app opens the enforcement overlay for it.
- `:enforce [app]`, and a key on the app overlay, open the **enforcement overlay** for one app: five steps down the left, the selected step's detail on the right.
  1. **Readiness**: the numbers, the verdict, the warnings.
  2. **Rules**: the gaps. `G` opens the policy generator focused on the app (#33–#35); `w` on a gap opens the rule form for that tuple; `m` marks a gap as accept-blocked (flow mark form).
  3. **Provision**: the app's pending changes. `P` provisions them, through the existing provision path with its preview.
  4. **Enforce**: choose `full` or `selective` and the visibility level, then go through the existing bulk confirm (typed count). Blocked when the verdict isn't `ready`, unless typed through (type `enforce`, as `revert` is typed for reverts).
  5. **Watch**: after a move, the blocked flows since the change time (one query, on `r` only; nothing refreshes by itself). `R` rolls back (preview plus confirm).
- After any write, steps re-evaluate from the refreshed data. Pending reloads after a provision; workloads reload after a bulk move.

### Change record

`~/.local/state/illumio-tui/enforcements/<app>-<UTC timestamp>.json`, written right after a successful (or partially successful) bulk move:

```json
{ "app": "shop/prod", "profile": "default", "pce": "pce.example", "org_id": 5,
  "at": "2026-10-19T10:00:00Z", "to": {"mode": "full", "visibility": null},
  "readiness": { "verdict": "ready", "coverage": 1.0, "window": "7d", "flows": 412 },
  "workloads": [ {"href": "...", "hostname": "web-1",
                  "before": {"mode": "visibility_only", "visibility": "flow_summary"},
                  "result": "updated"} ] }
```

- Rollback reads the newest record for the app, unless `--record` names one.
- It re-reads the workloads first. A workload whose current mode is no longer the one we set has been changed by someone else since: it is **skipped and reported**, not overwritten.
- A record is never deleted by rollback. Rollback writes its own record, with `rollback_of` naming the original.
- Records hold hrefs and hostnames only, never credentials.

## Safety

- Writes only through the guarded write layer: armed (`writes = true` / `ILLUMIO_WRITES=1` plus `:writes on` / `--yes`), previewed, audited (`writes.jsonl`), typed count above the threshold.
- Readiness is advisory but sticky. Moving a not-ready app needs `--force` or the typed `enforce`.
- Nothing refreshes without the user asking. *Watch* is a key or a command, not a timer.
- Requests are counted and shown like every other request.

## Out of scope

- Several apps in one move.
- Scheduling or staged rollouts (by role first, then the rest).
- Enforcement boundaries and deny-rule authoring for `selective`. `selective` is offered as a target, but readiness evaluates allow coverage only and says so.
- Automatic accept-blocked.

## Delivery

Three issues, in this order:

1. **Readiness core**: a `pce::readiness` module (pure, from flows + workloads + pending + policy + marks), CLI `readiness`, and wiremock tests for the request sequence and cost.
2. **TUI readiness and enforcement overlay, steps 1–3**: `:readiness`, `:enforce`, the app overlay key, gaps feeding the generator and rule form, provision of the app's pending changes. Snapshots.
3. **Enforce, watch, rollback**: change records (`pce` or the app's state module), CLI `enforce` with `--watch` and `--rollback`, TUI steps 4–5, exit code 7. A live check on demo100 against a smoke app the test creates or a designated app.

## Testing

- **Unit:** verdict rules; coverage with accepted marks; tuple grouping; pending matched to the app by scope; little-evidence thresholds.
- **wiremock:** the CLI request sequences and counts for `readiness`, `enforce --dry-run`, `enforce --yes` (bulk plus record), `--watch`, and `--rollback` (re-read, skip changed, grouped bulk).
- **TUI:** keys per step; refusal when not ready; the typed `enforce` path; rollback preview; snapshots of both overlays at 120x40 and 80x24.
- **Live, by the controller on demo100:** readiness for real apps (read-only). Enforce and rollback on one designated workload's app only if the app has a single workload. Otherwise via the live suite on `PCE_LIVE_WORKLOAD`, restored.
