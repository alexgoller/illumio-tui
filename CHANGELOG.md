# Changelog

Every illumio-tui release, newest first. Downloads, packages and checksums for each one are on [GitHub Releases](https://github.com/alexgoller/illumio-tui/releases).

## v0.5.4 — 2026-10-24

### New

- **VEN health on Workloads** (#72). See the state of your VEN fleet without leaving the screen, from data already loaded. No extra requests.
  - A strip under the table: `VEN n · healthy · warning · error · offline · unmanaged`.
  - `:ven` opens an overlay with health, VEN versions (newest first, with how many VENs are behind), enforcement modes and the most common health messages with their severity. Enter on a line filters the table to it.
  - `v` cycles a health filter (healthy, warning, error). It shares one dimension with `o` (offline) and `h` (unhealthy): each replaces the others.
  - CLI: `workloads --health healthy,warning,error` (case-insensitive, combines with `--offline` / `--unhealthy`), `--ven-version 24.2.*` (exact or glob), and `workloads ven-summary` (table, `--json`, `--csv`).
  - Saved Workloads filters keep `health` and `ven_version`.
- **The changelog on the docs site**: every release's notes at [alexgoller.github.io/illumio-tui/changelog.html](https://alexgoller.github.io/illumio-tui/changelog.html). Release notes now come from `CHANGELOG.md`.

### Security

- tokio 1.54.1 (RUSTSEC-2026-0386).

## v0.5.3 — 2026-10-23

### New

- **Saved filters** (#70). Name the filters you use often and get them back in one step, per profile, on Traffic, Policy, Events and Workloads.
  - TUI: `:save NAME` saves the filter in force on the current screen. `:filters` (`:fl`) lists that screen's saved filters: Enter applies, `D` deletes. `:load NAME` applies one directly. The title shows `[NAME]` while the saved filter is unchanged.
  - CLI: `--filter NAME` on `traffic`, `policy search`, `events` and `workloads`; a flag you add replaces that field of the saved filter. `--save-filter NAME` saves the effective filter and exits **without contacting the PCE**. `filters list | show | rm` (with `--json` / `--csv`).
  - Stored in the config file, in the same syntax you type in `f`:
    ```toml
    [profiles.demo.filters.policy]
    deny-prod = "dst:env=Production deny"

    [profiles.demo.filters.events.quiet]
    type = "!agent.*"
    severity = "warning+"
    ```
  - Applying a saved traffic filter is the same as `f` + Enter: same checks, same cost confirm, one query you caused.
  - Shell completion offers the saved names.
- **A default filter per screen** (#74). `[profiles.demo.filter_defaults] policy = "deny-prod"`.
  - The TUI applies it at startup and after `:profile`, before the screen's first load. It never sends an extra request or starts a traffic query on its own.
  - `*` in `:filters` sets or unsets the default; `:default NAME` and `:default off` do the same by command.
  - On the CLI a default is never applied implicitly: `--default-filter` opts in. `filters default SCREEN NAME | --unset`.

### Changed

- Loading a saved Events filter keeps the profile's `exclude_types` in force, on top of the filter's own type terms. Before, a saved type replaced them. To see a hidden type for one session, edit the filter with `f`.

### Notes

- Saving a filter or a default rewrites the config file, as `profile add` does. Settings, the secret and unknown keys are kept; TOML comments are not.
- A malformed `filters` or `filter_defaults` entry never stops the TUI or other commands; it is ignored with a warning and kept as written.

## v0.5.2 — 2026-10-21

### New

- **Event type filters** (#67). The events type filter takes several terms and excludes:
  - `rule_set, sec_policy.*` (either), `!user.login` (hide), `=user.login` / `!=user.login` (exactly that type), `*` globs. Same syntax in the TUI `f` Type field and in `events --type` (repeatable), plus `--exclude-type`.
  - TUI Events screen: `x` hides the selected row's type, `o` shows only it, `t` opens a per-type count summary (Enter = only, `x` = hide). Ctrl-N completes types from the loaded events.
  - The header always says what is hidden: `hiding 2 types · warning+ · 1 of 4 · capped at 4`. CLI JSON reports `meta.hidden` and `meta.local_filter`.
  - `--severity warning+` (and the TUI severity field): that severity and worse.
  - Default excludes per profile: `[profiles.<name>.events] exclude_types = ["=user.login"]`; `events --all-types` ignores them.
  - Type and `X+` changes refilter the loaded events: no request. Before, every Enter in the events filter sent one.
- **Policy search: where, what not, which kind** (#68). TUI `f` on Policy and `policy search`:
  - `scope:app=shop`, `src:role=web` (consumers), `dst:role=db` (providers); `src:`/`dst:` also take `ipl:`, `wl:`, `all`; `scope:global`.
  - `src:!env=prod` finds rules whose consumers use an *All except env=prod* exclusion.
  - A leading `-` negates any term: `app=shop -deny -scope:env=dev`. On the command line `-terms` work without `--`; quote `!` terms in bash/zsh (`'src:!env=prod'`).
  - `intra`, `extra`, `enabled`, `allow`.
  - Results say `N of M rules matched`. Labels complete after the prefixes, in the TUI and in the shell.

### Changed

- The TUI now applies the profile's limits at startup, not only after `:profile`. In particular `traffic_auto_backfill = true` / `ILLUMIO_AUTO_BACKFILL=1` now takes effect from the first truncated traffic query you run (it was silently ignored before a profile switch). Nothing new is requested at startup.

## v0.5.1 — 2026-10-21

### New

- **`profile add`** (#66): create a profile without hand-editing `config.toml`.
  - From an env file, e.g. the one an Illumio MCP server uses: `illumio-tui profile add demo --from-env-file path/.env` reads `PCE_HOST`, `PCE_PORT`, `PCE_ORG_ID`, `API_KEY`, `API_SECRET` (and `PCE_SCHEME`, `PCE_TLS_VERIFY`). `--from-env-file -` reads pasted text; CRLF, `export`, quotes, comments and a BOM are handled.
  - Or from flags: `--host`, `--port`, `--org-id`, `--api-key`, `--insecure`, `--writes`, `--default`.
  - The secret can be plain text (env file, `--api-secret-stdin`, or `--api-secret VALUE`, which may land in shell history) or a command (`--api-secret-cmd "op read …"`), which stores nothing secret. The secret is never printed, and env-file errors give the line number, never its content.
  - Refuses to replace an existing profile without `--force`. `--force` updates the connection and credentials and keeps everything else (limits, label model, generator settings, unknown keys).
  - The first profile becomes the default. Since v0.5.0, PCE name completion needs a config-file profile, so this is the quickest way to get it: `profile add`, then `completions refresh`.

### Fixes

- Rewriting `config.toml` (`profile add`, `profile use`) is now atomic and safe against a planted temp-file symlink, leaves the file at mode 0600, and keeps unknown keys inside `limits`, `labels` and `generator` (they were dropped before). Comments are still not kept.
- The test suite no longer writes completion caches into the developer's real `~/.cache`.

## v0.5.0 — 2026-10-21

### Security

- **TLS dependency update** for RUSTSEC-2026-0382 (rustls-webpki: distrusted intermediates) and RUSTSEC-2026-0383 (webpki-root-certs: emSign roots). All earlier releases are affected; please upgrade.

### New

- **Shell completion of PCE names** (#64). Tab now completes labels (`env=P` → `PCI`, `Production`), apps, workload hostnames, services (`svc:NAME`), IP lists and label groups in bash, zsh, fish and PowerShell.
  - Tab never contacts the PCE: it reads a small per-profile cache file (~7 ms).
  - Fill it once with `illumio-tui completions refresh` (5 requests); normal list commands and the TUI keep it current. Nothing refreshes on its own. `completions refresh --clear` deletes it.
  - Only for profiles from the config file (host and org not overridden by flags or env), so one PCE's names never end up under another profile. Set `completion_cache = false` to keep PCE names off disk.
  - Re-run `illumio-tui completions <shell>` to install the new script.
- **Intra- and extra-scope rules made visible and safe** (#65).
  - The rule form's `Unscoped y/n` is now **Scope: intra | extra** (Space switches, `i`/`e` pick).
  - A line under Consumers says what the rule means as you type, e.g. "role=web inside app=shop env=prod" or "role=Web in any app, env".
  - Switching never widens a rule: to intra drops only labels the ruleset scope already pins; to extra in the `w` form adds the flow source's scope labels; switching back restores what was there.
  - Warnings in the form, at confirm, and on `policy rules add` (no extra request): "matches role=Web in every app, env", "never matches: env=Development is outside the ruleset scope", and intra-scope in a global ruleset.
  - Policy rules pane: scope column, intra-scope rules first, "N intra · M extra" in the title. Rule detail says intra-scope / extra-scope.
  - CLI: `--extra-scope` / `--intra-scope` aliases for `--unscoped-consumers` / `--scoped-consumers`.

## v0.4.2 — 2026-10-20

v0.4.2 lets you negate conditions and use port ranges in traffic queries. It also adds winget packaging.

### Traffic: exclusions (#62)
- **TUI** (`f` on Traffic): a `!` in front of a term excludes it, in the Src, Dst and Service fields: `env=prod, !app=monitoring` · `!22/tcp` · `!svc:SSH`.
  - Exclusions go into Explorer's own exclude lists, so the PCE filters them out and the query costs nothing extra.
  - An exclusion applies to the whole field, even inside a `|` group, and the editor says so.
  - A field with only `!` terms means "everything except".
  - Completion works after `!`.
- **CLI:** `--exclude-service` joins `--exclude-src` / `--exclude-dst` on every traffic subcommand:
  `illumio-tui traffic query --dst env=prod --exclude-src app=monitoring --exclude-service 22/tcp --exclude-service svc:SSH`
- **`svc:NAME`** expands to the service's ports, included or excluded, in the TUI and the CLI.
- **Excludable:**
  - sources: labels, IP lists, workloads, IPs;
  - destinations: the same, plus `fqdn:`;
  - services: port, protocol, port/protocol, range, or a named service.
- **Behaviour change:** `fqdn:` on the source side is now rejected up front, even as an include. Explorer accepts FQDNs on destinations only, so the PCE used to refuse it instead.

### Traffic: port ranges (#63)
- `8000-8100/tcp` in the TUI Service field (`!8000-8100/tcp` excludes it). CLI: `--port 8000-8100 --proto tcp`, `--exclude-service 8000-8100/tcp`. `--port 443/tcp` also works now.
- Services that contain a range (e.g. `svc:Web` with 8000-8099/tcp) are accepted instead of rejected.

### Windows: winget (#61)
- winget manifests are rendered and tested on every release: validate, install, upgrade in place, uninstall on a Windows runner.
- The first submission is [microsoft/winget-pkgs#456974](https://github.com/microsoft/winget-pkgs/pull/456974). It has passed validation and is waiting for moderator review. Once it's merged: `winget install AlexGoller.IllumioTUI`, then `winget upgrade AlexGoller.IllumioTUI`.

Queries without exclusions or ranges send exactly the same request as before.

## v0.4.1 — 2026-10-20

v0.4.1 brings Windows packages that update in place (#60).

### Windows
- **MSI installer** (`illumio-tui-0.4.1-x86_64-pc-windows-msvc.msi`): installs for the current user without admin rights into `%LOCALAPPDATA%\Programs\illumio-tui` and adds it to your user `PATH`. A newer MSI upgrades the installed version in place, and uninstalling (Settings > Apps) removes the `PATH` entry too. Every release installs, upgrades and uninstalls the MSI on a Windows runner before publishing it.
  - Double-click it, or install silently: `msiexec /i illumio-tui-0.4.1-x86_64-pc-windows-msvc.msi /qn`
  - The MSI is not code-signed yet, so SmartScreen may warn: *More info*, then *Run anyway*.
- **Scoop:** `scoop bucket add illumio-tui https://github.com/alexgoller/illumio-tui`, then `scoop install illumio-tui`. `scoop update illumio-tui` picks up new releases; the manifest is updated on every release, like the Homebrew formula.
- **PowerShell completions:** `illumio-tui completions powershell | Out-String | Invoke-Expression` (add it to your `$PROFILE`).

The MSI is covered by `SHA256SUMS` like every other asset.

## v0.4.0 — 2026-10-20

v0.4.0 takes an app from observed traffic to enforced policy, with an exact rollback. It also adds explain, policy search and shell completions. Every write path was verified against a live PCE.

### From visibility to enforcement (#54, #55, #56)
- **`illumio-tui readiness`** judges every app: workloads, enforcement mix, coverage of the draft policy, the flows enforcement would block (as policy tuples), and the pending changes that reach it. The verdict is `not_evaluated`, `needs_rules`, `needs_provision`, `ready` or `enforced`. It costs one Explorer query plus the draft evaluation.
- **`illumio-tui enforce <app> --to full|selective`** refuses unless the app is ready (or `--force`). It moves the app's managed workloads in bulk and writes a change record (`~/.local/state/illumio-tui/enforcements/`) before and after the send.
- **`--watch`** lists what was blocked since the change, and exits 7 when there is anything.
- **`--rollback`** restores each workload's previous mode exactly. It skips workloads someone else changed since, and refuses a record that was already rolled back.
- **TUI:** `:readiness` shows every app's verdict. `:enforce <app>` (or `e` in the app overlay) opens a five-step overlay: Readiness, Rules (gaps to the generator `G`, the rule form `w`, accept-blocked marks `m`), Provision (`P`), Enforce, and Watch with rollback (`R`).
  - A result that is filtered, truncated, incomplete or focused on another app counts as not ready, so you have to type `enforce`.

### Analyst workflow (#50, #51, #52, #53)
- **`E` explains a flow.** It shows the reported and draft decision and the rules that allow or block the flow, resolved to ruleset, scope, consumers → providers and services. With no rule it says why the flow still passes (e.g. "allowed now only because the destination is not fully enforced"). `Enter` jumps to the rule in Policy. CLI: `traffic explain --src … --dst … --port … --proto …`.
- **Policy search:** `f` on Policy, or `policy search`. Rules across every ruleset by label (label groups included, exclusions honoured), IP list, service or port, workload, href, `disabled`, `deny`.
- **`t`** enables or disables a rule, deny rule or ruleset (a draft change). CLI: `policy rules|rulesets enable|disable`.
- **`I`** lists the workloads a rule matches on each side. Search, export and jump work from that list. CLI: `--list`.

### Also
- **Shell completions:** `illumio-tui completions bash|zsh|fish`. Homebrew and the .deb/.rpm packages install them. Profile names complete from your config (#57).
- **Flows view:** the draft column shows only the draft decision (`→ allowed`, `→ blocked`), so it fits at 120 columns (#59).
- **Apple Terminal:** before macOS 26 it garbled the 24-bit theme colours. Colours are now mapped to the xterm 256 there; `ILLUMIO_TUI_COLORS=256|truecolor` overrides.
- **Docs:** eight workflows with real screenshots, including the `:readiness` / `:enforce` overlay and "Find rules fast" (`f`, `t`, `I`), plus "CLI: getting started" and "Cool things you can do with the CLI" (#58).
- **Live write suite:** `scripts/live-smoke.sh --env-file … --workload …` runs every write path against a PCE you designate and restores what it changes (#49). Its first run found two bugs, both fixed:
  - interface updates sent the read-only `network_detection_mode` (406);
  - the draft evaluation of an empty result failed (405).
- **Code layout:** `update.rs` and `model.rs` are split by screen (#48).

## v0.3.5 — 2026-10-19

### Ctrl-U empties the line, everywhere

Ctrl-U now clears the line being typed in every text input, as in a shell. Before, only the forms handled it; elsewhere it typed a literal `u`.

- the `:` command line
- `/` search (the search stays open)
- the traffic and events filter editors (only the active field is cleared)
- the Workloads label filter (`f`)
- the group-key picker (`x`) and the app picker
- the typed confirmations: the `revert` word and the bulk count

The help screen lists it under "Everywhere".

## v0.3.4 — 2026-10-18

### Workloads: labels and enforcement in bulk (#47)

Set labels or the enforcement mode on one workload, a marked list or a whole group, from the Workloads screen or the CLI. Changes go out in `bulk_update` requests of 100.

**TUI** (Workloads screen, after `:writes on`)
- `Space` marks a workload; `V` marks the group under the cursor or everything shown.
- `M` sets the enforcement mode and visibility level. Unmanaged workloads are skipped.
- `L` sets or removes labels; Ctrl-A creates a missing label.
- A preview lists every change and every skip before you confirm with `y`. `d` runs an optional draft check (≈ flows the rules would leave out). Delete moved to `X`.

**CLI**
- `illumio-tui workloads enforce <idle|visibility_only|selective|full> [--visibility LEVEL] <filters> --yes`
- `illumio-tui workloads relabel --set key=value --remove-label key <filters> --yes`
- `--dry-run` prints each request body before anything is sent.

**Safety**
- Writes must be armed: `writes = true` in the profile or `ILLUMIO_WRITES=1`, plus `--yes` (CLI) or `:writes on` (TUI).
- No filter at all needs `--all`.
- More than 25 workloads, or `selective`/`full` on more than one, asks you to type the count (`--confirm-count N`).
- Targets are re-read (or the selection read must be at most 5 s old). A workload that changed in between is dropped, not overwritten. A batch the PCE answered without per-item results is reported as "may have applied".
- Relabel sends each workload's full label set (the kept labels plus the changes), because `bulk_update` replaces it.
- Each write is logged in `~/.local/state/illumio-tui/writes.jsonl`.

### Fixes
- Workloads grouped by app|env: the section headers line up in columns, folded or open.
- A filtered list read (e.g. `workloads --hostname x`) is no longer reported as truncated. The PCE's `X-Total-Count` is the size of the whole collection, not the number of matches, so a filtered read now counts as truncated only when it fills the row cap. Before this, filtered `enforce`/`relabel --yes` refused.

## v0.3.3 — 2026-10-17

### What's new

#### Workloads: group by app|env and filter by labels (#46)
- **`x` groups the Workloads screen** by the label keys the Traffic `g` picker uses (app, env by default).
  - Each group's header shows the workload count, a health bar, the health breakdown and the enforcement mix.
  - `Space` / `z` fold, `s` orders the groups (size, name, most unhealthy), and `Enter` on a header narrows the list to that group.
  - Workloads missing a label show as `(no app)` / `(no env)`.
- **`f` opens a label filter:** `app=shop,crm env=prod !role=db loc=-`.
  - A comma means any of those values, a space means every term must match, `!` excludes, and `key=-` means "no label for this key".
  - `Ctrl-N` completes keys and values; unknown values are flagged.
  - `:filter <expr>` works on Workloads too, and `c` clears every filter.
- Grouping and filtering use the list already loaded and make **no extra PCE requests**.
- **Changed key:** deleting an unmanaged workload on Workloads is now **`X`**, because `x` groups there. Objects and Policy keep `x`.

#### CLI
- `workloads --group [app,env]` prints workloads grouped, with per-group counts and health. JSON output has `groups[]`; CSV has a leading `group` column you can paste into `--label`.
- `workloads --label app=shop,crm` matches any of the listed values and is still one request. `!` and `key=-` terms are filtered locally. Same-key terms that can't both match return an empty result without any request.

#### Docs
- New help screenshot showing the grouped, context-first help.

### Install
- Homebrew: `brew upgrade illumio-tui` (tap: `brew tap alexgoller/illumio-tui https://github.com/alexgoller/illumio-tui`)
- Debian/Ubuntu: `sudo apt install ./illumio-tui_0.3.3-1_amd64.deb`
- RHEL/Rocky/Alma: `sudo dnf install ./illumio-tui-0.3.3-1.x86_64.rpm`
- Archives for every platform plus `SHA256SUMS` are attached below.

## v0.3.2 — 2026-10-17

### What's new

#### Flow detail in columns (#45)
In the Flows view, the detail pane now shows **SOURCE | DESTINATION | SERVICE** side by side:
- **Source and destination columns:** the name, then the IP, then the labels one per line in your configured key order (ignored keys hidden). IP-list endpoints show their list names instead.
- **Service column:** port/proto, process, user, the policy decision and the draft decision when it's loaded.
- **Full-width lines below:** first/last seen, bytes, rules, and the mark line.
- **Smaller terminals:** columns need at least 92×31. Below that you get the stacked layout, which now cuts long lines with `…` instead of wrapping, so the mark line stays visible.

#### Help that starts with the screen you're on (#44)
- `?` opens on the keys of the current screen or window (Flows, App-to-app, Blocked, Mesh, Treemap, Policy, Objects, Workloads, Events, the app view, the generator, the pending list, the marks list, an event detail), followed by the shared keys.
- Every screen's help page fits on one screen: the Flows page is 31 lines, down from 45.
- `Tab` / `S-Tab` step through the other groups, `a` shows them all, and `Esc` returns you to the window you were in.
- Detail lines (window steps, cap steps, draft-query notes) only show on a group's own page or with `a`.

### Install
- Homebrew: `brew tap alexgoller/illumio-tui https://github.com/alexgoller/illumio-tui && brew install illumio-tui` (or `brew upgrade illumio-tui`)
- Debian/Ubuntu: `sudo apt install ./illumio-tui_0.3.2-1_amd64.deb`
- RHEL/Rocky/Alma: `sudo dnf install ./illumio-tui-0.3.2-1.x86_64.rpm`
- Archives for every platform plus `SHA256SUMS` are attached below.

## v0.3.1 — 2026-10-17

### What's new

#### Traffic: app sections (#43)
- **App-to-app:** in section mode (`x`), `s` cycles the section order: connections ↓, app name A→Z, flows ↓. The panel title shows the current order.
- **Flows view:** `x` / `X` cycle flat → by source app → by destination app. Each section is a header with its totals and the raw flows below it.
  - `Space` / `z` fold sections, and `s` sorts them.
  - `Enter` on a header drills into that app. If a drill is already active, it only narrows it, for example from one app to an app pair.
  - A search or sort set in the flat view carries into the sections.
- Fast at 200k flows: grouping takes 54–75 ms, folding and sorting 2–11 ms. A speed check in CI guards these numbers.

#### Fixes
- **Events (#37):** the "truncated" marker now appears only when the row cap is actually reached. `X-Total-Count` on `/events` counts every retained event, so the marker used to show up too often.

### Install
- Homebrew: `brew tap alexgoller/illumio-tui https://github.com/alexgoller/illumio-tui && brew install illumio-tui`
- Debian/Ubuntu: `sudo apt install ./illumio-tui_0.3.1-1_amd64.deb`
- RHEL/Rocky/Alma: `sudo dnf install ./illumio-tui-0.3.1-1.x86_64.rpm`
- Archives for every platform plus `SHA256SUMS` are attached below.

## v0.3.0 — 2026-10-17

### Install

- **Homebrew:** `brew tap alexgoller/illumio-tui https://github.com/alexgoller/illumio-tui`, then `brew install illumio-tui`. The formula updates itself on every release.
- **Debian / Ubuntu:** `sudo apt install ./illumio-tui_0.3.0-1_amd64.deb` (arm64 too).
- **RHEL / Rocky / Alma / Fedora:** `sudo dnf install ./illumio-tui-0.3.0-1.x86_64.rpm` (aarch64 too).

The x86_64 packages carry the static musl build, so they run on Debian 11+ and EL8+. CI installs every package in Debian 12, Ubuntu 22.04, Rocky 9 and Alma 8 containers before the release is published. SHA256SUMS covers every asset.

### Flow marks (#40, #41, #42)

Mark flows you have looked at, kept in `marks.toml` next to your config, never on the PCE.

- **A mark** is a match (src, dst, service, decision, and optionally the draft decision), plus an action, an expiry and a **required comment** saying why the flows were marked.
  - Actions, strongest first: `critical`, `important`, `revisit`, `accept-blocked`, `reviewed`, `squelch`.
  - A highlight always beats a squelch.
- **Scope:** marks are portable across PCEs by label names, IPs and IP lists. A mark can also be scoped to one PCE; marks created in the TUI default to the current PCE.
- **Expiry:** default expiries per action (squelch and accept-blocked 90d, reviewed 30d, revisit 7d, critical and important never).
- **TUI:**
  - `m` on any traffic row opens a mark form filled in from the row, with a live preview.
  - Squelched flows leave every view, and each view says how many it hides; `M` shows them.
  - critical rows are red, important yellow, revisit get ◆, reviewed are dimmed; Blocked counts accept-blocked flows as accepted.
  - `:marks` lists marks to edit, delete (`x`) or extend (`e`).
- **CLI:**
  - `illumio-tui marks list|add|rm|export|import`.
  - `traffic`, `app` and `gen` apply marks: `meta.squelched`, `mark` fields on rows, `--show-squelched`, `--no-marks`.
  - Marks with a draft decision apply with `--draft`.
- **The policy generator ignores squelch**, because legit flows still need allow rules; its header shows how many of its flows are squelched.

### Policy generator

- **IP list rows (#38):**
  - IP peers in an existing IP list are grouped into one row per list.
  - Membership is checked against the draft ranges.
  - Lists that cover everything are never used.
  - The narrowest list wins, unless an existing rule in the target ruleset already names a list that covers the flow's port.
- **Fan-in consolidation (#39):**
  - A port reached by at least 5 peers, or 3 on infrastructure ports (DNS, Kerberos, LDAP, SMB, NTP, syslog, SNMP, monitoring), is flagged.
  - `c` cycles off, Any (destination enforces; checked managed peers keep their own rules) and All Workloads (one extra-scope rule; every VEN recomputes).
  - On the CLI: `gen --consolidate <port>/<proto>=any|all`.
- **Crash fix:** the generator no longer panics in a narrow terminal. Every screen is now render-tested at every width from 20 to 130 columns.

### Events

- **Detail navigation:** in the detail, `j`/`k` or the arrow keys step to the previous or next event; `Ctrl-d`/`Ctrl-u`, `PgDn`/`PgUp`, `Ctrl-e`/`Ctrl-y` and `g`/`G` scroll.

### Changes for scripts

- **`gen` JSON:**
  - List rows have `kind: "ip_list"` with `ip_list {name, href}` and `ips`, and `peer` is display text.
  - The `uses IP list` warning is kept but deprecated.
  - New `consolidations` array.
- **Traffic JSON:** with active marks, `meta.squelched` and `meta.marks` are present, and squelched rows are omitted unless `--show-squelched` is given.

### Docs

- A full documentation pass, with regenerated screenshots and tour.

Known: `events` can warn "result truncated" even when it returned fewer events than `max_results` (#37).

## v0.2.0 — 2026-10-16

### Policy generator (#33, #34, #35)

Proposes the allow rules one app needs, built from its own traffic, and writes them in one confirmed batch.

- **Open it:** `G` in the app view (`A`), or `:gen <app>/<env>`.
- **Granularity:** a slider with four stops: app·all, app·ports (the default), role·ports and role·all. `←`/`→` sets it for every row; `<`/`>` sets it for the selected row only.
- **Peers:** one row per peer app|env, one for traffic inside the app, and one per IP peer.
  - `Space` checks or unchecks a row. Same-env peers start checked; other envs and IP peers start unchecked.
  - Rows carry flags: env mismatch, uses IP list, ephemeral port, override deny, no target.
  - An IP peer already in an IP list uses that list; one in no list gets a new list.
- **Inbound** rules go into the app's ruleset. A new ruleset is named by the profile setting `[profiles.<name>.generator] ruleset_name`, default `RF-{app}-{env}`. **Outbound** (`o`, off by default) rules go into the peers' rulesets.
- **Preview:** the rule count and how many flows are covered or not, computed locally. `p` lists the exact rules.
- **Dedupe:** rules that already exist in the draft are not proposed again. Rules that differ only in one label key merge (role=web, role=db → one rule).
- **Incomplete traffic** (truncated, filtered, or one direction missing) is refused; `r` re-queries.
- **`Enter`:** one fresh read of the draft, then a single confirmation that shows every request. Writes must be on.
  - Requests are sent one at a time and the batch stops at the first failure.
  - Each request is written to the audit log.
  - Before each rule update the rule is read again, and the batch stops if someone else changed it.
  - A batch report is kept (`:batch`).
  - The draft decision is re-evaluated once after the batch.
- **CLI:** `illumio-tui gen <app>/<env> [--granularity …] [--outbound]` is a read-only dry run that prints the same proposal as JSON or a table.

### Events diff (#36)

- **Detail:** the event detail is now a readable change diff. Updates show only the changed fields, before (red) and after (green). Lists are compared item by item, and objects are shown by name. `J` toggles the raw JSON.
- **CLI:** `illumio-tui events --diff` prints the same per-event diff. It loads the cached name index, so 5 requests instead of 1. Plain `events` is unchanged.

### Also

- A rule write re-evaluates the draft decision on screen (#32).
- rustls 0.23.46 (RUSTSEC-2026-0346 to 0349).
- `?` opens the help in the app view and the generator; closing it returns there.
- `events` table and CSV: the actor column names more creator kinds, e.g. `service_account:ci-bot`.

Known: `events` can warn "result truncated" even when it returned fewer events than `max_results` (#37).

## v0.1.12 — 2026-10-15

### Security update

v0.1.12 is v0.1.11 with rustls, the TLS library the PCE client uses, updated from 0.23.45 to 0.23.46. That update fixes four advisories published on 2026-10-14:

- RUSTSEC-2026-0346: `Debug` impls for client session values expose TLS secrets
- RUSTSEC-2026-0347: buffered application data sent to the outer server after ECH rejection
- RUSTSEC-2026-0348: Brotli certificate decompression can allocate up to 1 GiB before authentication
- RUSTSEC-2026-0349: an unauthenticated ServerHello can downgrade later connections to classical key exchange

The v0.1.11 downloads were built with the affected version, so upgrade to v0.1.12. Nothing else changed. See [v0.1.11](https://github.com/alexgoller/illumio-tui/releases/tag/v0.1.11) for its features: a rule write re-evaluates the draft decision on screen (#32).

## v0.1.11 — 2026-10-14

> **Superseded by [v0.1.12](https://github.com/alexgoller/illumio-tui/releases/tag/v0.1.12):** these downloads use rustls 0.23.45 (RUSTSEC-2026-0346 to 0349). Use v0.1.12.

### What's new

**A rule write re-evaluates the draft decision on screen (#32).** Before, a successful write cleared the draft decisions and you pressed `D` again. Now a successful write re-evaluates them itself when:
- draft decisions were on screen (or a draft run was in flight), or
- the rule came from the Traffic `w` rule form, including from the app overlay.

How it works:
- It is the same request as `D`: one `update_rules` PUT per query, or two for a both-directions focus. If the cached draft still stands (for example after a label write), no PUT is sent.
- The status line shows the run. `Esc` cancels it; off the Traffic screen the status line says to cancel from there.
- With `Ctrl-R` (new ruleset, then rule) it runs once, after the rule. The draft stays on screen through the ruleset step and if you cancel the rule.
- A failed write, an unanswered write, or a write with no Traffic result open sends nothing. Where `D` would ask for confirmation, nothing runs, and the status says "draft cleared: D re-evaluates".
- A draft answer overtaken by a later write is never shown or cached, so two quick writes cannot bring back decisions from before the second one.

## v0.1.10 — 2026-10-14

Draft policy decisions, a better `w` rule form, Workloads filters, and two app-view fixes.

### New

- **Draft policy decision (#29).** `D` in Traffic, or `--draft` on `traffic query|groups|mesh` and `app`, shows for each flow what the **draft** policy would decide, next to the reported decision. Use it to check whether your draft rules fix the blocked flows before you provision.
  - The PCE's own Explorer `update_rules` re-evaluates the query you already ran. That costs one status check, one PUT on our own query, polls and one download. It changes no policy, needs no writes armed, and is not audited. Without a known query, `D` asks first, then re-runs.
  - Flows gets a `draft` column (e.g. `blocked → allowed`). Pairs, sections, Mesh and the app overlay show `would allow` / `would block` counts. `d` gains a `fixed by draft` step. The Result header shows `draft ✓ N fixed`.
  - In JSON each flow carries `draft_policy_decision`, and `meta.draft` summarises the changes. CSV gets `draft_allowed` / `draft_blocked`.
  - Draft answers are cached for 2 minutes and dropped on every policy or workload write.
- **Workloads filters (#31).** All filters are local and send no request; they combine with `h` and `/`.
  - `E` cycles enforcement (all → full → selective → visibility_only → idle → unmanaged).
  - `m` cycles managed → unmanaged.
  - `o` shows offline workloads only.
  - `u` shows unlabeled workloads (missing any scope key or role), with a `missing` column.
  - `c` clears the filters and `h`.
  - A filter line shows what is on, with the count.
  - CLI: `workloads --offline` and `workloads --unlabeled [keys]`, with `meta.local_filter` and `meta.warnings`.

### Improved

- **The `w` rule form (#30).**
  - The form now loads the draft rulesets itself (one counted, cached request). It pre-selects the ruleset whose scope covers the flow's destination (any of its labels; label groups are expanded) and says how many rulesets cover it.
  - Tab and `Ctrl-N` complete ruleset names.
  - `Ctrl-R` creates a new ruleset scoped like the destination. It checks the PCE first and never creates a duplicate: if the name exists, the form comes back naming it and its scope.
  - A form opened from the app overlay returns to it.
- **App overlay.**
  - A warning or error, such as `writes off; :writes on to enable` after `w`, now takes the status line instead of being cut off by the key hints.
  - The cursor always sits on the peer panel that the direction shows.
- **Tables** keep the selected row in view when a filter shrinks the list.
- Traffic and draft polling give up after 30 minutes (the PCE may still finish the query).

### Changed

- The CLI `--table` draft cell shows the change (`blocked → allowed`).

Download the archive for your platform below and check it against `SHA256SUMS`.

## v0.1.9 — 2026-10-14

Traffic accuracy and speed. This release fixes the findings of an external side-by-side test against workloader. **It has breaking changes; read the Changed and Removed sections.**

### Fixed

- **Inflated connection counts (#22).** Explorer returns stored aggregate records that overlap the query window, with lifetime counters; it does not split them by time. Backfill used to sum overlapping slices, which inflated connections about 12×. Flows are now merged by record (flow tuple + `first_detected`, the latest version wins) and never summed.
- **False "truncated" (#23).** A complete result was flagged truncated because `matches_count` was read as a total. Truncation is now `flows_count >= cap`, per region on a supercluster; a region that did not answer sets the new `incomplete` flag instead.
- **Unmanaged workloads (#24)** show and count as `unmanaged` (Workloads screen, `workloads` CLI, app overlay), not as the `visibility_only` the API reports. JSON keeps the raw `enforcement_mode` and adds an effective `enforcement`; CSV has both.
- **Unused labels (#25).** `labels --unused` and TUI `u` use the PCE's own usage flags (`labels?usage=true`): one request instead of four, and labels held by pairing profiles, permissions, enforcement boundaries etc. are no longer reported unused. The TUI shows what holds each label.
- **Speed (#28).** gzip downloads, polls capped at 3 s, and the CLI submits the query while label data loads. A one-day query on a demo PCE went from ~17 s to ~6 s. Large results get their own 300 s download timeout.

### New

- **Per-decision connections (#26).** `traffic groups` (pairs, `--services`, sections, `traffic query --blocked`) adds `allowed_conns`, `pot_blocked_conns`, `blocked_conns` and `unknown_conns` (they sum to `connections`) in JSON and CSV, appended after the existing columns. CLI tables print the unit: `flows` = flow records, `conns`/`bytes` = lifetime counts of those records.
- **Transmission filter (#27).** `--transmission unicast|broadcast|multicast` on `traffic query|groups|mesh` and `app`; a transmission field in the TUI filter and `:unicast`. The default stays all kinds (workloader defaults to unicast only).
- **`meta.cap`, `meta.cap_steps`, `meta.incomplete`.**

### Changed (breaking)

- **`--backfill` / `B` raise the cap** instead of splitting the window into time slices: a result truncated below 10,000 re-runs at 10,000, then 25k → 50k → 100k → 200k, each step above 10,000 confirmed (`y` / `--yes`). With a focus set, `B` raises the focused query's cap. `traffic_auto_backfill` raises to 10,000 at most.
- **`meta.covered`** is the window when the result is complete and `null` when it is truncated (it used to be a span estimated from `last_detected`).
- `--enforcement <mode>` now also sends `managed=true` (so workloads without a VEN stay out); `--enforcement unmanaged` sends `managed=false`. Invalid values exit 2.
- The TUI `:export` of App-to-app pairs now writes exactly the `traffic groups --csv` columns.
- CLI `--table` traffic output starts with a unit line.

### Removed (breaking)

- **The app timeline**: the overlay's "blocked over time" panel, `t`, spikes, `app --timeline` and the `timeline` section. Explorer's aggregated, lifetime-counted records allow no honest view over time. The Traffic summary sparkline went for the same reason.
- **Time-slice backfill**: `--max-slices` (now exits 2 with an explanation) and `meta.slices`.

Download the archive for your platform below and check it against `SHA256SUMS`.

## v0.1.8 — 2026-10-14

### New

- **App focus (#19).** In Traffic, `F` on any row narrows every view (Flows, App-to-app, Mesh, Treemap, Blocked) to that row's app; `X` cycles in → out → both; `Esc` clears it. `:focus <app>` opens a picker. The header shows `focus shop/prod · in`. Focusing sends nothing. If the loaded result was cut at the cap, `r` re-queries just that app (one query per direction, two for `both`, confirmed and cached); clearing the focus brings the full result back from memory.
  - CLI: `traffic query|groups|mesh --app shop/prod --direction in|out|both` puts the app's labels into the query, so the cap is spent on that app. `meta.queries` counts the queries.
- **App overlay (#20).** `A` on a row, or `:app <app>`, opens everything about one app: workloads with their enforcement modes and VEN health, inbound and outbound peers by service and decision, the rulesets that cover or name it (draft or active, label-group sub-groups followed), and an approximate blocked timeline. Opening it sends at most two counted requests (workloads, and rulesets if not cached); reopening the same app sends none. `Tab` moves between panels, `Enter` jumps to the peer's flows, the workload or the ruleset, `w` drafts an allow rule from a blocked peer, `r` reloads after asking.
  - CLI: `illumio-tui app <app> [--direction] [--section workloads|inbound|outbound|policy|timeline]`, one envelope with all sections (`--table` prints each; `--csv` needs `--section`).
- **Exact blocked timeline and spikes (#21).** In the overlay, `t` replaces the approximate timeline with an exact one: one blocked-only query per time bucket (1d = 24×1h, 1w = 28×6h, 4w = 28×1d, 30d = 30×1d, otherwise up to 24 buckets of at least 5 minutes), confirmed with the query count, run one at a time. `Esc` stops; `t` resumes with only the missing buckets. A bucket that hit the cap is a lower bound (`+`). Both timelines name the worst spike: a bucket at least 3× the median of the non-zero buckets (needs 4 non-zero buckets).
  - CLI: `illumio-tui app <app> --timeline`; above 24 queries it needs `--yes`.

### Changed

- **`traffic groups --app` now focuses the query.** `--app` used to keep one app's sections from the loaded flows; it now also narrows the query to that app, works without `--by` (pairs), and an app that matches no labels exits 2 with the closest names instead of printing empty sections. `--direction` conflicts with `--by`. `--backfill` is refused together with `--app`.
- Traffic results now keep separate cache slots for the full result, the focused sides and timeline buckets, so they no longer evict each other.
- Deny-rule fallback requests are only sent from the Policy screen.

Related: an "app profile" skill for agents is proposed in [illumio-mcp-skills#1](https://github.com/alexgoller/illumio-mcp-skills/issues/1).

Download the archive for your platform below and check it against `SHA256SUMS`.

## v0.1.7 — 2026-10-13

### New

- **App-to-app pairs by port/proto (#17).** In Traffic → App-to-app pairs, `o` switches to one row per source group → destination group → port/proto (`pairs · by service`). Hosts, IPs, users and processes collapse, so each row is close to one allow rule. No request: it regroups the result on screen.
  - `Enter` drills into the flows behind the row.
  - With writes on, `w` drafts the allow rule from the row: consumers = the row's source group, providers = its destination group (under the `g` keys), service = its port/proto, description naming the groups. A ruleset is pre-selected only when the `g` keys cover the profile's scope keys. A `-` slot (no label for a key) widens that side to the labels it has.
  - `:export` writes the same columns as the CLI: `src, dst, service, flows, conns, bytes, allowed, pot.blocked, blocked`.
  - CLI: `traffic groups --services` (table, `--json`, `--csv`); rejected together with `--by`, whose sections already list one row per service.
- **`d` filters the pairs (#18).** The decision filter (`all` → `blocked` = blocked and potentially blocked → `blocked only`) now applies to App-to-app pairs too, plain and by service. Rows count only the kept flows, rows with none disappear, the header shows the filter, and drills match the row. The treemap keeps every flow. CLI: `traffic groups --show blocked` now works for pairs and `--services`, e.g. `traffic groups --services --show blocked --csv` for the list of rules still to write.

### Changed

- `traffic groups --show` without `--by` used to be a usage error; it now filters the pairs. `--app` still needs `--by`.

Download the archive for your platform below and check it against `SHA256SUMS`.

## v0.1.6 — 2026-10-13

### ⚠️ Read first

- **The default traffic cap is now 10000 rows** (was 1000). Queries, `B` backfill and opt-in auto-backfill download up to ten times more per request by default. Set `traffic_max_results = 1000` in a profile's `[limits]` to keep the old start.
- **`traffic_max_results` no longer limits the cap.** It is now only the starting cap; `]`, `--max-results` and backfill can always go up to the hard maximum of 200000. Values of 0 or above 200000 are rejected when the config loads.
- Anything above 10000 still asks first (`--yes` in the CLI, `y` in the TUI). One `--yes` on `--backfill` allows up to 50 slice queries of up to 200000 rows each.

### New

- **CSV output (#15).** `--csv` on every command that prints a table: RFC 4180, CRLF, UTF-8 without BOM, a header row, full untrimmed columns (exact numbers, every label including ignored keys, lists joined with `; `), and the meta line on stderr so stdout is pure CSV. Commands without a table (writes, `health`, `provision`, `revert`, traffic `--dry-run`, ...) refuse `--csv` before sending anything. `--csv` conflicts with `--json` and `--table`.
- **`:export <path>` in the TUI (#15).** Writes the table on screen as CSV: filter, search and sort applied, folded sections included, cells as in the CLI's CSV. Refuses to overwrite an existing file; `:export! <path>` overwrites. Sends nothing to the PCE.
- **Traffic cap 10000, steppable to 200000 (#16).** `]` / `[` step 500, 1000, 2000, 5000, 10000, 25000, 50000, 100000, 200000; a profile value between rungs is its own rung. Auto-refresh stays off for a cap above 10000, since it would need a confirmation every run.

Download the archive for your platform below and check it against `SHA256SUMS`.

## v0.1.5 — 2026-10-12

### ⚠️ Read first

- **Breaking: `traffic query --blocked --json` changed shape.** It now returns `{"sections": [...]}` (the in+blocked preset of the new app-sections engine). Scripts reading the old blocked-group output need updating.
- **Policy revert is not yet verified against a live PCE.** It uses `PUT /orgs/N/sec_policy/delete` with a `change_subset` body and is covered by tests only. Run `illumio-tui policy revert ... --dry-run` against a lab PCE before using it for real.

### New

- **Policy revert (#7).** `policy revert [--all | targets] [--dry-run | --yes]` in the CLI. In the TUI pending overlay, `R` reverts the marked changes after you type `revert` and press Enter; the confirm lists each object and what happens to it.
- **Label-group diffs (#8).** The pending overlay shows label-group changes; `policy pending --diff` prints full diffs (a `diff` array in JSON with the flag).
- **App sections (#12).** App-to-app cycles pairs / in / out / both with `x`/`X`, `d` filters by decision, `Space`/`z` fold, Enter drills into a peer or app, `w` works on peer rows. Blocked is now a preset of the same view. CLI: `traffic groups --by/--show/--app`.
- **Coverage and backfill (#13).** The result header shows how much of the window the capped result covers (e.g. `covers 4m of 1d`). `B` proposes a cap that fits, or walks back in batches of up to 50 slices, one request at a time. Opt-in auto-backfill via `traffic_auto_backfill` / `ILLUMIO_AUTO_BACKFILL=1`. CLI: `--backfill [--max-slices 1..50]`, with `meta.covered` and `meta.slices`.
- **Filter editor completion (#14).** `Ctrl-N`/`Ctrl-P` complete labels, IP lists, workloads already loaded, services, protocols, decisions and windows in the traffic filter editor.

Download the archive for your platform below and check it against `SHA256SUMS`.

## v0.1.4 — 2026-10-12

**Full Changelog**: https://github.com/alexgoller/illumio-tui/compare/v0.1.3...v0.1.4

## v0.1.3 — 2026-10-12

**Full Changelog**: https://github.com/alexgoller/illumio-tui/compare/v0.1.2...v0.1.3

## v0.1.2 — 2026-10-12

**Full Changelog**: https://github.com/alexgoller/illumio-tui/compare/v0.1.1...v0.1.2

## v0.1.1 — 2026-10-12

**Full Changelog**: https://github.com/alexgoller/illumio-tui/compare/v0.1.0...v0.1.1

## v0.1.0 — 2026-10-12

**Full Changelog**: https://github.com/alexgoller/illumio-tui/commits/v0.1.0
