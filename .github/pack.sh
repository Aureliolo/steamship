#!/usr/bin/env bash
# Packs one built binary as the release ships it: the binary with the README, both licences, a
# man page per command and tab completion per shell, archived with a .sha256 beside it.
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
mkdir -p "dist/${name}"
cp "target/${target}/${profile}/${binary}" README.md LICENSE-APACHE LICENSE-MIT "dist/${name}/"
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
