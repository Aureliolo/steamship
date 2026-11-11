#!/usr/bin/env bash
# Writes a published release's Homebrew formula, Scoop manifest and winget manifests from its
# checksums, each checksum verified against the release's attestation the way a user's download
# would be.
#
#   GH_TOKEN=<token> .github/release-packages.sh <tag> <output folder>
set -euo pipefail

: "${GITHUB_REPOSITORY:?GITHUB_REPOSITORY names the repository}"
if [[ $# -ne 2 ]]; then
  echo "usage: $0 <tag> <output folder>" >&2
  exit 2
fi
tag="$1"
out="$2"
version="${tag#v}"

bundle="steamship-${version}.intoto.jsonl"
bash .github/release-download.sh "${tag}" release '*.sha256' steamship.json "${bundle}"
for subject in release/*.sha256 release/steamship.json; do
  gh attestation verify "${subject}" --bundle "release/${bundle}" \
    --repo "${GITHUB_REPOSITORY}" \
    --signer-workflow "${GITHUB_REPOSITORY}/.github/workflows/release-build.yml" \
    --source-ref "refs/tags/${tag}" --deny-self-hosted-runners
done
.github/packages.sh "${version}" release "${out}"
# The bucket and the release's own attachment are one manifest.
cmp "${out}/bucket/steamship.json" release/steamship.json
