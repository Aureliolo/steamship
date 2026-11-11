#!/usr/bin/env bash
# Writes a published release's Homebrew formula, Scoop manifest and winget manifests from its
# checksums, each checksum verified against the release's attestation the way a user's download
# would be. For release.yml's packages (write) job and winget.yml's submit jobs.
#
#   GH_TOKEN=<token> .github/release-packages.sh <version> <output folder>
#
# Run on the release's tag, which GITHUB_REF_NAME and GITHUB_REF name.
set -euo pipefail

if [[ $# -ne 2 ]]; then
  echo "usage: $0 <version> <output folder>" >&2
  exit 2
fi
version="$1"
out="$2"

bundle="steamship-${version}.intoto.jsonl"
bash .github/release-download.sh "${GITHUB_REF_NAME}" release '*.sha256' steamship.json "${bundle}"
for subject in release/*.sha256 release/steamship.json; do
  gh attestation verify "${subject}" --bundle "release/${bundle}" \
    --repo "${GITHUB_REPOSITORY}" \
    --signer-workflow "${GITHUB_REPOSITORY}/.github/workflows/release-build.yml" \
    --source-ref "${GITHUB_REF}" --deny-self-hosted-runners
done
.github/packages.sh "${version}" release "${out}"
# The bucket and the release's own attachment are one manifest.
cmp "${out}/bucket/steamship.json" release/steamship.json
