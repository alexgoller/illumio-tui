# illumio-tui

Terminal UI and CLI for the Illumio Core PCE, built for administrators,
Illumio Professional Services, and agents. Read-only in this release.

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

`--env-file .env` loads a dotenv file for one run. Connection settings can
also be overridden per run with global flags: `--host`, `--pce-port`,
`--org-id`, `--profile`, `--config`, `--insecure`, `--no-cache` (the
connection-port flag is `--pce-port`, not `--port`, because `traffic query`
has its own `--port` service-port filter).

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
config, 3 auth, 4 PCE error, 5 rate limited.

## PCE protection

Every request is user-caused, throttled (2/s), cached, and counted.
Explorer queries default to a 24 h window and 1000 rows; larger windows
(> 7 d) or caps (> 10 000) need `--yes`. `--dry-run` shows the request
body without sending it.

## Development

    cargo test --workspace
    cargo xtask record --env-file ../illumio-mcp-server/.env   # refresh fixtures/recorded
    PCE_LIVE=1 cargo test -p pce --test live                    # needs PCE_* env

Design: `docs/superpowers/specs/2026-10-10-illumio-tui-design.md`.

## License

GPL-3.0-only. See `LICENSE`.
