# illumio-tui

Terminal UI and CLI for the Illumio Core PCE, built for administrators,
Illumio Professional Services, and agents. Reads by default; writes for labels, IP lists,
services, unmanaged workloads and policy (rulesets, rules, provisioning) are opt-in (see Writes).

Documentation: https://alexgoller.github.io/illumio-tui/

Screenshots and a short recording: https://alexgoller.github.io/illumio-tui/#screens

Workflows (marks, draft, explain, generator, bulk, readiness and enforce, policy search) with screenshots:
https://alexgoller.github.io/illumio-tui/#workflows

What changed in each release: [CHANGELOG.md](CHANGELOG.md), also on the site at
https://alexgoller.github.io/illumio-tui/changelog.html

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

    sudo apt install ./illumio-tui_0.5.7-1_amd64.deb

### RHEL, Rocky, Alma, Fedora

Download the `.rpm` (`x86_64` or `aarch64`) from the release page, then:

    sudo dnf install ./illumio-tui-0.5.7-1.x86_64.rpm

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
install (example: macOS Apple Silicon, version 0.5.7):

    sha256sum --check --ignore-missing SHA256SUMS      # macOS: shasum -a 256 -c --ignore-missing SHA256SUMS
    tar -xzf illumio-tui-0.5.7-aarch64-apple-darwin.tar.gz
    install -m 755 illumio-tui-0.5.7-aarch64-apple-darwin/illumio-tui ~/.local/bin/   # any directory on PATH
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
`--remove-label`, `workloads add --label`, `policy search --label`,
`pairing-profiles create --label`), apps
(`readiness --app`, `enforce`, `app`, `gen`, `traffic --app`), workload
hostnames (`--hostname`, `workloads update`), traffic terms (`--src`, `--dst`,
`--exclude-src`, `--exclude-dst`: `key=value`, `wl:HOST`, `ipl:NAME`),
`svc:NAME` (`--exclude-service`), rule sides and scopes (`group:NAME`,
`iplist:NAME`, `workload:HOST`), label keys, services and IP lists, and the
object of `where-used label|label-group|iplist|service`.

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
2. Write the release notes at the top of `CHANGELOG.md` as
   `## v0.5.7 — 2026-10-26` (an em dash, the release date), their sections
   as `###` (`### New`, `### Changed`, `### Fixes`). Reference issues as
   `#NN`; on the site they link to the issue.
3. Run `cargo xtask changelog`, which renders `docs/changelog.html` (the
   site's changelog page) from `CHANGELOG.md`. CI fails while the page is
   stale (`cargo xtask changelog --check`).
4. Commit, then tag and push: `git tag v0.5.7 && git push origin main --tags`.

The `Release` workflow builds all targets, builds the `.deb`/`.rpm` packages
(nfpm, `packaging/nfpm.yaml`) and install-tests them in Debian, Ubuntu, Rocky
and Alma containers, builds the Windows MSI (WiX, `packaging/windows/`) and
installs, upgrades and uninstalls it on a Windows runner, then publishes the
release; it fails if the tag does not match the Cargo version. The release
body is the tag's section of `CHANGELOG.md` (`scripts/release-notes.sh v0.5.7`
prints it, its headings back at `##`); a tag without a section gets GitHub's
generated notes instead, as before. Afterwards it renders `Formula/illumio-tui.rb` from
`packaging/homebrew/illumio-tui.rb.tmpl` and `bucket/illumio-tui.json` from
`packaging/scoop/illumio-tui.json.tmpl` with the new sha256s and commits both
to `main` as `brew, scoop: illumio-tui <version>`
(with the workflow's own `GITHUB_TOKEN`; no secret to set up, pre-release
tags such as `v1.2.3-rc1` are skipped). Pull before your next commit. Edit
the templates, not the formula or manifest. Never change the MSI's
`UpgradeCode` in `packaging/windows/illumio-tui.wxs`: it is what lets a new
MSI replace an installed one. Run the workflow manually
(`workflow_dispatch`) for a dry run that builds and tests everything and only
uploads artifacts (the Cargo version's `CHANGELOG.md` section among them, as
`release-notes`); it also installs the committed Scoop manifest with Scoop
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

    [profiles.prod.events]
    exclude_types = ["=user.login", "agent.*"]   # event types hidden by default (see Events)

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
profiles and unknown keys (not comments). An existing profile is changed only with `--force`,
which sets the connection settings given and keeps the rest (limits, labels, generator, other
keys; a new secret source replaces the old one). The first profile in a file, or `--default`,
becomes `default_profile`. Only flags and the env file count, not the real environment; dotenvy
expands `$VAR` in values that are not single-quoted (a warning says so for `API_KEY` and
`API_SECRET`). The output says whether a key and secret are `set` (or `cmd`, `none`), never the
secret.

`[profiles.<name>.labels]` and `[profiles.<name>.generator]` are under Label model below.
`[profiles.<name>.events] exclude_types` lists event types the Events screen and `events` hide
by default, each a substring, a `*` glob or an exact type (`=user.login`), as a `!term` of the
type filter (below; an `=` only at the start);
`events --all-types` shows them, and in the TUI the Type field starts with them, so `f` can
drop one for the session. The tool only reads it, never writes it.
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
| 7 | flows blocked since an enforcement (`enforce --watch`), or flows the draft would newly block (`policy impact --fail-on-blocked`) |
| 8 | a risky service exposed (`exposure --fail-on-exposed`) |
| 9 | snapshots differ (`snapshot diff --fail-on-change`) |

Writes: enable them (`writes = true` or `ILLUMIO_WRITES=1`, else exit 6 even for a dry run),
look with `--dry-run` (method, path and body; nothing sent), send with `--yes` (plus
`--confirm-count N` for bulk enforcement), and find each sent request in
`~/.local/state/illumio-tui/writes.jsonl`. Shell completions: see
[Shell completions](#shell-completions). More recipes with real output:
https://alexgoller.github.io/illumio-tui/#recipes

## Workflows

Nine everyday jobs as keys and commands. Screenshots and recordings of each, taken on a demo
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
   (`app=ordering,pos`, `ipl:Corp`, `svc:443/tcp`, `wl:web01`, `disabled`, `deny`, where the
   label sits with `scope:` / `src:` / `dst:`, exclusions with `!`, `-term` to leave out,
   `intra` / `extra`; no request) and shows one flat list; `Enter` opens the rule, `I` lists the
   workloads it matches per side (`Tab` switches), `t` enables or disables a rule or ruleset
   (draft, writes armed). CLI:

       illumio-tui policy search --label app=shop,crm --service 443/tcp --table
       illumio-tui policy search src:role=web dst:role=db extra --table
       illumio-tui policy rules impact <rule-href> --list providers --csv
       illumio-tui policy rules disable <rule-href> --dry-run

8. **Small things.** `Ctrl-U` clears the line in every text input; `Ctrl-N` / `Ctrl-P`
   complete without a request; `:export <path>` writes the table on screen as CSV; shell
   completions for bash, zsh and fish; Apple Terminal gets the theme in 256 colours by itself
   (`ILLUMIO_TUI_COLORS=256|truecolor` overrides).

9. **What is new since? (compare, `c`).** `c` on Traffic asks for a baseline length (`7d`;
   `Ctrl-N` / `Ctrl-P` step) and queries the same filter over the window just before the
   result's start. Every view but the Treemap marks its rows `NEW`, `CHG` (another set of
   decisions, the `was→now` column saying `allow→pot.bl`; on a plain pair what changed:
   `+1 svc · allow→block`) or `GONE` (dim, only in the
   baseline), the Result header counts the rows the view shows: `vs 7d before · 12 new · 3
   changed · 5 gone`, `n` shows only one kind, `C` leaves. Change checks, new talkers, new
   blocked flows (Blocked view). CLI:

       illumio-tui traffic groups --by pairs --services --since 2026-10-23T18:00 --baseline 30d --yes
       illumio-tui traffic query --window 24h --baseline 7d --changes new --csv > new-flows.csv

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
`:readiness`, `:enforce [app]`, `:exposure` (also `9`), `:used` (where the selected object is used, as `W`), `:hygiene` (unused objects, workloads missing labels), `:impact` (what provisioning the draft would change, see [Provision impact](#provision-impact)), `:batch`, `:snapshot [label]` (see [Snapshots](#snapshots)), `:save <name>`, `:filters` (`:fl`), `:load <name>`,
`:default <name>|off`, `:help` and `:q` (each described below; saved filters under [Saved filters](#saved-filters)). `:export <path>` writes the table on
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
  destination, `Esc` zooms out. None of this sends a request.
  **Exposure** (`9` or `:exposure`, not in `v`'s cycle) answers which risky services reach
  what, and under which decision: RDP, SMB, NetBIOS, RPC, WinRM, SSH, Telnet, VNC and the
  database and directory ports (the profile's `[exposure]` list, see
  [Exposure](#exposure)). It runs one query, through the cost guard, with the filter's window,
  cap and sides and every port of the list as its services, in its own cache slot. The
  traffic result on screen is kept aside, and `Esc` on the view puts it back, filter and view
  too, without a request (a write's re-evaluation clears its draft decisions there too). Each row is a destination group (the `g`
  keys) and a risky service: its ports seen, its sources (`intra`, the group's own: both ends
  labelled workloads with every group key set and equal; anything else is from outside; `N apps`,
  other labelled groups; `N ext`, unlabelled hosts and IP lists, `(any)` for the Any list,
  `(internet)` for a public address) and its flows per decision. Exposed rows (allowed from
  outside the group, in the bad colour) come first, then potentially blocked, blocked and
  intra-only ones; the title has the selected service's summary (`rdp: 12 destinations · 4
  exposed (allowed from other apps) · 3 potentially blocked · 5 blocked`). `Enter` drills into
  Flows (the risky service's flows into that group), `E` (or `e`) explains the row's busiest
  exposing flow from the loaded policy (no request; `D` in the overlay asks first, as
  everywhere), squelched flows are left out (`M` shows them; the title counts the exposed rows
  they hide, `1 exposed squelched (M)`); `r`, `B`, `+`/`-` and `]`/`[` re-run the exposure
  query in its slot. `D`, `F` and `c` work on its result as on any other, in their usual
  slots (the draft, focus and baseline ones), so after `Esc` the result put back may need its
  `D`, `F` or `c` query again (the view itself has no `Δ` column). `v` moves to the other views of the exposure result; `9` comes back.
  With writes on, `w` turns a
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
  **New since.** `N` proposes only for the traffic that is new: it asks for a baseline length
  as Traffic `c` does (`7d`; `Ctrl-N`/`Ctrl-P` step) and runs the generator's own focused query
  (`in`, plus `out` with `o` on) over exactly the window before the result's start, in its own
  cache slots, through the usual cost guard, counted in the header. Each recent flow record is
  judged as Traffic compare judges it (endpoints, port, proto, process, user and direction, no decision):
  only those whose key the baseline lacks feed the rows, the rules and the fan-in lines; the
  rest are left out, so a row with no new flows disappears. The header (and a line under the
  target) says `new since 7d before · 40 flows seen before`. Seen-before flows are left out of
  the proposal, neither covered nor uncovered: the footer reads `covers 3 of 5 new flows · 2 not
  · 40 seen before` (`nothing new since 7d before: 40 flows seen before` when nothing is), `p`
  ends with `left out: 40 flows seen before (baseline 7d)`. A truncated baseline warns (`new
  over-reports`). Rules already in the draft still
  show as `exists`. `N` again shows every flow again without a request; `o` or another filter
  ends it (`N` compares again), `r` keeps it. It needs complete data first.
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
  `c` **compares** the result with a baseline: a one-line prompt for its length (`7d`;
  `Ctrl-N` / `Ctrl-P` step through the window steps, `Ctrl-U` clears) and `Enter` runs the same
  filter over exactly `[start − 7d, start)` of the result on screen, one query in its own cache
  slot (the same filter and window again is a cache hit; above 7d or a 10k cap it asks first).
  Rows are matched on each view's key without the decision and without `first_detected`
  (Flows: both endpoints by workload, else IP, port/proto, process, user, direction; App-to-app
  and Blocked: source group, destination group, service; Mesh: its row key) and the views
  gain a first `Δ` column: `NEW` (only recent), `CHG` (in both, another set of decisions; the
  `was→now` column after `Δ` says which, on `CHG` rows only and in the warning colour, with
  short names: `allow→pot.bl`, `allow→allow,block` (`allow`, `pot.bl`, `block`, `unk`); it is
  as wide as the longest one, the other columns giving up room so the table fits at 80
  columns, cut with `…` beyond that; the Flows detail's title says it too; `:export` leaves
  it out, its `change` column has the transition), `GONE` (only in the baseline, dim, with
  the baseline's values), blank for the same. A key is judged on every decision, then the
  view's decision filter picks its rows: Blocked shows a flow allowed then and potentially
  blocked now as `CHG`, and a gone row only when it was blocked. A plain App-to-app pair (`o`
  off) is judged by its service keys: `NEW` when all are new, `GONE` when all are gone, `CHG`
  when any is new, gone or changed (`o` shows which); its `was→now` says what changed instead
  of the sets: services that came or went (`+2 svc −1 svc`) and, for the services on both
  sides, `allow→block` when their decision sets differ, else `~2 svc` for those whose
  decisions changed (one allow→block, another block→allow), joined with ` · ` (`+1 svc ·
  allow→block`), never two equal sides and never empty. A table none of whose rows has a transition (Flows with nothing
  changed) has no `was→now` column, so the others keep their widths. The Result header says
  `vs 7d before 10-26 09:00 · 12 new · 3 changed · 5 gone` (where the baseline ends, UTC) for
  the view on screen, counting the rows that view shows (App-to-app: pairs with `o` off,
  service rows with it on), so `n` shows exactly as many rows as it names; a section of gone
  rows only reads `gone`, and section headers count recent rows only. `n` cycles all → new →
  changed → gone, `C` leaves.
  New means not seen in the baseline window at Explorer's record granularity: a record whose
  lifetime spans the cut-off comes back from both queries and reads as the same (or changed),
  and the counts stay each side's own lifetime counters, never subtracted. Nothing refreshes
  on its own: `r` re-runs the recent query and keeps the baseline; another filter or window (`f`,
  `+`, `-`, a saved filter) ends compare mode (`filter changed: c compares again`); only `c`
  queries the baseline (the status says `filter changed: c compares again` while the new query
  runs and with its result). A truncated or incomplete side shows in the header in the warning
  colour (`baseline truncated: new/gone over-report`) and `B` backfills whichever side is
  truncated, each rung asked as usual. Squelch hides keys on both sides alike: a key with a
  squelched record on either side leaves both (a mark on the draft decision, which the baseline
  never carries, cannot make it gone; `M` shows them on both); a focus narrows both sides; a focused result (`r` on a partial
  focus) refuses `c` and ends compare mode. The Treemap has no `Δ`.
- **Policy**: `d` toggle draft / active, `p` pending changes (`Enter` on one shows its diff against active), `Tab` switch panes, `Enter` rule
  detail, or on a ruleset load its deny rules. Rules are listed in evaluation order: override
  deny, allow, deny; the allow rules intra-scope first, then extra-scope (the `scope` column,
  counted in the pane's title bar, `3 intra · 1 extra`). With writes on, `n` / `e` / `x` create, edit and delete rulesets and rules
  in draft (`N` a deny rule), `t` enables or disables the selected rule (allow or deny) or
  ruleset (a draft change; disabled ones show dimmed with `✗`), and the pending overlay marks
  with `Space`, provisions with `P` and reverts the marked changes with `R`. `I` in the
  pending overlay (or `:impact` anywhere) shows what provisioning the draft would change: the
  pending objects, each pending ruleset's rules added, removed and changed with the workloads
  they match active → draft, and one traffic query re-evaluated against the draft (the flows
  that would become blocked, then allowed, per app pair). It loads once when asked (the pending
  list, the rulesets and lists when a ruleset is pending, one draft query, through the cache);
  reopening it shows the summary with its age and sends nothing; `r` in it loads it again past
  the cache; `j`/`k` scroll, `Esc` goes back to the pending list. See [Provision impact](#provision-impact).
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
  count, nor does `All workloads`), `!env=prod` (an exclusion actor of the label, and only
  those), `ipl:Corp`, `svc:PostgreSQL` or `svc:443/tcp` (inline ports
  and service objects that hold it, protocol-only entries such as all TCP ports included),
  `svc:tcp` / `svc:udp` / `svc:icmp` / `svc:icmpv6` (any entry of that protocol), `wl:web01` (a
  workload actor, named from the cached workloads: press `i` first), `href:sec_rules/9`,
  `disabled`, `enabled`, `deny`, `allow`, `intra` / `extra` (an allow rule's consumers inside
  its ruleset's scope or anywhere; deny rules are neither). A label term without a prefix looks
  anywhere; `scope:` looks in the ruleset's scopes only (`scope:global`: a scope without
  labels), `src:` (or `consumer:`) in the consumers, `dst:` (or `provider:`) in the providers,
  and those two also take `ipl:NAME`, `wl:NAME` and `all` (All Workloads); a deny rule's scope
  is its ruleset's. A leading `-` keeps the rules a term does not match (`-deny`,
  `-scope:env=dev`, `-svc:443/tcp`). Terms apart by spaces all hold, `Ctrl-N` completes (after
  `scope:`, `src:`, `dst:`, `!` and `-` too), `Ctrl-U` clears, `?` the help. For example:
  `src:role=web dst:role=db extra` (extra-scope rules from web to db), `scope:app=payment
  -svc:443/tcp` (payment rules not covering 443), `src:!env=prod` (consumers that say *All
  except prod*), `src:all -deny` (allow rules open to All Workloads). An IP list or service name is exact, else the one name it starts (any case:
  `ipl:Any` is `Any (0.0.0.0/0 and ::/0)`); naming several or none is said, with the closest
  names. Service objects' process and Windows service entries are not modelled: only their port
  entries count. The screen then shows one flat list (`Search · app=shop` in its title:
  ruleset, enabled, kind, scope, consumers, providers, services; `:export` adds the href; the
  meta says how many rules matched of those loaded, `3 of 40 rules`);
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
  `W` (or `:used`) on a row of any tab shows where the object is used, before you change or
  delete it: the title names it (`where used: env=Production`, with the total), then
  the sections with their counts, empty ones left out: Rulesets (their scopes), Rules
  (`Shop-Prod · #2 · provider · allow`, `deny` or `override deny`, plus `excluded` when it
  names the object as an exclusion, `pending create`, `pending update` or `pending delete`
  for a draft change not provisioned yet (the rule's, else its ruleset's), and `disabled`;
  `#n` is the rule's place as the Policy screen lists it), Label groups (direct
  members, and groups holding it through a nested group, `via` that group; a rule naming such
  a group is listed `via` it too) and, for a label, Pairing profiles (those that give the
  label to the workloads they pair, by name, with `locked` when the profile locks that key:
  role, app, env or loc; `disabled` when the profile is off) and Workloads (how many carry
  it). It searches the draft policy, what the Policy screen shows and what you would edit.
  It reads what is loaded: the draft rulesets (else loaded once, as a `w` form loads them:
  one cached request, `loading rulesets…` in the overlay, counted in its title and the api
  counter), the label groups, for a label the pairing profiles (one cached read the first
  time, `loading pairing profiles…`, counted the same way; held for every later `W`; a
  failed read is named in the overlay with its error and `W` asks again), and the workload
  list for the count; with no list it asks one cached read of at most one row filtered by
  the label, so the count is exact at 0 and `≥1` (at least one) otherwise: the PCE's
  `X-Total-Count` is the size of the whole collection, not the label's.
  `Enter` on that line opens Workloads filtered by the label, the exact list. Nothing runs on a timer, reopening loads nothing, and
  `u` is left as it is. `Enter` opens a rule or ruleset on Policy (selected), a label group on
  its tab, or the Workloads screen filtered by the label; on a pairing profile, which has no
  screen, the status line names `illumio-tui pairing-profiles list` and the overlay stays;
  `/` searches the list. Not searched (the overlay says so): virtual services, enforcement
  boundaries and container workload profiles. Pairing profiles are searched for a label only:
  they hold no label groups, IP lists or services.
  `:hygiene` (from any screen) is the reverse, before a cleanup: the labels, label groups, IP
  lists and services nothing names, and the workloads missing label keys. The header counts
  each (`Unused: labels 3 · label groups 1 · IP lists 2 · services 0`, `Missing labels: role 4
  · app 12 · env 12 · loc 30 (of 120 workloads)`) and always says, whole, what is not
  searched: active policy (it reads the draft: an object still in provisioned policy whose
  last draft use was removed is listed until you provision), virtual services and servers,
  enforcement boundaries, container workloads and their profiles, cloud resources, label
  mapping rules, RBAC scopes and firewall settings. An unused row is a candidate, never a
  verdict. "Used" is what `W` finds, built once over the same data, and more: every href a
  draft ruleset carries (scopes, rule providers, consumers and services, deny rules, custom
  iptables rules, egress services), the label groups holding it, and for a label the pairing
  profiles giving it, the workloads carrying it and the PCE's own usage flags (the request
  `u` makes), which also name the holders above. It never guesses: a kind a list not read in
  full may hold (the workload list cut short, a failed read) is not listed and the header
  says `not listed: labels (workloads not read in full …)`, unless the PCE's flags settle
  each label; rows that rest on rulesets whose deny rules are not loaded are marked
  `unused (uncertain)`, labels the flags settle too (and when the rulesets were cut short),
  since whether the flags count draft rules is not known; the counts are the rows shown and the header names the kinds (selecting a ruleset on Policy loads
  its deny rules). Every warning shows, each on its own lines. The PCE's built-ins (the
  `Any (0.0.0.0/0 and ::/0)` IP list, `All Services`, anything the PCE ships as
  `ilo_defaults`), deleted labels and objects pending deletion (already on their way out)
  are never listed. The label keys are the profile's label model (`[profiles.<name>.labels]`
  scope keys and tier key); the default model checks `role`, `app` and `env`, and `loc` when
  the PCE has a `loc` label (a profile naming `app`, `env` / `role` itself is the default
  model); managed and unmanaged workloads alike. It reads the object lists from the name
  index (or the Objects tabs once loaded) and loads the draft rulesets, the label usage
  flags, the label groups (their tab's read says whether it was cut short; a cache hit after
  the index), the pairing profiles and the workload list once if missing (cached, counted in its
  title); unused rows wait until all are in, a failed read is named and `:hygiene` again
  asks. `Enter` selects the object on its Objects tab, where `x` deletes it with the usual
  confirm (a tab still loading selects it when it arrives), or opens the workload's detail
  on Workloads; `W` runs where-used for the object as a check, `Esc` there comes back to the
  report; `/` searches.
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
  **VEN health.** The line above the table is the fleet: `VEN 120 · healthy 100 · warning 5
  · error 2 · offline 13 · unmanaged 40`, each state in its colour, `· of 500 loaded
  (capped)` when the list was capped (narrow terminals shorten the words: `ok 100 · warn 5`).
  `v` cycles a health filter (all, warning, error, offline, all; `H` stays the health sort).
  Health is one filter: `v` or a health row in `:ven` turns `o` and `h` off, and `o` or `h`
  turns the health filter off, so the filter line always says what is in force.
  `:ven` opens the fleet view: counts per health state, per VEN version (newest first, marked,
  with how many VENs are behind it), per enforcement mode, and the most common health
  messages (errors and warnings, grouped). `Enter` on a row filters the list to that bucket
  (a state, a version, a mode or a message; other filters stay) and closes it; the filter
  line shows it (`health warning · ven 23.2.31-3113`) and `c` clears it. All of it comes from
  the loaded list (`agent.status`: version, heartbeat, sync, health errors and warnings): no
  request.
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
  **Compare with a snapshot.** `b` (`c` clears the filters here) picks a [snapshot](#snapshots)
  of the profile that holds workloads, one at a time on the bottom line (newest first,
  `latest` shown first; `←`/`→` older and newer, `Enter` compares, `Esc` leaves things as they
  were), and compares the loaded list with it: no request, the file only. Each row gains a `Δ`
  column, `NEW` (not in the snapshot), `CHG` or blank (the same), and a `was→now` column
  naming what changed on a `CHG` row (`app: shop→cart · mode: visibility→selective · ven:
  active→stopped · ven 23.2→23.5 · health: healthy→offline`; the detail pane has it in
  full); a workload gone since is a dim `GONE` row with the snapshot's values (its health then).
  The header reads `vs snapshot 10-20 06:00 · 3 new · 5 changed · 1 gone`, the counts of
  the rows the filters keep; `n` shows only new, changed, gone, then all again, `C` leaves.
  Every filter applies to both sides (a gone row by its values then), so `v` on offline with
  `n` on changed lists the VENs newly offline; grouped, each header counts the workloads
  there now and their changes, the gone ones apart (`12 wl · 2 new 1 chg · 1 gone`; health
  and enforcement are of what is there now). The VEN strip stays the fleet loaded now; `r` reloads the list and
  compares it again. While comparing, `n` is the change filter (it creates a workload
  otherwise); a gone row cannot be marked, edited or sent to Traffic. A snapshot of another
  profile, PCE or org is refused, as `snapshot diff` refuses it.
- **Events**: severity-coloured audit events, `f` edit severity / type / actor / window, `S` sort by
  severity. The Type field takes comma-separated terms, any of which keeps an event
  (`sec_policy, rule_set`); `!term` hides (`!user.login, !agent.update`), applied after the
  others; a term with `*` must match the whole type (`sec_policy.*`, `*.delete`), as must one
  starting with `=` (`=user.login` is that type only, `!=user.login` hides only it and keeps
  `user.login_session_terminated`), else it is a substring; case-insensitive. `Ctrl-N`/`Ctrl-P`
  complete the event types of the loaded events, also after `,`, `!`, `=` or `!=`. A severity is sent to the PCE (it takes one); `warning+` is
  that and worse (`err+`, `info+` …), filtered here, as is the type. The Actor field takes the
  same terms over the actor the Actor column shows (`user:alex@ryte.de`, `agent:web01`,
  `system`, `service_account:ci`, `?`); a term matches the whole actor or the name after its
  kind, so `=alex@ryte.de`, `user:*`, `!system` and `!agent:*` all work, and `Ctrl-N`/`Ctrl-P`
  complete the actors of the loaded events. `x` (or `!`) hides the
  selected row's exact type (`!=type`; hiding the type you show only empties the view), `o`
  shows only it (`=type`); `U` hides the row's actor (`!=actor`) and `u` shows only it
  (`=actor`). `t` lists the loaded types by count (`Enter` or `o` only this one, `x` hide
  it, `Tab` switches to the actors by count, where the same keys act on the actor, `Esc` close;
  a type or actor whose events another filter hides says which: `(severity)`, `(actor)`,
  `(type)`). None of these sends a request: they filter the loaded
  events, and only a new PCE severity or window in `f` (or `r`) loads again. The PCE is never
  asked to filter by actor. The header says
  what is hidden (`hiding 3 types · actor !system · warning+ · 812 of 1,240`) and `capped at 500` when the PCE
  returned a full page, so more events may exist than were filtered. `Enter` the event as a
  change diff: a header (time,
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
    illumio-tui where-used label env=Production --table
    illumio-tui where-used iplist Corp --csv
    illumio-tui hygiene --table
    illumio-tui hygiene --kind ip_list --csv --fields name,href,certain
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
    illumio-tui policy search scope:app=payment -svc:443/tcp --table
    illumio-tui policy search src:all -deny --json
    illumio-tui iplists --table
    illumio-tui pairing-profiles list --table      # alias: pp list
    illumio-tui policy pending
    illumio-tui policy pending --diff --table
    illumio-tui policy impact --table
    illumio-tui policy impact --window 7d --fail-on-blocked --json
    illumio-tui events --since 1h --severity err
    illumio-tui events --type sec_rule --diff
    illumio-tui events --exclude-type =user.login --exclude-type 'agent.*' --severity warning+
    illumio-tui events --type 'sec_policy, rule_set, !*.delete' --all-types --table
    illumio-tui events --actor =alex@ryte.de --since 7d --table
    illumio-tui events --exclude-actor system --exclude-actor 'agent:*' --csv
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
    illumio-tui gen shop/prod --window 24h --baseline 7d --table
    illumio-tui readiness --table
    illumio-tui readiness --app shop/prod --app crm/prod --json
    illumio-tui exposure --table
    illumio-tui exposure --service rdp,smb --only exposed --fail-on-exposed
    illumio-tui snapshot save --label "before the Q4 change"
    illumio-tui snapshot diff --table
    illumio-tui snapshot diff latest now --section rulesets,workloads
    illumio-tui workloads --baseline-snapshot latest --changes new,changed,gone --table
    illumio-tui readiness --trend --table
    illumio-tui traffic mesh --window 1h --no-user --json
    illumio-tui traffic query --window 30d --dry-run
    illumio-tui traffic query --window 24h --backfill --json
    illumio-tui traffic query --window 24h --baseline 7d --json
    illumio-tui traffic groups --by pairs --since 2026-10-23T18:00 --baseline 30d --yes
    illumio-tui traffic groups --by in --show blocked --since 2026-10-24 --baseline 7d --changes new,changed
    illumio-tui traffic mesh --window 24h --baseline 7d --changes new,changed,gone,same --csv
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
`blocked_since_enforcement`) or flows the draft would newly block (`policy impact
--fail-on-blocked`, `would_block`), 8 a risky service exposed (`exposure --fail-on-exposed`,
`exposed`), 9 snapshots differ (`snapshot diff --fail-on-change`, `snapshots_differ`), 141 stdout closed by its reader (`| head`; quiet, no error).

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
`policy rules`, `policy rules impact --list`, `policy search`, `policy pending`, `policy impact` (the pair rows), `traffic query`/`groups`/`mesh`, `app` (with `--section`),
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

`--health warning,error` keeps the workloads whose VEN health is any of those (`healthy`,
`warning`, `error`, `offline`, `unmanaged`; the app's `v`), and `--ven-version V` those whose
VEN reports version `V`, exactly or as a `*` glob (`23.2.*`); both filter the one read
locally. Health is one filter: with `--health`, `--offline` adds `offline` to its states and
`--unhealthy` adds `warning,error,offline` (a union; `meta.local_filter` names the one set), and
a saved filter with both `offline = true` and `health` reads the same way. `workloads ven-summary` takes the same filters and prints what the app's `:ven`
shows, from that one read (cached) and nothing else: counts per health state, per VEN version
(newest first, with how many are behind it), per enforcement mode, and per health message.
The table prints a section each; JSON puts `{workloads, vens, truncated, health, versions,
newest, behind, no_version, enforcement, messages}` in `data`; CSV is one row per bucket,
`section,name,count,note` (`version,23.2.31-3113,2,newest`, `message,…,3,error`).

    illumio-tui workloads --health warning,error,offline --label env=prod
    illumio-tui workloads ven-summary --label env=prod --table

`--baseline-snapshot latest|previous|TIMESTAMP` compares the listed workloads with a
[snapshot](#snapshots)'s (among the profile's snapshots that hold workloads; a time's prefix is
enough): one unfiltered list read (the read `snapshot save` makes), then the file, no other
request. Rows gain `change`
(`new`, `changed`, `gone`, `same`) and `was` (a changed row's transitions, `app: shop→cart ·
mode: visibility→selective · ven: active→stopped · health: healthy→offline`; `null` on any
other); a gone row carries the snapshot's values. `--changes` picks the rows printed, any of
`new,changed,gone,same` (default `new,changed,gone`). Every filter is applied locally, to
both sides, as the app does: gone means no longer on the PCE, and a workload that moved out of
the filter (`app=shop` then, `app=crm` now, with `--label app=shop`) is in neither side, not
gone (with `--label app=crm` it is changed, `app: shop→crm`). A snapshot keeps no interfaces,
so with `--ip` no row is gone. `--max-results` caps that one unfiltered read of the whole
fleet, before any filter: a list cut by the cap judges nothing gone (a workload past the cap
would read as gone), so no row is `gone`, `meta.compare.counts.gone` is `null`, and the meta
line says `gone not judged: list cut at N` (the TUI's header too). `meta.compare` names the snapshot (`snapshot`,
`taken_at`, `label`, `changes`, `counts` of the rows the filters keep, `truncated` (the
snapshot's workloads were capped: new over-reports), `list_cut_at`); the table
and CSV add `change` and `was` columns. A snapshot of another profile, PCE or org is refused
(exit 2) before any request.

    illumio-tui workloads --baseline-snapshot latest --health offline --changes changed --table

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

**Compare (what is new since).** `--baseline DURATION` on `traffic query`, `groups` and `mesh`
also runs the same filter over the window just before the recent one, exactly `[start −
DURATION, start)` of the recent query's bounds, and marks every row `new` (only recent),
`changed` (in both, another set of decisions), `gone` (only in the baseline; its values the
baseline's) or `same`. Rows are matched on the view's key without the decision and without
`first_detected`: flows on both endpoints (workload href, else IP), port/proto, process, user
and direction; `--services` and sections on source group, destination group and service; plain
pairs (`groups` without `--services`, `query --group`) by their service keys (`new` when all are
new, `gone` when all are gone, `changed` when any is new, gone or changed; `baseline_decision`
the union; a last `change_detail` column / JSON field says what changed on a changed pair:
`+2 svc −1 svc` for the services that came or went, `allow→block` when the decision sets of
the services on both sides differ, else `~2 svc` for those whose decisions changed, joined
with ` · `; empty in table and CSV, `null` in JSON on every other pair row); the mesh on its row key. `meta.compare.counts` counts the rows of the output (pairs
without `--services`, service rows with it), before `--changes` picks them. Each key is judged
on every decision, then `--show` / `--blocked` pick the rows: a flow allowed then and
potentially blocked now is `changed` in the blocked sections, and a gone row shows only when its
baseline records pass the filter. "New" means not seen in the baseline window at Explorer's
record granularity: a record spanning the cut-off is returned by both queries and reads as the
same (or changed, and an old record overlapping the recent start can make the recent set
`allowed,potentially_blocked`); connection and byte counts stay each side's lifetime counters,
never per window and never subtracted. The recent window is `--window`, `--from`/`--to`, or
`--since TIME` (`--from` with `--to` defaulting to now; RFC 3339, or `2026-10-23T18:00` and
`2026-10-23` in the local time zone, `TZ`; `--from`/`--to` read `2026-10-23T18:00` as local time
too and a date alone in UTC; `meta` windows are always UTC). `--changes new,changed,gone,same`
picks the rows (default `new,changed,gone`; only with `--baseline`, else exit 2). Table and CSV
gain `change` and `baseline_decision` (the baseline's decisions for the key, `allowed,blocked`)
columns, JSON the same two fields per row and `meta.compare` with both exact windows, each
side's truncation, cap, cap steps, queries and cache, and the counts per change (before
`--changes`); `meta.queries` and `meta.cost` count every query of both sides (each backfill
step, each side of `--app`). `--dry-run` prints both bodies (recent first). The cost guard
judges each query on its own (`expensive baseline query: window is 30d (> 7d)`); `--yes` covers
both. `--backfill` climbs each side on its own. A truncated side is a warning on stderr and in
`meta.warnings` (`baseline truncated, capped at 10000: new/gone over-report`); the exit code
stays 0. Squelch acts on both sides alike: a key with a squelched record on either side leaves
both (a mark on the draft decision, which the baseline never carries, cannot make it gone), and
`--show-squelched` and `--no-marks` act on both. `--draft` evaluates the recent side only.
`traffic explain` takes no `--baseline`; a saved filter keeps no baseline.

A nightly "what is new" mail from cron: the last 24 hours against the 7 days before them, mailed
only when a row came back (the exit code stays 0, so test the rows):

    0 6 * * * illumio-tui traffic groups --services --window 24h --baseline 7d --changes new,changed --json > /var/tmp/new.json 2>>/var/tmp/new.log && jq -e '.data | length > 0' /var/tmp/new.json >/dev/null && jq -r '.data[] | "\(.change) \(.src|join("/")) -> \(.dst|join("/")) \(.service) was \(.baseline_decision // "unseen")"' /var/tmp/new.json | mail -s "new traffic" alex@ryte.de

Two Explorer queries a night (13 s on the demo PCE). Propose the rules too: `gen shop/prod
--baseline 7d --table` lists the rules only the new traffic needs. The full recipe with its output:
https://alexgoller.github.io/illumio-tui/#recipes

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
[--window W | --since T | --from T --to T] [--max-results N] [--yes] [--consolidate <port>/<proto>=any|all ...]
[--baseline DURATION [--changes new|new,changed]]` proposes the allow rules the app's observed traffic
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

`gen <app> --baseline 7d` (#81) proposes only the rules the **new** traffic needs: the same
focused queries again over `[start - 7d, start)` (in their own `focus-baseline` cache slots,
through the same cost guard; `--since`/`--from` set the recent window, local time as in
`traffic`), each recent flow record judged as `traffic --baseline` judges it, and only the
records whose key the baseline lacks feed `rows`, `rules` and `consolidations`;
`--changes new,changed` also feeds a key whose decisions changed (allowed then, blocked now).
The others are left out of the proposal, neither covered nor uncovered:
`preview.seen_before_flows` counts them (`covered_flows + uncovered_flows + seen_before_flows`
is every flow record of the app in force; without `--baseline` the field is absent). A row
with no flows left is not listed, rules already in the draft are still `exists`. With
`--table` or `--csv` a line on stderr says `new since 7d before: 40 flows seen before` (or
`nothing new since …`). `data.compare` is `{length, changes,
recent, baseline, counts, seen_before}`, as `meta.compare` of `traffic` (`counts` in the app's
flow records), and `meta.queries`/`meta.cost` count both sides. A truncated baseline warns
(`baseline truncated, capped at N: new over-reports`), a capped or incomplete recent
side still exits 2 (or 4), before any baseline query is sent. `--changes` takes `new` or
`new,changed` (either order); anything else, or `--changes` without `--baseline`, exits 2
before any request.

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

`where-used label KEY=VALUE`, `where-used label-group NAME`, `where-used iplist NAME` and
`where-used service NAME` (each also takes the object's href) list what names the object in
the draft policy, as the TUI's `W` does: one row per reference with `kind` (`scope`, `rule`,
`label_group`, `pairing_profile`, `workloads`), `ruleset`, `rule` (its place as the Policy
screen lists it), `side` (`provider`, `consumer`, `service`), `action` (`allow`, `deny`,
`override_deny`), `excluded` (named as an exclusion), `pending` (the draft change not
provisioned yet, the PCE's `update_type`: `create`, `update` or `delete`; `null`, or empty
in CSV, when provisioned as it is), `disabled` (for a pairing profile: `enabled: false`),
`via` (the label group it is reached through), `name` (a group's or a pairing profile's),
`count` and `exact` (the `workloads` row of a label), `locked` (a `pairing_profile` row:
`true` when the profile locks the label's key, role, app, env or loc; `null`, empty in CSV,
for other rows and keys with no lock) and `href`; `meta.where_used` repeats the object, the
total, the workload count and what is not searched. A name that is unknown or ambiguous is
exit 2 with the candidates: similar names, else every name when there are few, else how many
there are; for an ambiguous one, the hrefs to use instead. Requests, all cached and shown on
the meta line: the object's own list, the draft rulesets, the label groups (labels and
groups only) and, for a label, the pairing profiles (one read, also under `--no-cache`) and
one workloads read of at most one row filtered by it (0 rows is an exact 0, one row `≥1`,
`exact: false`: the PCE's `X-Total-Count` is the whole collection's size, not the label's;
`workloads --label KEY=VALUE` gives the exact list). Deny rules count as the ruleset list
carries them. A failed pairing profile read (a role that may not list them: 403, or a server
error) does not fail the command: the other rows print, a warning names the error on stderr
(and in `meta.warnings`), and the not-searched text of that run adds `pairing profiles (read
failed: …)`; the exit code stays 0. For example, before deleting a label or an IP list:

    illumio-tui where-used label env=Production --table
    illumio-tui where-used label /orgs/1/labels/42 --json --fields kind,ruleset,rule,via
    illumio-tui where-used label role=web --csv --fields kind,name,locked
    illumio-tui where-used iplist Corp --csv > corp-uses.csv
    illumio-tui where-used label-group "All prod envs"

`hygiene [--kind label,label_group,ip_list,service,workload]` is the reverse of where-used,
in one pass (the TUI's `:hygiene`): one row per label, label group, IP list and service that
nothing in the draft policy (every href a ruleset carries: scopes, rule providers, consumers
and services, deny rules as the ruleset list carries them, custom iptables rules, egress
services), the label groups, the pairing profiles (labels) or the workloads (labels) names,
each kind by name, then one per workload missing one or more label keys, in list order. A
label is unused only when the PCE's own usage flags (`labels?usage=true`, read instead of the
plain list: still one request) hold it nowhere too; a PCE without flags leaves it to the
scan. Columns: `kind`, `name` (`key=value` for a label, a workload's hostname else its name),
`href`, `reason` (`unused`, or `missing labels: app,env`), `missing` (a workload's keys, a
list; `null` for the other kinds, `;`-joined in CSV) and `certain` (`false`: the row rests on
rulesets whose deny rules were not read, or, for a label the PCE's flags settle, on a
ruleset list cut short: whether the flags count draft rules is not known) and, in JSON
only, `usage` (a label's usage object as the PCE returned it, passed through; `null` when
it returned none and for the other kinds). It never guesses: a kind that a
list not read in full may hold (rulesets, label groups, pairing profiles or workloads cut
short, or a failed pairing profile read) is not listed, labels only where the PCE's flags do
not settle them; `meta.hygiene.withheld` names those kinds, `meta.hygiene.uncertain` the
kinds with `certain: false` rows, and each gets a warning (stderr and `meta.warnings`). An
unused row is still a candidate: `meta.not_searched` names what may use it (active policy:
draft only, so an object still in provisioned policy shows until you provision; virtual
services and servers, enforcement boundaries, container workloads and their profiles, cloud
resources, label mapping rules, RBAC scopes, firewall settings; and a pairing profile read
that failed). `meta.hygiene` also has the kinds checked, the unused rows per kind listed,
the keys and the workloads missing each, the workloads read and the rulesets whose deny
rules were not searched. Built-ins (`Any (0.0.0.0/0 and ::/0)`, `All Services`,
`ilo_defaults`), deleted labels and objects pending deletion are never listed. The keys are
the profile's label model; the default model checks `role`, `app`, `env`, and `loc` when the
PCE has a `loc` label (by the labels read, else the workloads' labels). Requests, all cached
and made only for the kinds asked: labels with usage flags (`label`), label groups (`label`,
`label_group`), IP lists, services, the draft rulesets (every object kind), the pairing
profiles (`label`) and the workload list (`label`, `workload`). `--kind` takes
`label-group` and `ip-list` too. Exit 0 whatever it finds.

    illumio-tui hygiene --table
    illumio-tui hygiene --kind workload --csv > missing-labels.csv
    illumio-tui hygiene --kind ip_list --csv --fields name,href,certain > unused-ip-lists.csv
    illumio-tui hygiene --json --fields kind,name,reason

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
The TUI's search terms work as arguments too (`policy search app=shop ipl:Corp deny`), with
the same grammar: `scope:`, `src:` (`consumer:`) and `dst:` (`provider:`) put a label term on
one side (`src:`/`dst:` also take `ipl:NAME`, `wl:NAME` and `all`; `scope:global`), `!key=value`
finds exclusion actors, `enabled`, `allow`, `intra` and `extra` test the rule, and a leading `-`
negates a term: `policy search src:role=web dst:role=db extra`, `policy search
scope:app=payment -svc:443/tcp`, `policy search 'src:!env=prod'` (quote a `!` for the shell), `policy search src:all -deny`.
A `-term` needs no `--` (the CLI takes it as a term, flags keep working); after `--` every word
is a term. On Windows PowerShell 5.1, quote a term that starts with `-` and holds a `.`
(`'-dst:wl:web01.corp'`): it splits such a word otherwise. `--label` keeps its meaning (no `!`: use the term). `meta.matched` is `{"matched",
"loaded"}`: the rules found of the rules searched (stderr: `3 of 40 rules matched`). An IP
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

`--trend` adds each app's road to enforcement: `trend: [{taken_at, verdict, gaps}]`, its
verdict and gap count in the newest 14 of the profile's [snapshots](#snapshots) that kept
readiness (`snapshot save --with-readiness`), oldest first (an unreadable one or another
PCE's is skipped and an older one taken), then the live result as the last point when it is
judged on the same basis (7d, the draft; with `--window 24h` it is left out, a warning says
`live point left out: window 1d differs from the snapshots' 7d`). It reads the files only (no request beyond readiness's own); snapshots of another PCE
are skipped with a warning. The table adds a `trend` column, a sparkline of the gaps (two
points at least); CSV the points (`2026-10-20T06:00:00Z needs_rules 12; …`).

### Snapshots

The PCE keeps no history of its object lists. `illumio-tui snapshot save` keeps one locally, so
"what changed in policy this month" or "which workloads lost labels since last week" has an
answer later, at no cost to the PCE:

    illumio-tui snapshot save [--what policy,labels,workloads] [--with-readiness [--yes]] [--label TEXT]
    illumio-tui snapshot list
    illumio-tui snapshot diff [A] [B] [--section rulesets,labels,label-groups,iplists,services,boundaries,workloads] [--fail-on-change]
    illumio-tui snapshot rm TIMESTAMP
    illumio-tui snapshot prune --keep N

`save` makes the same cached reads as every other command (`--no-cache` for fresh ones), one
after another, counted in `meta`: `policy` is the draft and active rulesets with their rules (and
the deny rules the ruleset list carries), the label groups, IP lists, services and enforcement
boundaries (six GETs; a PCE that answers 404 or 403 for the boundaries gets a snapshot without
them, a warning, and diffs that list `boundaries` as not compared); `labels` one; `workloads` one unfiltered list (each workload's href,
name, hostname, labels, enforcement mode, managed flag and the VEN's `agent.status`: status,
version, heartbeat, health). `--what` picks sections (default all three); `--with-readiness`
adds every app's [readiness](#readiness) (its one Explorer query and reads, through its cost
guard; `--yes` confirms an expensive one). It prints the file's path and each section's items
and bytes. The TUI's `:snapshot [label]` makes the same reads (mostly cache hits after the
screens loaded) and shows the path in the status line.

Files: `$XDG_DATA_HOME/illumio-tui/snapshots/<profile>/<UTC time>.json` (else
`~/.local/share/illumio-tui/snapshots/...`), one JSON file each, `{version, profile, pce, org,
taken_at, label, truncated, sections}` (`pce` is the host and port); plain JSON (gzip it yourself if you keep many). They hold
what the reads returned (rules, object names, hostnames, labels, VEN status), never credentials;
mind who reads the directory. A read cut by its row cap is named in `truncated` and warned
about in every diff. A file of a newer format, or one that is not a snapshot, is refused with
why (`list` shows it with its `problem`). Only files named like a snapshot
(`2026-10-27T031500Z[-N].json`) are snapshots; `latest`, `previous` and `prune` skip unreadable
ones, and `prune` never deletes them (`rm` with the time does). `prune --keep` is at least 1.

`diff` compares A (default `previous`) with B (default `latest`); each is a snapshot's time (a
unique prefix is enough: `2026-10-01`), `latest`, `previous` or `now`, a live read of only the
sections the other side holds (and `--section` asks for). Per section it lists what was added,
removed or changed, matched by href, with the changed `fields` and `details` (`-` before, `+`
after): rulesets in both the draft and the active policy, with each rule as `policy pending
--diff` shows it; labels (key, value, deleted); label groups, IP lists and services as the
pending diff shows them; boundaries (name, providers, consumers, services); workloads (name,
hostname, labels, `enforcement_mode`, managed, `ven_status`, `ven_version`). A section one side
lacks is listed under `not_compared`. Snapshots of another profile, PCE or org are never
compared (exit 2, naming both). The exit code is 0 whatever changed; `--fail-on-change` exits 9
after the report when something did. `--table` prints the counts and the changes with their
lines; `--csv` prints the changes.

`list`, `rm`, `prune` and a diff of two files read only the files: no request, and only the
profile's name is resolved (no credentials, no `api_secret_cmd` run).

A nightly snapshot and a Monday mail of the week's changes:

    30 2 * * * illumio-tui snapshot save --label nightly >/dev/null 2>>/var/tmp/snapshot.log && illumio-tui snapshot prune --keep 60 >/dev/null
    0 7 * * 1 illumio-tui snapshot diff "$(illumio-tui snapshot list --json | jq -r '[.data[] | select(.problem == null and .taken_at < (now - 7*86400 | todate))] | last.id // "previous"')" latest --fail-on-change --table > /var/tmp/week.txt 2>>/var/tmp/snapshot.log; [ $? -eq 9 ] && mail -s "PCE changes this week" alex@ryte.de < /var/tmp/week.txt

The jq picks the newest readable snapshot taken over 7 days ago (whatever was saved by hand in
between; `previous` while there is none yet); exit 9 means something changed, so a quiet week
sends no mail. On the demo PCE a save is eight GETs and half a megabyte:

    $ illumio-tui snapshot save --label first --table
    snapshot 2026-10-27T233054Z · ~/.local/share/illumio-tui/snapshots/demo/2026-10-27T233054Z.json · 539753 bytes
    section    items  bytes
    ────────────────────────
    policy     236    203877
    labels     134    21951
    workloads  216    313749
    demo · 8 api calls · 920.2 KB · cache miss · 2908 ms

    $ illumio-tui snapshot list --table
    snapshots · demo · 2
    id                  taken_at                     label  sections                 bytes   problem
    ────────────────────────────────────────────────────────────────────────────────────────────────
    2026-10-27T232643Z  2026-10-27T23:26:43.340678Z  first  policy,labels,workloads  539753
    2026-10-27T233054Z  2026-10-27T23:30:54.516853Z  first  policy,labels,workloads  539753
    demo · 0 api calls · 0.0 KB · cache none · 10 ms

`cache miss` is expected: the cache lives in memory for one run, so a CLI command reads each list
once (the TUI's `:snapshot` reuses what its screens loaded). The full recipe, with an
illustrative diff: https://alexgoller.github.io/illumio-tui/#recipes

Snapshots are also the baseline of the screens without a time window: `workloads
--baseline-snapshot latest` (and `b` on Workloads) compares the workloads and their VENs with
a snapshot's, and `readiness --trend` (the `trend` column of `:readiness`) shows each app's
gaps and verdict over the snapshots saved with `--with-readiness`. Save those nightly to have a
trend:

    30 2 * * * illumio-tui snapshot save --with-readiness --label nightly >/dev/null 2>>/var/tmp/snapshot.log
    illumio-tui readiness --trend --table

### Exposure

`illumio-tui exposure [--window W] [--service rdp,ssh] [--only exposed] [--keys app,env]
[--src ...] [--dst ...] [--exclude-src ...] [--exclude-dst ...] [--exclude-service ...]
[--max-results N] [--backfill] [--yes] [--draft] [--no-marks] [--fail-on-exposed]` answers the
first question of a ransomware readiness review: which risky services can reach what, today,
and under which decision. It runs one Explorer query over the window (the profile's traffic
window, 24h by default) whose services are every port of the risky list, with no other filter
than the sides given (`--src`, `--dst` and the excludes, as on `traffic`), through the cost
guard (a window over 7d or a cap over 10000 needs `--yes`), in its own cache slot;
`--backfill` raises a truncated result's cap as on `traffic`, `--draft` adds the draft
decision (one PUT `update_rules`, no policy change). Squelch (`marks.toml`) hides rows from
the table and CSV, never an exposure: an exposed row only squelched flows make is still in
`--json` (`squelched: true`), named in `meta.warnings` and the title (`1 exposed row
squelched`), and counted by `--fail-on-exposed`. Read-only.

The default list: `rdp` 3389/tcp, 3389/udp · `smb` 445/tcp, 139/tcp · `netbios` 137/udp,
138/udp · `rpc` 135/tcp · `winrm` 5985/tcp, 5986/tcp · `ssh` 22/tcp · `telnet` 23/tcp · `vnc`
5900/tcp · `mssql` 1433/tcp · `mysql` 3306/tcp · `postgres` 5432/tcp · `oracle` 1521/tcp ·
`mongodb` 27017/tcp · `redis` 6379/tcp · `ldap` 389/tcp, 636/tcp. A profile replaces it
(`services`) or adds to it (`extra`; a name already listed is replaced), `PORT/PROTO` or
`FROM-TO/PROTO` each:

    [profiles.prod.exposure]
    extra = { nfs = ["2049/tcp"], kafka = ["9092/tcp"] }
    # services = { rdp = ["3389/tcp", "3389/udp"], ssh = ["22/tcp"] }   # the whole list

A malformed entry is a config error naming it (`exposure.extra.nfs: "2049": give a port and a
protocol, e.g. 3389/tcp`; exit 2 before any request, the TUI's status line); other commands
run as before. `--service` narrows the list (and the query) to some names; an unknown one is
exit 2 with the names it knows.

One row per destination group (`--keys`, default the profile's scope keys) and risky service:
`service`, `ports` seen, `dst`, `status`, `sources` (`intra`: flow records from the group's
own workloads; `apps`: other labelled groups; `ip_lists`: unlabelled hosts and IP lists, with
`any` for the Any (0.0.0.0/0) list and `internet` for a public address among them), flows
and connections per decision, `exposed_flows`, and `draft_allowed` / `draft_blocked` with
`--draft`. A flow is from inside the group only when both ends are labelled workloads whose
values are set and equal for every group key; an IP list, an unlabelled or unmanaged host or a
workload missing a key is from outside, so a report may over-report an exposure but never
hide one. `status` is judged on the flows from outside: `exposed` (allowed), else
`potentially_blocked`, else `blocked`, else `intra` (only the group's own flows); rows
sort in that order. `data.services` sums each service of the list (those no flow reached
too): `rdp: 12 destinations · 4 exposed (allowed from other apps) · 3 potentially blocked · 5
blocked`, the line the table prints above its rows. `--only` keeps rows of some statuses
(the summary still counts all). Which rule allows an exposed row is not looked up (a request
per row): each exposed row's `explain` holds the `traffic explain` arguments that answer it,
for its busiest exposing flow (`--dst` and the port, `--src` too when the row has one source
group; one query, run when you choose), and the table prints them under its rows, one line
each, at most 10 (`N more: --json has every command`):

    $ illumio-tui exposure --window 24h --service rdp,smb --table     # the demo PCE, trimmed
    exposure · 1d · 19 rows · 19 exposed
      rdp: 8 destinations · 8 exposed (allowed from other apps) · 0 potentially blocked · 0 blocked
      smb: 11 destinations · 11 exposed (allowed from other apps) · 0 potentially blocked · 0 blocked
    one row per destination group and risky service · flows: flow records · conns: their lifetime counts
    status   service  ports              dst                     sources                         flows  conns   allowed  pot.blocked  blocked
    ─────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────
    exposed  rdp      3389/tcp,3389/udp  jump-infra/Production   2 apps                          118    849316  80       0            38
    exposed  rdp      3389/tcp,3389/udp  ordering/Production     intra, 1 app, 2 ext (internet)  12     83893   7        0            5
    exposed  rdp      3389/tcp,3389/udp  ecomm/Production        intra, 1 app, 1 ext             10     64121   6        0            4
    …
    exposed  smb      445/tcp            ad/Production           7 apps, 1 ext                   30     329963  20       0            10
    exposed  smb      445/tcp            exchange/Production     intra, 1 ext                    4      43495   2        0            2
    exposed  smb      445/tcp            swift/Production        intra, 1 ext                    2      22505   1        0            1
    …

    which rule allows it (one Explorer query each, run on demand):
    rdp  jump-infra/Production   illumio-tui traffic explain --dst app=jump-infra,env=Production --port 3389 --proto tcp --window 24h
    rdp  ordering/Production     illumio-tui traffic explain --dst app=ordering,env=Production --port 3389 --proto tcp --window 24h
    rdp  ecomm/Production        illumio-tui traffic explain --dst app=ecomm,env=Production --port 3389 --proto tcp --window 24h
    …
    9 more: --json has every command

Every row there has more than one source group, so each command names no `--src` (it explains
every source of the row; a row with one source group gets its `--src`). `--csv` prints the rows with full columns (`explain` as the command line).
Exit 0 always, except `--fail-on-exposed`: exit 8 (`exposed`) after the report when a row is
exposed, and exit 4 when none is but the result is truncated or incomplete (the answer is not
complete), for CI and cron:

    illumio-tui exposure --service rdp --fail-on-exposed --json > rdp.json
    case $? in 0) ;; 8) echo "RDP exposed" ;; *) echo "no complete answer" ;; esac

### Provision impact

`illumio-tui policy impact [--window W] [--limit N] [--fail-on-blocked] [--yes]` says what
provisioning the draft would change, before anyone provisions it. Read-only. The PCE evaluates
against the whole draft, so the summary always covers everything pending. Three sections:

- **Changes**: the pending objects counted by object type and change type (`create`,
  `update`, `delete`), from the pending list.
- **Rules**: per pending ruleset (a pending rule counts through its ruleset), its rules added,
  removed and changed, draft against active, matched by href as `policy pending --diff`
  matches them. Each carries its workload coverage active → draft (`providers 12→40 ·
  consumers 3→3`): the active side through the active label groups and IP lists, the draft side
  through the draft ones, over one workload list. `-` means not in force on that side (absent,
  or it or its ruleset disabled); `≥` a workload list cut short. A rule is changed when its
  text, its state or its coverage differs (`was:` under it gives the active text when it
  differs; none means only its coverage moved: the ruleset's scopes or a label group). A
  ruleset pending deletion counts its rules as removed. An extra-scope rule (consumers from
  outside the ruleset's scopes) ends its text with `(extra-scope)`, which is why it can match
  more consumers than an intra-scope rule of the same text. When neither the draft nor the
  active ruleset list carries any deny rule inline (an older PCE keeps them apart), deny rules
  are not compared, and `not counted` says so.
- **Traffic**: one Explorer query over `--window` (the profile's, 24h by default; over 7d needs
  `--yes`) at the profile's cap, run and re-evaluated against the draft (`update_rules`; no
  policy change) in its own cache slot: every flow record reported → draft (each changed cell,
  the rest as one `unchanged` count), then the app/env group pairs (the profile's scope keys)
  whose flows get a stricter decision (newly blocked: allowed → potentially blocked or blocked,
  and potentially blocked → blocked), then those the draft allows (newly allowed: blocked or
  potentially blocked → allowed; blocked → potentially blocked is only in the table), busiest
  first, with their services by port. Flows the draft re-evaluation did not reach are counted
  in the heading (`· 3 not evaluated`). The table shows 20 pairs per
  direction (`--limit N`); `--json` and `--csv` carry all of them unless `--limit` is given
  (`blocked_pairs_total`, `allowed_pairs_total` count them).

With nothing pending it says so, exits 0 and sends no query (one pending read). Requests: the
pending list; the object lists (labels, label groups, IP lists, services); when a ruleset is
pending, the draft and active rulesets, the active label groups and IP lists and one workloads
read (all cached); then one POST, status polls, one PUT `update_rules`, polls and one download.
`meta.cost` says it in words, `meta.api_calls` counts them, `meta.draft` has the newly blocked
and fixed counts as `traffic --draft` counts them (allowed ↔ not allowed only, so a potentially
blocked → blocked flow is in `data.traffic.transitions.newly_blocked` but not in
`meta.draft.newly_blocked`). Not counted: workload enforcement modes and labels (never pending, never
provisioned), virtual services, virtual servers and enforcement boundaries in rule coverage (the
traffic section shows their effect), the rules of rulesets with no pending change whose label
groups or IP lists are pending, and a partial provision (the PCE cannot evaluate one).
`--csv` prints the pair rows (`direction,src,dst,services,flows,connections,transitions`);
`illumio-tui schema policy impact` prints the JSON shape.

<!-- real output: illumio-tui policy impact --window 24h --table -->

    $ illumio-tui policy impact --window 24h --table     # illustrative: shapes as printed, numbers made up
    changes · 3 pending changes
    object_type  change_type  count
    ───────────────────────────────
    ip_lists     create       1
    rule_sets    create       1
    sec_rules    update       1

    rules · 2 pending rulesets · 1 added · 0 removed · 1 changed
    providers, consumers: workloads matched, active → draft (- not in force)
    ruleset       change   kind   rule                                                 providers  consumers
    ───────────────────────────────────────────────────────────────────────────────────────────────────────
    Global-Infra  added    allow  allow all ← group:All prod envs All Services         -→12       -→40
    Shop-Prod     changed  allow  allow role=web ← iplist:Corp 443/tcp, 8000-8099/tcp (extra-scope)  4→4  3→0
      was: allow role=web ← role=db 443/tcp, 8000-8099/tcp (extra-scope)

    traffic · 24h · 8812 flow records · 14 newly blocked · 409 newly allowed
    reported → draft       flows  conns
    ───────────────────────────────────
    allowed → blocked      14     2210
    pot.blocked → allowed  409    51873
    unchanged              8389

    newly blocked or potentially blocked · 3 pairs
    …

`policy provision ... --impact` prints the same summary in front of the request (`--dry-run`),
or computes it after the guard and the plan and before the send (`--yes`), as `rules add
--impact` does, over the profile's default window; in JSON it is `data.impact`. With named
targets the changes section marks how many of each are in the request (`in_request`), and
`meta.warnings` says the rules and traffic sections cover the whole draft.

**Check before you provision.** In a change window or a pipeline, `--fail-on-blocked` exits 7
(`would_block`) after the report when a flow's draft decision is stricter than today's (allowed
→ potentially blocked or blocked, potentially blocked → blocked); without one, a result that is
truncated, incomplete, or has flows the draft re-evaluation did not reach is exit 4, never a
silent pass:

    illumio-tui policy impact --window 7d --fail-on-blocked --json > impact.json
    case $? in
      0) ILLUMIO_WRITES=1 illumio-tui policy provision --all --yes ;;
      7) mail -s "the draft would block flows" alex@ryte.de < impact.json ;;
      *) echo "no complete answer" ;;
    esac

The check and the provision are two steps, not one: `provision --all` sends whatever is
pending when it runs, draft edits made after the check included. Name the targets the check
saw (`policy pending --json` gives their hrefs), or compare `policy pending --json` before and
after, to provision only what was checked.

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

### Saved filters

Long filters (traffic `!` excludes and port ranges, `src:`/`dst:` rule searches, event type
terms) can be saved by name, per profile, in the config file. Traffic, Policy (rule search),
Events and Workloads have them; each field holds the text you type into that screen's `f`
editor, unchanged, and a policy filter is one rule search query. A field left out is not set;
on Traffic a filter without `window` or `max_results` keeps the ones in force.

```toml
[profiles.demo.filters.traffic.prod-blocked]
src = "env=prod"
dst = "!app=backup"
service = "!53/udp, 8000-8100/tcp"
decisions = ["blocked", "potentially_blocked"]
transmission = ["unicast"]
window = "7d"
max_results = 20000

[profiles.demo.filters.policy]
deny-prod = "dst:env=prod deny"
pos-in = "dst:app=pos -src:app=pos"

[profiles.demo.filters.events.noise-free]
type = "!agent.*, !=user.login"
actor = "!system"
severity = "warning+"
window = "7d"

[profiles.demo.filters.workloads.prod-unenforced]
labels = "env=prod"
enforcement = "visibility_only"
managed = true

[profiles.demo.filters.workloads.unhealthy-vens]
health = ["warning", "error", "offline"]
ven_version = "23.2.*"   # exact, or a * glob
```

Names are `A-Z a-z 0-9 . _ -`, up to 64, case-sensitive, not starting with `-`. A filter that does not parse stops
nothing at startup (nor does a screen key, a `filters` or a `filter_defaults` that is not a
table: the TUI warns, the value is kept as written); using it says which profile, screen,
filter and field.

In the TUI, `:save <name>` saves the filter in force on the screen (asking before it replaces
one), `:filters` (`:fl`) lists the screen's saved filters with a summary each (type to narrow,
`Enter` applies, `D` deletes after a confirm), and `:load <name>` (or `:filter <name>`; `Tab`
completes the name) applies one. Applying is the same as typing it into `f` and pressing
`Enter`: Traffic runs one query through the usual checks and cost confirm, Policy, Workloads
and Events filter locally (Events asks the PCE again only for a new window or severity). The
filter line shows `[name]` until you change the filter. After `:profile X` the filters are X's.
An events filter's type terms get the profile's `events.exclude_types` on top (each one the
text lacks is added as a `!` term), so both apply; `f` drops one for the session.

Each screen can have one **default** filter, a saved filter's name under its own table:

```toml
[profiles.demo.filter_defaults]
traffic = "prod-blocked"
events = "noise-free"
```

The TUI puts the defaults in force at startup and after `:profile X` (X's defaults), before a
screen loads anything, and shows `[name]` as `:load` does. A default sends no request of its
own: Events and Workloads load once on the first visit as always, with the default in force;
Policy searches locally; Traffic still runs nothing until you press `r` (or `f`), and a default
over the cost thresholds (long window, large cap) asks first like any other filter. A default
that names no filter, or one that does not parse, is ignored with a warning on the status
line; startup goes on. In `:filters` the default is marked `*`, and `*` makes the selected
filter the default (again: no default); `:default <name>` sets it and `:default off` clears
it, both written to the config file. Editing the filter, `:load`, or a screen's clear changes
only the session. Deleting a filter that is a default (`D`, `filters rm`) removes the default
too and says so.

On the CLI, `--filter <name>` on `traffic query|groups|mesh`, `policy search`, `events` and
`workloads` starts from a saved filter; a flag replaces that field (`--src` the source,
`--window` the window; `policy search` adds its terms to the saved query). `--save-filter
<name>` saves the filter the flags (and `--filter`) make and exits without contacting the PCE
(`--force` replaces an existing one); it prints `saved <screen> filter <name> to <path>`, or with
JSON `data.screen`, `data.name`, `data.config_path` and `data.filter`. Repeated `--src` is `|` in the text, `--exclude-src X`
is `!X`; what the text cannot hold (`--process`, `--from`/`--to`, `--status`, a bare `--proto`
number other than 6, 17 or 1, a `|` or leading `!` inside a flag value, ...) is an error.

```sh
illumio-tui traffic groups --src env=prod --exclude-dst app=backup --decision blocked --save-filter prod-blocked
illumio-tui traffic groups --filter prod-blocked --window 1d
illumio-tui filters list --screen traffic      # also --json, --csv
illumio-tui filters show traffic prod-blocked
illumio-tui filters rm traffic prod-blocked
illumio-tui filters default traffic prod-blocked   # --unset clears it
illumio-tui traffic groups --default-filter --window 1d
```

The CLI never applies a default on its own, so scripts and agents get the same output whatever
the profile says. `--default-filter` (on the same commands as `--filter`, and not together with
it) starts from the screen's default; it is an error when there is none or it does not parse.
`filters default <screen> <name>` sets a default (the filter must exist) and `filters default
<screen> --unset` clears it; `filters list` has a `default` column (`yes` in the table and CSV,
`default: true` in JSON) and a row for a default that names no filter.

`--filter`, `filters show|rm` and `filters default <screen>` complete saved names from the config file. Saved filters need
a profile from the config file: with `--env-file` or an env-only connection there are none.

## Agents and scripts

Every command that prints data prints one JSON envelope on stdout: `{"data": ..., "meta":
...}`. JSON is the default when stdout is not a terminal (a pipe, a file, an agent's
subprocess); `--table` and `--csv` override it, `--json` forces it at a terminal. `data` is the
command's result; `meta` says what it cost (`api_calls`, `bytes`, `elapsed_ms`, `cache`),
whether it is complete (`truncated`, and for traffic `covered`, `cap`, `incomplete`), any
`warnings`, and `meta.schema`: the name of the schema the envelope follows.

`illumio-tui schema` lists every command and its schema, plus the shared shapes (`envelope`,
`meta`, `error`). `illumio-tui schema workloads ven-summary` (the command's words) or
`illumio-tui schema write` (a schema name) prints the JSON Schema (draft 2020-12) of that
envelope: `data` of the command, `meta` shared under `$defs`, and, by command words, the
`--csv` columns in its top-level `$comment`. Several commands share one: every write prints
`write`, `--save-filter` prints `saved-filter`, traffic `--dry-run` prints `traffic-dry-run`; a
row with a flag types as listed (`illumio-tui schema events --save-filter`). `schema` needs no
profile and sends no request. Objects are closed
(`additionalProperties: false`) except where a PCE object is passed through as the PCE sent it
(a `$comment` says so); every envelope the CLI's tests read is checked against its schema, and
each schema has such a test, so a field the schema does not name is a bug.

    illumio-tui schema
    illumio-tui schema workloads ven-summary
    illumio-tui schema --json | jq -r '.data[] | select(.kind == "command") | .command'

`--fields` keeps only the named fields, a pure transform of what the command built: it never
changes what is asked of the PCE.

- With `--json` it filters `data`: each element of a list, or the object itself. A dotted path
  reaches into nested objects and through lists element by element, and keeps the nesting:
  `--fields hostname,labels.key` gives `{"hostname": "web01", "labels": [{"key": "app"}, ...]}`.
  `meta` is never filtered. The names come from the command's schema, so an unknown name is exit
  2 before any request, also when the result would be empty, and the message lists the valid
  ones. Where the output is one of several shapes, the names are those of the shape the flags
  print (`group_by` needs `workloads --group`). Below a PCE object passed through as is
  (`events` `action`, `health` `status`) any name goes: `--fields action.api_endpoint`.
- With `--csv` the names are column headers, not the JSON names (`profile list` has the column
  `default`, JSON `is_default`): `--fields` picks the columns and sets their order (`--fields
  href,key`). They are checked before any request against every column the command can print
  (`schema COMMAND` lists them; `--csv` without `--fields` prints the header row); a column this
  run does not print (a draft column without `--draft`) comes out empty. A dotted path or an
  unknown column is exit 2.
- With table output it is exit 2: `--fields needs --json or --csv`.

Comma-separated or repeated (`--fields a,b --fields c`); duplicates are dropped.

    illumio-tui workloads --label app=shop --fields hostname,health,labels.value --json
    illumio-tui policy search deny --fields ruleset,href,consumers.name --json
    illumio-tui labels --csv --fields key,value > labels.csv

A failed command prints `{"error": {"kind", "message", "exit_code"}}` to stderr (schema
`error`) and exits with `exit_code` (argument errors clap catches, such as an unknown flag or a
missing value, print clap's usage text instead, exit 2): 1 generic, 2 arguments or config, 3
auth, 4 PCE error, 5 rate limited, 6 write refused, 7 flows blocked since an enforcement. When
the reader closes stdout early (`illumio-tui schema | head -3`) the command stops quietly, with
no error, and exits 141 (128 + SIGPIPE, as a shell reports a process the signal ended).

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

### Pairing profiles

`pairing-profiles` (alias `pp`) lists the PCE's pairing profiles and creates
or deletes one. Pairing keys (activation codes) are left out: a key is a
secret. Updating a profile is not supported; delete and create it.

    illumio-tui pairing-profiles list --table
    illumio-tui pp list --csv --fields name,enabled,labels
    illumio-tui pairing-profiles create shop-prod --label app=shop --label env=prod \
        --enforcement visibility_only --visibility flow_summary --ven-type server \
        --uses 1 --lifespan 7d --lock app,env,enforcement --unlock role,loc,visibility,log-traffic \
        --description "shop web tier" --dry-run
    illumio-tui pairing-profiles create lab-endpoints --disabled --yes
    illumio-tui pairing-profiles delete shop-prod --yes

`list` is one cached read of `/pairing_profiles`, plus the labels list
(cached) when a profile carries labels, to print them as `key=value`.
Columns: name, enabled, labels, enforcement, visibility, ven type, uses per
key, key lifespan, the locks set, href; `--json` passes every profile
through as the PCE returned it, each label given its `key` and `value` from
the labels list; `--csv` prints the lifespan in seconds. The CSV columns are
the table's names (`enforcement`, `uses_per_key`, `locks`); `--json --fields`
takes the PCE's (`enforcement_mode`, `allowed_uses_per_key`, `*_lock`).

`create NAME` sends `name` plus only the flags given, so the PCE applies its
own defaults to the rest:

| Flag | Sent as |
|------|---------|
| `--description TEXT` | `description` |
| `--label KEY=VALUE` (repeatable, one per key) | `labels: [{href}]`, resolved through the labels list |
| `--enforcement idle\|visibility_only\|selective\|full` (`visibility-only` too) | `enforcement_mode` |
| `--visibility flow_summary\|flow_drops\|flow_off\|enhanced_data_collection` (or `blocked-allowed`, `blocked`, `off`, `enhanced`) | `visibility_level` |
| `--ven-type server\|endpoint\|specified-during-activation` | `ven_type` (`specified_during_activation`) |
| `--uses N\|unlimited` | `allowed_uses_per_key`: a number ≥ 1 or `"unlimited"` |
| `--lifespan DURATION\|unlimited` (`3600`, `1h`, `7d`) | `key_lifespan` in seconds, or `"unlimited"` |
| `--disabled` | `enabled: false` (the default is enabled) |
| `--lock role,app,env,loc,enforcement,visibility,log-traffic` | `role_label_lock`, `app_label_lock`, `env_label_lock`, `loc_label_lock`, `enforcement_mode_lock`, `visibility_level_lock`, `log_traffic_lock`: `true` |
| `--unlock` (the same names) | the same fields: `false` |

The PCE locks every setting whose `*_lock` is not sent: a profile created
with `--lock env` comes back with `app_label_lock: true` too. Name what
should stay changeable at pairing with `--unlock`; a name in both `--lock`
and `--unlock` is exit 2.

The name must be new: a name already in the pairing profile list (one
cached read) is exit 2, as is an unknown label (with the similar ones
listed), a second label of one key, an unknown lock or a bad value; nothing
is sent. A truncated pairing profile list is exit 2 too: no name is checked,
or deleted by name, against part of it. Sent, `data` is the created profile as the PCE returned it (the
table shows its name and href), with `meta.write` as for every write.
`delete NAME|HREF` resolves an exact name against the list (exit 2 if
unknown or ambiguous; an href is checked (this org, a pairing profile) and
used with no read), sends
`DELETE` on the href and prints `deleted NAME` (`--json`:
`{"deleted", "href"}`). Pairing profiles are not policy objects: a write
applies at once, nothing waits for provisioning. A dry run prints the request
as every write does.

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

`policy provision --impact` adds what provisioning would change (see
[Provision impact](#provision-impact)).

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

`events --type` takes the Events screen's type terms (comma-separated, `!term` hides, `*`
globs and a leading `=` match the whole type, else a substring; case-insensitive) and is repeatable; `--exclude-type X`
is `--type '!X'`. The profile's `events.exclude_types` apply unless `--all-types`. `--severity
warning+` (that and worse) sends no severity and filters here; a plain severity goes to the
PCE as before. All of these filter the loaded page: the cap (`--max-results`, default 500)
still applies, and `meta.truncated` says when the page was full. `events --actor` takes the
same terms over the actor column (`user:alex@ryte.de`, `agent:web01`, `system`), each matching
the whole actor or the name after its kind (`--actor =alex@ryte.de`, `--actor 'user:*'`), and
is repeatable; `--exclude-actor X` is `--actor '!X'` (a comma list hides each). The actor
filter is local too: the PCE events API is not asked to filter by actor (its `created_by`
query is not documented well enough to rely on). It works with `--json`, `--csv`, `--table`,
`--filter` and `--save-filter` (a saved events filter's `actor`). `meta.local_filter` names the
local filters and `meta.hidden` counts the rows they hid. They work with `--diff`.

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
refreshes by itself. With [snapshots](#snapshots) that kept readiness (`snapshot save
--with-readiness`), `:readiness` adds a `trend` column: each app's gap count over the newest
14 of them as a sparkline (`█▅▁▁`), each block in the colour of that snapshot's verdict
(needs rules red, needs provision yellow, ready and enforced green); read from the files each
time the list opens (a local listing, no request); nothing shows with fewer than two such
snapshots, nor for an app with fewer than two points.
Snapshots of another PCE are skipped (the status line says so).

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
marked ones, or everything pending when none is marked. `I` first shows what
provisioning the draft would change (the whole draft, marked or not; see
[Provision impact](#provision-impact)). The confirm overlay
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
    cargo xtask changelog                                       # CHANGELOG.md -> docs/changelog.html (--check: is it current?)
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
