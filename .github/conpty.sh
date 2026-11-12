#!/usr/bin/env bash
# Puts Microsoft's ConPTY, conpty.dll and OpenConsole.exe, into a folder for one architecture.
#
#   bash .github/conpty.sh <x64|arm64> <folder>
#
# The package is pinned by version and SHA-256 below. Renovate raises the version and
# .github/workflows/pins.yml writes the SHA-256. The folder is made if missing and files already
# in it stay. Stdout stays empty, because the release reads pack.sh's stdout as the archive's name.
set -euo pipefail

# renovate: datasource=nuget depName=Microsoft.Windows.Console.ConPTY
version=1.25.261109001
sha256=3d77f535df848e44225c2a8912ac844fecb23f3057732b639f3034b587496f09

if [[ $# -ne 2 || ! "$1" =~ ^(x64|arm64)$ ]]; then
  echo "usage: bash $0 <x64|arm64> <folder to write into>" >&2
  exit 2
fi
arch="$1"
out="$2"

work="$(mktemp -d)"
trap 'rm -rf "${work}"' EXIT

curl -fsS --proto '=https' --tlsv1.2 --retry 6 --retry-all-errors -o "${work}/conpty.nupkg" \
  "https://api.nuget.org/v3-flatcontainer/microsoft.windows.console.conpty/${version}/microsoft.windows.console.conpty.${version}.nupkg"
if ! echo "${sha256}  ${work}/conpty.nupkg" | sha256sum --check --strict >&2; then
  echo ".github/conpty.sh: nuget.org's package ${version} is not the one sha256= names. On a Renovate branch, pins.yml runs again on a push that sets sha256= to 64 zeros." >&2
  exit 1
fi

unzip -q "${work}/conpty.nupkg" -d "${work}/package"
mkdir -p "${out}"
cp "${work}/package/runtimes/win-${arch}/native/conpty.dll" "${out}/conpty.dll"
cp "${work}/package/build/native/runtimes/${arch}/OpenConsole.exe" "${out}/OpenConsole.exe"
