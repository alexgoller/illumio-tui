# illumio-tui

Terminal UI and CLI for the Illumio Core PCE, built for administrators,
Illumio Professional Services, and agents. Read-only in this release.

## Install

    cargo install --path crates/illumio-tui

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
