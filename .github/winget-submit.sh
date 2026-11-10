#!/usr/bin/env bash
# Opens the pull request that submits a release's winget manifests to microsoft/winget-pkgs, for
# release.yml's winget job.
#
#   GH_TOKEN=<classic token, public_repo> .github/winget-submit.sh <version> <manifest folder>
#
# wingetcreate is not used: its newest release cannot read the manifest schema packages.sh writes.
# The token's own fork of winget-pkgs carries the branch. A re-run finds what an earlier run
# made and never opens a second pull request. Text read from GitHub is checked against a pattern
# before it is used and is never printed, since the repositories it comes from are not ours.
set -euo pipefail

if [[ $# -ne 2 ]]; then
  echo "usage: $0 <version> <manifest folder>" >&2
  exit 2
fi
version="$1"
folder="$2"

upstream=microsoft/winget-pkgs
package=Aureliolo.steamship
title="New version: ${package} version ${version}"

fail() {
  echo "::error::$*" >&2
  exit 1
}

[[ "${version}" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || fail "${version} is not a release version."

# Sets found to 1 when the API path exists and to 0 on a 404. Any other failure ends the run,
# since a rate limit read as "absent" would open a duplicate pull request.
found=0
probe() {
  local path="$1" what="$2" answer
  if answer="$(gh api "${path}" 2>&1)"; then
    found=1
  elif [[ "${answer}" == *"HTTP 404"* ]]; then
    found=0
  else
    fail "GitHub did not answer whether ${what} exists."
  fi
}

shopt -s nullglob
files=("${folder}"/*.yaml)
shopt -u nullglob
if ((${#files[@]} != 3)); then
  fail "${folder} holds ${#files[@]} manifest files, not 3."
fi
for file in "${files[@]}"; do
  [[ "$(basename "${file}")" =~ ^Aureliolo\.steamship(\.installer|\.locale\.[A-Za-z-]+)?\.yaml$ ]] \
    || fail "${file} is not a manifest of ${package}."
done
grep -qxF "PackageVersion: ${version}" "${folder}/${package}.yaml" \
  || fail "${folder}/${package}.yaml is not for version ${version}."

login="$(gh api user --jq .login)"
[[ "${login}" =~ ^[A-Za-z0-9-]+$ ]] || fail "The token's account has a name this script does not accept."
fork="${login}/winget-pkgs"
parent="$(gh api "repos/${fork}" --jq 'select(.fork) | .parent.full_name' 2> /dev/null || true)"
if [[ "${parent}" != "${upstream}" ]]; then
  fail "The token's account has no fork of ${upstream}. Fork it to that account, as .github/release-process.md says."
fi

probe "repos/${upstream}/contents/manifests/a/Aureliolo/steamship/${version}" "${package} ${version} upstream"
if [[ "${found}" == 1 ]]; then
  echo "${upstream} already holds ${package} ${version}: nothing to submit."
  exit 0
fi

branch="${package}-${version}"
open="$(gh api --method GET "repos/${upstream}/pulls" -f state=open -f "head=${login}:${branch}" --jq length)"
searched="$(gh api --method GET search/issues -f "q=repo:${upstream} is:pr is:open author:${login} in:title ${package}")"
named="$(jq --arg version "${version}" \
  '[.items[] | select(.title | test("Aureliolo\\.steamship version " + ($version | gsub("\\."; "\\.")) + "(?![0-9.])"))] | length' \
  <<< "${searched}")"
if [[ "${open}" != 0 || "${named}" != 0 ]]; then
  echo "A pull request for ${package} ${version} is already open on ${upstream}: nothing to submit."
  exit 0
fi

base="$(gh api "repos/${upstream}" --jq .default_branch)"
[[ "${base}" =~ ^[A-Za-z0-9._-]+$ ]] || fail "${upstream}'s default branch has a name this script does not accept."
head="$(gh api "repos/${upstream}/commits/HEAD" --jq '.sha + " " + .commit.tree.sha')"
tree_base="${head#* }"
head="${head%% *}"
[[ "${head}" =~ ^[0-9a-f]{40}$ && "${tree_base}" =~ ^[0-9a-f]{40}$ ]] \
  || fail "${upstream}'s newest commit has an id this script does not accept."

request="$(mktemp)"
trap 'rm -f "${request}"' EXIT
entries='[]'
for file in "${files[@]}"; do
  entries="$(jq -c --arg path "manifests/a/Aureliolo/steamship/${version}/$(basename "${file}")" \
    --rawfile content "${file}" \
    '. + [{path: $path, mode: "100644", type: "blob", content: $content}]' <<< "${entries}")"
done
jq -n --arg base "${tree_base}" --argjson entries "${entries}" '{base_tree: $base, tree: $entries}' > "${request}"
tree="$(gh api "repos/${fork}/git/trees" --input "${request}" --jq .sha)"
[[ "${tree}" =~ ^[0-9a-f]{40}$ ]] || fail "GitHub made a tree with an id this script does not accept."
commit="$(gh api "repos/${fork}/git/commits" -f "message=${title}" -f "tree=${tree}" -f "parents[]=${head}" --jq .sha)"
[[ "${commit}" =~ ^[0-9a-f]{40}$ ]] || fail "GitHub made a commit with an id this script does not accept."

# A branch an earlier failed run left is moved to the new commit, so the pull request carries one.
probe "repos/${fork}/git/ref/heads/${branch}" "the branch ${branch}"
if [[ "${found}" == 1 ]]; then
  gh api --method PATCH "repos/${fork}/git/refs/heads/${branch}" -f "sha=${commit}" -F force=true > /dev/null
else
  gh api "repos/${fork}/git/refs" -f "ref=refs/heads/${branch}" -f "sha=${commit}" > /dev/null
fi

# The boxes ticked are the two this job proves: release.yml ran winget validate on these files,
# and the pull request holds one manifest. The rest stay the author's to tick.
template="$(gh api -H 'Accept: application/vnd.github.raw+json' "repos/${upstream}/contents/.github/PULL_REQUEST_TEMPLATE.md")"
body="$(sed -E \
  -e 's/^- \[ \] (This PR only modifies one \(1\) manifest)/- [x] \1/' \
  -e 's/^- \[ \] (Validated manifest locally with `winget validate)/- [x] \1/' <<< "${template}")"
ticked="$(grep -c '^- \[x\] ' <<< "${body}" || true)"
if [[ "${ticked}" != 2 ]]; then
  echo "::warning::The pull request template changed, so not every box this job proves was ticked."
fi

number="$(gh api "repos/${upstream}/pulls" -f "title=${title}" -f "head=${login}:${branch}" -f "base=${base}" \
  -f "body=${body}" --jq .number)"
[[ "${number}" =~ ^[0-9]+$ ]] || fail "GitHub opened a pull request with a number this script does not accept."
echo "Opened pull request #${number} on ${upstream} for ${package} ${version}."
