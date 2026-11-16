#!/usr/bin/env bash
# Prints one value of .github/tools.env. The value is checked for its kind (a tag or a SHA-256)
# before it is used, so a step never passes an odd string on to a download or a shell.
#
#   .github/pin.sh <NAME_TAG | NAME_SHA256>
set -euo pipefail

if [[ $# -ne 1 ]]; then
  echo "usage: $0 <NAME_TAG | NAME_SHA256>" >&2
  exit 2
fi
name="$1"
if [[ ! "${name}" =~ ^[A-Z0-9_]+_(TAG|SHA256)$ ]]; then
  echo "::error::${name} is not a pin name ending in _TAG or _SHA256." >&2
  exit 2
fi

found="$(awk -F= -v name="${name}" '$1 == name { print substr($0, length(name) + 2) }' \
  "$(dirname "${BASH_SOURCE[0]}")/tools.env")"
found="${found//$'\r'/}"
if [[ -z "${found}" || "${found}" == *$'\n'* ]]; then
  echo "::error::.github/tools.env needs exactly one ${name}." >&2
  exit 1
fi
if [[ "${name}" == *_SHA256 && ! "${found}" =~ ^[0-9a-f]{64}$ ]] \
  || [[ "${name}" == *_TAG && ! "${found}" =~ ^[A-Za-z0-9._+-]+$ ]]; then
  echo "::error::${name} in .github/tools.env is not a ${name##*_} value." >&2
  exit 1
fi
printf '%s\n' "${found}"
