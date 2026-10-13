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
install (example: macOS Apple Silicon, version 0.1.7):

    sha256sum --check --ignore-missing SHA256SUMS      # macOS: shasum -a 256 -c --ignore-missing SHA256SUMS
    tar -xzf illumio-tui-0.1.7-aarch64-apple-darwin.tar.gz
    install -m 755 illumio-tui-0.1.7-aarch64-apple-darwin/illumio-tui ~/.local/bin/   # any directory on PATH
    illumio-tui --version

macOS may quarantine binaries downloaded with a browser; clear it with
`xattr -d com.apple.quarantine ~/.local/bin/illumio-tui`. On Windows, check
the hash with `Get-FileHash illumio-tui-<version>-x86_64-pc-windows-msvc.zip`
(compare with the line in `SHA256SUMS`), extract the zip, and move
`illumio-tui.exe` into a folder listed in your `PATH`.

### Cutting a release (maintainers)

1. Bump `version` in `[workspace.package]` of the root `Cargo.toml` and run
   `cargo check` so `Cargo.lock` follows.
2. Commit, then tag and push: `git tag v0.1.7 && git push --tags`.

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
    traffic_auto_backfill = false   # true: backfill a truncated result without asking (see TUI)

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
  has loaded them), protocols and services as `port/proto`, the decision words and the
  window steps, never with a request. `+` / `-` step
  the window along `1h 6h 12h 1d 2d 3d 1w 2w 4w 90d`: `+` fetches only the older slice and
  merges it into what is on screen (a flow seen in both slices stays one row with the
  connections and bytes added up; the header counts the slices), `-` drops the older flows
  without a request. `r` after widening runs one query for the whole window and asks first
  when that is expensive. A truncated result that covers under half its window says how much
  in the Result header (`10000 flows · covers 25m of 24h · truncated`) and the status line,
  with what `]` and `B` would do. Explorer rows are not time-ordered within the cap, so the
  truncated rows say nothing reliable about any part of the window: `B` fetches the whole
  window again, backwards from its end, in slices sized to fit the cap (the measured rate,
  `0.8 × cap`, rounded down to 1m 5m 15m 30m 1h 2h 6h 12h 1d). When the cap cannot fit the
  window in 50 slices, `B` proposes the smallest cap of the `]` ladder (up to 200000) that
  can. It asks once, for example after 10000 flows covering 2h 30m of 24h (`backfill 1d in
  12 slices of 2h at cap 10000: 12 queries (≥36 API calls)? y/n`) or 25 minutes of 24h (`cap
  25000 exceeds 10000. raise the cap to 25000 and backfill 1d in 48 slices of 30m: 48 queries
  (≥144 API calls)? y/n`): each query is at least a submit, a status poll and a download, and
  a cap above 10000 is named as expensive first.
  It then sends one slice at a time, newest first, and counts them in the header
  (`slice 3 of 12`): the first slice replaces the truncated rows, later ones merge like `+`,
  and the result stays marked truncated until the slices reach the window start. A slice that
  still truncates is fetched again in halves (never below 1m); a 1m slice that truncates stops
  the backfill (`] raises the cap`). A run sends at most 50 slices: when even a cap of
  200000 needs more, `B` offers the newest 50 (`backfill the next 50 slices of 15m:
  12h 30m of the remaining 1d, …; B again continues`). `Esc` stops after the running slice, and
  `B` again continues where a run stopped. A backfill keeps auto-refresh off, the header shows
  its real coverage and slice count (`1d · 12 slices`, or `covers 6h of 1d · 3 slices ·
  truncated`), and `+` extends it by one slice of the same length.
  The profile setting `traffic_auto_backfill = true` (or `ILLUMIO_AUTO_BACKFILL=1`) starts that
  run right after a truncated, low-coverage query without the `y`, with the same limits,
  progress and `Esc`: one run (at most 50 slices) per query, never chained, and a cap above
  10000 is still asked. It is off by default because every request must be user-caused: setting
  it is the user saying yes in advance, once, in writing. `]` / `[` step the result cap
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
  without a request.
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
  header: `key app,env · 212 of 4310 · 38 hidden`), and `u` keeps the labels no workload,
  label group, ruleset scope or rule uses, counted in the policy scope the Policy screen has
  loaded (draft by default, active once `d` toggles it); the `workloads` and `rules` columns
  sort and show `-` until Workloads and Policy are loaded, and `u` asks before loading them.
  A label used only by a deny rule the PCE serves through the per-ruleset fallback endpoint
  counts once that ruleset's deny rules have been loaded, and a truncated workload list is
  named in the header (`unused (workloads truncated)`).
- **Workloads**: VEN health columns and a summary line, `h` unhealthy only, `H` sort by
  severity, `t` / `T` jump to Traffic with the workload as source / destination.
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
costs one workloads read on `i`, none when the list is cached. Everything else is caused by a key you press. The header shows
the API call and byte counters at all times.

## CLI

    illumio-tui health
    illumio-tui labels --key app
    illumio-tui labels --key app --key env --value-contains pay --ignore-keys
    illumio-tui labels --unused --json
    illumio-tui workloads --label app=web --online true
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
    illumio-tui traffic mesh --window 1h --no-user --json
    illumio-tui traffic query --window 30d --dry-run
    illumio-tui traffic query --window 24h --backfill --json
    illumio-tui workloads --csv > workloads.csv
    illumio-tui traffic groups --window 24h --by in --csv > peers.csv
    illumio-tui profile list
    illumio-tui profile use prod

JSON output (default when piped) is `{"data": ..., "meta": {...}}` where
`meta` reports API calls, bytes, cache status, truncation, and the query
window; traffic commands add `meta.covered`, the span the flows' last-seen times cover
(`"4m"`; the whole window when the result is complete). With `--backfill` (`query`, `groups`
and `mesh`) a truncated result's window is fetched again in slices as the TUI's `B` does,
newest first, one query at a time, the cap raised along the ladder (up to 200000) when
the slices would not fit in 50 (past 10000 only with `--yes`, so one `--yes` on a
`--backfill` run allows up to 50 slice queries of up to 200000 rows each); `meta.slices` counts the slices
merged and `meta.api_calls` every request (at least 3 per query). A run sends at most
`--max-slices` queries (1 to 50, default 50); when the window needs more, stderr names the
`--from` / `--to` that continue and `meta.truncated` stays true. Errors go to stderr as JSON. Exit codes: 1 generic, 2 arguments or
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
Commands that print no table (writes, `health`, `policy rules impact`, `profile use`, a
traffic `--dry-run`) refuse it with exit 2 before any request.

`traffic groups` and `traffic mesh` take the `traffic query` filter flags (and its dry run,
cost guard and cache) and print the same envelope. `groups` gives one row per source and
destination group (`src`, `dst`, `flows`, `connections`, `bytes`, `allowed`,
`potentially_blocked`, `blocked`; `--services` splits each pair by service and adds a
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
request (the labels are fetched, cached). `--backfill` does not combine with `--app`.
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
the labels no workload, rule, ruleset scope or label group uses: it reads the workloads, the
draft rulesets and the label groups (through the cache, counted in `meta`) and adds
`workloads` and `rules` to each row; deny rules count when the ruleset carries them inline
(no fallback reads). Without it the rows are unchanged.

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
service is the flow's `port/proto`. The ruleset is pre-filled when exactly
one loaded draft ruleset is scoped like the destination; otherwise the hint
names that scope (`scope app=shop, env=prod`). Type the ruleset's name or
href; a name costs one draft ruleset read at submit, and an unknown name
brings the form back with similar names. When the destination has a scope,
the form offers `Ctrl-R` (the Ruleset hint says `Ctrl-R: create it, scoped
…`): it creates the ruleset under that name scoped like the destination (its
own confirm and POST), then plans the rule into it for a second confirm;
cancelling either sends nothing more. Editing the name withdraws the offer
(press Enter to check the new name first).

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

## PCE protection

Every request is user-caused, throttled (2/s), cached, and counted.
Explorer queries default to a 24 h window and 10 000 rows; larger windows
(> 7 d) or caps (> 10 000, at most 200 000) need `--yes`. `--dry-run` shows the request
body without sending it. A traffic backfill sends nothing before `B` and `y` (or
`--backfill`, or the written `traffic_auto_backfill` opt-in), never more than one slice at a
time and at most 50 queries per run, each at least 3 API calls.

## Development

    cargo test --workspace
    cargo xtask record --env-file ../illumio-mcp-server/.env   # refresh fixtures/recorded
    PCE_LIVE=1 cargo test -p pce --test live                    # needs PCE_* env
    PCE_LIVE_WRITES=1 cargo test -p pce --test live_writes      # creates and deletes one label; non-production PCE only

Design: `docs/superpowers/specs/2026-10-10-illumio-tui-design.md`.

## License

GPL-3.0-only. See `LICENSE`.
