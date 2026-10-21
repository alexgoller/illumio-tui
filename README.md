# illumio-tui

Terminal UI and CLI for the Illumio Core PCE, built for administrators,
Illumio Professional Services, and agents. Reads by default; writes for labels, IP lists,
services, unmanaged workloads and policy (rulesets, rules, provisioning) are opt-in (see Writes).

Documentation: https://alexgoller.github.io/illumio-tui/

Screenshots and a short recording: https://alexgoller.github.io/illumio-tui/#screens

Workflows (marks, draft, explain, generator, bulk, readiness and enforce, policy search) with screenshots:
https://alexgoller.github.io/illumio-tui/#workflows

## Install

### Homebrew (macOS, Linux)

This repository is its own Homebrew tap:

    brew tap alexgoller/illumio-tui https://github.com/alexgoller/illumio-tui
    brew install illumio-tui

`brew upgrade illumio-tui` picks up new releases. The formula installs the
prebuilt binaries from the GitHub Release (static musl build on Linux x86_64).

### Debian, Ubuntu

Download the `.deb` for your architecture (`amd64` or `arm64`) from the
[release page](https://github.com/alexgoller/illumio-tui/releases), then:

    sudo apt install ./illumio-tui_0.5.0-1_amd64.deb

### RHEL, Rocky, Alma, Fedora

Download the `.rpm` (`x86_64` or `aarch64`) from the release page, then:

    sudo dnf install ./illumio-tui-0.5.0-1.x86_64.rpm

Both packages install `/usr/bin/illumio-tui` with no dependencies (the x86_64
binary is static, the arm64 one needs glibc 2.18 or newer) and are tested on
Debian 12, Ubuntu 22.04, Rocky Linux 9 and AlmaLinux 8 before each release.
Their sha256 is in the release's `SHA256SUMS`.

### Windows

**winget**. Available once the package is merged into
[winget-pkgs](https://github.com/microsoft/winget-pkgs) (pending):

    winget install AlexGoller.IllumioTUI
    winget upgrade AlexGoller.IllumioTUI

winget installs the MSI below, for the current user.

**MSI** (v0.4.1 and later). Download
`illumio-tui-<version>-x86_64-pc-windows-msvc.msi` from the
[release page](https://github.com/alexgoller/illumio-tui/releases) and
double-click it, or install it silently:

    msiexec /i illumio-tui-<version>-x86_64-pc-windows-msvc.msi /qn

It installs for the current user only (no admin rights) into
`%LOCALAPPDATA%\Programs\illumio-tui` and adds that folder to your user
`PATH` (open a new terminal to pick it up). A newer MSI upgrades the installed
version in place; uninstall from Settings > Apps (or `msiexec /x` with the
MSI), which also removes the `PATH` entry. The MSI is not code-signed, so
SmartScreen may warn when you open it: *More info*, then *Run anyway*. Its sha256 is
in the release's `SHA256SUMS`. Each release installs, upgrades and uninstalls
it on a Windows runner before publishing.

**Scoop**. This repository is its own bucket:

    scoop bucket add illumio-tui https://github.com/alexgoller/illumio-tui
    scoop install illumio-tui

`scoop update illumio-tui` picks up new releases (the manifest,
`bucket/illumio-tui.json`, installs the release zip and is updated on `main`
with each release).

### From source

From a clone of this repository (Rust 1.88 or newer):

    cargo install --path crates/illumio-tui

### Release archives

Prebuilt binaries are attached to each
[GitHub Release](https://github.com/alexgoller/illumio-tui/releases), one
archive per platform, the `.deb`/`.rpm` packages, and a `SHA256SUMS` file
covering all of them:

| Platform | Archive |
|---|---|
| Linux x86_64 (glibc) | `illumio-tui-<version>-x86_64-unknown-linux-gnu.tar.gz` |
| Linux x86_64 (static musl, any distro) | `illumio-tui-<version>-x86_64-unknown-linux-musl.tar.gz` |
| Linux arm64 | `illumio-tui-<version>-aarch64-unknown-linux-gnu.tar.gz` |
| macOS Apple Silicon | `illumio-tui-<version>-aarch64-apple-darwin.tar.gz` |
| macOS Intel | `illumio-tui-<version>-x86_64-apple-darwin.tar.gz` |
| Windows x86_64 | `illumio-tui-<version>-x86_64-pc-windows-msvc.zip` |

Download the archive for your platform and `SHA256SUMS`, then verify and
install (example: macOS Apple Silicon, version 0.5.0):

    sha256sum --check --ignore-missing SHA256SUMS      # macOS: shasum -a 256 -c --ignore-missing SHA256SUMS
    tar -xzf illumio-tui-0.5.0-aarch64-apple-darwin.tar.gz
    install -m 755 illumio-tui-0.5.0-aarch64-apple-darwin/illumio-tui ~/.local/bin/   # any directory on PATH
    illumio-tui --version

macOS may quarantine binaries downloaded with a browser; clear it with
`xattr -d com.apple.quarantine ~/.local/bin/illumio-tui`. On Windows, check
the hash with `Get-FileHash illumio-tui-<version>-x86_64-pc-windows-msvc.zip`
(compare with the line in `SHA256SUMS`), extract the zip, and move
`illumio-tui.exe` into a folder listed in your `PATH`.

### Shell completions

bash, zsh and fish completions come with the Homebrew formula and the
`.deb`/`.rpm` packages (`/usr/share/bash-completion/completions/illumio-tui`,
`/usr/share/zsh/site-functions/_illumio-tui`,
`/usr/share/fish/vendor_completions.d/illumio-tui.fish`); nothing to do. For
`cargo install`, a release archive, the MSI (also via winget) or Scoop, write the script
yourself:

    # zsh: a directory on $fpath, before compinit runs in ~/.zshrc
    mkdir -p ~/.zfunc && illumio-tui completions zsh > ~/.zfunc/_illumio-tui
    #   in ~/.zshrc: fpath=(~/.zfunc $fpath); autoload -Uz compinit && compinit
    # bash (needs the bash-completion package)
    mkdir -p ~/.local/share/bash-completion/completions
    illumio-tui completions bash > ~/.local/share/bash-completion/completions/illumio-tui
    # fish
    illumio-tui completions fish > ~/.config/fish/completions/illumio-tui.fish
    # PowerShell: add to your profile
    illumio-tui completions powershell | Out-String | Invoke-Expression

Every subcommand and flag completes (with descriptions in zsh and fish), and
so do enum values (`--direction`, `--format`, `--theme`, `events --severity`
...), files (`--config`, `--env-file`, `marks import`) and hosts (`--host`).
`--profile` and `profile use` complete the profile names of the config file
(or of `--config` given earlier on the line).

PCE names complete too, from a per-profile names cache on disk: labels as
`key=value` (`workloads --label`, `relabel --set`, `--add-label`,
`--remove-label`, `workloads add --label`, `policy search --label`), apps
(`readiness --app`, `enforce`, `app`, `gen`, `traffic --app`), workload
hostnames (`--hostname`, `workloads update`), traffic terms (`--src`, `--dst`,
`--exclude-src`, `--exclude-dst`: `key=value`, `wl:HOST`, `ipl:NAME`),
`svc:NAME` (`--exclude-service`), rule sides and scopes (`group:NAME`,
`iplist:NAME`, `workload:HOST`), label keys, services and IP lists.

    illumio-tui completions refresh           # fill it: one read each of labels, label groups,
                                              # IP lists, services and workloads; prints counts and age
    illumio-tui completions refresh --clear   # delete it; no request

The cache is `$XDG_CACHE_HOME/illumio-tui/<profile>/names.json` (else
`~/.cache/...`), mode 0600. It needs a profile from the config file whose
`host` and `org_id` are its own: when `--host`, `--pce-port`, `--org-id`,
`PCE_HOST`, `PCE_PORT` or `PCE_ORG_ID` (also from `--env-file`) name the PCE,
nothing is written and `completions refresh` refuses, so one profile's cache
never holds another PCE's names. The file records the host and org id; after
a profile's PCE changes, its old names are not offered and the next write
starts over. Besides `completions refresh`, the lists the CLI and the TUI
read anyway (`labels`, `label-groups`, `iplists`, `services`, `workloads`)
keep it current; nothing refreshes it on its own. A filtered or truncated
workloads list only adds hostnames and apps; they are pruned by a whole
list, `completions refresh` or `--clear`. The helper and `--clear` find the
profile from `--profile`, `ILLUMIO_PROFILE` in the shell or the config's
`default_profile`; an `ILLUMIO_PROFILE` set only in an `--env-file` is not
seen. Completion
itself never contacts the PCE: the hidden `illumio-tui __complete` helper
reads the config file and that cache file, and offers nothing when there is
no cache. The cache holds PCE object names in plain text; `completion_cache =
false` (top level of the config, or in a profile) keeps them off disk: nothing
is written or read. Regenerate the script after an upgrade if you wrote it
yourself.

### Cutting a release (maintainers)

1. Bump `version` in `[workspace.package]` of the root `Cargo.toml` and run
   `cargo check` so `Cargo.lock` follows.
2. Commit, then tag and push: `git tag v0.3.5 && git push --tags`.

The `Release` workflow builds all targets, builds the `.deb`/`.rpm` packages
(nfpm, `packaging/nfpm.yaml`) and install-tests them in Debian, Ubuntu, Rocky
and Alma containers, builds the Windows MSI (WiX, `packaging/windows/`) and
installs, upgrades and uninstalls it on a Windows runner, then publishes the
release with generated notes; it fails if the tag does not match the Cargo
version. Afterwards it renders `Formula/illumio-tui.rb` from
`packaging/homebrew/illumio-tui.rb.tmpl` and `bucket/illumio-tui.json` from
`packaging/scoop/illumio-tui.json.tmpl` with the new sha256s and commits both
to `main` as `brew, scoop: illumio-tui <version>`
(with the workflow's own `GITHUB_TOKEN`; no secret to set up, pre-release
tags such as `v1.2.3-rc1` are skipped). Pull before your next commit. Edit
the templates, not the formula or manifest. Never change the MSI's
`UpgradeCode` in `packaging/windows/illumio-tui.wxs`: it is what lets a new
MSI replace an installed one. Run the workflow manually
(`workflow_dispatch`) for a dry run that builds and tests everything and only
uploads artifacts; it also installs the committed Scoop manifest with Scoop
and checks its `autoupdate` against the release's `SHA256SUMS`.

winget: the workflow renders the winget manifests (`packaging/winget/`, schema
1.9.0) for the new MSI, validates them and installs, upgrades and uninstalls
with winget from them on a Windows runner (dry runs serve the MSI locally and
also render the manifests for the latest published release, as the
`winget-manifests` artifact). On a release tag (not a pre-release) it then
submits the update to `microsoft/winget-pkgs` with
[Komac](https://github.com/russellbanks/Komac), which forks winget-pkgs into
the token owner's account and opens the PR from there. That needs the
repository secret `WINGET_TOKEN`: a classic personal access token with the
`public_repo` scope. Without it, or while `AlexGoller.IllumioTUI` is not yet
in winget-pkgs (the first version is a manual PR), the step is skipped with a
notice. It runs after the release is published and never blocks it.

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
    completion_cache = false   # optional: no PCE names on disk for shell completion (also top level)
    # also: port (443), scheme ("https"), tls_verify (true), api_secret (plaintext, instead of _cmd)

    [profiles.prod.limits]
    traffic_max_results = 10000     # the starting traffic row cap (default 10000, at most 200000)
    traffic_default_window = "24h"
    traffic_auto_backfill = false   # true: re-run a truncated result below 10000 at 10000 (see TUI)
    requests_per_second = 2         # the request throttle (default 2)
    auto_refresh_min_seconds = 60   # the TUI's auto-refresh interval (default and minimum 60)

`profile add` writes a profile for you (local, no request), from flags or a dotenv file:

    illumio-tui profile add demo --from-env-file path/.env        # PCE_HOST, PCE_PORT, PCE_ORG_ID, API_KEY, API_SECRET
    illumio-tui profile add demo --from-env-file -                # paste the env text, then Ctrl-D
    illumio-tui profile add prod --host pce.example --org-id 1 --api-key api_... \
        --api-secret-cmd "op read op://vault/pce/secret" --writes --default

Flags override imported values (`--host`, `--port`, `--org-id`, `--scheme`, `--api-key`,
`--insecure`, `--writes`). The secret comes from the env file, `--api-secret-stdin` (one line),
`--api-secret` (lands in shell history; prefer the other two) or `--api-secret-cmd`, which wins
over an imported `API_SECRET` and keeps plaintext out of the file; a plaintext secret is stored
as `api_secret`. The file is written atomically, mode 0600 (directory 0700), keeping the other
profiles and unknown keys (not comments). An existing profile is replaced only with `--force`;
the first profile in a file, or `--default`, becomes `default_profile`. The output says whether
a key and secret are `set` (or `cmd`, `none`), never the secret.

`[profiles.<name>.labels]` and `[profiles.<name>.generator]` are under Label model below.
Environment variables outrank the profile. `PCE_TLS_VERIFY=false` skips certificate checks
like `--insecure`, `PCE_SCHEME` overrides the scheme; `ILLUMIO_PROFILE` picks
the profile and `ILLUMIO_TUI_THEME` the TUI theme; `ILLUMIO_TUI_COLORS=256` maps the theme's
24-bit colours to the xterm 256 (the default in Apple Terminal, which garbles 24-bit colour
before macOS 26; `truecolor` forces 24-bit); `ILLUMIO_WRITES` and
`ILLUMIO_AUTO_BACKFILL` (`1`/`0`, `true`/`false`) outrank the profile's `writes` and
`traffic_auto_backfill`. `--env-file .env` loads a dotenv file for one run. Connection settings can
also be overridden per run with global flags: `--host`, `--pce-port`,
`--org-id`, `--profile`, `--config`, `--insecure`, `--no-cache` (the
connection-port flag is `--pce-port`, not `--port`, because `traffic query`
has its own `--port` service-port filter).

## Getting started with the CLI

Connect with a dotenv file or a profile (above), then check it:

    $ illumio-tui --env-file pce.env health
    field   value
    ok      true
    host    demo100.illum.io
    org_id  5636114
    status  [{"generated_at":"2026-10-19T21:21:07+00:00","listen_only_m…
    writes  false
    default · 1 api call · 0.1 KB · cache none · 2102 ms

The last line (stderr) is what the command cost. First useful commands:

    illumio-tui labels --key app
    illumio-tui workloads --label app=infra-mgmt,monitoring        # comma: any of
    illumio-tui traffic query --window 24h --group app,env           # one Explorer query
    illumio-tui traffic groups --window 24h --services --csv > tuples.csv
    illumio-tui app infra-mgmt/Production --table                    # one app: peers, policy, workloads
    illumio-tui gen infra-mgmt/Production --table                    # the allow rules it needs (writes nothing)
    illumio-tui readiness --app infra-mgmt/Production                # verdict, gaps, pending changes
    illumio-tui traffic explain --src app=payment --dst app=ad --port 53 --proto udp

Output is a table at a terminal and JSON (`{"data": ..., "meta": ...}`) when piped;
`--table`, `--json` and `--csv` force one. JSON is made for `jq`:

    $ illumio-tui readiness --json | jq -r '.data.apps | sort_by(-.coverage.ratio) | .[:3][]
        | "\(.app)\t\(.verdict)\t\(.coverage.ratio*1000|floor/10)%"' | column -t -s $'\t'
    infra-mgmt/Production  needs_rules  90.3%
    soc/Production         needs_rules  89.5%
    monitoring/Production  needs_rules  86%

| Exit | Meaning |
|---|---|
| 0 | success |
| 1 | generic failure |
| 2 | arguments or configuration (an unknown label, an expensive query without `--yes`, a missing `--confirm-count`) |
| 3 | authentication |
| 4 | PCE error (a failed request, an incomplete write) |
| 5 | rate limited |
| 6 | write refused (writes not enabled; `enforce` on an app that is not ready) |
| 7 | flows blocked since an enforcement (`enforce --watch`) |

Writes: enable them (`writes = true` or `ILLUMIO_WRITES=1`, else exit 6 even for a dry run),
look with `--dry-run` (method, path and body; nothing sent), send with `--yes` (plus
`--confirm-count N` for bulk enforcement), and find each sent request in
`~/.local/state/illumio-tui/writes.jsonl`. Shell completions: see
[Shell completions](#shell-completions). More recipes with real output:
https://alexgoller.github.io/illumio-tui/#recipes

## Workflows

Eight everyday jobs as keys and commands. Screenshots and recordings of each, taken on a demo
PCE: https://alexgoller.github.io/illumio-tui/#workflows

1. **Find and silence noise (marks).** `m` on a row of any traffic view opens a mark form
   prefilled from the row: `squelch` hides known noise, `critical` / `important` / `revisit`
   flag, `reviewed` / `accept-blocked` dim; it previews its matches and asks why. `M` shows
   the squelched rows, `:marks` lists and edits. No request; `marks.toml` is shared by the
   TUI and the CLI (`marks add|list|export|import`).

       illumio-tui marks add --action squelch --src app=monitoring --dst any --service 5666/tcp --comment "Nagios NRPE"

2. **What would my draft do? (`D`).** On any traffic view: a `draft` column (`blocked →
   allowed`, `66 would allow`, `+40 allow -24 block`), `draft ✓ 409 fixed` in the Result
   header, `draft allows N` in Blocked. One PUT on the query you already ran, polls and a
   download; no policy change, no armed writes. CLI: `--draft`.
3. **Why is this flow allowed or blocked? (`E`).** The rules the PCE names for the flow,
   resolved through the draft policy (`allowed by 1 rule (draft)`: kind, ruleset, scope,
   consumers → providers, services), or `no rule allows this flow` and `w` to draft one.
   `Enter` opens the rule on Policy. No request. CLI:

       illumio-tui traffic explain --src app=pos,env=Staging --dst app=infra-mgmt,env=Production --port 7680 --proto tcp

4. **Policy from traffic (the generator).** `A` on a row (or `:app X`), then `G` (or `:gen X`):
   one row per peer, the slider (`h`/`l`) from `app·all` to `role·all`, `c` on a fan-in line
   (`123/udp from 7 peers (NTP?)`) folds it into one rule from Any or All Workloads with the
   catch spelled out, `Space` checks a peer, `p` previews every rule and the coverage
   (`27 of 27 flows`), `Enter` plans every write into one confirm (`4 requests: 1 rule_set, 3
   rules`), `y` sends, `n` cancels. CLI: `illumio-tui gen X` (read-only).
5. **Bulk labels and enforcement (Workloads).** `x` groups by app and env, `f` filters by label
   (`app=infra-mgmt,monitoring,soc env=Production`), `V` marks a group or everything shown,
   `Space` one workload, `M` sets the enforcement mode and `L` the labels of the marked
   workloads (writes armed). One confirm lists every change and request, wants the count
   typed for full or selective on several workloads, and counts the loaded traffic they would
   block; `d` runs a draft check on demand. CLI: `workloads enforce` / `workloads relabel` with
   `--dry-run`, then `--yes --confirm-count N`.
6. **From visibility to enforcement (`:readiness`, `:enforce`).** `:readiness` lists every app
   of the result on screen with its verdict, coverage, pending changes and warnings (`D` first
   for the draft). `Enter` on an app, `:enforce <app>` or `e` in the app overlay opens five
   steps: 1 Readiness, 2 Rules (`G` generator, `w` rule for the gap, `m` accept-blocked), 3
   Provision (`P`), 4 Enforce (`j`/`k` full or selective, `v` visibility, `Enter` the typed-count
   confirm), 5 Watch (`r` blocked since, `R` rollback preview). Opening it costs nothing once the
   workloads, pending changes and draft rulesets are loaded. CLI: `readiness` ranks every app
   (one 7-day query with the draft), `readiness --app X` names its gaps, `enforce X --to full
   --dry-run` refuses with exit 6 until it is ready, `--yes --confirm-count N` moves it and
   writes a change record, `enforce X --watch` exits 7 when something is blocked since,
   `enforce X --rollback` restores the record's modes exactly.
7. **Find rules fast (`f`, `t`, `I`).** On Policy, `f` searches every loaded ruleset
   (`app=ordering,pos`, `ipl:Corp`, `svc:443/tcp`, `wl:web01`, `disabled`, `deny`; no request) and
   shows one flat list; `Enter` opens the rule, `I` lists the workloads it matches per side
   (`Tab` switches), `t` enables or disables a rule or ruleset (draft, writes armed). CLI:

       illumio-tui policy search --label app=shop,crm --service 443/tcp --table
       illumio-tui policy rules impact <rule-href> --list providers --csv
       illumio-tui policy rules disable <rule-href> --dry-run

8. **Small things.** `Ctrl-U` clears the line in every text input; `Ctrl-N` / `Ctrl-P`
   complete without a request; `:export <path>` writes the table on screen as CSV; shell
   completions for bash, zsh and fish; Apple Terminal gets the theme in 256 colours by itself
   (`ILLUMIO_TUI_COLORS=256|truecolor` overrides).

**One app, start to finish:** `readiness` (infra-mgmt/Production is closest, 90% covered) →
`readiness --app` (two gaps, two pending rulesets) → TUI: `r`, `m` the noise, `D` → `E` on the
gap flow → `A`, `G`, `c`, `Space`, `p`, `Enter`, `y` → provision with `P` → `:readiness` (or
`readiness --app`) until `ready` → `:enforce infra-mgmt/Production`, step 4, `Enter` and the
typed count (CLI: `enforce --to full --dry-run`, then `--yes --confirm-count 2`) → `enforce
--watch` from cron → `R` or `--rollback` if needed.

## TUI

    illumio-tui                      # opens the interactive UI on the default profile
    illumio-tui --profile prod
    illumio-tui --theme light        # dark is the default

Five screens: `1` Traffic, `2` Policy, `3` Objects, `4` Workloads, `5` Events. The `:` prompt
takes `:traffic`, `:policy`, `:objects`, `:workloads`, `:events`, `:iplists` (Objects on its
IP lists tab), `:filter <expr>`, `:window <dur>`, `:profile <name>`, `:writes [on|off]`,
`:export <path>`, `:draft`, `:active`, `:unicast`, `:focus [app]`, `:app [app]`, `:gen [app]`,
`:readiness`, `:enforce [app]`, `:batch`, `:help` and `:q` (each described below). `:export <path>` writes the table on
screen as CSV (the rows shown, with the search and filters applied and in sort order; every
column, whatever the terminal width, each cell whole; `~/` is the home directory), the same
format as the CLI's `--csv`, and sends nothing; it refuses an existing file, `:export! <path>`
overwrites it. Cells match the CLI's for the same data (Workloads: every label, ignored keys
too, and `heartbeat_age_secs`). Sectioned views (App-to-app `in`/`out`/`both`, Blocked) export
every peer, folded sections included, in the columns of `traffic groups --by ... --csv`; Flows
by app exports every flow, folded sections included, in the Flows columns after a `section`
column naming its app; the Treemap has no table.
`?` shows the keys of where you are first (the traffic view, Policy, an event's detail,
the app overlay, the generator…), then related groups and the keys that work everywhere;
`Tab` / `S-Tab` (Shift-Tab) step through the other groups one at a time, `a` shows them all and
`j`/`k` scroll. `Esc` closes overlays, `q` or `Ctrl-C` quits. The mouse wheel
scrolls the current table and a click on a tab switches screen; on a terminal narrower than
the full tab row (69 columns) the tabs read `Traf`, `Pol`, `Obj`, `Wl`, `Ev`.

- **Traffic**: `f` edit the filter, `r` run (bypasses the cache), `a` auto-refresh, `s` sort,
  `/` search; a filter that would be expensive asks for confirmation first. In src, dst and
  service `,` is AND, `|` is OR and a `!` term excludes (`env=prod, !app=monitoring`,
  `tcp, !22/tcp`, `443/tcp | 80/tcp`, `!svc:SSH`): it goes into Explorer's own exclude list,
  so the PCE leaves it out at no extra cost. A `!` starts its term (`env=prod !app=x` asks for
  the missing `,`). A port range is `FROM-TO/PROTO` with tcp or udp, both ends included
  (`8000-8100/tcp`, `!8000-8100/udp`; `1 <= FROM < TO <= 65535`), sent as Explorer's
  `port`/`to_port`. `svc:NAME` stands for the service's ports and port ranges, each one more
  alternative (or one more exclusion), from the cached service list; a service of every
  protocol is a form error. That list is flat, so a `!` leaves its match out of the whole
  field, whichever `|` group it is written in, and a field of `!` terms only keeps every flow
  but those; the editor says so under the fields. Explorer takes labels, IP lists, workloads,
  IPs and ports/protocols there; `fqdn:` only on the destination side (a source `fqdn:`, to
  include or exclude, is a form error, and src completion does not offer it). In the filter
  editor `Ctrl-N` / `Ctrl-P` complete like a form (after a `!` too): labels, `ipl:` IP lists and the grammar's
  `ip:` `ipl:` `wl:` `fqdn:` in src and dst (`wl:` hostnames only once the Workloads screen
  has loaded them), protocols and services as `port/proto` or `from-to/proto`, the decision and transmission
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
  the name column) and, on `Enter`, a detail with both label sets and the bytes (from 92
  columns and 31 rows in source | destination | service columns, one label per line, IP
  lists where labels would be, `+N more` when they run out; stacked below that); `x` / `X`
  group it into app sections (`flat` → `by src app` → `by dst app`): one header per app (by
  the `g` keys; an IP list or FQDN by its name) with its flow and connection counts, then its
  flows in the table's sort (set it with `s` while flat). There `s` orders the sections, `Space`
  / `z` fold them (by source and by destination app each keep their own folds), `x` keeps the
  cursor on its flow, `Enter` on a flow opens its detail and on a header drills into the
  app's flows (under a drill it narrows it, to a pair where one drill can say it, and
  otherwise folds the section: `Esc` first); a flat search applies too (`flows /…` in the
  title); marks, the focus, `w`, `m`, `F` (the section's app first) and `:export` work on the
  sections, and an app whose every flow is squelched has none. **App-to-app**
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
  all, `s` orders the sections (connections ↓, app name A→Z, flows ↓; the rows inside keep
  theirs; the order shows in the title, e.g. `in · by app A→Z`, survives `g`, `r`, `+`/`-` and
  `f`, and `:profile` resets it; in `pairs` `s` sorts the table), `/` keeps whole sections, and
  `Enter` drills into a pair, a peer row or a whole app.
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
  confirmed step. One row per peer and side: intra-app, each app peer (`web/prod`), each
  existing IP list its IP peers are in (`IP list rfc1918 · 7 IPs`: one rule naming the list,
  the services of all its IPs; an IP in several lists goes to the narrowest, else the first
  by name, a list a rule of the target ruleset already names for those services first, and the
  row says so; the Any list
  and any list covering all addresses never group), each IP peer in no list
  and each unlabelled workload, with the services seen and the reported decisions (`✓`
  allowed, `✗` blocked or potentially blocked; with `D` loaded, `would allow N`). Intra-app
  and same-env peers start checked, the rest (IP rows too) unchecked with the reason (`⚠ env
  mismatch`, `⚠ needs an IP list`, which checking plans). `p` and the confirm list the IPs
  seen under each rule naming a list (`ipl:Corp: 7 IPs seen: 10.1.4.9, ... +4`). Past 25 IP
  rows on a side the rest fold into one read-only row (`12 more IPs: not proposed`). `Space`
  checks or unchecks a row, `Tab` moves between the inbound and outbound sections, `q` (like `Esc`)
  goes back. The slider `app·all ─ app·ports ─
  role·ports ─ role·all` (`←`/`→`) sets how coarse the rules are: all workloads of the app or
  one rule per role pair, All Services or the services seen; `<`/`>` give a row its own stop
  (`*`), `=` drops it. `o` adds the outbound section: rules to other apps go into the peers'
  rulesets (a new `RF-crm-prod` when none covers the peer), to IP peers into the app's own.
  **Fan-in ports.** An inbound port reached by many peers (at least
  `generator.consolidate_min_peers`, default 5, or 3 on an infrastructure port: DNS 53, NTP
  123, Kerberos 88/464, LDAP 389/636/3268, SMB 445, syslog 514, SNMP 161/162, monitoring
  agents 5666/9100/10050) gets a line above the inbound rows, `⚠ 5666/tcp from 9 peers
  (monitoring agent?) · c consolidates: off`, the cursor's first stop. `c` on it cycles that
  port `off → Any → All Workloads → off`: **Any** is one rule, the built-in Any IP list (never
  created; without it the option is skipped and says why) → the app on that port. It takes
  the IP side (list, IP and folded rows) and unchecked workload peers; checked workload peers
  (intra-app, app peers, workloads) keep their own rule for the port, since only the
  destination enforces an IP list consumer and their rule also gives their outbound (the line
  says `keeps N managed peer rules (their outbound)`, and that All Workloads covers managed
  sources in one rule). Unchecked workload sources it takes are allowed at the destination
  only, named in the warning (Explorer's flows carry no enforcement mode, so they are named,
  not sorted by mode). **All Workloads** is one extra-scope rule, which every VEN
  in the org recomputes (warned); IP peers keep their own rows. At an `·ports` app stop the
  rule opens the port on every workload of the app; at a role stop on the destination roles
  its flows reached (the app when one had no role), and the line says which (`→ role=db of
  shop/prod`). The rows it takes lose that port (a row left with none drops out); a row it
  covers shows `⚠ consolidated` first and, when unchecked, `[~]`, so it never looks
  excluded. Its flows count as covered, and the lines under the port give the catch first,
  then what the rule also allows (unchecked peers, unseen sources). At an all-services stop
  a checked peer's own rule still covers the port. An equal or broader existing rule makes it
  `already allowed by …`; override denies still count; a choice that yields no rule says why
  (`Any unavailable`, `All Workloads covers none of these peers (IP only)`). The choice is
  kept by port across rebuilds and forgotten when the port stops being a candidate; `p` and
  the confirm show the rule and its notes.
  The target line names the app's ruleset (`R` picks when several cover it, or a new one by
  `generator.ruleset_name`, default `RF-{app}-{env}`). Rules already in the draft show as
  `exists` and are not written; an existing rule with the same actors gets the missing
  services. A row whose rule an override deny rule blocks anyway shows `⚠ override deny:
  N` (N flows stay blocked), one whose ruleset cannot be named `⚠ no target`; flags that do
  not fit show the first and `+N`. The footer counts the rules and
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
  the window`. A Traffic filter other than window and cap (`svc 443`, `decision blocked`) is
  a gap too (`:filter` clears it), and so is a query still running; with `o` on, the `out`
  side must be loaded as well (`r` then runs both sides). While `Enter` plans, the rows are
  frozen; a plan whose proposal changed meanwhile is discarded, nothing sent. The confirm's
  status names what the fresh read changed (`1 rule is in the draft since the preview: not
  written`). The rows scroll with the cursor (`↑ N more above · ↓ N more below`), `j` runs on
  into the outbound section. `Esc` returns (to the app overlay when opened from it).
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
  `E` **explains** the flow under the cursor (Flows, an App-to-app pair holding one flow, the
  app overlay's Inbound and Outbound rows): which rule allows or blocks it. It shows the
  reported decision and, once `D` has run, the draft one, then each rule the PCE names for the
  flow after that draft evaluation, resolved through the loaded draft policy: allow, deny or
  override deny, ruleset and scope, consumers → providers, services, enabled. A rule the
  loaded policy does not hold keeps its href. With no rule it says `no rule allows this flow`,
  with the destination's enforcement from the loaded workloads (enforced: blocked; visibility:
  potentially blocked), and `w` drafts the allow rule. Explain sends nothing: before the draft
  evaluation it says so, and `D` there asks first (the same PUT, polls and download as `D`).
  `Enter` opens the selected rule on the Policy screen in the draft, its ruleset and rule
  selected (a policy or deny rules still to load are selected when they land; `Esc`, `d` or
  leaving Policy drops that); `Esc` goes back. With only the active policy loaded, rules
  resolve through it, marked `(active)`.
  `m` **marks** the row under the cursor (Flows, App-to-app pairs and sections, Mesh, Blocked,
  the app overlay's Inbound and Outbound): `squelch` hides known noise, `reviewed` and
  `accept-blocked` dim what you have judged, `critical`, `important` and `revisit` flag what
  matters (`●` `◆` in a gutter before the row; the strongest mark wins, so a squelch never hides
  a critical flow). The form is prefilled from the row (the labels under the keys in force,
  else `wl:`, `ipl:` or `ip:`; `Ctrl-A` every label of the endpoint, `Ctrl-T` another kind,
  `Ctrl-U` clears a field; `Decision` matches the reported decision, `Draft` the draft one once
  `D` has loaded it), defaults to this PCE and an expiry by action (squelch and
  accept-blocked 90 days, reviewed 30, revisit 7, critical and important never), previews what
  it matches in the loaded result, and needs a comment: why. `Enter` saves to `marks.toml`
  next to `config.toml`. Marks change nothing on the PCE and send no request. Squelched flows
  leave every view (a row goes when all its flows do; `1 ~1` beside a count says one is
  hidden) and the panel says `4 rows (9 flows) squelched · M shows them`; `M` shows them,
  dimmed `~`. Blocked counts the flows accepted as blocked apart (`10 blocked flows · 2
  accepted`). `:marks` lists every mark with its hits (`Enter` edit, `x` delete, `e` 30 more
  days, asking why again); expired marks stay in the file, greyed. The generator's rules
  ignore marks; its header counts the squelched flows.
- **Policy**: `d` toggle draft / active, `p` pending changes (`Enter` on one shows its diff against active), `Tab` switch panes, `Enter` rule
  detail, or on a ruleset load its deny rules. Rules are listed in evaluation order: override
  deny, allow, deny; the allow rules intra-scope first, then extra-scope (the `scope` column,
  counted in the pane's title bar, `3 intra · 1 extra`). With writes on, `n` / `e` / `x` create, edit and delete rulesets and rules
  in draft (`N` a deny rule), `t` enables or disables the selected rule (allow or deny) or
  ruleset (a draft change; disabled ones show dimmed with `✗`), and the pending overlay marks
  with `Space`, provisions with `P` and reverts the marked changes with `R`.
  `i` reads the workload list once (unless the Workloads screen already did) for rule impact:
  the rule detail says how many workloads the rule matches (`impact      12 providers · 3
  consumers`, `≥` when the list was truncated) and the size of each IP list it names, the
  rules pane's `P/C` column fills in (`1.2K/5.7K` from 1000 on), a note names what was not
  counted (virtual services, a side resolved as `virtual_services`), and a rule's confirm
  overlay shows the same for the rule about to be written. Impact uses the ruleset's scopes, unscoped consumers, label groups (with
  their sub-groups), exclusions, `all` and workload actors; IP lists are sized, never expanded.
  `I` (rules pane, rule detail or a search row, once the workloads are cached) lists the
  workloads behind the counts, providers first and `Tab` the consumers: hostname, labels,
  enforcement, online; `/` searches it, `:export <path>` writes it as CSV, `Enter` shows the
  workload on Workloads, `Esc` goes back (to the rule detail when opened from it), `?` the help.
  A side of IP lists says so (addresses, not workloads: nothing to list). No request.
  `f` searches rules across every loaded ruleset (draft or active, no request): `app=shop,crm`
  (a label in a scope, consumer or provider, directly or through a label group; a comma is any
  of; an exclusion such as a scope `All except app=shop` or a consumer `!role=web` does not
  count, nor does `All workloads`), `ipl:Corp`, `svc:PostgreSQL` or `svc:443/tcp` (inline ports
  and service objects that hold it, protocol-only entries such as all TCP ports included),
  `svc:tcp` / `svc:udp` / `svc:icmp` / `svc:icmpv6` (any entry of that protocol), `wl:web01` (a
  workload actor, named from the cached workloads: press `i` first), `href:sec_rules/9`,
  `disabled`, `deny`; terms apart by spaces all hold, `Ctrl-N` completes, `Ctrl-U` clears, `?`
  the help. An IP list or service name is exact, else the one name it starts (any case:
  `ipl:Any` is `Any (0.0.0.0/0 and ::/0)`); naming several or none is said, with the closest
  names. Service objects' process and Windows service entries are not modelled: only their port
  entries count. The screen then shows one flat list (`Search · app=shop` in its title:
  ruleset, enabled, kind, scope, consumers, providers, services; `:export` adds the href);
  `enabled` is the rule's own flag, `(ruleset off)` added when its ruleset is disabled (`(rs
  off)` below 120 columns). `Enter` opens the rule in its ruleset (`f` then `Enter` comes back),
  `t` toggles it, `I` lists its workloads, `c` clears the search. Deny rules count where loaded
  (inline, or fetched for a selected ruleset); on a PCE that lists none inline (older ones keep
  them at a per-ruleset endpoint) the meta says how many rulesets may hold deny rules not
  loaded.
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
  `f` edits a label filter on one line, applied locally like the others:
  `app=shop,crm env=prod !role=db` (a comma lists values any of which matches, a space
  separates terms that must all match, `!` excludes, `key=-` means no label for the key,
  quote a value with spaces: `loc="New York"`). `Ctrl-N` completes keys and values from the
  label list; a key or value no label has is accepted and named dimmed (`no match: unknown
  value app=shpo`); a malformed filter stays in the editor with the error and the old one
  holds. `:filter <expr>` on Workloads sets it too (elsewhere it edits the traffic source),
  and `c` clears it with the rest.
  `x` groups the list by the traffic group keys (`g`; `app,env` by default) and back: one
  section per group, its header `▾ shop | prod   12 wl  ██████  10 ok · 1 offline · 1
  unmanaged   full 8 vis 3 idle 1` (narrow terminals drop parts from the right), a missing
  label shown as `(no app)`. `Space` folds a section, `z` folds or unfolds all, `s` orders
  the sections (size ↓, name A→Z, unhealthy ↓; flat, `s` still sorts the table), and
  `Enter` on a header narrows the label filter to that group (`app=shop env=prod`, a missing
  label as `app=-`). Search, `E m o u h`, the table sort (within each section) and the
  detail pane keep working; grouping reuses the loaded list, no request. With writes on,
  `X` deletes an unmanaged workload here (`x` groups).
  `Space` marks the workload under the cursor (`●` in its row; on a group header `Space`
  still folds), `V` marks the group under the cursor (grouped) or every workload shown (flat),
  again unmarks them, and `Esc` clears the marks (after closing the detail pane; above 20
  marks it asks once: `Esc again clears N marks`); the summary line counts them (`3 marked`,
  `5 marked (2 hidden by the filter)`).
  Marks survive filters, search, sort and `x`. With writes on, `L` sets labels and `M` the
  enforcement of the marked workloads, else of the group under the cursor (on its header),
  else of the row: see [Bulk labels and enforcement](#bulk-labels-and-enforcement).
- **Events**: severity-coloured audit events, `f` edit severity / type / window, `S` sort by
  severity, `Enter` the event as a change diff: a header (time,
  type, severity and status, actor, API request and source), then one block per resource
  change. An update shows only the changed fields, `-` before in red and `+` after in green;
  lists (providers, consumers, services, scopes, IP ranges, labels) are diffed by element, so
  you see which item was added or removed. A create or delete lists the object's fields as all
  `+` or all `-`. Label, label group, IP list, service and ruleset hrefs read as names from the
  index already loaded or from the event itself (an unknown href stays an href; names are
  today's); notifications (a failed login, a refused request) and a failed request's errors
  are a short list. `J` toggles the raw JSON. In the detail `↑`/`↓` or `j`/`k` step to the previous or next event; `Ctrl-d`/`Ctrl-u`, `PgDn`/`PgUp`, `Ctrl-e`/`Ctrl-y` and `g`/`G` scroll. No extra request.

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
    illumio-tui workloads --label app=shop,crm --label env=prod --group
    illumio-tui workloads --label '!role=db' --label loc=- --group app,env --csv
    illumio-tui workloads --offline --enforcement full
    illumio-tui policy rulesets
    illumio-tui policy rules "Web-DB"
    illumio-tui policy rules "Web-DB" --impact --json
    illumio-tui policy rules impact /orgs/1/sec_policy/draft/rule_sets/7/sec_rules/3
    illumio-tui policy rules impact /orgs/1/sec_policy/draft/rule_sets/7/sec_rules/3 --list consumers --csv
    illumio-tui policy search --label app=shop,crm --service 443/tcp --table
    illumio-tui policy search ipl:Corp deny --csv > corp-deny.csv
    illumio-tui iplists --table
    illumio-tui policy pending
    illumio-tui policy pending --diff --table
    illumio-tui events --since 1h --severity err
    illumio-tui events --type sec_rule --diff
    illumio-tui traffic query --window 24h --src app=web --dst app=db --json
    illumio-tui traffic query --window 1h --group app,env
    illumio-tui traffic query --window 1h --blocked
    illumio-tui traffic query --dst env=prod --exclude-src app=monitoring --exclude-service 22/tcp
    illumio-tui traffic groups --window 1h --keys app,env,role
    illumio-tui traffic groups --window 1h --services --csv > tuples.csv
    illumio-tui traffic groups --window 1h --services --show blocked --csv > still-blocked.csv
    illumio-tui traffic groups --window 1h --by in --show blocked --app pos/PCI
    illumio-tui traffic groups --window 1h --app shop/prod --direction in
    illumio-tui marks add --action squelch --src app=monitoring --dst any --service 5666/tcp --comment "Nagios NRPE, known"
    illumio-tui marks list --table
    illumio-tui traffic groups --window 1h --by in --show-squelched
    illumio-tui app shop/prod --window 24h --json
    illumio-tui app shop/prod --direction in --csv --section inbound > inbound.csv
    illumio-tui gen shop/prod --window 24h --json
    illumio-tui gen shop/prod --granularity role-ports --outbound --table
    illumio-tui readiness --table
    illumio-tui readiness --app shop/prod --app crm/prod --json
    illumio-tui traffic mesh --window 1h --no-user --json
    illumio-tui traffic query --window 30d --dry-run
    illumio-tui traffic query --window 24h --backfill --json
    illumio-tui traffic explain --src app=web --dst app=db --port 5432 --proto tcp --json
    illumio-tui workloads --csv > workloads.csv
    illumio-tui traffic groups --window 24h --by in --csv > peers.csv
    illumio-tui profile add demo --from-env-file path/.env
    illumio-tui profile list
    illumio-tui profile use prod
    illumio-tui completions zsh > ~/.zfunc/_illumio-tui

`traffic explain` answers which rule allows or blocks the flows of one source, destination
and port: one Explorer query narrowed to them, evaluated against the draft (one POST, polls,
one PUT `update_rules`, polls, one download; it changes no policy), then the draft rulesets
(one GET, only when a flow names a rule) and the deny rules of a ruleset that does not carry
them inline (one GET each). JSON: `data.flows[]` with `reported`, `draft`, `verdict` and
`rules[]` (`kind` allow, deny, override_deny or essential, `ruleset`, `scopes`, `consumers`,
`providers`, `services`, `enabled`; `resolved: false` keeps an href the policy does not hold);
`meta.cost` says what it requested. `--src` or `--dst` alone is allowed: a wider query.
With no rule, the verdict says why a flow still passes (a destination that is not fully
enforced, or no workload). Since this feature, `traffic query` JSON `rules` and the flow
detail's rules line also list the deny hrefs some PCEs put beside `rules`
(`override_deny_rules`, `deny_rules`, `enforcement_boundaries`).

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
config, 3 auth, 4 PCE error, 5 rate limited, 6 write refused (by the guard, or `enforce` on an
app that is not ready), 7 flows blocked since an enforcement (`enforce --watch`,
`blocked_since_enforcement`).

`--csv` (beside `--table` and `--json`, exclusive with both) prints the same table as RFC 4180
CSV for spreadsheets: a header row, `,` separators, quotes only around cells holding `,`, `"`
or a line break, CRLF line ends, UTF-8 without BOM. The columns are the full set, untrimmed:
no `…` clipping, lists joined with `; ` where the table folds them (`3 hosts`, a label
group's member count), plain numbers (`6472`, not `6.5K`), `true`/`false`, and an empty cell
for nothing (`workloads` gives `heartbeat_age_secs` and every label, ignored keys too;
`events` every resource change, with `--diff` a `diff` column; `policy rules` adds each rule's `href`; `policy pending --diff`
a `diff` column, one diff line per line of the cell). `traffic groups --by ...` and
`traffic query --blocked` print one row per peer with `app`, `direction`, the peer and app host
counts and their names. The meta line always goes to
stderr, so a redirect holds only CSV. `--csv` sends the same requests as the table;
`workloads`, `labels`, `label-groups`, `services`, `iplists`, `events`, `policy rulesets`,
`policy rules`, `policy rules impact --list`, `policy search`, `policy pending`, `traffic query`/`groups`/`mesh`, `app` (with `--section`),
`gen` and `profile list` take it.

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

`--label` takes the app's label filter grammar, one term per flag (flags AND): `app=shop,crm`
is any of the values. The plain terms go into the one list request as the PCE's `labels`
parameter, whose outer list is OR and inner lists AND, so `--label app=shop,crm --label
env=prod` sends `[[shop,prod],[crm,prod]]` (label hrefs, from the cached label list): still one
request, and `--label k=v` alone sends exactly what it always did. More than 50 combinations
is exit 2 before any request. `!role=db` (exclude) and `loc=-` (no `loc` label) cannot be
asked of the PCE: they filter the answer locally and `meta.local_filter` names them
(`labels !role=db loc=-`). `--group [keys]` groups what is listed (after every filter) by those
label keys (comma-separated; alone, the profile's scope keys): the table prints a heading per
group (`shop | prod · 12 workloads · healthy 10, offline 2 · full 8, visibility_only 4`), JSON
puts `{"group_by": [...], "groups": [{"key": {"app": "shop", "env": null}, "filter": "app=shop
env=-", "count", "health", "enforcement", "workloads": [...]}]}` in `data`, and CSV keeps one
record per workload with a leading `group` column in the filter grammar (`app=shop env=-`),
largest group first. Grouping is local: no extra request.
Commands that print no table (writes, `health`, `policy rules impact`, `profile use`/`add`, a
traffic `--dry-run`) refuse it with exit 2 before any request.

`--exclude-src`, `--exclude-dst` (same syntax as `--src` / `--dst`) and `--exclude-service`
(`22/tcp`, `22`, `udp`, a range `8000-8100/tcp` or `svc:NAME`, the service's ports and
ranges, read from the cached service list), each repeatable, fill Explorer's exclude lists
(`--port 8000-8100 --proto tcp` includes a range the same way): `--dst env=prod
--exclude-src app=monitoring --exclude-service 22/tcp` is every flow into prod except from
monitoring and except SSH. With no `--src`, `--exclude-src` alone means every source but
those, and the query stays unbounded (only an include narrows it). `fqdn:` is a destination
filter only (exit 2 on `--src` or `--exclude-src`). `--dry-run`
prints the body with the `exclude` lists.

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
[--window W] [--max-results N] [--yes] [--consolidate <port>/<proto>=any|all ...]` proposes the allow rules the app's observed traffic
needs, as the TUI's generator would plan them, and writes nothing. `data` holds `app`,
`keys`, `window`, `granularity`, `outbound`, `target` (the app's ruleset: `name`, `href`,
`action` `existing`, `create`, `ambiguous` or `none`), `rows`, `ip_lists`, `rules`,
`consolidations` and `preview`. `rows` holds one row per
peer and side, its `kind` `intra`, `app` (each app peer), `ip_list` (each existing IP list
with its IP peers: `ip_list: {name, href}` and `ips`), `ip` (each IP peer in no list),
`workload` (each unlabelled workload) or `folded_ips` (the IP rows past 25, read-only);
`peer` is display text (`IP list rfc1918 · 7 IPs`), not a key to parse. Since #38 an IP in a
list is no longer its own row with `peer` its IP: read `ip_list` and `ips`; the `uses IP list
<name>` warning on those rows is kept for compatibility but deprecated and will go in a later
release. Each row has its `side`, its default
checkbox (`checked`: intra-app and same-env app peers; `checkable`), its `granularity`, `flows`, the services seen, the reported
decisions and its warnings (`env mismatch`, `needs an IP list`, `2 IPs also in rfc1918:
narrowest list`, `ephemeral port?`, ...); `rules` the merged rules at the chosen granularity, each with its
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
profile's `[profiles.<name>.generator] ruleset_name` (below). `consolidations` lists the
inbound fan-in ports (always, consolidated or not): `service`, `peers` (count) and
`peer_names`, `reason` (`min_peers` or `infra:<name>`, e.g. `infra:dns`), `mode` (`off`,
`any`, `all`), `rule` (1-based into `rules`, `null` when off or when it yields no rule), `to`
(the workloads it opens the port on), `also_allows` and `warnings`; a row the rule takes
ports from has `consolidated: ["5666/tcp"]`, and that rule's `rows` are the rows whose
flows it takes. `--consolidate 5666/tcp=any` (repeatable; `all` for All Workloads, `off`)
switches one of them to its one rule, as `c` does in the TUI; a malformed value or a port
given twice exits 2 before any request, a port that is not listed (or `any` without the
built-in Any IP list) exits 2, and `off` on a port that is not listed only warns.

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
through the cache; `meta.truncated` includes it. With `--list providers|consumers` it prints
that side's workloads instead (`hostname`, `name`, `labels`, `enforcement`, `online`; CSV adds
the `href`; JSON `{"side", "count", "note", "truncated", "workloads": [...]}`); a side of IP
lists lists nothing and `note` (stderr with `--table`/`--csv`) says why. `iplists` gives every list a `size`: `v4` and
`v6` address counts as strings, `unbounded` (the included `0.0.0.0/0` or `::/0`, else `null`),
`ranges` and `fqdns`.

`policy search` finds rules across every ruleset (draft, or `--active`) by what they
reference, from one rulesets read and the name index (cached): `--label key=value[,value]`
(any of the values, in a scope, a consumer or a provider, directly or through a label group
and its sub-groups; exclusion actors, such as a scope `All except app=shop` or a consumer
`!role=web`, and `All workloads` (`ams`) do not count), `--ip-list NAME`, `--service NAME` or
`--service 443/tcp` (`443` alone: tcp or udp; an inline port or range that holds it, or a
service object with such an entry, `All Services` and protocol-only entries included),
`--service tcp|udp|icmp|icmpv6` (any entry of that protocol), `--workload NAME` (hostname or
name, one cached workloads read; or the href's id), `--href TEXT`, `--disabled` (the rule or
its ruleset is off) and `--deny`; every flag repeats where it makes sense and all must hold.
The TUI's search terms work as arguments too (`policy search app=shop ipl:Corp deny`). An IP
list or service name is matched exactly, else as the start of exactly one name, any case
(`--ip-list Any` finds `Any (0.0.0.0/0 and ::/0)`); a name that starts several is exit 2
listing them, one that names nothing is exit 2 with the closest names. Service objects'
`windows_services` (process and Windows service names) are not modelled: only their port
entries count. Rows carry the ruleset, kind, `enabled`, the ruleset's scopes, consumers,
providers and services (CSV adds the rule's `href`), each ruleset's rules in evaluation order.
`enabled` in the table and CSV is the rule's own flag (`on`, `off`), with `(ruleset off)` when
its ruleset is disabled; JSON has `enabled` and `ruleset_enabled`. Deny rules are searched as
the ruleset list carries them (no per-ruleset reads); when no ruleset carries any inline (older
PCEs keep them at a per-ruleset endpoint) `meta.warnings` (stderr with `--table`/`--csv`) says
the rulesets may hold deny rules not loaded.

### Readiness

`illumio-tui readiness [--window 7d] [--app shop/prod]... [--max-results N] [--yes] [--no-marks]`
judges, per app (a group of the profile's scope keys), whether its workloads could move to
enforcement under the draft policy. It runs one Explorer query over the window (default 7d;
unicast only) re-evaluated against the draft policy (the `--draft` path: one PUT
`update_rules`, more status polls, one download, no policy change), then reads the workloads,
the pending changes and the draft rulesets (one GET each, cached) and applies `marks.toml`.
With `--app` (repeatable) the one query is narrowed to those apps on either side (the apps'
labels in sources and destinations, joined with `or`); without it every labelled app in the
result is judged. An app with a `-` slot (a workload missing a scope label) is listed but
cannot be named with `--app`.
The cost rules of `traffic query` apply (a window over 7d or a cap over 10000 needs `--yes`,
refused before any request), and `meta` counts the requests (`api_calls`, `queries`).

Each app reports `workloads` (total, managed, unmanaged, the enforcement mix, offline VENs,
managed workloads without the tier label), `evidence` (window, flows, connections, `little`
under 7 days or 20 flows), `coverage` (connections in, out and total: allowed by the draft,
`would_block`, and `accepted`, would-block flows a `squelch` or `accept-blocked` mark covers;
`ratio` is allowed plus accepted over all; `undecided` flows, without a draft decision, count
as would-block), `pending` (every change to a ruleset whose scope covers the app, its rules and
the objects they use; in any other ruleset, unscoped or scoped elsewhere, a change counts when
an enabled rule of it reaches the app's workloads, e.g. a peer's rule consuming the app with
unscoped consumers, and an object when such a rule uses it), `gap_count`, `verdict` and
`warnings`. A flow inside the app counts once, as inbound. The verdict is the first that
applies: `not_evaluated`, `needs_rules` (would-block flows), `needs_provision` (pending
changes: enforcement uses the active policy), `ready`; `enforced` when every managed workload
is in full enforcement already. Warnings (little evidence, missing tier
labels, offline VENs, unmanaged workloads, no managed workloads, a truncated or incomplete
result, undecided flows) never change the verdict.
With `--app` the output adds each app's `gaps`, the would-block flows as policy tuples (peer
group and service, largest first), and its pending changes. `--table` prints the apps table
(and with `--app` the gaps and pending tables); `--csv` prints the apps table with full
columns.

### Flow marks

`illumio-tui marks list|add|rm|export|import` reads and writes `marks.toml` (next to
`config.toml`, mode 0600, rewritten under a lock from a fresh read; comments are not kept, and
the first rewrite of a commented file keeps it in `marks.toml.bak`). It sends no request and
needs no profile, except `add --pce` without a host. `add` needs `--comment` (why) and prints
the new id; `export` prints `{"marks": [...]}` (or `--format toml`), which `import` reads.
Traffic commands and `app` apply the marks: squelched flows are left out (`--show-squelched`
keeps them, `--no-marks` ignores the file), rows carry `mark` (`{"action", "id"}`; aggregated
rows also `flows`, the per-action `marks` and the `squelched` records they hide), tables a
last `mark` column, and `meta.squelched` counts the rows and flows left out. `gen` proposes the
same rules either way and reports `meta.squelched`. An invalid file is a warning (the command
runs without marks); `add`, `rm` and `import` refuse to write it (exit 2), a held lock exits 3.

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
    illumio-tui workloads enforce full --label app=shop --yes --confirm-count 14
    illumio-tui workloads relabel --set env=prod --label app=shop --yes

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
name matching more than one workload is refused ("use the href"); a managed workload's labels
change only with `--managed-labels` (below).

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
    illumio-tui policy rules add Web-Tier --provider app=web --consumer app=shop --consumer role=web --service 443/tcp --extra-scope --dry-run
    illumio-tui policy rules update /orgs/1/sec_policy/draft/rule_sets/7/sec_rules/9 --add-consumer '!iplist:Guest' --dry-run
    illumio-tui policy deny-rules add Web-Tier --provider all --consumer iplist:Any --service 23/tcp --override --yes
    illumio-tui policy rules disable /orgs/1/sec_policy/draft/rule_sets/7/sec_rules/9 --dry-run
    illumio-tui policy rulesets enable Web-Tier --yes
    illumio-tui policy provision --all --dry-run
    illumio-tui policy provision Web-Tier --description "web tier v2" --yes
    illumio-tui policy revert Web-Tier /orgs/1/sec_policy/draft/ip_lists/21 --dry-run
    illumio-tui workloads update web-01 --add-label env=prod --managed-labels --yes

A new allow rule is intra-scope (consumers inside the ruleset's scope);
`--extra-scope` (or `--unscoped-consumers`) lets its consumers come from
anywhere, and `rules update --extra-scope|--intra-scope` (or
`--unscoped-consumers|--scoped-consumers`) switches it. `rules add` prints
the TUI form's scope warnings on stderr (`warning: matches role=web in every
app, env`), never fatal and from what it reads anyway: with a ruleset href
(no rulesets read) only the extra-scope check runs.

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

`policy rules enable|disable <href>` flips one rule's `enabled` (an allow
rule's `sec_rules` href or a deny rule's `deny_rules` href) and `policy
rulesets enable|disable <name|href>` a ruleset's; the `PUT` carries
`enabled` only, and a rule or ruleset already in that state is `skipped`.
Like every policy write it is a draft change until provisioned. (`rules
update --enabled/--disabled` does the same with other edits.)

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
`n` creates, `e` edits the selected row and `x` deletes it (on Workloads `X`, as `x` groups
there), and every request is shown in a
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
request. `Ctrl-U` empties the line in every text input, as in a shell: the
`:` prompt, `/` search, the filter editors, the pickers and the typed
confirmations.
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

`events --diff` prints each event's resource changes under its row, as the Events detail
shows them (a `create` / `update` / `delete` title, then `+` / `-` lines), and a failed
request's errors and the notifications. With `--json` each event gains
`changes: [{change_type, resource_type, resource, href, fields}]`, each field
`{field, before, after}` for a value or `{field, added, removed}` for a list, only when
`--diff` is given; errors and notifications are in the event itself (`action.errors`,
`notifications`). Like `policy pending --diff`, it reads the name index (labels, label groups,
IP lists, services; cached, counted in `meta.api_calls`) to show names for hrefs; ruleset
names come from the event. Plain `events` sends no extra request. Names are today's: after a
rename, older events show the new name.

### Bulk labels and enforcement

One change to many workloads, sent as `PUT /orgs/N/workloads/bulk_update` in chunks of 100.
On Workloads (with `:writes on`), `L` opens a form with one field per label key (the
profile's scope keys and tier key, then the other keys the targets have), each showing
the targets' common value or `(mixed: N)`: empty keeps the key, a value sets it (replacing
that key's label), `-` removes it; `Ctrl-N` completes values and `Ctrl-A` creates a typed
label that does not exist yet (one POST before the bulk requests, its href put in them).
Managed and unmanaged workloads take labels alike; only `labels` is sent. `M` picks the
enforcement mode (idle, visibility_only, selective, full) and the visibility level
(default: each workload keeps its own) and shows the change as you move
(`14 workloads: 9 visibility_only → full · 3 already full (skipped) · 2 unmanaged
(skipped)`); unmanaged workloads are skipped.

`Enter` opens one confirm listing every workload's change and every request. Full or
selective on more than one workload, and any change to more than 25 workloads, needs the
workload count typed before `y` is accepted (the confirm says which: `full on 3 workloads`,
`30 workloads (more than 25)`); the library refuses such a plan without it too.
For an enforcement change the confirm says how many blocked and potentially blocked flows
of the loaded traffic these workloads have, in (to them) and out (from them to other hosts),
locally with no request; and `d` runs a draft check on demand: one Explorer query of their
traffic in and out (each workload a source or a destination) over the traffic filter's
window, evaluated against the draft policy (POST, polls, the draft PUT, the download;
`Esc` cancels it), listing what would be blocked, split in and out, with the requests it
took (`≈`: the client's count meanwhile). It never runs by itself.

`y` sends: first the re-read, past the cache (one GET per workload for up to five, else
one list read); a workload whose labels, enforcement, visibility or VEN changed since the
plan is skipped and reported (never overwritten), one that is gone too. Then the label creates, then the chunks one at a time,
throttled; each chunk's answer is read per workload and the send stops after a chunk with
any failure. A 2xx answer without per-workload results (a 204, a job object), or one that
leaves a workload out, is `not answered` for those workloads: it may have applied, the send
stops, nothing is retried, and the caches are dropped as after a write. Every request is one audit entry (a chunk's lists each workload's outcome);
quitting, `:profile` and `:writes` wait while it is sent. The status line says
`updated 12 of 14 · 1 failed · 1 changed since planned, skipped`; the report (opened when
something failed, else `:batch`) lists each request and each workload. A full success
clears the marks; the workload list reloads once (and the labels, a draft result on
screen and the usage flags as after any relabel).

The CLI takes the list filters (`--label` any-of / `!` / `-`, `--enforcement`, `--hostname`,
`--name`, `--ip`, `--managed`, `--online`, `--unhealthy`, `--offline`, `--unlabeled`) to
select the workloads: one list request (plus the labels list, one read, when `--label` or
`relabel` names labels), then the plan; `--dry-run` prints each workload's change and every
chunked request body, `--yes` sends it (creates, chunks, as above), neither is exit 2. The
CLI sends right after its own selection read, so it reads nothing again (the plan says
`no re-read (the selection read ran N ms before, in this run)`); a selection older than five
seconds is re-read as in the TUI. A plan with nothing to change prints its summary and exits
0. A truncated selection
(more workloads match than the read returned) is refused with `--yes` (exit 2): narrow the
filters or raise `--max-results`.

    illumio-tui workloads enforce full --label app=shop --label env=prod --dry-run
    illumio-tui workloads enforce full --label app=shop --label env=prod --yes --confirm-count 14
    illumio-tui workloads enforce visibility_only --visibility flow_drops --enforcement idle --yes --confirm-count 40
    illumio-tui workloads enforce visibility_only --all --dry-run
    illumio-tui workloads relabel --set env=prod,role=- --label app=shop --dry-run
    illumio-tui workloads relabel --add-label loc=eu --remove-label env=dev --hostname web --yes

`enforce` and `relabel` refuse without a filter (exit 2, `invalid`, before any request)
unless `--all` is given; `--all` with filters is allowed and the filters still apply, so
`--all` only says "every match is meant" (a dry run needs it as well). Full or selective on
more than one workload, and any change to more than 25 workloads, needs `--confirm-count N`
matching the number of workloads that change (the dry run prints it as `count`); a mismatch
or a missing count is exit 2 before any write. A label `relabel` names must exist (the CLI does
not create labels). A dry run's `data` is `{dry_run, change, summary, count,
needs_confirm_count, workloads, skipped, reads, requests}`; a sent one is the report:
`steps` (each request: status, HTTP status, error) and `items` (each workload: `updated`,
`failed` with the PCE's reason, `not_sent`, `unanswered`, `changed` or `missing` by the
re-read, `skipped` by the plan). Anything not updated that was planned exits non-zero after
the report, by its first reason: 2 (`invalid`: a refusal or a request the PCE called
invalid), 3 (`auth`), 5 (`rate_limited`), 6 (`write_refused`: the write guard), else 4
(`write_incomplete`: a failed request, refused workloads, an unknown outcome; `audit_failed`: a write applied but its audit entry could not be written). Workloads
skipped by the re-read are a warning while some were updated; none updated because every
one changed is exit 4 (`write_skipped`).

### Enforce an app

`illumio-tui enforce <app>` takes one app (`shop/prod`, one value per scope key) from ready to
enforced, watches what is blocked afterwards, and rolls back exactly. The flow:

    illumio-tui readiness --app shop/prod                      # verdict, gaps, pending
    illumio-tui enforce shop/prod --to full --dry-run          # readiness again, the plan
    illumio-tui enforce shop/prod --to full --yes --confirm-count 14
    illumio-tui enforce shop/prod --watch                      # exit 7 when something is blocked
    illumio-tui enforce shop/prod --rollback --dry-run
    illumio-tui enforce shop/prod --rollback --yes --confirm-count 14

`--to full|selective [--visibility LEVEL] [--window 7d] [--force]` runs readiness for the app
first (one Explorer query narrowed to the app, re-evaluated against the draft, and the cached
workloads, pending changes and draft rulesets: as `readiness --app`). A verdict other than
`ready` or `enforced` is refused with exit 6 (`write_refused`) before any write, dry run or
not; the message lists the blockers (the would-block tuples, the pending changes). A `ready`
verdict on a truncated or incomplete readiness result is refused the same way (flows are
missing: narrow `--window`/`--max-results`). `--force` moves it anyway and says so in
`meta.warnings` and the record (`forced`). The move is the bulk
enforcement of the app's managed workloads with every guard of `workloads enforce` (armed
writes, `--dry-run` or `--yes`, `--confirm-count`, the audit log, one `bulk_update` per 100,
a re-read when the workloads came from the cache). `data` holds `readiness` (the app's
report), `plan` (dry run) or `bulk` (the report), and `record`.

Each move writes a **change record**,
`$XDG_STATE_HOME/illumio-tui/enforcements/<app>-<UTC time>.json` (else
`~/.local/state/illumio-tui/enforcements/`): `app`, `profile`, `pce`, `org_id`, `at` (when it
was sent), `to`, `readiness` (`verdict`, `coverage`, `window`, `flows`, `forced`) and each
planned workload's `href`, `hostname`, `before` and `after` (`mode`, `visibility`) and
`result` (`updated`, `failed`, `unanswered`, `not_sent`, `changed`, `missing`). No credentials.
The directory is checked writable before any request (else exit 6, nothing sent); the record
is written before the send with every result `pending` and rewritten with the results after
it, so a run killed mid-send still leaves one. Should the rewrite fail, the record goes to
stderr whole and the command exits 1.

`--watch [--since <record path|RFC 3339 time>]` runs one Explorer query, blocked flows only,
the app on either side, from the change time (the app's newest record by default) to now, and
lists them as tuples (direction, peer, service, flows, connections). Flows a `squelch` or
`accept-blocked` mark accepts are counted (`accepted`) but do not alert (`--no-marks`: they
do). Exit 0 when there are none, 7 (`blocked_since_enforcement`) when there are, after the
report; none found in a truncated or incomplete result is exit 4. A record named with
`--since` must be of this app, PCE and org. A window over 7d needs `--yes`. It runs only when asked; nothing polls.

`--rollback [--record <path>]` takes the app's newest move record made on this PCE and org
(never a rollback record) unless one is named; it refuses (exit 6) a record already rolled
back (name it with `--record` to do it again), a record of another PCE or org, and a rollback
record unless `--force` (that re-applies a move without readiness). It reads the app's
workloads once past the cache and restores each workload still as the move set it to its
`before` mode, and its `before` level only when the move set a level, one `bulk_update` per
`(mode, visibility)` (per 100), with the same guards (the typed count covers the whole
rollback). A workload changed by someone else since, gone, not moved by that change, or whose
`unanswered`/`pending` move did not apply (`not applied`) is skipped and listed in `skipped`,
never overwritten. The rollback writes its own record with `rollback_of`; no record is ever
deleted.

In the TUI, `:readiness` lists every app of the result on screen with its verdict, workloads,
flows, would-block connections, coverage, pending changes and warnings (`s` sorts, `/`
searches; `not evaluated` until `D`, which goes through `D`'s own confirm). `Enter` on an
app, `:enforce <app>` (`:enforce` alone: the picker) or `e` in the app overlay opens the
**enforcement overlay**: five steps on the left, the selected one's detail on the right
(`Tab` or `1`-`5`). 1 Readiness: the numbers, the verdict, the warnings. 2 Rules: the gaps;
`G` opens the generator for the app, `w` the allow-rule form for the gap's tuple, `m` the
mark form as `accept-blocked`. 3 Provision: the app's pending changes; `P` provisions them
through the usual preview. 4 Enforce: `j`/`k` full or selective, `v` the visibility level,
`Enter` the bulk confirm (typed count); an app that is not ready asks for the word `enforce`
first. The change record is written exactly as the CLI writes it (the directory checked
before any request, the record before the send, the results after it; a record that cannot
be updated opens loudly with the record to keep). 5 Watch: `r` runs the blocked-since query
(one query, only on `r`), `R` re-reads the workloads and previews the rollback, `y` sends it;
a record already rolled back is refused. Opening either overlay sends nothing when the
workloads, pending changes and draft rulesets are loaded (else one cached read each, counted
in the header); after a write the steps re-evaluate from the reloaded data, and nothing
refreshes by itself.

### Policy editing in the TUI

On the Policy screen, in the draft scope, the same three keys edit policy:
on the rulesets pane `n` opens a new ruleset (Name, Description, Enabled
`y`/`n`, Scopes), `e` edits the selected one and `x` deletes it; on the
rules pane `n` adds an allow rule and `N` a deny rule to the selected
ruleset, `e` edits the selected rule (allow or deny) and `x` deletes it;
`t` on either pane flips the selected ruleset's or rule's `enabled` (one
`PUT` of `enabled`, shown first, `y` sends it; it then shows in Pending). The
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

An allow rule's **Scope** is `intra` (consumers inside the ruleset's scope)
or `extra` (consumers anywhere; `unscoped_consumers` on the wire); `Space`
switches it (`i`, `e` pick one). A line under Consumers says what they
match as you type: `role=web inside app=shop env=prod`, or extra-scope
`role=web in any app, env` (the scope keys the consumers do not pin; a
label group pins its key); IP lists and workloads, which no scope narrows,
read `wherever they are`. It warns, without blocking, of the classic
mistakes: an extra-scope rule whose consumers pin no scope key (`matches
role=web in every app, env`), an intra-scope consumer label every ruleset
scope rules out (`never matches: app=crm is outside the ruleset scope`; an
excluded scope label or a label group on the key rules out nothing), and
intra-scope in a global ruleset (`the same as extra-scope here`); the
confirm's status line repeats them. Switching rewrites Consumers so the
rule keeps its meaning: to extra adds the flow source's scope labels (a `w`
form; `all` becomes them), to intra drops only the labels every ruleset
scope pins to the same value (redundant there; any other label stays).
Switching back restores what the switch took or added, so there and back
never changes the rule. IP lists, workloads, groups and exclusions are
never touched. The rule detail says `intra-scope` or `extra-scope`.

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
tier (`app=crm, env=prod, role=web`) with Scope `extra`; an endpoint without
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
`consolidate_min_peers = 5` (the default; at least 2) is how many inbound peers on one
port make it a fan-in line; infrastructure ports need 3.

## PCE protection

Every request is user-caused, throttled (2/s, burst 5; `requests_per_second` in the profile),
cached (labels, label groups, IP lists and services 15 minutes; policy and workloads 5;
events 1), and counted.
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
    PCE_LIVE_WRITES=1 cargo test -p pce --test live_writes -- --test-threads=1   # non-production PCE only

`scripts/live-smoke.sh --env-file <path> [--workload <hostname>]` runs both
live suites against the PCE the env file names (sourced, never printed) and
prints a pass/fail summary. The write suite exercises every write path the
`pce` crate has: labels, services, IP lists and unmanaged workloads (create,
update, delete), a ruleset and rule lifecycle with `enabled` toggles, a batch
with placeholders, and a draft revert of its own change; a provision is only
planned, never sent. Everything it makes is named `illumio-tui-smoke-<ts>` and
removed again, also when a check fails. The label test uses the existing key
`role`: a PCE refuses labels of keys it has no dimension for. `--workload`
names one VEN-managed workload (exact hostname) for the bulk test, which
changes its visibility level, its enforcement to `visibility_only` and one
label to another existing value, puts each back and checks the original state
by a fresh read; without it the bulk test is skipped. Use it only on a PCE and
a workload you designate for testing.

Design: `docs/superpowers/specs/2026-10-10-illumio-tui-design.md`.

## License

GPL-3.0-only. See `LICENSE`.
