#!/usr/bin/env bash
# Runs the action's steps as action.yml gives them, against a fake steamship, and checks what the
# step refuses, what it starts steamship with and what the files around it say. Nothing reaches
# Steam.
#
#   .github/action-run-test.sh
#
# ACTION_YML, ACTION_RUN_SH, RELEASE_YML and CI_DOC name other copies of those files to test, such
# as one with a rule left out. ACTION_COMMANDS_DIR names another copy of src/commands.
# ACTION_CASE names the one case to run.
# shellcheck disable=SC2016 # Expected text holds literal ${...} of a workflow.
# shellcheck disable=SC2034 # The IN_ variables are read by name in evaluate.
# shellcheck disable=SC2310,SC2312 # A failed check is reported by fail, never by set -e.
set -euo pipefail
export LC_ALL=C

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
root="$(cd "${here}/.." && pwd)"
yml="${ACTION_YML:-${root}/action.yml}"
script="${ACTION_RUN_SH:-${here}/action-run.sh}"
release="${RELEASE_YML:-${here}/workflows/release.yml}"
doc="${CI_DOC:-${root}/docs/ci.md}"
commands="${ACTION_COMMANDS_DIR:-${root}/src/commands}"
work="$(mktemp -d)"
trap 'rm -rf "${work}"' EXIT

# The action's own folder, with the script the step runs from it, and the program the step starts.
mkdir -p "${work}/action/.github" "${work}/bin" "${work}/empty" "${work}/temp"
cp "${script}" "${work}/action/.github/action-run.sh"
cp "${here}/action-run-fake-steamship.sh" "${work}/bin/steamship"
chmod +x "${work}/bin/steamship"

want_login='steamship-login-2:2026-11-13:build_bot:0a1b2c'
want_key='0123456789ABCDEF0123456789ABCDEF'
failures=0
status=0

fail() {
  echo "FAIL: $*" >&2
  failures=$((failures + 1))
}

# The inputs of a run, one variable each, with their defaults.
reset_inputs() {
  IN_command=upload IN_app='' IN_dry_run=false IN_account='' IN_login="${want_login}"
  IN_web_api_key="${want_key}" IN_home='' IN_version='' IN_default=false IN_demo=false
  IN_allow_ungreen=false IN_new=false IN_for_launch=false IN_visibility='' IN_name='' IN_items=''
  IN_content='' IN_preview_image='' IN_title='' IN_description='' IN_change_note=''
}

# What an expression of action.yml's env gives in the run being tested.
evaluate() {
  local expression="${1}" name fallback
  local plain='^\$\{\{ inputs\.([a-z-]+) \}\}$'
  local coalesced='^\$\{\{ inputs\.([a-z-]+) \|\| format\(.\{0\}/([a-z-]+).,[ ]runner\.temp\) \}\}$'
  if [[ "${expression}" =~ ${plain} ]]; then
    name="IN_${BASH_REMATCH[1]//-/_}"
    echo "${!name}"
  elif [[ "${expression}" =~ ${coalesced} ]]; then
    name="IN_${BASH_REMATCH[1]//-/_}"
    fallback="${work}/temp/${BASH_REMATCH[2]}"
    echo "${!name:-${fallback}}"
  elif [[ "${expression}" == '${{ steps.install.outputs.program }}' ]]; then
    echo "${work}/bin/steamship"
  elif [[ "${expression}" == '${{ github.action_path }}' ]]; then
    echo "${work}/action"
  else
    echo "UNKNOWN ${expression}"
  fi
}

# The lines of the step called $1.
step_block() {
  awk -v want="    - name: ${1}" '/^    - name: / { on = ($0 == want) } on' "${yml}"
}

# Runs the step called $1 in an empty folder with its env as action.yml gives it, leaving its exit
# status in status, what it printed in ${work}/out and what it said in ${work}/err.
run_step() {
  local block line run
  local -a pairs=()
  block="$(step_block "${1}")"
  while IFS= read -r line; do
    if [[ "${line}" =~ ^\ {8}([A-Z_]+):\ (.*)$ ]]; then
      pairs+=("${BASH_REMATCH[1]}=$(evaluate "${BASH_REMATCH[2]}")")
    fi
  done <<< "${block}"
  run="$(grep -m1 '^      run: ' <<< "${block}" || true)"
  run="${run#      run: }"
  if [[ "${run}" == \'*\' ]]; then
    run="${run:1:${#run}-2}"
  fi
  status=0
  (
    cd "${work}/empty"
    env -i PATH="${PATH}" SYSTEMROOT="${SYSTEMROOT-}" TEMP="${work}/temp" \
      WANT_LOGIN="${want_login}" WANT_KEY="${want_key}" FAKE_EXIT="${FAKE_EXIT:-0}" \
      ${pairs[@]+"${pairs[@]}"} bash -c "${run}"
  ) > "${work}/out" 2> "${work}/err" || status=$?
}

upload() {
  run_step Upload
}

# The input a variable of reset_inputs holds.
input_name() {
  local name="${1#IN_}"
  echo "${name//_/-}"
}

expect_status() {
  [[ "${status}" == "${2}" ]] || fail "${1}: exit status ${status}, not ${2}"
}

expect_refusal() {
  local have
  expect_status "${1}" 2
  have="$(< "${work}/err")"
  [[ "${have}" == "${2}" ]] || fail "${1}: said
${have}
but should have said
${2}"
  [[ ! -s "${work}/out" ]] || fail "${1}: steamship started"
}

# The arguments steamship got, in order, against $2 and on.
expect_args() {
  local name="${1}" have want
  shift
  have="$(grep '^arg ' "${work}/out" || true)"
  want="$(printf 'arg %s\n' "$@")"
  [[ "${have}" == "${want}" ]] || fail "${name}: arguments were
${have}
but should have been
${want}"
}

# The lines of the file's block under the key $2, which is at the indent of $3 spaces, of the
# section $1 at the top. The key's own line and the lines indented deeper.
block_of() {
  awk -v section="${1}:" -v key="${2}:" -v indent="${3}" '
    /^[a-z]/ { in_section = ($0 == section); in_key = 0; next }
    !in_section { next }
    match($0, /^ */) {
      if (RLENGTH == indent && $0 != "") { in_key = ($0 == sprintf("%" indent "s%s", "", key)) }
      if (in_key) { print }
    }' "${yml}"
}

# Cases. Each is a function named after what it shows. Part of the description of each is the
# wrong build it exists to catch.

a_boolean_is_true_or_false() {
  local value
  for value in True yes 1 ''; do
    reset_inputs
    IN_dry_run="${value}"
    upload
    expect_refusal "dry-run ${value}" "::error::input dry-run is \"${value}\". Give true or false."
  done
  reset_inputs
  IN_default=1
  upload
  expect_refusal "default 1" '::error::input default is "1". Give true or false.'
  reset_inputs
  IN_new=TRUE
  upload
  expect_refusal "new TRUE" '::error::input new is "TRUE". Give true or false.'
}

true_and_false_pass() {
  reset_inputs
  IN_app=steam/app_build.vdf IN_dry_run=true
  upload
  expect_status "dry-run true" 0
  expect_args "dry-run true" upload steam/app_build.vdf --dry-run
  IN_dry_run=false
  upload
  expect_status "dry-run false" 0
  expect_args "dry-run false" upload steam/app_build.vdf
}

an_input_of_the_other_command_is_refused() {
  local setting
  for setting in IN_default=true IN_version=1.0.0 IN_demo=true; do
    reset_inputs
    IN_command=workshop
    printf -v "${setting%%=*}" '%s' "${setting#*=}"
    upload
    expect_refusal "workshop with ${setting}" \
      "::error::$(input_name "${setting%%=*}") is an input of command upload. Leave it out when command is workshop."
  done
  for setting in IN_new=true IN_title=T; do
    reset_inputs
    printf -v "${setting%%=*}" '%s' "${setting#*=}"
    upload
    expect_refusal "upload with ${setting}" \
      "::error::$(input_name "${setting%%=*}") is an input of command workshop. Leave it out when command is upload."
  done
}

false_and_empty_of_the_other_command_pass() {
  reset_inputs
  IN_command=workshop IN_default=false IN_version=''
  upload
  expect_status "workshop with false and empty" 0
  expect_args "workshop with false and empty" workshop
}

an_empty_boolean_is_refused_whatever_the_command() {
  reset_inputs
  IN_command=workshop IN_default=''
  upload
  expect_refusal "workshop with default empty" '::error::input default is "". Give true or false.'
  reset_inputs
  IN_new=''
  upload
  expect_refusal "upload with new empty" '::error::input new is "". Give true or false.'
}

each_boolean_is_false_when_left_out() {
  local name
  for name in dry-run default demo allow-ungreen new for-launch; do
    block_of inputs "${name}" 2 | grep -qxF '    default: "false"' \
      || fail "${name} has no default: \"false\""
  done
}

every_input_reaches_its_flag() {
  reset_inputs
  IN_app=steam/app_build.vdf IN_dry_run=true IN_version=1.2.3 IN_default=true IN_demo=true
  IN_allow_ungreen=true IN_account=build_bot
  upload
  expect_status "every upload input" 0
  expect_args "every upload input" upload steam/app_build.vdf --dry-run --version 1.2.3 \
    --default --demo --allow-ungreen
  grep -qxF 'account build_bot' "${work}/out" || fail "the account did not reach STEAMSHIP_ACCOUNT"
  reset_inputs
  IN_command=workshop IN_app=workshop/item.vdf IN_dry_run=true IN_new=true IN_for_launch=true
  IN_visibility=public IN_name=example IN_items=steam/items.json IN_content=content
  IN_preview_image=preview.png IN_title='Two words' IN_description='A description'
  IN_change_note='Fixed it'
  upload
  expect_status "every workshop input" 0
  expect_args "every workshop input" workshop workshop/item.vdf --dry-run --new --for-launch \
    --visibility public --name example --items steam/items.json --content content \
    --preview-image preview.png --title 'Two words' --description 'A description' \
    --change-note 'Fixed it'
}

the_login_and_the_key_reach_steamship_by_name() {
  reset_inputs
  upload
  grep -qxF 'login matches' "${work}/out" || fail "STEAMSHIP_LOGIN is not the login"
  grep -qxF 'key matches' "${work}/out" || fail "STEAMSHIP_WEB_API_KEY is not the key"
  ! grep -qF -e "${want_login}" -e "${want_key}" "${work}/out" \
    || fail "a secret reached the output"
}

a_games_folder_needs_neither_app_nor_version() {
  reset_inputs
  upload
  expect_status "upload alone" 0
  expect_args "upload alone" upload
}

an_unknown_command_is_refused() {
  reset_inputs
  IN_command=deploy
  upload
  expect_refusal "command deploy" '::error::command is "deploy". Give upload or workshop.'
}

the_steps_code_is_steamships() {
  reset_inputs
  FAKE_EXIT=5 upload
  expect_status "steamship's code" 5
}

the_home_is_kept_and_the_login_is_not() {
  local last
  ! grep -qF 'rm -rf' "${yml}" || fail "action.yml deletes something"
  last="$(grep '^    - name: ' "${yml}" | tail -n 1)"
  step_block "${last#    - name: }" | grep -qxF '      if: always()' \
    || fail "the last step does not run whatever came before"
}

logout_gets_the_uploads_home() {
  local kept last
  last="$(grep '^    - name: ' "${yml}" | tail -n 1)"
  last="${last#    - name: }"
  for kept in '' kept; do
    reset_inputs
    IN_home="${kept}"
    upload
    kept="$(grep '^home ' "${work}/out" || true)"
    run_step "${last}"
    expect_status "logout" 0
    expect_args "logout" logout
    [[ "$(grep '^home ' "${work}/out" || true)" == "${kept}" && -n "${kept}" ]] \
      || fail "logout's home is not the upload's, ${kept}"
  done
  reset_inputs
  IN_home=kept
  run_step "${last}"
  grep -qxF 'home kept' "${work}/out" || fail "logout ignores home"
}

the_step_runs_the_actions_own_script_from_any_folder() {
  reset_inputs
  upload
  expect_status "from an empty folder" 0
  grep -qxF 'arg upload' "${work}/out" || fail "steamship did not start"
}

the_cache_leaves_the_logs_out() {
  local have want
  have="$(awk '
    block && match($0, /^ */) && RLENGTH > indent { sub(/^ +/, ""); print; next }
    { block = 0 }
    /^ *path: \|$/ { match($0, /^ */); indent = RLENGTH; block = 1 }' "${doc}")"
  want='${{ runner.temp }}/steamship/apps
!${{ runner.temp }}/steamship/apps/*/output/*.log
${{ runner.temp }}/steamship/recorded-builds'
  [[ "${have}" == "${want}" ]] || fail "the cache path was
${have}
but should have been
${want}"
}

# The lines of the step of release.yml that uses the action of this repository.
release_step() {
  awk '/^      - / { if (step ~ /uses: \.\/ / ) { printf "%s", step } step = "" } { step = step $0 "\n" }
    END { if (step ~ /uses: \.\/ /) { printf "%s", step } }' "${release}"
}

release_tries_the_action_with_its_inputs() {
  local key keys
  keys="$(release_step | awk '/^        with:$/ { on = 1; next } on && /^          [a-z-]+:/ { sub(/^ +/, ""); sub(/:.*/, ""); print }')"
  [[ -n "${keys}" ]] || fail "release.yml's step that uses the action has no inputs"
  for key in ${keys}; do
    block_of inputs "${key}" 2 | grep -q . || fail "release.yml gives ${key}, which action.yml lacks"
  done
  grep -qF 'steamship-login-2:' "${release}" || fail "release.yml packs no login of format 2"
  grep -qF "grep -rqF 'Failed to initialize build on server (Access Denied)'" "${release}" \
    || fail "release.yml does not read Steam's refusal"
}

release_reads_the_refusal_in_the_home_it_gives() {
  local home folder
  home="$(release_step | sed -n 's/^          home: //p')"
  folder="${home//'${{ runner.temp }}'/'${RUNNER_TEMP}'}/apps/480/output"
  grep -qF "\"${folder}\"" "${release}" \
    || fail "release.yml does not read ${folder}, the home it gives"
}

each_output_reads_what_steamship_writes() {
  local name count
  for name in build-id item-id; do
    block_of outputs "${name}" 2 | grep -qxF "    value: \${{ steps.upload.outputs.${name} }}" \
      || fail "output ${name} reads another name"
    count="$( (grep -rF "output(\"${name}\"" "${commands}" || true) | wc -l)"
    [[ "${count}" -eq 1 ]] || fail "steamship writes output ${name} on ${count} lines, not 1"
  done
}

cases=(
  a_boolean_is_true_or_false
  true_and_false_pass
  an_input_of_the_other_command_is_refused
  false_and_empty_of_the_other_command_pass
  an_empty_boolean_is_refused_whatever_the_command
  each_boolean_is_false_when_left_out
  every_input_reaches_its_flag
  the_login_and_the_key_reach_steamship_by_name
  a_games_folder_needs_neither_app_nor_version
  an_unknown_command_is_refused
  the_steps_code_is_steamships
  the_home_is_kept_and_the_login_is_not
  logout_gets_the_uploads_home
  the_step_runs_the_actions_own_script_from_any_folder
  the_cache_leaves_the_logs_out
  release_tries_the_action_with_its_inputs
  release_reads_the_refusal_in_the_home_it_gives
  each_output_reads_what_steamship_writes
)
for case in "${cases[@]}"; do
  if [[ -z "${ACTION_CASE-}" || "${ACTION_CASE}" == "${case}" ]]; then
    "${case}"
  fi
done

if [[ "${failures}" -ne 0 ]]; then
  echo "${failures} failed" >&2
  exit 1
fi
echo "action-run-test: all cases passed"
