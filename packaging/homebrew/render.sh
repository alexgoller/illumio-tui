#!/usr/bin/env bash
# Render the Homebrew formula from the template.
#   render.sh <version> <SHA256SUMS> [template] > illumio-tui.rb
# Fails if any archive the formula needs is missing from SHA256SUMS.
set -euo pipefail
version="$1"
sums="$2"
tmpl="${3:-$(dirname "$0")/illumio-tui.rb.tmpl}"

out=$(cat "$tmpl")
out="${out//@VERSION@/$version}"
for target in aarch64-apple-darwin x86_64-apple-darwin aarch64-unknown-linux-gnu x86_64-unknown-linux-musl; do
  file="illumio-tui-$version-$target.tar.gz"
  sha=$(awk -v f="$file" '$2 == f || $2 == "*" f { print $1 }' "$sums")
  if [[ ! "$sha" =~ ^[0-9a-f]{64}$ ]]; then
    echo "render.sh: no sha256 for $file in $sums" >&2
    exit 1
  fi
  key="@SHA256_$(echo "$target" | tr 'a-z-' 'A-Z_')@"
  out="${out//$key/$sha}"
done
if grep -q '@[A-Z0-9_]*@' <<<"$out"; then
  echo "render.sh: unfilled placeholder left in template" >&2
  exit 1
fi
printf '%s\n' "$out"
