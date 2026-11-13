#!/usr/bin/env bash
# The action's step: refuses a bad boolean, an unknown command and an input of the other command,
# then replaces itself with steamship.
# Each input arrives in the environment under its upper-case name, and the account, the login and
# the key arrive as steamship reads them, so none of them is in a command line.
#
# Every boolean is read before anything else, so that `yes` is refused and never read as false.
# An input of the other command is refused rather than ignored, so a workflow never runs something
# other than what it asks for.
#
#   .github/action-run.sh
#
# Exits 2 on a refusal, as steamship does, and otherwise with steamship's own code.
# shellcheck disable=SC2154 # The step's environment sets every upper-case name.
set -euo pipefail

refuse() {
  echo "::error::$1" >&2
  exit 2
}

# The input's name, then its variable.
booleans=(
  dry-run:DRY_RUN default:DEFAULT demo:DEMO allow-ungreen:ALLOW_UNGREEN new:NEW
  for-launch:FOR_LAUNCH
)
for pair in "${booleans[@]}"; do
  variable="${pair#*:}"
  value="${!variable-}"
  [[ "${value}" == true || "${value}" == false ]] || refuse "input ${pair%%:*} is \"${value}\". Give true or false."
done

command="${COMMAND-}"
case "${command}" in
  upload) other=workshop ;;
  workshop) other=upload ;;
  *) refuse "command is \"${command}\". Give upload or workshop." ;;
esac

# What only upload takes, then what only workshop takes, each with its variable.
only_upload=(version:VERSION default:DEFAULT demo:DEMO allow-ungreen:ALLOW_UNGREEN)
only_workshop=(
  new:NEW for-launch:FOR_LAUNCH visibility:VISIBILITY name:ITEM_NAME items:ITEMS
  content:CONTENT preview-image:PREVIEW_IMAGE title:TITLE description:DESCRIPTION
  change-note:CHANGE_NOTE
)
if [[ "${command}" == upload ]]; then
  foreign=("${only_workshop[@]}")
else
  foreign=("${only_upload[@]}")
fi
for pair in "${foreign[@]}"; do
  variable="${pair#*:}"
  value="${!variable-}"
  if [[ -n "${value}" && "${value}" != false ]]; then
    refuse "${pair%%:*} is an input of command ${other}. Leave it out when command is ${command}."
  fi
done

# A flag that takes text goes in when the text is not empty, a switch when it is true.
arguments=("${command}")
[[ -z "${APP-}" ]] || arguments+=("${APP}")
[[ "${DRY_RUN}" != true ]] || arguments+=(--dry-run)
if [[ "${command}" == upload ]]; then
  [[ -z "${VERSION-}" ]] || arguments+=(--version "${VERSION}")
  [[ "${DEFAULT}" != true ]] || arguments+=(--default)
  [[ "${DEMO}" != true ]] || arguments+=(--demo)
  [[ "${ALLOW_UNGREEN}" != true ]] || arguments+=(--allow-ungreen)
else
  [[ "${NEW}" != true ]] || arguments+=(--new)
  [[ "${FOR_LAUNCH}" != true ]] || arguments+=(--for-launch)
  for pair in visibility:VISIBILITY name:ITEM_NAME items:ITEMS content:CONTENT \
    preview-image:PREVIEW_IMAGE title:TITLE description:DESCRIPTION change-note:CHANGE_NOTE; do
    variable="${pair#*:}"
    [[ -z "${!variable-}" ]] || arguments+=("--${pair%%:*}" "${!variable}")
  done
fi
exec "${STEAMSHIP}" "${arguments[@]}"
