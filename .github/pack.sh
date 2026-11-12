#!/usr/bin/env bash
# Packs one built binary as the release ships it: the binary with the README, both licences, a
# man page per command and tab completion per shell and, in a Windows archive, Microsoft's console
# with its licence, archived with a .sha256 beside it.
#
#   .github/pack.sh <version> <target> <cargo profile>
#
# It writes into dist/ and prints the archive's name.
set -euo pipefail

if [[ $# -ne 3 ]]; then
  echo "usage: $0 <version> <target> <cargo profile>" >&2
  exit 2
fi
version="$1"
target="$2"
profile="$3"

# Named for the target without Rust's placeholder vendor, which says nothing to someone choosing
# a download: x86_64-linux-musl, not x86_64-unknown-linux-musl.
name="steamship-${version}-${target/-unknown-/-}"
binary="steamship"
if [[ "${target}" == *windows* ]]; then
  binary="steamship.exe"
fi
built="target/${target}/${profile}/${binary}"
# A test build reads this variable to run a fake steamcmd in place of Valve's, and waits for a file
# of this name before it counts a limit. The feature that adds both is for tests alone and must
# never ship.
if grep -q -a -e STEAMSHIP_FAKE_STEAMCMD -e steamship-stand-in-ready "${built}"; then
  echo "${built} was built with the fake-steamcmd feature, which never ships." >&2
  exit 1
fi
mkdir -p "dist/${name}"
cp "${built}" README.md LICENSE-APACHE LICENSE-MIT "dist/${name}/"
# steamcmd runs on Microsoft's console host, whose typed input Windows' own one can leave unread.
if [[ "${target}" == *windows* ]]; then
  case "${target}" in
    x86_64*) architecture=x64 ;;
    aarch64*) architecture=arm64 ;;
    *)
      echo "${target} has no Microsoft console build." >&2
      exit 1
      ;;
  esac
  bash .github/conpty.sh "${architecture}" "dist/${name}"
  cp .github/LICENSE-CONPTY "dist/${name}/"
fi
# Made by the docs generator from the same definitions as `--help`, so the program itself carries
# no man writer.
cargo run --locked --quiet -p steamship-docs-site -- package "dist/${name}"

cd dist
if [[ "${target}" == *windows* ]]; then
  archive="${name}.zip"
  7z a -tzip -bso0 "${archive}" "${name}"
else
  archive="${name}.tar.gz"
  tar -czf "${archive}" "${name}"
fi
rm -rf "${name}"
if command -v sha256sum > /dev/null; then
  sha256sum "${archive}" > "${archive}.sha256"
else
  shasum -a 256 "${archive}" > "${archive}.sha256"
fi
echo "${archive}"
