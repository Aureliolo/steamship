#!/usr/bin/env bash
# Downloads a release's files matching each pattern into a folder, asking again when GitHub fails
# to answer: it has answered "release not found" for a published release, once, while another job
# downloaded it at the same moment.
#
#   .github/release-download.sh <tag> <folder> <pattern>...
set -euo pipefail
: "${GITHUB_REPOSITORY:?GITHUB_REPOSITORY names the repository}"

tag="$1"
folder="$2"
shift 2
patterns=()
for pattern in "$@"; do
  patterns+=(--pattern "${pattern}")
done

for attempt in 1 2 3 4 5; do
  if gh release download "${tag}" --repo "${GITHUB_REPOSITORY}" --dir "${folder}" --clobber \
    "${patterns[@]}"; then
    exit 0
  fi
  if ((attempt < 5)); then
    echo "GitHub did not hand over ${tag}'s files; asking again in 10 seconds." >&2
    sleep 10
  fi
done
echo "::error::GitHub did not hand over ${tag}'s files five times in a row." >&2
exit 1
