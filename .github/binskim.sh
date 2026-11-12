#!/usr/bin/env bash
# Unpacks Microsoft's BinSkim, whole, into a folder.
#
#   bash .github/binskim.sh <folder>
#
# The NuGet package, which Microsoft signs, is pinned by version and SHA-256 below. Renovate raises
# the version and .github/workflows/pins.yml writes the SHA-256.
set -euo pipefail

# renovate: datasource=nuget depName=Microsoft.CodeAnalysis.BinSkim
version=4.4.9.14
sha256=ea2f48ed02e6d7ff569614d0895b80c2669fbc261c8b386639de551af049bcf4

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
  chmod +x "${out}"/tools/linux-x64/BinSkim
fi
