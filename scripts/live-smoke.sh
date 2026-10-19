#!/usr/bin/env bash
# Live smoke against a real PCE you designate as non-production:
# the read-only suite (PCE_LIVE=1), then the write suite (PCE_LIVE_WRITES=1),
# which creates objects named illumio-tui-smoke-<ts> and removes or restores
# everything it changes. Never in CI.
#
#   scripts/live-smoke.sh --env-file <path> [--workload <hostname>]
#
# The env file sets PCE_HOST, PCE_ORG_ID, API_KEY, API_SECRET (optionally
# PCE_PORT, PCE_TLS_VERIFY). It is sourced, never printed. --workload names
# ONE VEN-managed workload (exact hostname) for the bulk test: its
# visibility, enforcement and one label are changed and put back. Without
# it the bulk test is skipped.
set -uo pipefail

usage() {
    echo "usage: $0 --env-file <path> [--workload <hostname>]" >&2
    exit 2
}

env_file=""
workload=""
while [ $# -gt 0 ]; do
    case "$1" in
        --env-file) [ $# -ge 2 ] || usage; env_file="$2"; shift 2 ;;
        --workload) [ $# -ge 2 ] || usage; workload="$2"; shift 2 ;;
        -h|--help) usage ;;
        *) echo "unknown argument: $1" >&2; usage ;;
    esac
done
[ -n "$env_file" ] || usage
[ -r "$env_file" ] || { echo "cannot read env file: $env_file" >&2; exit 2; }

cd "$(dirname "$0")/.." || exit 2

# Loaded quietly: nothing of the file reaches the terminal.
set +x
set -a
# shellcheck disable=SC1090
source "$env_file" >/dev/null 2>&1
set +a
for v in PCE_HOST PCE_ORG_ID API_KEY API_SECRET; do
    [ -n "${!v:-}" ] || { echo "$v is not set by $env_file" >&2; exit 2; }
done
if [ -n "$workload" ]; then
    export PCE_LIVE_WORKLOAD="$workload"
else
    unset PCE_LIVE_WORKLOAD
fi

echo "== read-only smoke (PCE_LIVE=1) against org ${PCE_ORG_ID}"
PCE_LIVE=1 cargo test -p pce --test live -- --nocapture
reads=$?

echo "== write suite (PCE_LIVE_WRITES=1)${workload:+, bulk on $workload}"
PCE_LIVE_WRITES=1 cargo test -p pce --test live_writes -- --nocapture --test-threads=1
writes=$?

verdict() { [ "$1" -eq 0 ] && echo "PASS" || echo "FAIL (exit $1)"; }
echo
echo "== summary"
echo "  read-only smoke: $(verdict $reads)"
echo "  write suite:     $(verdict $writes)"
[ -z "$workload" ] && echo "  (bulk test skipped: no --workload)"
[ "$reads" -eq 0 ] && [ "$writes" -eq 0 ]
