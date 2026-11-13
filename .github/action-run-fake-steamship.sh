#!/usr/bin/env bash
# A stand-in for steamship, for action-run-test.sh. It prints what it was started with, one fact
# per line, and exits with FAKE_EXIT. The login and the key are shown only as whether they equal
# WANT_LOGIN and WANT_KEY, so no secret reaches a log.
set -euo pipefail

for argument in "$@"; do
  echo "arg ${argument}"
done
echo "account ${STEAMSHIP_ACCOUNT-}"
echo "home ${STEAMSHIP_HOME-}"
if [[ "${STEAMSHIP_LOGIN-}" == "${WANT_LOGIN-}" ]]; then
  echo "login matches"
else
  echo "login differs"
fi
if [[ "${STEAMSHIP_WEB_API_KEY-}" == "${WANT_KEY-}" ]]; then
  echo "key matches"
else
  echo "key differs"
fi
exit "${FAKE_EXIT:-0}"
