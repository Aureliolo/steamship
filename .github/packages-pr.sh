#!/usr/bin/env bash
# Lands a release's Homebrew formula and Scoop manifest on main through a pull request that
# merges once every required check passes, for release.yml's packages (main) job.
#
#   .github/packages-pr.sh open <version> <folder>  opens the pull request, or finds the one open;
#                                                   prints pr= and head=, or done=true when main
#                                                   already holds these files
#   .github/packages-pr.sh wait <pr>                waits for its checks, printing state=clean,
#                                                   behind or merged; anything else fails
#   .github/packages-pr.sh act <pr> <state>         merges a clean one, brings a behind one up to
#                                                   date with main
#
# Every refusal ends the job with the reason: a formula that never reaches main has to be seen.
set -euo pipefail
: "${GH_REPO:?GH_REPO names the repository}"

files=(Formula/steamship.rb bucket/steamship.json)

fail() {
  echo "::error::$*" >&2
  exit 1
}

open_pr() {
  local version="$1" folder="$2"
  local branch="packages/v${version}"
  local main path have changed=0
  main="$(gh api "repos/${GH_REPO}/git/ref/heads/main" --jq .object.sha)"
  for path in "${files[@]}"; do
    if ! have="$(gh api "repos/${GH_REPO}/contents/${path}?ref=${main}" --jq .content 2> /dev/null | base64 -d)"; then
      have=""
    fi
    if [[ "${have}" != "$(cat "${folder}/${path}")" ]]; then
      changed=1
    fi
  done
  if [[ "${changed}" == 0 ]]; then
    echo "done=true"
    return
  fi

  # Left by an earlier run: started again from main, so the pull request carries one commit.
  if gh api "repos/${GH_REPO}/git/ref/heads/${branch}" > /dev/null 2>&1; then
    gh api -X PATCH "repos/${GH_REPO}/git/refs/heads/${branch}" -f "sha=${main}" -F force=true > /dev/null
  else
    gh api "repos/${GH_REPO}/git/refs" -f "ref=refs/heads/${branch}" -f "sha=${main}" > /dev/null
  fi

  # Committed through the API, which GitHub signs; main takes only signed commits.
  local head request
  request="$(mktemp)"
  jq -n \
    --arg repository "${GH_REPO}" \
    --arg branch "${branch}" \
    --arg main "${main}" \
    --arg headline "Package v${version} for Homebrew and Scoop" \
    --arg formula "$(base64 -w0 "${folder}/Formula/steamship.rb")" \
    --arg manifest "$(base64 -w0 "${folder}/bucket/steamship.json")" \
    '{
      query: "mutation($input: CreateCommitOnBranchInput!) { createCommitOnBranch(input: $input) { commit { oid } } }",
      variables: {input: {
        branch: {repositoryNameWithOwner: $repository, branchName: $branch},
        expectedHeadOid: $main,
        message: {headline: $headline},
        fileChanges: {additions: [
          {path: "Formula/steamship.rb", contents: $formula},
          {path: "bucket/steamship.json", contents: $manifest}
        ]}
      }}
    }' > "${request}"
  head="$(gh api graphql --input "${request}" --jq .data.createCommitOnBranch.commit.oid)"
  rm -f "${request}"

  local pr
  pr="$(gh pr list --head "${branch}" --state open --json number --jq '.[0].number // empty')"
  if [[ -z "${pr}" ]]; then
    pr="$(gh pr create --base main --head "${branch}" \
      --title "Package v${version} for Homebrew and Scoop" \
      --body "The Homebrew formula and the Scoop manifest for v${version}, written by the release from its signed checksums. It merges once every required check passes." \
      | sed -E 's|.*/pull/([0-9]+)$|\1|')"
  fi
  echo "pr=${pr}"
  echo "head=${head}"
}

wait_for() {
  local pr="$1"
  local deadline=$((SECONDS + 100 * 60))
  local view state head held failed
  while ((SECONDS < deadline)); do
    view="$(gh pr view "${pr}" --json state,mergeStateStatus,reviewDecision,headRefOid,statusCheckRollup)"
    state="$(jq -r .state <<< "${view}")"
    case "${state}" in
      MERGED)
        echo "state=merged"
        return
        ;;
      CLOSED) fail "Pull request #${pr} was closed without merging." ;;
    esac
    head="$(jq -r .headRefOid <<< "${view}")"
    held="$(gh api "repos/${GH_REPO}/actions/runs?head_sha=${head}" \
      --jq '[.workflow_runs[] | select(.status == "action_required")] | length')"
    if [[ "${held}" != 0 ]]; then
      fail "Pull request #${pr}'s checks are waiting for someone to approve them, which happens when it is not opened as the packaging app."
    fi
    failed="$(jq -r '.statusCheckRollup[]
      | select(((.conclusion // "") | IN("FAILURE", "CANCELLED", "TIMED_OUT", "ACTION_REQUIRED", "STARTUP_FAILURE"))
          or ((.state // "") | IN("FAILURE", "ERROR")))
      | .name // .context' <<< "${view}")"
    if [[ -n "${failed}" ]]; then
      fail "Pull request #${pr} failed: $(tr '\n' ',' <<< "${failed}" | sed 's/,$//')."
    fi
    if [[ "$(jq -r .reviewDecision <<< "${view}")" == "REVIEW_REQUIRED" ]]; then
      fail "Pull request #${pr} needs an approving review before it can merge; see .github/release-process.md."
    fi
    case "$(jq -r .mergeStateStatus <<< "${view}")" in
      CLEAN)
        echo "state=clean"
        return
        ;;
      BEHIND)
        echo "state=behind"
        return
        ;;
      DIRTY) fail "Pull request #${pr} conflicts with main." ;;
    esac
    sleep 30
  done
  fail "Pull request #${pr}'s checks did not finish within 100 minutes."
}

act() {
  local pr="$1" state="$2"
  local head
  head="$(gh pr view "${pr}" --json headRefOid --jq .headRefOid)"
  case "${state}" in
    merged) ;;
    clean) gh pr merge "${pr}" --squash --match-head-commit "${head}" ;;
    behind)
      gh api -X PUT "repos/${GH_REPO}/pulls/${pr}/update-branch" -f "expected_head_sha=${head}" > /dev/null
      ;;
    *) fail "No action for state ${state}." ;;
  esac
}

case "${1:-}" in
  open) open_pr "$2" "$3" ;;
  wait) wait_for "$2" ;;
  act) act "$2" "$3" ;;
  *)
    echo "usage: $0 open <version> <folder> | wait <pr> | act <pr> <state>" >&2
    exit 2
    ;;
esac
