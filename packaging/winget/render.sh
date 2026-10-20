#!/usr/bin/env bash
# Render the winget manifests (schema 1.9.0: version, installer, defaultLocale)
# for the per-user MSI.
#   render.sh <version> <SHA256SUMS> <outdir>
# Writes <outdir>/AlexGoller.IllumioTUI{,.installer,.locale.en-US}.yaml.
# Environment:
#   BASE_URL      where the MSI is downloaded from; default: the GitHub
#                 release of v<version>. CI points it at a local server for
#                 builds that are not published.
#   PRODUCT_CODE  the MSI's ProductCode ({GUID}); optional. Set, it is added
#                 to the installer entry. It is new on every MSI build, so it
#                 must come from the MSI that BASE_URL serves.
# The UpgradeCode in installer.yaml.tmpl must match packaging/windows/illumio-tui.wxs.
# Fails if the MSI is missing from SHA256SUMS.
set -euo pipefail
version="$1"
sums="$2"
outdir="$3"
here=$(dirname "$0")
id=AlexGoller.IllumioTUI
file="illumio-tui-$version-x86_64-pc-windows-msvc.msi"
base_url="${BASE_URL:-https://github.com/alexgoller/illumio-tui/releases/download/v$version}"

sha=$(awk -v f="$file" '$2 == f || $2 == "*" f { print $1 }' "$sums")
if [[ ! "$sha" =~ ^[0-9a-fA-F]{64}$ ]]; then
  echo "render.sh: no sha256 for $file in $sums" >&2
  exit 1
fi
sha=$(tr 'a-f' 'A-F' <<<"$sha")
product_code="${PRODUCT_CODE:-}"
if [[ -n "$product_code" && ! "$product_code" =~ ^\{[0-9A-F]{8}(-[0-9A-F]{4}){3}-[0-9A-F]{12}\}$ ]]; then
  echo "render.sh: PRODUCT_CODE '$product_code' is not an uppercase {GUID}" >&2
  exit 1
fi

render() {
  local out
  out=$(cat "$1")
  out="${out//@VERSION@/$version}"
  out="${out//@INSTALLER_URL@/$base_url/$file}"
  out="${out//@INSTALLER_SHA256@/$sha}"
  if grep -q '@[A-Z0-9_]*@' <<<"$out"; then
    echo "render.sh: unfilled placeholder left in $1" >&2
    exit 1
  fi
  if [[ -n "$product_code" ]]; then
    out=$(awk -v pc="$product_code" '{ print } /^  InstallerSha256: / { print "  ProductCode: '"'"'" pc "'"'"'" }' <<<"$out")
  fi
  printf '%s\n' "$out"
}

mkdir -p "$outdir"
render "$here/version.yaml.tmpl" > "$outdir/$id.yaml"
render "$here/installer.yaml.tmpl" > "$outdir/$id.installer.yaml"
render "$here/locale.en-US.yaml.tmpl" > "$outdir/$id.locale.en-US.yaml"
