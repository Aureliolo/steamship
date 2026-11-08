#!/usr/bin/env bash
# Downloads one file of another project's release into a folder and refuses it unless it has the
# pinned SHA-256. gh fetches it through GitHub's API: github.com's releases/download addresses
# have answered 504 for every repository while the API served the same files.
#
#   .github/tool-download.sh <repository> <tag> <file> <sha256> <folder>
set -euo pipefail

if [[ $# -ne 5 ]]; then
  echo "usage: $0 <repository> <tag> <file> <sha256> <folder>" >&2
  exit 2
fi
repository="$1"
tag="$2"
file="$3"
sha256="$4"
folder="$5"

downloaded=false
for wait in 1 2 4 8 16 32 0; do
  if gh release download "${tag}" --repo "${repository}" --pattern "${file}" --dir "${folder}" \
    --clobber; then
    downloaded=true
    break
  fi
  if ((wait > 0)); then
    echo "GitHub did not hand over ${repository} ${tag}'s ${file}; asking again in ${wait} s." >&2
    sleep "${wait}"
  fi
done
if [[ "${downloaded}" != true ]]; then
  echo "::error::GitHub did not hand over ${repository} ${tag}'s ${file} seven times in a row." >&2
  exit 1
fi

# macOS has a sha256sum of BSD's, which knows no --strict.
if [[ "$(uname -s)" == Darwin ]]; then
  echo "${sha256}  ${folder}/${file}" | shasum -a 256 --check
else
  echo "${sha256}  ${folder}/${file}" | sha256sum --check --strict
fi
