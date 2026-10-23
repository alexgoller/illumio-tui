#!/usr/bin/env bash
# Print the release notes for one tag from CHANGELOG.md, as the GitHub
# release body: the section under `## <tag> — <date>`, without that heading,
# its `###` headings back at `##`. Exits 1 with nothing on stdout when
# CHANGELOG.md has no section for the tag (the release workflow then falls
# back to GitHub's generated notes).
#
#   scripts/release-notes.sh v0.5.3 [CHANGELOG.md]
set -euo pipefail

tag=${1:?usage: $0 TAG [CHANGELOG.md]}
file=${2:-CHANGELOG.md}
[[ -f "$file" ]] || exit 1

notes=$(awk -v tag="$tag" '
    # A version heading: "## v0.5.3 — 2026-10-23" (or "## v0.5.3").
    function is_version(line) { return line ~ /^## v[0-9]/ }
    !fence && is_version($0) {
        if (found) exit
        split($0, f, " ")
        if (f[2] == tag) { found = 1; next }
    }
    !found { next }
    /^[ \t]*```/ { fence = !fence; print; next }
    !fence && /^###+ / { sub(/^#/, "") }
    { print }
' "$file")

# Trim the blank lines around the section.
notes=$(printf '%s\n' "$notes" | sed -e '/./,$!d')
[[ -n "$notes" ]] || exit 1
printf '%s\n' "$notes"
