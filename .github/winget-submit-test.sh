#!/usr/bin/env bash
# Runs winget-submit.sh against a fake gh and checks the calls it makes and what it prints, for
# the cases a release's re-run meets. No call reaches GitHub.
#
#   .github/winget-submit-test.sh
set -euo pipefail
export LC_ALL=C

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
work="$(mktemp -d)"
trap 'rm -rf "${work}"' EXIT

mkdir "${work}/bin" "${work}/manifests"
cp "${here}/winget-submit-fake-gh.sh" "${work}/bin/gh"
chmod +x "${work}/bin/gh"
export PATH="${work}/bin:${PATH}"

version=1.2.3
for name in Aureliolo.steamship.yaml Aureliolo.steamship.installer.yaml Aureliolo.steamship.locale.en-GB.yaml; do
  printf 'PackageIdentifier: Aureliolo.steamship\nPackageVersion: %s\n' "${version}" > "${work}/manifests/${name}"
done

cat > "${work}/template.md" << 'TEMPLATE'
<!-- PR Title Format: "New package: Publisher.Name version X.Y.Z" or "Update: Publisher.Name to X.Y.Z" -->

## Checklist

- [ ] Signed the [Contributor License Agreement](https://cla.opensource.microsoft.com)

## Manifest Checklist

- [ ] Checked that there aren't other open pull requests for the same manifest update/change
- [ ] This PR only modifies one (1) manifest
- [ ] Validated manifest locally with `winget validate --manifest <path>` (validation guide)
- [ ] Tested manifest locally with `winget install --manifest <path>`
TEMPLATE

failures=0
fail() {
  echo "FAIL: $*" >&2
  failures=$((failures + 1))
}

# Each case sets its scenario in the environment, runs the script and leaves its exit status in
# status, and its output and writing calls in ${work}/<name>.*
run_case() {
  local name="$1"
  export FAKE_LOG="${work}/${name}.log" FAKE_WRITES="${work}/${name}.writes"
  export FAKE_TEMPLATE="${work}/template.md" FAKE_VERSION="${version}"
  : > "${FAKE_LOG}"
  : > "${FAKE_WRITES}"
  status=0
  GH_TOKEN=fake bash "${here}/winget-submit.sh" "${version}" "${work}/manifests" \
    > "${work}/${name}.out" 2> "${work}/${name}.err" || status=$?
  sed -i -E 's# --input .*# --input FILE#' "${FAKE_WRITES}"
}

expect_status() {
  [[ "${status}" == "$2" ]] || fail "$1: exit status ${status}, not $2"
}

expect_file() {
  local name="$1" extension="$2" want="$3" have
  have="$(< "${work}/${name}.${extension}")"
  [[ "${have}" == "${want}" ]] || fail "${name}: ${extension} was
${have}
but should have been
${want}"
}

expect_no_writes() {
  local written
  written="$(< "${work}/$1.writes")"
  [[ -z "${written}" ]] || fail "$1: wrote to GitHub: ${written}"
}

# The number of lines of a case's writing calls that hold a fixed string.
expect_writes() {
  local name="$1" needle="$2" want="$3" what="$4" have
  have="$(grep -cF -- "${needle}" "${work}/${name}.writes" || true)"
  [[ "${have}" == "${want}" ]] || fail "${name}: ${have} writing calls hold '${needle}', so not ${what}"
}

printf -v head_id '%040d' 1
printf -v base_tree_id '%040d' 2
printf -v tree_id '%040d' 3
printf -v commit_id '%040d' 4

title="New version: Aureliolo.steamship version ${version}"
branch="Aureliolo.steamship-${version}"
paths="manifests/a/Aureliolo/steamship/${version}"
tree_call="gh api repos/releaser/winget-pkgs/git/trees --input FILE"
tree_body="{\"base_tree\":\"${base_tree_id}\",\"paths\":[\"${paths}/Aureliolo.steamship.installer.yaml\",\"${paths}/Aureliolo.steamship.locale.en-GB.yaml\",\"${paths}/Aureliolo.steamship.yaml\"]}"
commit_call="gh api repos/releaser/winget-pkgs/git/commits -f message=${title} -f tree=${tree_id} -f parents[]=${head_id} --jq .sha"
ref_call="gh api repos/releaser/winget-pkgs/git/refs -f ref=refs/heads/${branch} -f sha=${commit_id}"
patch_call="gh api --method PATCH repos/releaser/winget-pkgs/git/refs/heads/${branch} -f sha=${commit_id} -F force=true"
pull_call="gh api repos/microsoft/winget-pkgs/pulls -f title=${title} -f head=releaser:${branch} -f base=master -f body="
opened="Opened pull request #42 on microsoft/winget-pkgs for Aureliolo.steamship ${version}."

# The template with the two boxes the job proves ticked, as the pull request body carries it.
ticked="$(sed -e 's/^- \[ \] \(This PR only\)/- [x] \1/' -e 's/^- \[ \] \(Validated\)/- [x] \1/' "${work}/template.md")"

export FAKE_FORK=1 FAKE_UPSTREAM=0 FAKE_OPEN=none FAKE_DECOY=1 FAKE_BRANCH=0

# A new version makes one branch, one commit of exactly the three files and one pull request, and
# ticks only the two boxes the job proves. The decoy is an open pull request for version 1.2.30,
# which must not be taken for this one.
run_case new
expect_status new 0
expect_file new out "${opened}"
expect_file new err ""
expect_file new writes "${tree_call}
${tree_body}
${commit_call}
${ref_call}
${pull_call}${ticked} --jq .number"

# The version is already in winget-pkgs.
export FAKE_UPSTREAM=1
run_case upstream
expect_status upstream 0
expect_file upstream out "microsoft/winget-pkgs already holds Aureliolo.steamship ${version}: nothing to submit."
expect_no_writes upstream
export FAKE_UPSTREAM=0

# A pull request is open on the version's branch, or open under another branch the search finds.
for open in branch search; do
  export FAKE_OPEN="${open}"
  run_case "open-${open}"
  expect_status "open-${open}" 0
  expect_file "open-${open}" out "A pull request for Aureliolo.steamship ${version} is already open on microsoft/winget-pkgs: nothing to submit."
  expect_no_writes "open-${open}"
done
export FAKE_OPEN=none

# An earlier run failed after making the branch: it is moved to the new commit, not made again,
# and one pull request is opened.
export FAKE_BRANCH=1
run_case branch
expect_status branch 0
expect_file branch out "${opened}"
expect_file branch err ""
expect_writes branch "${tree_call}" 1 "one tree"
expect_writes branch "${commit_call}" 1 "one commit"
expect_writes branch "${patch_call}" 1 "the existing branch moved to the new commit"
expect_writes branch 'git/refs ' 0 "no second branch"
expect_writes branch "${pull_call}" 1 "one pull request"
export FAKE_BRANCH=0

# The token's account has no fork.
export FAKE_FORK=0
run_case no-fork
expect_status no-fork 1
expect_file no-fork err "::error::The token's account has no fork of microsoft/winget-pkgs. Fork it to that account, as .github/release-process.md says."
expect_file no-fork out ""
expect_no_writes no-fork
export FAKE_FORK=1

if ((failures > 0)); then
  echo "${failures} check(s) failed." >&2
  exit 1
fi
echo "winget-submit.sh: every case passed."
