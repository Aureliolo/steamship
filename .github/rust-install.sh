#!/usr/bin/env bash
# Installs the toolchain rust-toolchain.toml pins, and each target named, through the runner's
# rustup, which checks every component against the hashes in Rust's release manifest. rustup's own
# retries follow each other at once, so a runner without DNS for a few seconds would fail the job.
# Each step is asked again over about a minute.
#
#   .github/rust-install.sh [target ...]
set -euo pipefail

again() {
  local wait
  for wait in 1 2 4 8 16 32 0; do
    if "$@"; then
      return 0
    fi
    if ((wait > 0)); then
      echo "rustup could not reach Rust's servers; asking again in ${wait} s." >&2
      sleep "${wait}"
    fi
  done
  echo "::error::rustup could not reach Rust's servers seven times in a row: $*" >&2
  return 1
}

again rustup toolchain install
for target in "$@"; do
  again rustup target add "${target}"
done
