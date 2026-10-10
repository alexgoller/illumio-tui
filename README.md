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
install (example: macOS Apple Silicon, version 0.1.0):

    sha256sum --check --ignore-missing SHA256SUMS      # macOS: shasum -a 256 -c --ignore-missing SHA256SUMS
    tar -xzf illumio-tui-0.1.0-aarch64-apple-darwin.tar.gz
    install -m 755 illumio-tui-0.1.0-aarch64-apple-darwin/illumio-tui ~/.local/bin/   # any directory on PATH
    illumio-tui --version

macOS may quarantine binaries downloaded with a browser; clear it with
`xattr -d com.apple.quarantine ~/.local/bin/illumio-tui`. On Windows, check
the hash with `Get-FileHash illumio-tui-<version>-x86_64-pc-windows-msvc.zip`
(compare with the line in `SHA256SUMS`), extract the zip, and move
`illumio-tui.exe` into a folder listed in your `PATH`.

### Cutting a release (maintainers)

1. Bump `version` in `[workspace.package]` of the root `Cargo.toml` and run
   `cargo check` so `Cargo.lock` follows.
2. Commit, then tag and push: `git tag v0.1.0 && git push --tags`.

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
    writes = true    # optional: allow write commands (they still need --allow-writes)

`--env-file .env` loads a dotenv file for one run. Connection settings can
also be overridden per run with global flags: `--host`, `--pce-port`,
`--org-id`, `--profile`, `--config`, `--insecure`, `--no-cache` (the
connection-port flag is `--pce-port`, not `--port`, because `traffic query`
has its own `--port` service-port filter).

## TUI

    illumio-tui                      # opens the interactive UI on the default profile
    illumio-tui --profile prod
    illumio-tui --theme light        # dark is the default

Five screens: `1` Traffic, `2` Policy, `3` Labels & objects, `4` Workloads, `5` Events. The
`:` prompt takes `:traffic`, `:policy`, `:objects`, `:workloads`, `:events`, `:filter <expr>`,
`:window <dur>`, `:profile <name>`, `:draft`, `:active`, `:help` and `:q`. `?` shows the full
key map, `Esc` closes overlays, `q` or `Ctrl-C` quits. The mouse wheel scrolls the current
table and a click on a tab switches screen.

- **Traffic**: `f` edit the filter, `r` run (bypasses the cache), `v` cycle Flows / App-to-app /
  Blocked, `g` change the group keys, `s` sort, `/` search, `Enter` detail (flows) or drill into
  a pair (app-to-app), `a` auto-refresh. A filter that would be expensive asks for
  confirmation first.
- **Policy**: `d` toggle draft / active, `p` pending changes, `Tab` switch panes, `Enter` rule
  detail, or on a ruleset load its deny rules.
- **Labels & objects**: `Tab` next object type, `Enter` detail.
- **Workloads**: VEN health columns and a summary line, `h` unhealthy only, `H` sort by
  severity, `t` / `T` jump to Traffic with the workload as source / destination.
- **Events**: severity-coloured audit events, `f` edit severity / type / window, `S` sort by
  severity, `Enter` the event as JSON.

Auto-refresh (`a`) is opt-in per screen, has a 60 s minimum, shows a countdown in the panel
title, and switches itself off after a failed refresh.

`:profile <name>` reconnects to another profile from the config file. It runs that profile's
`api_secret_cmd`, so the command must be non-interactive (no prompts): the TUI owns the
terminal. Exported `PCE_HOST`, `PCE_PORT`, `PCE_ORG_ID`, `API_KEY` and `API_SECRET` still
outrank the target profile on a switch, as they do at startup; the header shows the host
actually in use.

Startup makes exactly five requests (health plus the four object lists). The first visit to
Workloads and to Events costs one request each; going back to them costs none. Everything
else is caused by a key you press. The header shows the API call and byte counters at all
times.

## CLI

    illumio-tui health
    illumio-tui labels --key app
    illumio-tui workloads --label app=web --online true
    illumio-tui policy rulesets
    illumio-tui policy rules "Web-DB"
    illumio-tui policy pending
    illumio-tui traffic query --window 24h --src app=web --dst app=db --json
    illumio-tui traffic query --window 1h --group app,env
    illumio-tui traffic query --window 1h --blocked
    illumio-tui traffic query --window 30d --dry-run
    illumio-tui profile list
    illumio-tui profile use prod

JSON output (default when piped) is `{"data": ..., "meta": {...}}` where
`meta` reports API calls, bytes, cache status, truncation, and the query
window. Errors go to stderr as JSON. Exit codes: 1 generic, 2 arguments or
config, 3 auth, 4 PCE error, 5 rate limited, 6 write refused by the guard.

## Writes

Writes are off until two things say yes: the profile (`writes = true`, or
`ILLUMIO_WRITES=1` in the environment) and the invocation (`--allow-writes`).
Every write command prints the exact request with `--dry-run` and needs
`--yes` to send it. Each sent request is appended to
`~/.local/state/illumio-tui/writes.jsonl` (or `$XDG_STATE_HOME/illumio-tui/`),
mode 0600, with the body sent and the status received. Nothing is ever retried.
IP lists and services are written to draft and wait for provisioning
(`"pending": true`); labels and workloads take effect at once.

    illumio-tui --allow-writes labels add app=web --yes
    illumio-tui --allow-writes labels add app=web --if-missing --yes     # no-op when it exists
    illumio-tui --allow-writes iplists add Corp --range 10.0.0.0/8 --fqdn '*.corp.example' --yes
    illumio-tui --allow-writes iplists update Corp --add-range 10.2.0.0/16 --dry-run
    illumio-tui --allow-writes services add PostgreSQL --port 5432/tcp --yes
    illumio-tui --allow-writes workloads add printer-1 --ip 10.0.0.50/24 --label app=print --yes
    illumio-tui --allow-writes workloads update printer-1 --add-label env=lab --yes
    illumio-tui --allow-writes workloads delete printer-1 --yes

Names resolve by exact match; an ambiguous or unknown name is exit code 2
with the candidates listed, before any request. An update with no change
flags is exit code 2 as well. Update commands re-fetch the object and send
the full list the PCE expects (`ip_ranges`, `service_ports`, `labels`), so
`--remove-range` and friends must match an existing entry. Updates of IP lists
and services refuse (exit 2) objects carrying fields this tool cannot
preserve: services with `windows_services`, `icmp_type`/`icmp_code`, or a port
without a protocol. Only unmanaged workloads can be written: one that the PCE
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
`"writes": true|false`. Rulesets, rules and provisioning are the next stage.

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
