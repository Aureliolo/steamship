#!/usr/bin/env bash
# A stand-in for gh that rerun-test.sh puts first on PATH. It records every call and answers from
# fixture files, so the test never reaches GitHub. A call it has no answer for exits 99.
#
#   FAKE_CALLS   file that every call is appended to
#   FAKE_DIR     folder of the fixtures:
#                  run.json, jobs.json, failed.log, newest.json   the answers to the reads
#                  run-2.json, newest-2.json   the answers to a second read of the same call
#                  <name>.fail   a file that makes that read (or the call, for post) fail with
#                                gh's exit code 1, printing the file's text on stdout
set -euo pipefail

FAKE_CALLS="${FAKE_CALLS:?}"
FAKE_DIR="${FAKE_DIR:?}"

call="$*"
earlier=0
if [[ -f "${FAKE_CALLS}" ]]; then
  earlier="$(grep -c -x -F -e "gh ${call}" "${FAKE_CALLS}" || true)"
fi
echo "gh ${call}" >> "${FAKE_CALLS}"

# Answers the call from <stem>.<extension>, or <stem>-2.<extension> on a second read when that
# exists, and fails first when the matching .fail file exists.
answer() {
  local stem="${1}" extension="${2}" chosen="${1}"
  if ((earlier > 0)) && [[ -e "${FAKE_DIR}/${stem}-2.${extension}" || -e "${FAKE_DIR}/${stem}-2.fail" ]]; then
    chosen="${stem}-2"
  fi
  if [[ -e "${FAKE_DIR}/${chosen}.fail" ]]; then
    cat "${FAKE_DIR}/${chosen}.fail"
    echo "gh: the fixture ${chosen}.fail made this call fail" >&2
    exit 1
  fi
  cat "${FAKE_DIR}/${chosen}.${extension}"
}

case "${call}" in
  "api -X POST repos/"*"/actions/runs/"*"/rerun-failed-jobs")
    if [[ -e "${FAKE_DIR}/post.fail" ]]; then
      cat "${FAKE_DIR}/post.fail"
      echo "gh: the fixture post.fail made this call fail" >&2
      exit 1
    fi
    echo '{}'
    ;;
  "api repos/"*"/actions/runs/"*"/attempts/1/jobs?per_page=100") answer jobs json ;;
  "api repos/"*"/actions/runs/"[0-9]*) answer run json ;;
  "run view "*" --repo "*" --attempt 1 --log-failed") answer failed log ;;
  "api -X GET repos/"*"/actions/workflows/"*"/runs -f branch="*" -f event="*" -f per_page=1") answer newest json ;;
  *)
    echo "The fake gh has no answer for: gh ${call}" >&2
    exit 99
    ;;
esac
