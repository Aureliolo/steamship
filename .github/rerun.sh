#!/usr/bin/env bash
# Re-runs, once, the failed jobs of a run that a 504 from GitHub alone failed, an hour after it
# ended. Every job that failed must have ended at a recorded 504 line, and the run must still be
# the newest of its branch, so a newer commit's checks are never cancelled by a re-run of an
# older one. Only the GitHub API and the failed log are read before the one re-run call.
#
#   .github/rerun.sh <run id> <wait seconds>
#   .github/rerun.sh --dry-run <run id>
#
# GITHUB_REPOSITORY names the repository. A dry run prints what it would do and writes nothing.
set -euo pipefail
shopt -s inherit_errexit
export LC_ALL=C

# The workflows whose runs are asked. rerun.yml lists the same four.
workflows=(Binaries CI CodeQL Packages)

usage() {
  echo "Usage: rerun.sh <run id> <wait seconds> | rerun.sh --dry-run <run id>, with digits only." >&2
  exit 2
}

dry_run=0
if [[ "${1:-}" == --dry-run ]]; then
  dry_run=1
  shift
  [[ $# == 1 ]] || usage
  run_id="${1}"
  delay=0
else
  [[ $# == 2 ]] || usage
  run_id="${1}"
  delay="${2}"
fi
[[ "${run_id}" =~ ^[0-9]+$ && "${delay}" =~ ^[0-9]+$ ]] || usage

repo="${GITHUB_REPOSITORY:?GITHUB_REPOSITORY names the repository the run belongs to.}"

run_json=""
reason=""

# Prints yes when every step of the job's log that holds an error line also holds one of the
# lines GitHub's 504 leaves, as the whole line, and at least one step failed. A step starts at a
# group line that opens a run block, whichever way the log names its steps.
ended_at_network_step() {
  JOB="${1}" awk '
    BEGIN {
      job = ENVIRON["JOB"]
      bom = "\357\273\277"
      recorded["curl: (22) The requested URL returned error: 504"] = 1
      recorded["The remote server returned an error: (504) Gateway Time-out."] = 1
      recorded["ERROR The operation has timed out."] = 1
      recorded["0x801901f8 : Gateway timeout (504)."] = 1
      recorded["Unexpected HTTP response: 504"] = 1
    }
    function end_step() {
      if (failed) {
        failed_steps++
        if (!held) {
          unexplained = 1
        }
      }
      failed = 0
      held = 0
    }
    {
      first = index($0, "\t")
      if (first == 0 || substr($0, 1, first - 1) != job) {
        next
      }
      rest = substr($0, first + 1)
      second = index(rest, "\t")
      if (second == 0) {
        next
      }
      text = substr(rest, second + 1)
      if (substr(text, 1, 3) == bom) {
        text = substr(text, 4)
      }
      sub(/^[0-9][0-9][0-9][0-9]-[0-9][0-9]-[0-9][0-9]T[0-9:.]+Z ?/, "", text)
      if (substr(text, 1, 13) == "##[group]Run ") {
        end_step()
        in_step = 1
        next
      }
      if (!in_step) {
        if (substr(text, 1, 9) == "##[error]") {
          unexplained = 1
        }
        next
      }
      if (substr(text, 1, 9) == "##[error]") {
        failed = 1
      }
      if (text in recorded) {
        held = 1
      }
    }
    END {
      end_step()
      print (failed_steps > 0 && !unexplained) ? "yes" : "no"
    }
  ' <<< "${log}"
}

# Sets reason to why the run is not one to re-run, from what the run itself says.
scope_reason() {
  local name repository event branch attempt conclusion listed=0 candidate
  name="$(jq -r '.name // ""' <<< "${run_json}")"
  repository="$(jq -r '.head_repository.full_name // ""' <<< "${run_json}")"
  event="$(jq -r '.event // ""' <<< "${run_json}")"
  branch="$(jq -r '.head_branch // ""' <<< "${run_json}")"
  attempt="$(jq -r '.run_attempt // ""' <<< "${run_json}")"
  conclusion="$(jq -r '.conclusion // ""' <<< "${run_json}")"
  for candidate in "${workflows[@]}"; do
    if [[ "${name}" == "${candidate}" ]]; then
      listed=1
    fi
  done
  reason=""
  if [[ "${listed}" == 0 ]]; then
    reason="it is a ${name} run."
  elif [[ "${repository}" != "${repo}" ]]; then
    reason="it is a run of ${repository:-an unknown repository}."
  elif [[ "${event}" != pull_request && ! ("${event}" == push && "${branch}" == main) ]]; then
    reason="it is a ${event} run of ${branch}."
  elif [[ "${attempt}" != 1 ]]; then
    reason="it is on attempt ${attempt}."
  elif [[ "${conclusion}" != failure ]]; then
    reason="it ended ${conclusion}."
  fi
}

# Sets reason when a newer run of the same workflow, event and branch exists. An answer without
# a run id is an error, never "no newer run".
newer_reason() {
  local workflow_id branch event answer newest
  workflow_id="$(jq -r '.workflow_id // ""' <<< "${run_json}")"
  branch="$(jq -r '.head_branch // ""' <<< "${run_json}")"
  event="$(jq -r '.event // ""' <<< "${run_json}")"
  answer="$(gh api -X GET "repos/${repo}/actions/workflows/${workflow_id}/runs" \
    -f "branch=${branch}" -f "event=${event}" -f per_page=1)"
  newest="$(jq -r '.workflow_runs[0].id // ""' <<< "${answer}")"
  if [[ ! "${newest}" =~ ^[0-9]+$ ]]; then
    echo "GitHub named no newest run of workflow ${workflow_id} on ${branch}, so run ${run_id} is left to a person." >&2
    exit 1
  fi
  if [[ "${newest}" != "${run_id}" ]]; then
    reason="run ${newest} is newer."
  fi
}

leave() {
  echo "Run ${run_id} is left as it is: ${reason}"
  exit 0
}

run_json="$(gh api "repos/${repo}/actions/runs/${run_id}")"
jobs_json="$(gh api "repos/${repo}/actions/runs/${run_id}/attempts/1/jobs?per_page=100")"
total="$(jq -r '.total_count // ""' <<< "${jobs_json}")"
if [[ ! "${total}" =~ ^[0-9]+$ ]]; then
  echo "GitHub answered no job count for run ${run_id}, so it is left to a person." >&2
  exit 1
fi

failed_names="$(jq -b -r '.jobs[] | select(.conclusion != "success" and .conclusion != "skipped") | .name' <<< "${jobs_json}")"
failed_jobs=()
if [[ -n "${failed_names}" ]]; then
  mapfile -t failed_jobs <<< "${failed_names}"
fi

log=""
if ((${#failed_jobs[@]} > 0)); then
  log="$(gh run view "${run_id}" --repo "${repo}" --attempt 1 --log-failed)"
fi

asked=0
first_bad=""
for job in "${failed_jobs[@]}"; do
  if [[ "${job}" == *" passed" ]]; then
    echo "${job}: sums up the jobs it needs."
    continue
  fi
  asked=$((asked + 1))
  ended="$(ended_at_network_step "${job}")"
  if [[ "${ended}" == yes ]]; then
    echo "${job}: ended at a network step."
  else
    echo "${job}: did not end at a network step."
    first_bad="${first_bad:-${job}}"
  fi
done

scope_reason
if [[ -z "${reason}" ]]; then
  if ((total > 100)); then
    reason="it has more jobs than one read holds."
  elif ((asked == 0)); then
    reason="no job but an aggregator failed."
  elif [[ -n "${first_bad}" ]]; then
    reason="${first_bad} did not end at a network step."
  else
    newer_reason
  fi
fi
if [[ -n "${reason}" ]]; then
  leave
fi

if ((dry_run == 1)); then
  echo "Run ${run_id} would be re-run."
  exit 0
fi

echo "Run ${run_id} is re-run in ${delay} s."
sleep "${delay}"

run_json="$(gh api "repos/${repo}/actions/runs/${run_id}")"
scope_reason
if [[ -z "${reason}" ]]; then
  newer_reason
fi
if [[ -n "${reason}" ]]; then
  leave
fi

gh api -X POST "repos/${repo}/actions/runs/${run_id}/rerun-failed-jobs" > /dev/null
echo "Re-ran the failed jobs of run ${run_id}."
