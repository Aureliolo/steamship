#!/usr/bin/env bash
# Runs rerun.sh against a fake gh and checks the calls it makes and what it prints, for the logs
# and runs a re-run decision meets. No call reaches GitHub.
#
#   .github/rerun-test.sh
#
# RERUN_SH names another copy of the script to test, such as one with a rule left out.
# RERUN_CASE names the one case to run.
# shellcheck disable=SC2016 # Log lines hold literal ${...} and backtick text from real logs.
set -euo pipefail
export LC_ALL=C

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
rerun="${RERUN_SH:-${here}/rerun.sh}"
work="$(mktemp -d)"
trap 'rm -rf "${work}"' EXIT

mkdir "${work}/bin"
cp "${here}/rerun-fake-gh.sh" "${work}/bin/gh"
chmod +x "${work}/bin/gh"
export PATH="${work}/bin:${PATH}"
export GITHUB_REPOSITORY=Aureliolo/steamship GH_TOKEN=fake

repo=Aureliolo/steamship
stamp=2026-11-08T10:15:20.5162667Z
bom=$'\xef\xbb\xbf'
scoop_job='packages (scoop x64)'
scoop_step='Install by address, then from this repository as a bucket'
failures=0
dir=""
status=0

fail() {
  echo "FAIL: $*" >&2
  failures=$((failures + 1))
}

# Fixtures. A log line is "job<TAB>step<TAB>stamp text", as gh run view --log-failed prints it.
write_run() {
  jq -n --arg name "${2}" --arg event "${3}" --arg branch "${4}" --arg attempt "${5}" \
    --arg conclusion "${6}" --arg repository "${7}" \
    '{id: 1001, name: $name, head_branch: $branch, event: $event, status: "completed",
      conclusion: $conclusion, workflow_id: 368372655, run_attempt: ($attempt | tonumber),
      head_repository: {full_name: $repository}}' > "${1}"
}

write_newest() {
  jq -n --argjson id "${2}" '{total_count: 1, workflow_runs: [{id: $id, run_attempt: 1}]}' > "${1}"
}

# write_jobs <file> <total count> <name=conclusion>...
write_jobs() {
  local file="${1}" total="${2}"
  shift 2
  printf '%s\n' "$@" | jq -R -s --argjson total "${total}" \
    '{total_count: $total, jobs: (split("\n")[:-1] | map(capture("^(?<name>.*)=(?<conclusion>[a-z_]+)$")
      | {id: 128700321237, run_id: 1001, run_attempt: 1, status: "completed", name, conclusion}))}' > "${file}"
}

log_line() {
  printf '%s\t%s\t%s %s\n' "${1}" "${2}" "${stamp}" "${3}" >> "${dir}/failed.log"
}

# A step of the older form: the failed step alone, named.
scoop_lines() {
  local line
  log_line "${scoop_job}" "${scoop_step}" "##[group]Run \$ErrorActionPreference = 'Stop'"
  for line in "$@"; do
    log_line "${scoop_job}" "${scoop_step}" "${line}"
  done
  log_line "${scoop_job}" "${scoop_step}" '##[error]Process completed with exit code 1.'
}

machete_lines() {
  local job="${1}" step="${2}"
  log_line "${job}" "${step}" '##[group]Run cargo machete'
  log_line "${job}" "${step}" '^[[1m^[[91merror^[[0m: no such command: `machete`'
  log_line "${job}" "${step}" '##[error]Process completed with exit code 101.'
}

# Fixture P: Packages run 1001, a pull request, attempt 1, failed, newest of its branch, with the
# Scoop job failed on the given line.
fixture_p() {
  dir="${work}/${1}"
  mkdir -p "${dir}"
  write_run "${dir}/run.json" Packages pull_request ci-tools-through-api 1 failure "${repo}"
  write_jobs "${dir}/jobs.json" 2 "${scoop_job}=failure" 'packages (winget x64)=success'
  write_newest "${dir}/newest.json" 1001
  : > "${dir}/failed.log"
  scoop_lines "${2:-curl: (22) The requested URL returned error: 504}"
}

# Runs rerun.sh with the arguments and leaves its exit status in status, its output in
# ${dir}/out and the calls the fake gh saw in ${dir}/calls.
run_rerun() {
  export FAKE_DIR="${dir}" FAKE_CALLS="${dir}/calls"
  : > "${FAKE_CALLS}"
  status=0
  bash "${rerun}" "$@" > "${dir}/out" 2> "${dir}/err" || status=$?
}

posts() {
  grep -c -F 'api -X POST' "${dir}/calls" || true
}

reads_of_newest() {
  grep -c -F 'actions/workflows/' "${dir}/calls" || true
}

expect_status() {
  [[ "${status}" == "${2}" ]] || fail "${1}: exit status ${status}, not ${2}"
}

expect_out() {
  local have
  have="$(< "${dir}/out")"
  [[ "${have}" == "${2}" ]] || fail "${1}: output was
${have}
but should have been
${2}"
}

expect_posts() {
  local have
  have="$(posts)"
  [[ "${have}" == "${2}" ]] || fail "${1}: ${have} re-run calls, not ${2}"
}

scoop_ended="${scoop_job}: ended at a network step."
scoop_not="${scoop_job}: did not end at a network step."

# Each case below is a function named after what it shows. Part of the description of each is
# the wrong build it exists to catch.

each_recorded_line_re_runs_its_run() {
  local line
  for line in \
    'curl: (22) The requested URL returned error: 504' \
    'The remote server returned an error: (504) Gateway Time-out.' \
    'ERROR The operation has timed out.' \
    '0x801901f8 : Gateway timeout (504).' \
    'Unexpected HTTP response: 504'; do
    fixture_p recorded "${line}"
    run_rerun 1001 0
    expect_status "each recorded line (${line})" 0
    expect_out "each recorded line (${line})" "${scoop_ended}
Run 1001 is re-run in 0 s.
Re-ran the failed jobs of run 1001."
    expect_posts "each recorded line (${line})" 1
  done
  local calls
  calls="$(< "${dir}/calls")"
  [[ "${calls}" == "gh api repos/${repo}/actions/runs/1001
gh api repos/${repo}/actions/runs/1001/attempts/1/jobs?per_page=100
gh run view 1001 --repo ${repo} --attempt 1 --log-failed
gh api -X GET repos/${repo}/actions/workflows/368372655/runs -f branch=ci-tools-through-api -f event=pull_request -f per_page=1
gh api repos/${repo}/actions/runs/1001
gh api -X GET repos/${repo}/actions/workflows/368372655/runs -f branch=ci-tools-through-api -f event=pull_request -f per_page=1
gh api -X POST repos/${repo}/actions/runs/1001/rerun-failed-jobs" ]] \
    || fail "each recorded line: the calls were
${calls}"
}

two_errors_in_one_step_are_one_step() {
  fixture_p two_errors
  : > "${dir}/failed.log"
  log_line "${scoop_job}" "${scoop_step}" '##[group]Run brew install steamship'
  log_line "${scoop_job}" "${scoop_step}" '##[error]Failed to download resource "steamship (0.9.91)"'
  local _
  for _ in 1 2 3 4; do
    log_line "${scoop_job}" "${scoop_step}" 'curl: (22) The requested URL returned error: 504'
  done
  log_line "${scoop_job}" "${scoop_step}" '##[error]Process completed with exit code 1.'
  run_rerun 1001 0
  expect_out "two errors in one step" "${scoop_ended}
Run 1001 is re-run in 0 s.
Re-ran the failed jobs of run 1001."
  expect_posts "two errors in one step" 1
}

an_aggregator_follows_its_jobs() {
  fixture_p aggregator_follows
  write_run "${dir}/run.json" CodeQL push main 1 failure "${repo}"
  write_jobs "${dir}/jobs.json" 2 'codeql (actions)=failure' 'codeql passed=failure'
  : > "${dir}/failed.log"
  log_line 'codeql (actions)' 'UNKNOWN STEP' '##[group]Run github/codeql-action/init@977e6ceaea7361825998245d787fa3b4d6b9e5df'
  log_line 'codeql (actions)' 'UNKNOWN STEP' 'Unexpected HTTP response: 504'
  log_line 'codeql (actions)' 'UNKNOWN STEP' '##[error]Unable to download and extract CodeQL CLI: Unexpected HTTP response: 504'
  log_line 'codeql passed' 'UNKNOWN STEP' '##[group]Run echo "codeql: ${RESULT}"'
  log_line 'codeql passed' 'UNKNOWN STEP' '##[error]Process completed with exit code 1.'
  run_rerun 1001 0
  expect_out "an aggregator follows its jobs" "codeql (actions): ended at a network step.
codeql passed: sums up the jobs it needs.
Run 1001 is re-run in 0 s.
Re-ran the failed jobs of run 1001."
  expect_posts "an aggregator follows its jobs" 1
}

an_aggregator_alone_is_left() {
  fixture_p aggregator_alone
  write_run "${dir}/run.json" CodeQL push main 1 failure "${repo}"
  write_jobs "${dir}/jobs.json" 2 'codeql (actions)=success' 'codeql passed=failure'
  : > "${dir}/failed.log"
  log_line 'codeql passed' 'UNKNOWN STEP' '##[group]Run echo "codeql: ${RESULT}"'
  log_line 'codeql passed' 'UNKNOWN STEP' '##[error]Process completed with exit code 1.'
  run_rerun 1001 0
  expect_out "an aggregator alone" "codeql passed: sums up the jobs it needs.
Run 1001 is left as it is: no job but an aggregator failed."
  expect_posts "an aggregator alone" 0
}

a_real_failure_beside_a_504_leaves_the_run() {
  fixture_p real_beside
  write_jobs "${dir}/jobs.json" 3 "${scoop_job}=failure" 'static checks=failure' 'packages (winget x64)=success'
  machete_lines 'static checks' 'UNKNOWN STEP'
  run_rerun 1001 0
  expect_out "a real failure beside a 504" "${scoop_ended}
static checks: did not end at a network step.
Run 1001 is left as it is: static checks did not end at a network step."
  expect_posts "a real failure beside a 504" 0
}

a_real_failed_step_after_a_504_step_leaves_the_run() {
  fixture_p real_after
  machete_lines "${scoop_job}" "${scoop_step}"
  run_rerun 1001 0
  expect_out "a real failed step after a 504 step" "${scoop_not}
Run 1001 is left as it is: ${scoop_job} did not end at a network step."
  expect_posts "a real failed step after a 504 step" 0
}

a_commit_sha_is_no_504() {
  fixture_p sha
  : > "${dir}/failed.log"
  log_line "${scoop_job}" "${scoop_step}" '##[group]Run .github/packages-pr.sh wait "${PR}"'
  log_line "${scoop_job}" "${scoop_step}" '91fe38348fe0353c1de35cc503ef47ed7761a24c'
  log_line "${scoop_job}" "${scoop_step}" '##[error]Pull request #278 failed: deny.'
  run_rerun 1001 0
  expect_out "a commit sha is no 504" "${scoop_not}
Run 1001 is left as it is: ${scoop_job} did not end at a network step."
  expect_posts "a commit sha is no 504" 0
}

a_504_inside_a_longer_line_is_no_match() {
  fixture_p longer_line
  : > "${dir}/failed.log"
  log_line "${scoop_job}" "${scoop_step}" '##[group]Run github/codeql-action/init@977e6ceaea7361825998245d787fa3b4d6b9e5df'
  log_line "${scoop_job}" "${scoop_step}" '##[error]Unable to download and extract CodeQL CLI: Unexpected HTTP response: 504'
  run_rerun 1001 0
  expect_out "a 504 inside a longer line" "${scoop_not}
Run 1001 is left as it is: ${scoop_job} did not end at a network step."
  expect_posts "a 504 inside a longer line" 0
}

a_504_in_a_step_that_passed_is_no_match() {
  fixture_p passed_step
  : > "${dir}/failed.log"
  log_line "${scoop_job}" 'UNKNOWN STEP' '##[group]Run set -euo pipefail'
  log_line "${scoop_job}" 'UNKNOWN STEP' 'curl: (22) The requested URL returned error: 504'
  machete_lines "${scoop_job}" 'UNKNOWN STEP'
  run_rerun 1001 0
  expect_out "a 504 in a step that passed" "${scoop_not}
Run 1001 is left as it is: ${scoop_job} did not end at a network step."
  expect_posts "a 504 in a step that passed" 0
}

a_failed_job_without_log_lines_leaves_the_run() {
  fixture_p no_log_lines
  write_jobs "${dir}/jobs.json" 2 "${scoop_job}=failure" 'packages (winget x64)=cancelled'
  run_rerun 1001 0
  expect_out "a failed job without log lines" "${scoop_ended}
packages (winget x64): did not end at a network step.
Run 1001 is left as it is: packages (winget x64) did not end at a network step."
  expect_posts "a failed job without log lines" 0
}

runs_outside_its_scope_are_left() {
  local label name event branch attempt conclusion repository reason
  while IFS='|' read -r label name event branch attempt conclusion repository reason; do
    fixture_p "scope_${label}"
    write_run "${dir}/run.json" "${name}" "${event}" "${branch}" "${attempt}" "${conclusion}" "${repository}"
    run_rerun 1001 0
    expect_out "scope: ${label}" "${scoop_ended}
Run 1001 is left as it is: ${reason}"
    expect_posts "scope: ${label}" 0
  done << 'SCOPE'
mutants|Mutants|pull_request|ci-tools-through-api|1|failure|Aureliolo/steamship|it is a Mutants run.
release|Release|push|main|1|failure|Aureliolo/steamship|it is a Release run.
winget|winget|push|main|1|failure|Aureliolo/steamship|it is a winget run.
links|Links|pull_request|ci-tools-through-api|1|failure|Aureliolo/steamship|it is a Links run.
schedule|CodeQL|schedule|main|1|failure|Aureliolo/steamship|it is a schedule run of main.
push_elsewhere|CI|push|feature|1|failure|Aureliolo/steamship|it is a push run of feature.
fork|Packages|pull_request|ci-tools-through-api|1|failure|someone/steamship|it is a run of someone/steamship.
cancelled|Packages|pull_request|ci-tools-through-api|1|cancelled|Aureliolo/steamship|it ended cancelled.
attempt_failed|Packages|pull_request|ci-tools-through-api|2|failure|Aureliolo/steamship|it is on attempt 2.
attempt_success|Packages|pull_request|ci-tools-through-api|2|success|Aureliolo/steamship|it is on attempt 2.
SCOPE
}

every_listed_workflow_re_runs() {
  local name
  for name in Binaries CI CodeQL Packages; do
    fixture_p "listed_${name}"
    write_run "${dir}/run.json" "${name}" pull_request ci-tools-through-api 1 failure "${repo}"
    run_rerun 1001 0
    expect_out "listed workflow ${name}" "${scoop_ended}
Run 1001 is re-run in 0 s.
Re-ran the failed jobs of run 1001."
    expect_posts "listed workflow ${name}" 1
  done
}

a_newer_run_leaves_the_run() {
  fixture_p newer
  write_newest "${dir}/newest.json" 1002
  run_rerun 1001 0
  expect_out "a newer run" "${scoop_ended}
Run 1001 is left as it is: run 1002 is newer."
  expect_posts "a newer run" 0
}

a_newer_run_during_the_wait_leaves_the_run() {
  fixture_p newer_wait
  write_newest "${dir}/newest-2.json" 1002
  run_rerun 1001 0
  expect_out "a newer run during the wait" "${scoop_ended}
Run 1001 is re-run in 0 s.
Run 1001 is left as it is: run 1002 is newer."
  expect_posts "a newer run during the wait" 0
  local reads
  reads="$(reads_of_newest)"
  [[ "${reads}" == 2 ]] || fail "a newer run during the wait: the newest run was read ${reads} times, not 2"
}

a_re_run_by_hand_during_the_wait_leaves_the_run() {
  fixture_p by_hand
  write_run "${dir}/run-2.json" Packages pull_request ci-tools-through-api 2 failure "${repo}"
  run_rerun 1001 0
  expect_out "a re-run by hand during the wait" "${scoop_ended}
Run 1001 is re-run in 0 s.
Run 1001 is left as it is: it is on attempt 2."
  expect_posts "a re-run by hand during the wait" 0
}

a_dry_run_writes_nothing() {
  fixture_p dry
  run_rerun --dry-run 1001
  expect_status "a dry run" 0
  expect_out "a dry run" "${scoop_ended}
Run 1001 would be re-run."
  expect_posts "a dry run" 0
}

an_id_that_is_not_digits_is_refused() {
  local args
  fixture_p not_digits
  for args in '12a 0' '1001 abc' '--dry-run 12a' '1001'; do
    # shellcheck disable=SC2086 # The arguments are split on purpose.
    run_rerun ${args}
    expect_status "arguments ${args}" 2
    [[ ! -s "${dir}/calls" ]] || fail "arguments ${args}: gh was called"
  done
}

a_gh_read_that_fails_stops_the_run() {
  local failing
  for failing in jobs newest newest-2 run-2; do
    fixture_p "read_fails_${failing}"
    printf '{"message":"Server Error"}\n' > "${dir}/${failing}.fail"
    run_rerun 1001 0
    expect_status "a failing ${failing} read" 1
    expect_posts "a failing ${failing} read" 0
  done
}

a_byte_order_mark_on_the_group_line_is_cut() {
  fixture_p bom
  : > "${dir}/failed.log"
  printf '%s\t%s\t%s%s %s\n' "${scoop_job}" "${scoop_step}" "${bom}" "${stamp}" \
    '##[group]Run .github/packages-pr.sh wait "${PR}"' >> "${dir}/failed.log"
  log_line "${scoop_job}" "${scoop_step}" 'curl: (22) The requested URL returned error: 504'
  log_line "${scoop_job}" "${scoop_step}" '##[error]Process completed with exit code 1.'
  run_rerun 1001 0
  expect_out "a byte-order mark on the group line" "${scoop_ended}
Run 1001 is re-run in 0 s.
Re-ran the failed jobs of run 1001."
  expect_posts "a byte-order mark on the group line" 1
}

an_error_before_the_first_step_leaves_the_run() {
  fixture_p early_error
  : > "${dir}/failed.log"
  log_line "${scoop_job}" 'UNKNOWN STEP' "Current runner version: '2.338.2'"
  log_line "${scoop_job}" 'UNKNOWN STEP' '##[error]Process completed with exit code 1.'
  scoop_lines 'curl: (22) The requested URL returned error: 504'
  run_rerun 1001 0
  expect_out "an error before the first step" "${scoop_not}
Run 1001 is left as it is: ${scoop_job} did not end at a network step."
  expect_posts "an error before the first step" 0
}

more_than_100_jobs_leave_the_run() {
  fixture_p many_jobs
  write_jobs "${dir}/jobs.json" 101 "${scoop_job}=failure" 'packages (winget x64)=success'
  run_rerun 1001 0
  expect_out "more than 100 jobs" "${scoop_ended}
Run 1001 is left as it is: it has more jobs than one read holds."
  expect_posts "more than 100 jobs" 0
}

a_409_on_the_re_run_is_one_call() {
  fixture_p conflict
  printf '{"message":"Cannot rerun a workflow that is already running"}\n' > "${dir}/post.fail"
  run_rerun 1001 0
  expect_status "a 409 on the re-run" 1
  expect_posts "a 409 on the re-run" 1
}

cases=(
  each_recorded_line_re_runs_its_run
  two_errors_in_one_step_are_one_step
  an_aggregator_follows_its_jobs
  an_aggregator_alone_is_left
  a_real_failure_beside_a_504_leaves_the_run
  a_real_failed_step_after_a_504_step_leaves_the_run
  a_commit_sha_is_no_504
  a_504_inside_a_longer_line_is_no_match
  a_504_in_a_step_that_passed_is_no_match
  a_failed_job_without_log_lines_leaves_the_run
  runs_outside_its_scope_are_left
  every_listed_workflow_re_runs
  a_newer_run_leaves_the_run
  a_newer_run_during_the_wait_leaves_the_run
  a_re_run_by_hand_during_the_wait_leaves_the_run
  a_dry_run_writes_nothing
  an_id_that_is_not_digits_is_refused
  a_gh_read_that_fails_stops_the_run
  a_byte_order_mark_on_the_group_line_is_cut
  an_error_before_the_first_step_leaves_the_run
  more_than_100_jobs_leave_the_run
  a_409_on_the_re_run_is_one_call
)

if [[ -n "${RERUN_CASE:-}" ]]; then
  cases=("${RERUN_CASE}")
fi

for name in "${cases[@]}"; do
  before="${failures}"
  "${name}"
  if ((failures == before)); then
    echo "ok   ${name}"
  else
    echo "FAIL ${name}"
  fi
done

if ((failures > 0)); then
  echo "${failures} checks failed." >&2
  exit 1
fi
echo "All ${#cases[@]} cases pass."
