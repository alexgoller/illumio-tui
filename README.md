# illumio-tui

Terminal UI and CLI for the Illumio Core PCE, built for administrators,
Illumio Professional Services, and agents. Reads by default; writes for labels, IP lists,
services, unmanaged workloads and policy (rulesets, rules, provisioning) are opt-in (see Writes).

Documentation: https://alexgoller.github.io/illumio-tui/

Screenshots and a short recording: https://alexgoller.github.io/illumio-tui/#screens

## Install

From a clone of this repository (Rust 1.88 or newer):

    cargo install --path crates/illumio-tui

### Releases

Prebuilt binaries are attached to each
[GitHub Release](https://github.com/alexgoller/illumio-tui/releases), one
archive per platform plus a `SHA256SUMS` file:

| Platform | Archive |
|---|---|
| Linux x86_64 (glibc) | `illumio-tui-<version>-x86_64-unknown-linux-gnu.tar.gz` |
| Linux x86_64 (static musl, any distro) | `illumio-tui-<version>-x86_64-unknown-linux-musl.tar.gz` |
| Linux arm64 | `illumio-tui-<version>-aarch64-unknown-linux-gnu.tar.gz` |
| macOS Apple Silicon | `illumio-tui-<version>-aarch64-apple-darwin.tar.gz` |
| macOS Intel | `illumio-tui-<version>-x86_64-apple-darwin.tar.gz` |
| Windows x86_64 | `illumio-tui-<version>-x86_64-pc-windows-msvc.zip` |

Download the archive for your platform and `SHA256SUMS`, then verify and
install (example: macOS Apple Silicon, version 0.1.12):

    sha256sum --check --ignore-missing SHA256SUMS      # macOS: shasum -a 256 -c --ignore-missing SHA256SUMS
    tar -xzf illumio-tui-0.1.12-aarch64-apple-darwin.tar.gz
    install -m 755 illumio-tui-0.1.12-aarch64-apple-darwin/illumio-tui ~/.local/bin/   # any directory on PATH
    illumio-tui --version

macOS may quarantine binaries downloaded with a browser; clear it with
`xattr -d com.apple.quarantine ~/.local/bin/illumio-tui`. On Windows, check
the hash with `Get-FileHash illumio-tui-<version>-x86_64-pc-windows-msvc.zip`
(compare with the line in `SHA256SUMS`), extract the zip, and move
`illumio-tui.exe` into a folder listed in your `PATH`.

### Cutting a release (maintainers)

1. Bump `version` in `[workspace.package]` of the root `Cargo.toml` and run
   `cargo check` so `Cargo.lock` follows.
2. Commit, then tag and push: `git tag v0.1.12 && git push --tags`.

The `Release` workflow builds all targets and publishes the release with
generated notes; it fails if the tag does not match the Cargo version. Run it
manually (`workflow_dispatch`) for a dry run that only uploads artifacts.

## Configure

Environment variables (same names as illumio-mcp-server):

    PCE_HOST=pce.example  PCE_PORT=443  PCE_ORG_ID=1  API_KEY=...  API_SECRET=...

or a profile in `~/.config/illumio-tui/config.toml`:

    default_profile = "prod"

    [profiles.prod]
    host = "pce.example"
    org_id = 1
    api_key = "api_..."
    api_secret_cmd = "op read op://vault/pce/secret"
    writes = true    # optional: allow writes (the CLI still needs --yes, the TUI :writes on)

    [profiles.prod.limits]
    traffic_max_results = 10000     # the starting traffic row cap (default 10000, at most 200000)
    traffic_default_window = "24h"
    traffic_auto_backfill = false   # true: re-run a truncated result below 10000 at 10000 (see TUI)

`PCE_TLS_VERIFY=false` skips certificate checks like `--insecure`; `ILLUMIO_PROFILE` picks
the profile and `ILLUMIO_TUI_THEME` the TUI theme. `--env-file .env` loads a dotenv file for one run. Connection settings can
also be overridden per run with global flags: `--host`, `--pce-port`,
`--org-id`, `--profile`, `--config`, `--insecure`, `--no-cache` (the
connection-port flag is `--pce-port`, not `--port`, because `traffic query`
has its own `--port` service-port filter).

## TUI

    illumio-tui                      # opens the interactive UI on the default profile
    illumio-tui --profile prod
    illumio-tui --theme light        # dark is the default

Five screens: `1` Traffic, `2` Policy, `3` Objects, `4` Workloads, `5` Events. The `:` prompt
takes `:traffic`, `:policy`, `:objects`, `:workloads`, `:events`, `:iplists` (Objects on its
IP lists tab), `:filter <expr>`, `:window <dur>`, `:profile <name>`, `:writes [on|off]`,
`:export <path>`, `:draft`, `:active`, `:help` and `:q`. `:export <path>` writes the table on
screen as CSV (the rows shown, with the search and filters applied and in sort order; every
column, whatever the terminal width, each cell whole; `~/` is the home directory), the same
format as the CLI's `--csv`, and sends nothing; it refuses an existing file, `:export! <path>`
overwrites it. Cells match the CLI's for the same data (Workloads: every label, ignored keys
too, and `heartbeat_age_secs`). Sectioned views (App-to-app `in`/`out`/`both`, Blocked) export
every peer, folded sections included, in the columns of `traffic groups --by ... --csv`; the
Treemap has no table.
`?` shows the full key map, `Esc` closes overlays, `q` or `Ctrl-C` quits. The mouse wheel
scrolls the current table and a click on a tab switches screen; on a terminal narrower than
the full tab row (69 columns) the tabs read `Traf`, `Pol`, `Obj`, `Wl`, `Ev`.

- **Traffic**: `f` edit the filter, `r` run (bypasses the cache), `a` auto-refresh, `s` sort,
  `/` search; a filter that would be expensive asks for confirmation first. In the filter
  editor `Ctrl-N` / `Ctrl-P` complete like a form: labels, `ipl:` IP lists and the grammar's
  `ip:` `ipl:` `wl:` `fqdn:` in src and dst (`wl:` hostnames only once the Workloads screen
  has loaded them), protocols and services as `port/proto`, the decision and transmission
  words and the window steps, never with a request. The editor's `transmission` field
  (`unicast`, `broadcast`, `multicast`, comma list; `all` by default) and `:unicast` (unicast
  only; again: every kind, one query through the same cost guard) narrow the query by how
  the flows were sent; the filter bar's title shows it (`window 24h · cap 10000 · unicast`) while it is narrowed. `+` / `-` step
  the window along `1h 6h 12h 1d 2d 3d 1w 2w 4w 90d`: `+` re-runs the wider window as one
  query (asking first when that is expensive), `-` drops the flows last seen before the new
  start without a request. Explorer returns the stored aggregate records whose lifetime
  overlaps the window, each with its own lifetime counters, so the rows of two windows are
  never added up: a record seen twice (its flow plus `first_detected`) is one row, the later
  version winning. A result is truncated when the PCE's `flows_count` reaches the cap (per
  region on a supercluster); it then covers an unknown part of its window, so the Result
  header claims no time span and says `10000 flows, capped at 10000; B raises the cap`.
  Explorer cannot page a result, so `B` re-runs the same window at a larger cap, one query
  whose answer replaces the truncated one: 10000 from below it (without asking), then
  `25000 50000 100000 200000`, one rung per `B`, each above 10000 asked first (`cap 25000
  exceeds 10000. Re-run the window at cap 25000 (1 query, replaces the 10000 flows capped at
  10000)? y/n`), even when the last result proved there is more. A region that did not
  respond marks the result `incomplete` in the header and the status line.
  The profile setting `traffic_auto_backfill = true` (or `ILLUMIO_AUTO_BACKFILL=1`) re-runs a
  truncated result below 10000 at 10000 right away, without `B`; one query per truncated
  result, never chained, and never above 10000 (`B` asks as usual there). It is off by
  default because every request must be user-caused: setting it is the user saying yes in
  advance, once, in writing. `]` / `[` step the result cap
  (`500 1000 2000 5000 10000 25000 50000 100000 200000`; the profile's `traffic_max_results`
  is where it starts and a rung of its own; above 10000 asks first, and auto-refresh stays off
  for such a cap) and re-run the query. `v` cycles five
  views of the one result, all grouped by the same label keys: the profile's scope keys
  (`app,env` by default) until `g` opens the picker (`Space` toggles a key, `all labels` takes
  every key the flows carry, typing edits the list as text, `Enter` applies, for this session).
  **Flows** shows each side's group (`shop/prod`; `-` for an IP list or FQDN, whose name is in
  the name column) and, on `Enter`, a detail with both label sets and the bytes. **App-to-app**
  pairs the groups; `o` splits each pair by service (`pairs · by service`: one row per source
  group, destination group and `port/proto`, the tuple an allow rule is written for; `Enter`
  drills into that service's flows, `w` drafts that rule: consumers the source group's labels,
  providers the destination group's, the row's service; a `-` slot is no label, so that side
  is only as narrow as the labels it has; `:export` writes the columns of
  `traffic groups --services --csv`); `x` / `X` switch it to sections per app (`in`: each destination app with
  its sources, one row per source group and service; `out`: each source app with its
  destinations; `both`: inbound rows `←` then outbound `→`), `d` cycles the decision filter
  (`all`, `blocked` with potentially blocked, `blocked only`) of the pairs, by service too, and
  the sections (`pairs · by service · blocked` lists just the tuples still blocked, the ones an
  allow rule is missing for; `Enter` drills into the flows the row counts; the Treemap keeps
  every flow), `Space` / `z` fold a section or
  all, `/` keeps whole sections, and `Enter` drills into a pair, a peer row or a whole app.
  **Mesh** lists user, process, source, destination, service and destination process like the
  Illumio UI's mesh (`u` / `p` hide the user / process column and merge rows, `Enter` drills).
  **Blocked** is App-to-app with `in` and `blocked`: each destination app with the sources
  and services blocked or potentially blocked towards it. **Treemap** draws each source as
  a box and its destinations as tiles sized by connections (`b` bytes) and coloured by
  decision: `h` / `j` / `k` / `l` move, `Enter` zooms into a source and then drills into a
  destination, `Esc` zooms out. None of this sends a request. With writes on, `w` turns a
  blocked flow (or the first one behind a peer row or a Mesh row) into a draft allow rule.
  `F` focuses all five views on one app, the row's source (on `in` sections and Blocked, its
  destination; `F` again takes the other side; on a row without an app, or with `:focus`
  alone, a picker of the loaded apps; `:focus shop/prod` names it): only that app's traffic is
  shown, `X` cycles `in` (it is the destination), `out` (the source) and `both`, the Result
  header reads `focus shop/prod · in`, and `d`, `o`, search, sort and drills work on top.
  `Esc` clears it; changing the keys with `g` clears it too. Setting it sends nothing. When the
  result was truncated, the header says `focus is partial: r re-queries shop/prod` and `r`
  runs the query again with the app's labels on its side (two queries for `both`, merged
  without duplicates, through the same cost guard); `Esc` then puts the unfocused result back
  without a request. While that focused result is shown, `B` and `]` / `[` re-run the focused
  query at the next or previous cap (one query per side, asking above 10000); `+` and `-` go
  back to the unfocused result first.
  `A` (or `:app shop/prod`) opens the app overlay for the row's app, at the focus direction
  (`X` cycles it there too): its workloads with their enforcement modes and VEN health
  (`Workloads 14   enforced 9 · selective 2 · visibility 3 · unmanaged 1`, `VENs 13 active · 1 stale`;
  a workload without a VEN counts as unmanaged, not visibility; active
  is healthy or warning, stale is offline or error), its inbound and outbound peers by service
  and decision (flows count flow records; conns and bytes are those records' lifetime counts),
  and the rulesets whose scope covers it (a global scope shows as `all`) or whose rules name
  its labels (draft or active, as on the Policy screen; its title says which). The traffic
  panels come from the loaded result; the workloads and the rulesets come from their screens
  when those are loaded, else one request each (cached), counted in the overlay's header.
  Reopening the same app sends nothing (after `:draft` / `:active` its policy panel loads
  again). `:app` also opens an app that has labels but no flows in the window. `Tab` moves
  between panels; `Enter` on a peer focuses that pair in Flows, on a workload opens its detail,
  on a ruleset opens it on the Policy screen; `w` drafts an allow rule from a blocked peer
  (writes on; the form replaces the overlay, and cancelling it or the write done comes back
  to the overlay without a request); `r` reloads the panels
  after asking; `Esc` returns.
  `G` in the app overlay (or `:gen shop/prod`, or `:gen` and the picker) opens the **policy
  generator** for that app: the allow rules its observed traffic needs, written in one
  confirmed step. One row per peer and side: intra-app, each app peer (`web/prod`), IP peer and
  unlabelled workload, with the services seen and the reported decisions (`✓` allowed, `✗`
  blocked or potentially blocked; with `D` loaded, `would allow N`). Intra-app and same-env
  peers start checked, the rest unchecked with the reason (`⚠ env mismatch`, `⚠ uses IP list
  Corp`, `⚠ needs an IP list`, which checking plans). The slider `app·all ─ app·ports ─
  role·ports ─ role·all` (`←`/`→`) sets how coarse the rules are: all workloads of the app or
  one rule per role pair, All Services or the services seen; `<`/`>` give a row its own stop
  (`*`), `=` drops it. `o` adds the outbound section: rules to other apps go into the peers'
  rulesets (a new `RF-crm-prod` when none covers the peer), to IP peers into the app's own.
  The target line names the app's ruleset (`R` picks when several cover it, or a new one by
  `generator.ruleset_name`, default `RF-{app}-{env}`). Rules already in the draft show as
  `exists` and are not written; an existing rule with the same actors gets the missing
  services. A row whose rule an override deny rule blocks anyway shows `⚠ override deny
  blocks N`, one whose ruleset cannot be named `⚠ no target`. The footer counts the rules and
  the flow records they cover, computed locally; `p` lists every rule and what is not
  covered, and why. Opening it costs at most the draft rulesets (one request,
  cached, counted in its header). On a truncated or incomplete result it shows the rows but
  refuses to plan (`incomplete: r re-queries shop/prod`); `r` runs the focused query through
  the usual cost guard. With writes on, `Enter` reads the draft rulesets fresh (dedupe again),
  then shows every request in one confirm (`6 requests: 1 ip_list, 1 rule_set, 4 rules ·
  planned with 2 reads`); `y` sends them in order and stops at the first failure (`wrote 5 of
  7 (pending); step 6 failed: …`). The policy view and the draft decisions are reloaded once
  per batch; `:batch` shows each step again; provision with `P` in the pending overlay as
  usual. A still-truncated focused result says `still capped at N: B raises the cap, or narrow
  the window`. `Esc` returns (to the app overlay when opened from it).
  `D` shows, next to each flow's reported decision, the one the **draft** policy would make,
  so you can check whether your draft rules fix the blocked flows before provisioning. It is
  the PCE's own Explorer draft view: the PCE re-evaluates the result's own async query
  (`PUT …/update_rules` on that query, then status polls and one download; no new POST) and
  **changes no policy**. The PUT is on the query resource illumio-tui created, so it needs no
  armed writes and is not in the write audit. Above 10000 rows or a 7 d window it asks first;
  a focused result is re-evaluated per side. When the query is no longer known (the PCE
  dropped it), `D` asks, then runs the query again with the draft (one POST, one PUT, polls and
  one download per query). While it runs, `Esc` cancels it; a new query, a focus change, a
  window or cap step or `:profile` cancels it too. With draft decisions loaded the Result
  header says `draft ✓ N fixed`; Flows gains a `draft` column (`blocked → allowed`, the row in
  its draft colour; below 120 columns `→ allow`); App-to-app pairs and sections a `draft`
  column (`2 would allow`, `1 would block`) and the section headers `draft allows N`; the app
  overlay `+N` / `-N` per peer and `draft: N blocked flows fixed`; Mesh a `draft` column too; and
  `d` gains a `fixed by draft` step (blocked or potentially blocked now, allowed in draft). `D` again goes back to the
  reported decisions with no request, and the next `D` answers from the draft cache (kept 2
  minutes, its age in the status line). A successful write drops the draft decisions (a
  rule, ruleset, label group, IP list, service, workload, provision or revert write also drops
  the cached draft; a label write leaves it) and
  runs `D`'s evaluation once more on its own, when draft decisions were on the result on
  screen or running, or the rule came from the `w` form: one PUT per query, or none when the
  cached draft still stands; `… · draft re-evaluation running · Esc cancels` (`Esc on
  Traffic cancels` from another screen or the app overlay). A draft run still on its way
  when a write lands is cancelled and its answer is not cached. One run per write, and none
  after a failed, unanswered or cancelled write, after the ruleset `Ctrl-R` creates first
  (the draft stays on screen until its rule is written), while a query runs, or where `D`
  would ask first (no query of this session behind the result, a large window or cap); if
  draft decisions were dropped, the status then says `draft cleared: D re-evaluates`.
- **Policy**: `d` toggle draft / active, `p` pending changes (`Enter` on one shows its diff against active), `Tab` switch panes, `Enter` rule
  detail, or on a ruleset load its deny rules. Rules are listed in evaluation order: override
  deny, allow, deny. With writes on, `n` / `e` / `x` create, edit and delete rulesets and rules
  in draft (`N` a deny rule), and the pending overlay marks with `Space`, provisions with `P`
  and reverts the marked changes with `R`.
  `i` reads the workload list once (unless the Workloads screen already did) for rule impact:
  the rule detail says how many workloads the rule matches (`impact      12 providers · 3
  consumers`, `≥` when the list was truncated) and the size of each IP list it names, the
  rules pane's `P/C` column fills in (`1.2K/5.7K` from 1000 on), a note names what was not
  counted (virtual services, a side resolved as `virtual_services`), and a rule's confirm
  overlay shows the same for the rule about to be written. Impact uses the ruleset's scopes, unscoped consumers, label groups (with
  their sub-groups), exclusions, `all` and workload actors; IP lists are sized, never expanded.
- **Objects**: four tabs, labels, label groups, services and IP lists; `Tab` / `Shift-Tab`
  next / previous tab, `Enter` detail. The IP lists tab shows name, size (addresses after
  exclusions: `65.5K`, `16.8M` for a /8; `any` only for an included `0.0.0.0/0` or `::/0`;
  `5 FQDNs` for a list of names only), ranges, FQDNs, `used by` (rules of the loaded draft
  rulesets that name the list; `-` until Policy is loaded) and pending; `Enter` shows the size,
  those rules, then the first ranges and FQDNs. With writes on, `n` / `e` / `x` create, edit
  and delete labels, services and IP lists. On the Labels tab `f` picks the keys shown (the
  `g` picker, with label counts), `/=pay` searches values only and `/app=pay` the values of
  one key, `i` shows or hides the profile's `ignore_keys` (hidden by default, counted in the
  header: `key app,env · 212 of 4310 · 38 hidden`), and `u` keeps the labels nothing holds,
  as the PCE's own usage flags say: one `labels?usage=true` request (cached like the labels),
  so pairing profiles, permissions, enforcement boundaries, container workload profiles,
  virtual services and the rest count, not just workloads and rules. Once the flags are in,
  the detail pane (and a `held by` column from 120 columns) names what holds each label. The
  `workloads` and `rules` columns sort and show `-` until Workloads and Policy are loaded
  (counted in the policy scope the Policy screen has loaded: draft by default, active once
  `d` toggles it). A PCE that answers without usage flags falls back to those counts: `u`
  then asks before loading Workloads and Policy, the header says `unused (workloads, rules
  and scopes only)`, a deny rule the PCE serves through the per-ruleset fallback endpoint
  counts once that ruleset's deny rules have been loaded, and a truncated workload list is
  named in the header (`unused (workloads truncated)`).
- **Workloads**: VEN health columns and a summary line, `h` unhealthy only, `H` sort by
  severity, `t` / `T` jump to Traffic with the workload as source / destination. The
  enforcement column is the effective one: a workload without a VEN is `unmanaged`, as in
  the PCE UI, whatever mode the API reports for it (often `visibility_only`); the detail pane's
  health line shows both (`healthy · full`, `unmanaged · PCE reports visibility_only`).
  Local filters, no request, combined with `h` and `/`: `E` cycles the enforcement (all,
  full, selective, visibility_only, idle, unmanaged), `m` managed / unmanaged, `o` offline
  only (the VEN health: `online` false or no heartbeat for 15 minutes), `u` unlabeled (no
  label for one of the profile's scope keys or its tier key; a `missing` column names them),
  `c` clears them and `h` (not the `/` search). A filter line above the table names what is
  on and the count (`enforcement selective · unlabeled (app, env, role) · 1 of 206`), and
  says the counts are partial when the list was truncated.
- **Events**: severity-coloured audit events, `f` edit severity / type / window, `S` sort by
  severity, `Enter` the event as JSON (`j` / `k` scroll a long one).

Auto-refresh (`a`) is opt-in per screen, has a 60 s minimum, shows a countdown in the panel
title, and switches itself off after a failed refresh.

`:profile <name>` reconnects to another profile from the config file. It runs that profile's
`api_secret_cmd`, so the command must be non-interactive (no prompts): the TUI owns the
terminal. Exported `PCE_HOST`, `PCE_PORT`, `PCE_ORG_ID`, `API_KEY` and `API_SECRET` still
outrank the target profile on a switch, as they do at startup; the header shows the host
actually in use. `:writes on` stays on across a switch, and the new profile's own `writes`
decides whether it may write.

Startup makes exactly five requests (health plus the four object lists). The first visit to
Workloads and to Events costs one request each; going back to them costs none. The IP lists
tab costs none within the cache TTL (15 minutes): the startup load already read them. Rule impact
costs one workloads read on `i`, none when the list is cached. `u` on the Labels tab costs one
labels read with usage flags, none when it is cached; while `u` is on, `r` on that tab and each
write re-read the flags once. Everything else is caused by a key you press. The header shows
the API call and byte counters at all times.

## CLI

    illumio-tui health
    illumio-tui labels --key app
    illumio-tui labels --key app --key env --value-contains pay --ignore-keys
    illumio-tui labels --unused --json
    illumio-tui workloads --label app=web --online true
    illumio-tui workloads --unlabeled --json
    illumio-tui workloads --offline --enforcement full
    illumio-tui policy rulesets
    illumio-tui policy rules "Web-DB"
    illumio-tui policy rules "Web-DB" --impact --json
    illumio-tui policy rules impact /orgs/1/sec_policy/draft/rule_sets/7/sec_rules/3
    illumio-tui iplists --table
    illumio-tui policy pending
    illumio-tui policy pending --diff --table
    illumio-tui events --since 1h --severity err
    illumio-tui traffic query --window 24h --src app=web --dst app=db --json
    illumio-tui traffic query --window 1h --group app,env
    illumio-tui traffic query --window 1h --blocked
    illumio-tui traffic groups --window 1h --keys app,env,role
    illumio-tui traffic groups --window 1h --services --csv > tuples.csv
    illumio-tui traffic groups --window 1h --services --show blocked --csv > still-blocked.csv
    illumio-tui traffic groups --window 1h --by in --show blocked --app pos/PCI
    illumio-tui traffic groups --window 1h --app shop/prod --direction in
    illumio-tui app shop/prod --window 24h --json
    illumio-tui app shop/prod --direction in --csv --section inbound > inbound.csv
    illumio-tui gen shop/prod --window 24h --json
    illumio-tui gen shop/prod --granularity role-ports --outbound --table
    illumio-tui traffic mesh --window 1h --no-user --json
    illumio-tui traffic query --window 30d --dry-run
    illumio-tui traffic query --window 24h --backfill --json
    illumio-tui workloads --csv > workloads.csv
    illumio-tui traffic groups --window 24h --by in --csv > peers.csv
    illumio-tui profile list
    illumio-tui profile use prod

JSON output (default when piped) is `{"data": ..., "meta": {...}}` where
`meta` reports API calls, bytes (decompressed: downloads are gzip-compressed on the wire, so
the transfer is smaller; the TUI header counts the same way), cache status, truncation, and the query
window; traffic commands add `meta.covered`, the window's length when the result is complete
(`"1d"`) and `null` when it is truncated (it covers an unknown part of the window),
`meta.cap` (the cap the result was fetched at) and `meta.incomplete` (a supercluster region
did not respond). `meta.truncated` is `flows_count >= cap` from the PCE's own status, per
region when there are regions; the status's `matches_count` is a bounded raw count, not a
total, and is never reported. With `--backfill` (`query`, `groups` and `mesh`) a truncated
result's window is re-run at a larger cap as the TUI's `B` does, each answer replacing the
last, until one is complete: 10000 from below it, then 25000, 50000, 100000 and 200000 only
with `--yes`; without it the run stops at 10000 and stderr names the next cap.
`meta.cap_steps` lists every cap tried (`[200, 10000]`) and `meta.api_calls` every request
(at least 3 per query). `--max-slices` was removed with time slices and exits 2.
Errors go to stderr as JSON. Exit codes: 1 generic, 2 arguments or
config, 3 auth, 4 PCE error, 5 rate limited, 6 write refused by the guard.

`--csv` (beside `--table` and `--json`, exclusive with both) prints the same table as RFC 4180
CSV for spreadsheets: a header row, `,` separators, quotes only around cells holding `,`, `"`
or a line break, CRLF line ends, UTF-8 without BOM. The columns are the full set, untrimmed:
no `…` clipping, lists joined with `; ` where the table folds them (`3 hosts`, a label
group's member count), plain numbers (`6472`, not `6.5K`), `true`/`false`, and an empty cell
for nothing (`workloads` gives `heartbeat_age_secs` and every label, ignored keys too;
`events` every resource change; `policy rules` adds each rule's `href`; `policy pending --diff`
a `diff` column, one diff line per line of the cell). `traffic groups --by ...` and
`traffic query --blocked` print one row per peer with `app`, `direction`, the peer and app host
counts and their names. The meta line always goes to
stderr, so a redirect holds only CSV. `--csv` sends the same requests as the table;
`workloads`, `labels`, `label-groups`, `services`, `iplists`, `events`, `policy rulesets`,
`policy rules`, `policy pending`, `traffic query`/`groups`/`mesh` and `profile list` take it.

`workloads` shows the effective enforcement: `unmanaged` for a workload without a VEN, which
the PCE reports with a mode of its own (often `visibility_only`), else the PCE's mode. JSON keeps the PCE's value in
`enforcement_mode` and adds the effective one as `enforcement`; the table and CSV
`enforcement` column is the effective one, and CSV appends `enforcement_mode`. `--enforcement` filters by it in the same request:
`unmanaged` sends `managed=false`, a mode (`full`, `selective`, `visibility_only`, `idle`) adds
`managed=true`, so `visibility_only` lists only workloads with a VEN; a `--managed` that
contradicts it is exit 2. `--offline` and `--unlabeled [keys]` filter the one read locally,
as the app's `o` and `u` do: offline is the VEN health (so a stale heartbeat counts, which
`--online false` would miss), unlabeled is a missing label for one of the keys
(comma-separated, matched exactly; alone, the profile's scope keys and tier key; a key that
is neither in the label model nor on any workload is warned about on stderr and in
`meta.warnings`). JSON adds `missing_labels`
with `--unlabeled` (the table a `missing` column), and `meta.local_filter` names the local
filters (`["offline", "unlabeled (app, env, role)"]`; the meta line `filtered locally: …`). CSV joins
the `missing` keys with `; `, as its other lists.
Commands that print no table (writes, `health`, `policy rules impact`, `profile use`, a
traffic `--dry-run`) refuse it with exit 2 before any request.

`traffic groups` and `traffic mesh` take the `traffic query` filter flags (and its dry run,
cost guard and cache) and print the same envelope. `groups` gives one row per source and
destination group (`src`, `dst`, `flows`, `connections`, `bytes`, `allowed`,
`potentially_blocked`, `blocked`, `allowed_conns`, `pot_blocked_conns`, `blocked_conns`,
`unknown_conns`;
`--services` splits each pair by service and adds a
`service` column and field, `5432/tcp` or `icmp`), or with `--by in|out|both` sections per app: a header row
with the app's totals, then its peer groups and services indented (JSON
`{"sections": [{app, direction, rows, totals}]}`); `--show blocked|blocked-only` counts only
those decisions among the loaded flows, in the pairs (with `--services` too) or the sections
(`--decision` filters the query itself); sections are already per service, so `--services`
with `--by` is exit 2. `--app shop/prod` (on `traffic query`, `groups` and `mesh`) focuses the
query on one app, a group of the keys the command groups by (`--keys` on `groups` and `mesh`,
`--group` on `query`, else the profile's scope keys): its labels go
into the query as destination includes (`--direction in`), source includes (`out`) or both
(`both`, the default: two queries, merged without duplicates), so the cap is spent on that app;
`meta.queries` counts them and a dry run prints one body per side. With `--by in|out|both` the
direction is the sections' (`--direction` is then exit 2) and only the app's sections are kept;
a name-keyed group such as `unmanaged-nas/-` keeps narrowing the sections of an unfocused query.
An app whose labels do not exist is exit 2 with the closest label values, before any traffic
request (the labels are fetched, cached). `--backfill` does not combine with `--app`: raise `--max-results` instead.

**Units.** `flows`, `allowed`, `pot.blocked` and `blocked` count flow records. `conns`
(`connections`), `bytes` and the per-decision `allowed_conns`, `pot_blocked_conns`,
`blocked_conns` and `unknown_conns` (no decision reported; the four sum to `connections`)
are the **lifetime** counts of those records, as the PCE UI shows them:
Explorer returns every aggregate record whose lifetime overlaps the window, each with its whole
lifetime's counters, not counts for the window alone. Tools that report connections per flow
summary (workloader) therefore match `conns`, not `flows`. Every CLI traffic table says so in
the line above it, and the TUI's traffic tables add `conns: lifetime` to their title where it
fits; JSON and CSV carry the per-decision connections for pairs, `--services` and
sections (sections: as the last four CSV columns).

**Draft decision.** `--draft` (on `traffic query`/`groups`/`mesh` and `app`) also asks for
each flow's decision under the draft policy, the PCE's own Explorer draft view: after the
query completes, one `PUT <query>/update_rules?label_based_rules=false&offset=0&limit=<cap>`
(empty body) on that query, status polls until its `rules` are completed, then the one download
(with `--app`, per side; with `--backfill`, only the result kept is re-evaluated). It changes no
policy: the PUT re-evaluates the query resource illumio-tui created, so it needs no armed writes
and is not in the write audit. Every flow then carries `draft_policy_decision` (`allowed`,
`potentially_blocked`, `blocked`, or what the PCE sends, such as `allowed_across_boundary`,
`blocked_by_boundary` or `blocked_by_override_deny`), `meta.draft` is `{"evaluated": true,
"fixed": N, "newly_blocked": M}` (flow records: blocked or potentially blocked now and allowed
in draft; allowed now and not in draft), pairs, sections, mesh rows and app peers carry `draft_allowed` and
`draft_blocked` (flow records), the tables a `draft` column and the CSVs `draft_allowed` and
`draft_blocked`. A PCE that refuses `update_rules` is a clear error. Each polling stage, the query and the draft
re-evaluation, gives up after 30 minutes.

**Transmission.** `--transmission unicast|broadcast|multicast` (repeatable or comma-separated,
on `traffic query`/`groups`/`mesh` and `app`) keeps the flows sent that way. Explorer can only
exclude a transmission, on the destination side, so the other kinds go into the query as
`destinations.exclude` entries (`{"transmission": "broadcast"}`, `{"transmission":
"multicast"}` for `--transmission unicast`); a `--dry-run` shows them, and nothing is filtered
locally. The default stays every kind, broadcast and multicast (mDNS, SSDP, NetBIOS, ...)
included, as in the PCE UI. workloader excludes broadcast and multicast by default, so to
compare counts with it add `--transmission unicast` (`:unicast` in the TUI).

`illumio-tui app <app> [--direction in|out|both] [--window W] [--keys K] [--active]` prints one
envelope for one app: `workloads` (`summary` with the enforcement breakdown, `unmanaged` for
workloads without a VEN, and the VEN counts, `rows` with name, role, `enforcement` (effective),
`enforcement_mode` (as the PCE reports it) and VEN health, `truncated` when the read hit its
cap), `inbound` and `outbound` (one row per peer group, service and decision), `policy`
(`scope` draft or active, `truncated`, and `rows`: rulesets by `reason`, `scope`,
`all_workloads` or `rules`, with their rule counts). There is no timeline: Explorer returns
aggregate records with lifetime counters, so no honest count over time can be built from them
(the `--timeline` flag, the `timeline` section and the overlay's `t` were removed in 0.1.9).
It costs the focused traffic
queries (one per side), one workloads read filtered by the app's labels and one rulesets read,
all cached; `meta` counts them, and `meta.truncated` is the traffic's. `--section
workloads|inbound|outbound|policy` keeps one section (JSON: `app`, `direction` and
it); `--table` prints each section under a title line; `--csv` prints one and needs
`--section` (without it, exit 2 before any request).

`illumio-tui gen <app> [--granularity app-all|app-ports|role-ports|role-all] [--outbound]
[--window W] [--max-results N] [--yes]` proposes the allow rules the app's observed traffic
needs, as the TUI's generator would plan them, and writes nothing. `rows` holds one row per
peer and side (intra-app, each app peer, IP peer and unlabelled workload) with its default
checkbox (`checked`: intra-app and same-env app peers), the services seen, the reported
decisions and its warnings (`env mismatch`, `uses IP list Corp`, `needs an IP list`,
`ephemeral port?`, ...); `rules` the merged rules at the chosen granularity, each with its
`ruleset`, `action` (`create`, `add_services` or `exists`, against the draft rulesets),
`providers`, `consumers`, `unscoped_consumers`, `services`, `rows` and the exact PCE `body`
(`{step:N}` stands for an IP list or ruleset the batch would create first); `ip_lists` the
IP lists it would create (`shop-prod peer 10.4.1.9`); `preview` the counts and the flow
records covered or not, by reason (`unchecked`, `folded_ips`, `unsupported_proto`,
`broadcast_multicast`, `no_target` when the ruleset is ambiguous or cannot be named,
`override_deny` when an override deny rule of any enabled draft ruleset whose scope holds them blocks them anyway; a deny it cannot evaluate exactly counts), and `blocker`
when rules cannot be written as shown. Rules to create that differ in one side only merge
when the union means exactly both (`role=web` and `role=db` providers become one rule). A
disabled ruleset is never a target; in a ruleset scoped wider than the app the rules name
the app's labels instead of All Workloads. tcp and udp services are inline `port/proto` (consecutive
seen ports fold into a range), icmp uses the ICMP service, other protocols the one service of
exactly that protocol, else they are not covered. It costs the focused traffic query (`in`,
plus `out` with `--outbound`), the name index and one draft ruleset read, cached; `meta`
counts them. A capped result exits 2 and an incomplete one (a region did not answer) exits 4,
with no rules printed. `--table` and `--csv` print the rules. A new ruleset is named by the
profile's `[profiles.<name>.generator] ruleset_name` (below).

`traffic query --blocked` prints the `--by in --show blocked`
sections the same way. `mesh` gives one row per source user, source process, source group,
destination group, port, protocol and destination process. The PCE reports one process per
flow, seen where the flow was reported: an inbound flow's process is the destination's,
anything else the source's. The user, like the process, is the one the reporting VEN saw:
shown as the source's user on outbound flows and left empty on inbound ones. `--keys` picks
the label keys (default: the profile's scope keys), `--no-user` / `--no-process` merge rows
across users or processes. An endpoint that is not a labelled workload is grouped under its
name or `ipl:<list>`. `bytes` is `0` in JSON and `-` in tables when the PCE reported none.

`labels` flags combine with AND: `--key` (repeatable, any of the keys) and `--value` match
exactly, `--value-contains` is a case-insensitive substring, `--ignore-keys` hides the
profile's `ignore_keys` (`--no-ignore-keys`, the default, keeps every key). `--unused` keeps
the labels nothing holds, as the PCE's own usage flags say: the one labels request asks for
`usage=true`, so a label held only by a pairing profile, a permission, an enforcement
boundary or a container workload profile is in use, and each row carries the PCE's `usage`
object (every flag false); `meta.usage` is `pce`. In this mode nothing is counted: the rows'
`workloads` and `rules` are 0 because the PCE's `workload` and `ruleset` flags are false, not
from reading workloads or rulesets, so the command stays at one request. A PCE
that answers without usage flags falls back to our own counts: it reads the workloads, the
draft rulesets and the label groups (through the cache, counted in `meta`), deny rules count
when the ruleset carries them inline (no fallback reads), and `meta.usage` (and the table's
meta line) says `usage from workloads, rules and scopes only`. Without `--unused` the rows
are unchanged.

`--impact` on `policy rules <ruleset>` adds each rule's `impact` (JSON) or a `P/C` column
(table): `providers` and `consumers` (workloads matched, unions over the ruleset's scopes),
`per_scope`, `ip_lists` (name, href, `size`, side), `unsupported` (what was not counted:
virtual services, a side resolved as `virtual_services` only, a label group unknown to the
index) and `truncated`. `policy rules impact <rule-href>` prints one rule the same way (exit
2 for an href of the wrong shape, 4 when the rule is not found). It costs one workloads read,
through the cache; `meta.truncated` includes it. `iplists` gives every list a `size`: `v4` and
`v6` address counts as strings, `unbounded` (the included `0.0.0.0/0` or `::/0`, else `null`),
`ranges` and `fqdns`.

## Writes

Writes are off unless the profile says yes (`writes = true`, or
`ILLUMIO_WRITES=1` in the environment); a profile without it is exit 6
(`write_refused`), even for a dry run. Every write command prints the exact
request with `--dry-run` and needs `--yes` to send it (the two flags
conflict; with neither, it is exit 2 and nothing is sent). `--allow-writes`
is still accepted but no longer needed; in the CLI it changes nothing (the TUI
starts with writes armed when it is given). A dry run's `data` is
`{method, path, body, summary}`: the method, the API path, the JSON body that
would be sent (absent for a delete) and a one-line summary. Each sent request
is appended to `~/.local/state/illumio-tui/writes.jsonl` (or
`$XDG_STATE_HOME/illumio-tui/`), mode 0600, with the body sent and the status
received; with neither `$XDG_STATE_HOME` nor `$HOME` set, writes are refused.
Nothing is ever retried.
IP lists and services are written to draft and wait for provisioning
(`"pending": true`); labels and workloads take effect at once.

    illumio-tui labels add app=web --yes
    illumio-tui labels add app=web --if-missing --yes     # no-op when it exists
    illumio-tui iplists add Corp --range 10.0.0.0/8 --fqdn '*.corp.example' --yes
    illumio-tui iplists update Corp --add-range 10.2.0.0/16 --dry-run
    illumio-tui services add PostgreSQL --port 5432/tcp --yes
    illumio-tui workloads add printer-1 --ip 10.0.0.50/24 --label app=print --yes
    illumio-tui workloads update printer-1 --add-label env=lab --yes
    illumio-tui workloads delete printer-1 --yes

`--port` takes `P/PROTO`, `P-Q/PROTO` (`443/tcp`, `8000-8100/udp`, `53/17`)
or a protocol name (`tcp`, `udp`, `icmp`, `icmpv6`); a bare number such as
`22` is refused as ambiguous.

Names resolve by exact match against the one list they name (draft objects
pending deletion excluded); an ambiguous or unknown name is exit code 2 with
the candidates (or up to five similar names) listed, before any request. An
update with no change flags is exit code 2 as well. Update commands re-fetch
the object and send only the keys that change; a changed list (`ip_ranges`,
`fqdns`, `service_ports`, workload `labels` or `interfaces`) is sent whole,
with existing entries carried through, so `--remove-range` and friends must
match an existing entry. A service update that changes ports refuses (exit 2)
services carrying fields this tool cannot preserve: `windows_services`,
`icmp_type`/`icmp_code`, or a port without a protocol. Only unmanaged workloads can be written: one that the PCE
reports as managed, or that has a VEN, is refused locally, and a hostname or
name matching more than one workload is refused ("use the href").

A sent write returns `data` with `href`, `pending`, `status`, `object`,
`dropped_fields` and `warnings`, and `meta.write` with the method, path,
status and audit location. `warnings` is how a write the PCE accepted but
whose re-fetch failed, a POST answered without an href, or an audit append
failure is reported: still success, never a retry. A timeout, lost
connection, gateway status (502/503/504) or undecodable 2xx body is
`write_unanswered` (exit 4, "write may have applied; check the PCE before
retrying"), audited with `phase: "sent"`. Exit code 6 means the guard refused
(profile, flag, or audit log not writable). `health --json` reports
`"writes": true|false`. Policy writes are below.

### Policy writes

Rulesets, allow rules and deny rules are written to draft (`"pending": true`)
and reach the workloads when they are provisioned. Every command takes
`--dry-run` and `--yes` like the object writes; a ruleset is named by its
name (case-insensitive, one list GET) or its href, a rule or deny rule only by
its href (`policy rules <ruleset> --json` lists them).

A rule's inline services are tcp or udp, with or without a port (`443/tcp`,
`tcp`); the PCE takes no other protocol there. `--service icmp` (or `icmpv6`)
is therefore resolved to the ICMP service object (the built-in one, whatever
its name), and refused when the PCE has none.

    illumio-tui policy rulesets add Web-Tier --scope app=web,env=prod --yes
    illumio-tui policy rulesets update Web-Tier --clear-scopes --dry-run
    illumio-tui policy rules add Web-Tier --provider app=web --consumer iplist:Corp --service 443/tcp --yes --json
    illumio-tui policy rules update /orgs/1/sec_policy/draft/rule_sets/7/sec_rules/9 --add-consumer '!iplist:Guest' --dry-run
    illumio-tui policy deny-rules add Web-Tier --provider all --consumer iplist:Any --service 23/tcp --override --yes
    illumio-tui policy provision --all --dry-run
    illumio-tui policy provision Web-Tier --description "web tier v2" --yes
    illumio-tui policy revert Web-Tier /orgs/1/sec_policy/draft/ip_lists/21 --dry-run
    illumio-tui workloads update web-01 --add-label env=prod --managed-labels --yes

Providers, consumers and scope members (`--provider`, `--consumer`,
`--scope`, `--add-*`/`--remove-*`):

| token | meaning |
|-------|---------|
| `app=web`, `label:app=web` | a label |
| `group:NAME` | a label group |
| `iplist:NAME` | an IP list |
| `workload:HOST` | a workload, by hostname, then by name |
| `all` | every workload |
| `!TOKEN` | exclusion (labels, label groups and IP lists only; quote it in the shell) |
| `/orgs/N/...` | an href of one of these kinds, checked without a request |

Services (`--service`): a service name (`HTTPS`, or `service:NAME` when the
name looks like a port), an inline port (`443/tcp`, `8000-8100/udp`, `icmp`)
or a service href. A `--scope` holds labels and label groups only,
comma-separated; repeat `--scope` for alternatives; no `--scope` means
everywhere. Each object list is read at most once per command, and only
when a token needs it, except that each `workload:HOST` token is looked up
on its own (one or two reads each); unknown names are exit 2 with up to five
similar names, before any write. A new rule's `resolve_labels_as` defaults
to `workloads` on both sides; `--resolve-providers-as` and
`--resolve-consumers-as` override it (deny rules carry none).

`rules update` and `deny-rules update` re-read the rule and send only the
keys that change; a changed provider, consumer or service list is sent whole
with the untouched entries exactly as the PCE returned them, and a
`--remove-*` that matches nothing is exit 2. Virtual service and virtual
server actors are not supported. A ruleset `PUT` never carries its rules.

`--impact` on `policy rules add|update` and `policy deny-rules add|update` reports what the
rule will touch, before anything is sent: with `--dry-run`, `data` is the request plus
`impact`; with `--yes`, the result plus `impact`. The rule's sides come from the planned body
(an update's untouched sides from the loaded rule), the scopes from the draft ruleset, and the
workloads from one cached read made after the guard and the plan, so a refused write costs
nothing.

    illumio-tui policy rules add Shop-Prod --provider role=web --consumer iplist:Corp --service 443/tcp --dry-run --impact --json

`policy provision --all` reads the pending list once and provisions every
pending object by explicit href, grouped by type (rulesets, IP lists,
services, label groups, virtual services, firewall settings, and so on); a
changed rule is provisioned through its ruleset. With names or hrefs instead
of `--all`, each must be pending and unrelated pending entries are left
alone. `data.counts` gives the number of objects per type; nothing pending
is `skipped` (exit 0). If the PCE rejects a type it does not accept in a
subset, the rejection is reported with its body. A pending href that is
not a draft policy object href (or cannot be parsed) refuses `--all` before
any write.

`policy revert` throws pending changes away: each object named goes back to
its active version, and one that only exists in draft (a create) is deleted.
It takes `--all` or names and hrefs like `policy provision` and sends
`PUT /orgs/N/sec_policy/delete` with the same explicit `change_subset` (never
the empty body, which would revert everything, including changes made after
the read); `data.summary` and `data.objects` name each object and its fate. The endpoint is the
one the Illumio REST API documents, not yet verified live from this project:
`--dry-run` prints the exact body, so check it against a lab PCE first.

`workloads update --managed-labels` changes the labels of a VEN-managed
workload with a `PUT` that carries only `labels`; `--name`, `--hostname`,
`--description`, `--add-ip` and `--remove-ip` with it are exit 2 before any
request. Without it, managed workloads stay refused.

### Editing in the TUI

The TUI starts with writes off. With writes enabled for the profile, `:writes on` arms
them for the session (not persisted; `:writes off` disarms them, `:writes` shows the state,
and `illumio-tui --allow-writes` starts with them on). Then the
header shows `✎ writes` before the host and two screens take three keys: on Objects (labels,
services, IP lists) and Workloads (unmanaged workloads only)
`n` creates, `e` edits the selected row and `x` deletes it, and every request is shown in a
confirm overlay before `y` sends it once. With writes off the keys print the reason in the
status line and send nothing. Label
groups are read-only.

A form takes one line per field; lists are comma-separated in the CLI
grammar: ranges `10.0.0.0/8, 10.1.0.0-10.1.455.255, !10.0.5.0/24`, FQDNs
`*.corp.example`, ports `443/tcp, 8000-8100/udp, icmp`, addresses
`10.0.0.5, 10.0.0.6/24`, labels `app=web, env=prod` (resolved through the
loaded index, never with a request; a label created in the TUI joins the
index when the labels reload after the write, and `r` on Labels refreshes it
from the PCE). `Tab`/`Shift-Tab` move between
fields, `Ctrl-U` clears one, `Enter` checks the form, `Esc` leaves without a
request.
`Ctrl-N` completes the token being typed (after the last comma) from what is
already loaded: label keys and values (`app=p` offers `app`'s values with a
`p`; a bare `pay` also offers `app=payment`; once a key completes to
`app=`, the next `Ctrl-N` offers its values, and `Ctrl-P` before that steps
back to the other keys), label groups (`group:`), IP lists
(`iplist:`), service names and the loaded draft rulesets; prefix matches
come first, any case, and a workload's labels or a scope never offer a key
they already have. Each further `Ctrl-N` cycles forward, `Ctrl-P` back; up to
five candidates show under the field. Any other key keeps the candidate,
`Esc` puts the typed text back. Completion never sends a request.
An edit sends only what changed; an edit that changes nothing says
`no changes`.

The confirm overlay is the TUI's dry run: it shows the summary,
`METHOD path` and the whole JSON body, long values wrapped (`j`/`k`
scroll). `y` sends it once, `n` or
`Esc` cancels with `cancelled, nothing sent`. The status line then reports
`created <href> (pending)`, `updated <href>` or `deleted <href>`, and that
table reloads once. A rejected write shows the PCE's
message and reloads nothing; an unanswered one says to check the PCE before
retrying and reloads. TUI writes go to the same audit log with
`"source": "tui"`.

On the Policy screen `p` lists pending changes; `j`/`k` select one, and
`Enter` on a pending IP list, service, label group or ruleset shows what it changes
against the active version (`+` added, `-` removed, unchanged entries dimmed; a label
group lists its labels as `key=value` and its sub-groups by name). That costs one GET
for the active list of the kind, cached for five minutes.

`policy pending --diff` prints the same lines under each change (`+`, `-` or a space,
indented two spaces; kinds without a diff say `(no diff)`). With `--json` each change
gains `diff: [{sign, text}]`, only when `--diff` is given. Each kind's draft and active
list is read once, counted in `meta.api_calls`. The CLI does not fetch deny rules served
through the fallback endpoint; such a ruleset shows them as "not compared". Label groups
have no description to compare.

### Policy editing in the TUI

On the Policy screen, in the draft scope, the same three keys edit policy:
on the rulesets pane `n` opens a new ruleset (Name, Description, Enabled
`y`/`n`, Scopes), `e` edits the selected one and `x` deletes it; on the
rules pane `n` adds an allow rule and `N` a deny rule to the selected
ruleset, `e` edits the selected rule (allow or deny) and `x` deletes it. The
rules pane title shows the selected ruleset's scopes
(`Rules · app=shop, env=prod | app=shop, env=staging`, or `Scopeless`). In
the active scope the keys say `switch to draft to edit policy`.

Rule fields use the CLI's token grammar, comma-separated: Providers and
Consumers take `app=web`, `group:Name`, `iplist:Corp`, `all`,
`workload:/orgs/…` (an href; workload names resolve in the CLI only) and
`!` before a label, label group or IP list; Services take `443/tcp`,
`8000-8100/udp`, `icmp` or a service name. Scopes are comma-separated
labels or label groups, alternatives separated by `|`
(`app=web, env=prod | app=web, env=staging`); an empty field means the
ruleset applies everywhere. Names resolve through the loaded index, with no
request. An edit form shows the rule's entries as tokens and sends only
what changed; an entry the grammar cannot express (a virtual service, an
`any` port) shows as its href or `?`, and changing that field is refused
with `use the CLI`.

In the pending overlay (`p`), `Space` marks entries and `P` provisions the
marked ones, or everything pending when none is marked. The confirm overlay
shows the exact `POST /orgs/N/sec_policy` body with its `change_subset`;
`y` sends it once, the status line says `provisioned <version href>: <counts>`,
and the pending list and the rulesets reload. `R` reverts the marked entries
(never everything; with none marked it says so): the confirm lists each
object and whether it goes back to active, is removed, or (when the PCE
named no change type) has a fate unknown; `↑`/`↓` scroll a long list while
the prompt stays in view, and only typing `revert` and `Enter` sends it. After any policy write the
draft rulesets reload once; when the selected ruleset's inline deny list is
empty, that may cost one more GET for its deny rules. `Enter` on a pending
ruleset shows its changes against active: name, description, state, scopes and one
line per rule (`+ allow app=web ← iplist:Corp 443/tcp`), for one GET of the
active rulesets, cached for five minutes. When one side's deny rules were
neither in its list payload nor loaded, the diff shows them unchanged under
`deny rules not compared (<side> not fetched)` rather than guessing.

On Traffic, `w` on a blocked or potentially blocked flow (Flows view or a
drilled pair; on a section's peer row or a Mesh row, the first such flow behind it) opens an allow-rule form pre-filled from the flow in the label
model below: the destination's tier (`role=db`) as providers; as consumers
the source's tier when it is in the destination's scope, else its scope and
tier (`app=crm, env=prod, role=web`) with Unscoped `y`; an endpoint without
the label asked for falls back to `workload:<href>`, else its IP lists, else
`all`. Other dimensions (`loc`, your own keys) are not pre-filled: `Ctrl-A`
swaps in every attached label and back (what you typed is kept). The
service is the flow's `port/proto`. The form opens at once; when the draft
rulesets are not loaded yet (the Policy screen never opened, or showing the
active ones) it sends one draft rulesets read, cached, and none when they are
held. The Policy screen keeps showing the scope you chose. The ruleset is
found by the destination's scope: the one scoped exactly like its labels
under the scope keys, else the only one whose scope covers it. A scope
covers the destination when each of its labels is one the destination has,
under any key (`app=shop` covers `app=shop, env=prod`; `app=shop, env=prod,
loc=eu` covers a destination with `loc=eu`), and a label group counts when
one of its members is; the global scope does not count. When the answer
arrives after you typed a name, your name stays. Several matches leave the
field empty and the hint counts them (`2 rulesets cover app=shop, env=prod:
Ctrl-N lists them first`); none says `no ruleset covers app=shop, env=prod:
type a new name, Ctrl-R creates it`. `Ctrl-N` completes ruleset names from
the drafts (any case, anywhere in the name; again cycles), the covering ones
first; `Tab` on part of a name completes it too, and a second `Tab` keeps
the shown name and moves on (on an empty field or an exact name `Tab` just
moves on). Type the ruleset's name or href; a name is checked at submit (the
cached draft read), and an unknown name brings the form back with `no
ruleset named "X": Ctrl-R creates it scoped app=shop, env=prod` and similar
names. `Ctrl-R` (listed under the form) creates the typed ruleset scoped like
the destination (its own confirm and POST), then plans the rule into it for
a second confirm; cancelling either sends nothing more. It reads the draft
rulesets once more, fresh, when planning: if a ruleset of that name exists
after all (made elsewhere, or the form's list was still loading), nothing is
created: the form comes back as you typed it with that name and its scope
(`ruleset "X" exists (scope app=crm, env=prod): Enter adds the rule to it`,
plus `· it does not cover this flow's destination` when it does not), and
Enter plans the rule into it with its own confirm. A name the loaded drafts already have is refused at
once (Enter adds the rule to it), as is one several rulesets share (use the
href). When the destination has no scope labels, or the row (a pair by
service grouped on part of the scope) spans scopes, there is nothing to
scope one new ruleset by: name an existing one or create it on the Policy
screen.

After the rule is written, the flows on screen are evaluated against the draft
once (as `D` does: one PUT per query), so the flow shows `blocked → allowed`
straight away, also when the form came from the app overlay. It does not run on
its own where `D` would ask first or while a query runs; see `D` above.

### Label model

Rulesets are scoped by `app` and `env`, rules select tiers by `role`. Other
names, per profile:

    [profiles.prod.labels]
    scope_keys = ["app", "env"]   # the labels a ruleset scope is made of
    tier_key = "role"             # the label a rule selects a tier by
    ignore_keys = ["DFIRBubble"]  # keys hidden from label columns and the g picker

or `ILLUMIO_SCOPE_KEYS=app,env` and `ILLUMIO_TIER_KEY=role`, which outrank
the profile. An empty `ILLUMIO_SCOPE_KEYS` counts as unset; a list of only
empty entries (`,`) is refused, as is an empty `scope_keys` in the profile.
The TUI's `w` form (pre-fill and `Ctrl-R`) uses them, the traffic views group by the scope keys
until `g` picks others (for the session), and `traffic groups`, `traffic mesh` and
`traffic query --blocked` default to them.
`ignore_keys` (or `ILLUMIO_IGNORE_KEYS=DFIRBubble,os`,
which outranks the profile; empty counts as unset) hides housekeeping keys from the label text
of the traffic and workload tables, the flow detail, `all labels` and the
`g` picker, and the Labels tab until `i`; JSON output and the workload edit form keep
them, and a scope or tier key is never ignored.

The policy generator names a ruleset it creates by a template over the scope keys:

    [profiles.prod.generator]
    ruleset_name = "RF-{app}-{env}"   # the default: RF-shop-prod

Every `{key}` must be a scope key and every scope key must be named, so each new ruleset
gets its own name; anything else fails the load and names the key.

## PCE protection

Every request is user-caused, throttled (2/s), cached, and counted.
Explorer queries default to a 24 h window and 10 000 rows; larger windows
(> 7 d) or caps (> 10 000, at most 200 000) need `--yes`. `--dry-run` shows the request
body without sending it. A traffic backfill sends nothing before `B` (or `--backfill`, or
the written `traffic_auto_backfill` opt-in) and is one query per cap rung, each at least 3 API
calls; every rung above 10000 asks first (`y`, or `--yes`). A draft re-evaluation (`D`,
`--draft`) is one PUT on our own query plus polls and one download per query, never without
`D`, `--draft` or a successful write of yours (one run per write), and cached. Status polls start at 1 s and back
off to at most 3 s; downloads are gzip-compressed. The CLI submits an unfocused query while it
loads the object lists for name resolution, so the PCE computes meanwhile without extra
requests.

## illumio-tui and workloader

[workloader](https://github.com/brian1917/workloader) is the established
batch tool for the PCE, and the two are meant to be used together.
illumio-tui is for interactive analysis: exploring traffic and policy with a
customer, finding blocked flows and drafting the rules they need, reviewing one
app, and giving scripts and AI agents a JSON CLI that counts every request.
For bulk import and export, automated labelling (hostnames, subnets, clouds,
vCenter), VEN lifecycle, rule and port usage reports, and running one command
across many PCEs, use workloader. illumio-tui does not plan to cover those.

## Development

    cargo test --workspace
    cargo xtask record --env-file ../illumio-mcp-server/.env   # refresh fixtures/recorded
    PCE_LIVE=1 cargo test -p pce --test live                    # needs PCE_* env
    PCE_LIVE_WRITES=1 cargo test -p pce --test live_writes      # creates and deletes one label; non-production PCE only

Design: `docs/superpowers/specs/2026-10-10-illumio-tui-design.md`.

## License

GPL-3.0-only. See `LICENSE`.
