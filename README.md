# illumio-tui

Terminal UI and CLI for the Illumio Core PCE, built for administrators,
Illumio Professional Services, and agents. Reads by default; writes for labels, IP lists, services and unmanaged workloads are opt-in (see Writes).

Website: https://alexgoller.github.io/illumio-tui/

## Install

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
install (example: macOS Apple Silicon, version 0.1.1):

    sha256sum --check --ignore-missing SHA256SUMS      # macOS: shasum -a 256 -c --ignore-missing SHA256SUMS
    tar -xzf illumio-tui-0.1.1-aarch64-apple-darwin.tar.gz
    install -m 755 illumio-tui-0.1.1-aarch64-apple-darwin/illumio-tui ~/.local/bin/   # any directory on PATH
    illumio-tui --version

macOS may quarantine binaries downloaded with a browser; clear it with
`xattr -d com.apple.quarantine ~/.local/bin/illumio-tui`. On Windows, check
the hash with `Get-FileHash illumio-tui-<version>-x86_64-pc-windows-msvc.zip`
(compare with the line in `SHA256SUMS`), extract the zip, and move
`illumio-tui.exe` into a folder listed in your `PATH`.

### Cutting a release (maintainers)

1. Bump `version` in `[workspace.package]` of the root `Cargo.toml` and run
   `cargo check` so `Cargo.lock` follows.
2. Commit, then tag and push: `git tag v0.1.1 && git push --tags`.

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

`--env-file .env` loads a dotenv file for one run. Connection settings can
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
`:draft`, `:active`, `:help` and `:q`.
`?` shows the full key map, `Esc` closes overlays, `q` or `Ctrl-C` quits. The mouse wheel
scrolls the current table and a click on a tab switches screen; on a terminal narrower than
the full tab row (69 columns) the tabs read `Traf`, `Pol`, `Obj`, `Wl`, `Ev`.

- **Traffic**: `f` edit the filter, `r` run (bypasses the cache), `a` auto-refresh, `s` sort,
  `/` search; a filter that would be expensive asks for confirmation first. `v` cycles five
  views of the one result, all grouped by the same label keys: the profile's scope keys
  (`app,env` by default) until `g` opens the picker (`Space` toggles a key, `all labels` takes
  every key the flows carry, typing edits the list as text, `Enter` applies, for this session).
  **Flows** shows each side's group (`shop/prod`; `-` for an IP list or FQDN, whose name is in
  the name column) and, on `Enter`, a detail with both label sets and the bytes. **App-to-app**
  pairs the groups; `Enter` drills into a pair's flows. **Mesh** lists user, process, source,
  destination, service and destination process like the Illumio UI's mesh (`u` / `p` hide the
  user / process column and merge rows, `Enter` drills). **Blocked** groups blocked and
  potentially blocked flows by destination group and service. **Treemap** draws each source as
  a box and its destinations as tiles sized by connections (`b` bytes) and coloured by
  decision: `h` / `j` / `k` / `l` move, `Enter` zooms into a source and then drills into a
  destination, `Esc` zooms out. None of this sends a request. With writes on, `w` turns a
  blocked flow (or the first one behind a Blocked or Mesh row) into a draft allow rule.
- **Policy**: `d` toggle draft / active, `p` pending changes, `Tab` switch panes, `Enter` rule
  detail, or on a ruleset load its deny rules. Rules are listed in evaluation order: override
  deny, allow, deny. With writes on, `n` / `e` / `x` create, edit and delete rulesets and rules
  in draft (`N` a deny rule), and the pending overlay marks with `Space` and provisions with `P`.
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
  and delete labels, services and IP lists.
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
    illumio-tui workloads --label app=web --online true
    illumio-tui policy rulesets
    illumio-tui policy rules "Web-DB"
    illumio-tui policy rules "Web-DB" --impact --json
    illumio-tui policy rules impact /orgs/1/sec_policy/draft/rule_sets/7/sec_rules/3
    illumio-tui iplists --table
    illumio-tui policy pending
    illumio-tui traffic query --window 24h --src app=web --dst app=db --json
    illumio-tui traffic query --window 1h --group app,env
    illumio-tui traffic query --window 1h --blocked
    illumio-tui traffic groups --window 1h --keys app,env,role
    illumio-tui traffic mesh --window 1h --no-user --json
    illumio-tui traffic query --window 30d --dry-run
    illumio-tui profile list
    illumio-tui profile use prod

JSON output (default when piped) is `{"data": ..., "meta": {...}}` where
`meta` reports API calls, bytes, cache status, truncation, and the query
window. Errors go to stderr as JSON. Exit codes: 1 generic, 2 arguments or
config, 3 auth, 4 PCE error, 5 rate limited, 6 write refused by the guard.

`traffic groups` and `traffic mesh` take the `traffic query` filter flags (and its dry run,
cost guard and cache) and print the same envelope. `groups` gives one row per source and
destination group (`src`, `dst`, `flows`, `connections`, `bytes`, `allowed`,
`potentially_blocked`, `blocked`); `mesh` one row per source user, source process, source group,
destination group, port, protocol and destination process. The PCE reports one process per
flow, seen where the flow was reported: an inbound flow's process is the destination's,
anything else the source's. The user, like the process, is the one the reporting VEN saw:
shown as the source's user on outbound flows and left empty on inbound ones. `--keys` picks
the label keys (default: the profile's scope keys), `--no-user` / `--no-process` merge rows
across users or processes. An endpoint that is not a labelled workload is grouped under its
name or `ipl:<list>`. `bytes` is `0` in JSON and `-` in tables when the PCE reported none.

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
grammar: ranges `10.0.0.0/8, 10.1.0.0-10.1.255.255, !10.0.5.0/24`, FQDNs
`*.corp.example`, ports `443/tcp, 8000-8100/udp, icmp`, addresses
`10.0.0.5, 10.0.0.6/24`, labels `app=web, env=prod` (resolved through the
loaded index, never with a request; a label created in the TUI joins the
index when the labels reload after the write, and `r` on Labels refreshes it
from the PCE). `Tab`/`Shift-Tab` move between
fields, `Ctrl-U` clears one, `Enter` checks the form, `Esc` leaves without a
request.
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
`Enter` on a pending IP list or service shows what it changes against the
active version (`+` added, `-` removed, unchanged entries dimmed). That costs
one GET for the active list of the kind, cached for five minutes.

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
and the pending list and the rulesets reload. After any policy write the
draft rulesets reload once; when the selected ruleset's inline deny list is
empty, that may cost one more GET for its deny rules. `Enter` on a pending
ruleset shows its changes against active: name, description, state, scopes and one
line per rule (`+ allow app=web ← iplist:Corp 443/tcp`), for one GET of the
active rulesets, cached for five minutes. When one side's deny rules were
neither in its list payload nor loaded, the diff shows them unchanged under
`deny rules not compared (<side> not fetched)` rather than guessing.

On Traffic, `w` on a blocked or potentially blocked flow (Flows view or a
drilled pair; on a Blocked or Mesh row, the first such flow behind it) opens an allow-rule form pre-filled from the flow in the label
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
the form offers `Ctrl-N` (the Ruleset hint says `Ctrl-N: create it, scoped
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
The TUI's `w` form (pre-fill and `Ctrl-N`) uses them, the traffic views group by the scope keys
until `g` picks others (for the session), and `traffic groups`, `traffic mesh` and
`traffic query --blocked` default to them.
`ignore_keys` (or `ILLUMIO_IGNORE_KEYS=DFIRBubble,os`, which outranks the
profile; empty counts as unset) hides housekeeping keys from the label text
of the traffic and workload tables, the flow detail, `all labels` and the
`g` picker; JSON output, the workload edit form and the Labels tab keep
them, and a scope or tier key is never ignored.

## PCE protection

Every request is user-caused, throttled (2/s), cached, and counted.
Explorer queries default to a 24 h window and 1000 rows; larger windows
(> 7 d) or caps (> 10 000) need `--yes`. `--dry-run` shows the request
body without sending it.

## Development

    cargo test --workspace
    cargo xtask record --env-file ../illumio-mcp-server/.env   # refresh fixtures/recorded
    PCE_LIVE=1 cargo test -p pce --test live                    # needs PCE_* env
    PCE_LIVE_WRITES=1 cargo test -p pce --test live_writes      # creates and deletes one label; non-production PCE only

Design: `docs/superpowers/specs/2026-10-10-illumio-tui-design.md`.

## License

GPL-3.0-only. See `LICENSE`.
