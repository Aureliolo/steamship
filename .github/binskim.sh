#!/usr/bin/env bash
# Unpacks Microsoft's BinSkim, whole, into a folder.
#
#   bash .github/binskim.sh <folder>
#
# The NuGet package, which Microsoft signs, is pinned by version and SHA-256 below. Renovate raises
# the version and .github/workflows/pins.yml writes the SHA-256.
set -euo pipefail

# renovate: datasource=nuget depName=Microsoft.CodeAnalysis.BinSkim
version=4.4.9.11
sha256=69678989cbc273b5b50fcf98fb0fd978e1e35a3f844acb24254b31f0ce90c447

if [[ $# -ne 1 ]]; then
  echo "usage: bash $0 <folder to unpack into>" >&2
  exit 2
fi
out="$1"

work="$(mktemp -d)"
trap 'rm -rf "${work}"' EXIT

curl -fsS --proto '=https' --tlsv1.2 --retry 6 --retry-all-errors -o "${work}/binskim.nupkg" \
  "https://api.nuget.org/v3-flatcontainer/microsoft.codeanalysis.binskim/${version}/microsoft.codeanalysis.binskim.${version}.nupkg"
if ! echo "${sha256}  ${work}/binskim.nupkg" | sha256sum --check --strict >&2; then
  echo ".github/binskim.sh: nuget.org's package ${version} is not the one sha256= names. On a Renovate branch, pins.yml runs again on a push that sets sha256= to 64 zeros." >&2
  exit 1
fi

# Unpacked whole, since Git for Windows' unzip will not let a wildcard cross a folder.
unzip -q "${work}/binskim.nupkg" -d "${out}"
# A zip carries no executable bit, which only Linux asks for.
if [[ "${OSTYPE}" == linux* ]]; then
  chmod +x "${out}"/tools/*/linux-x64/BinSkim
fi
