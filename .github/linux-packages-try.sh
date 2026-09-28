#!/usr/bin/env bash
# Run inside a bare Debian or Fedora container by the linux-packages action: installs the package
# for $SYSTEM from /packages, checks what it put where, uploads a preview of Valve's test app from
# /spacewar anonymously, and removes the package again.
set -euo pipefail
: "${SYSTEM:?}" "${VERSION:?}"

case "${SYSTEM}" in
  debian)
    export DEBIAN_FRONTEND=noninteractive
    apt-get update
    apt-get install --yes --no-install-recommends "/packages/steamship_${VERSION}-1_amd64.deb"
    dpkg --verify steamship
    zsh=/usr/share/zsh/vendor-completions/_steamship
    ;;
  fedora)
    dnf install --assumeyes --setopt=install_weak_deps=False "/packages/steamship-${VERSION}-1.x86_64.rpm"
    rpm --verify steamship
    zsh=/usr/share/zsh/site-functions/_steamship
    test -f /usr/share/licenses/steamship/LICENSE-MIT
    ;;
  *)
    echo "No package for ${SYSTEM}." >&2
    exit 2
    ;;
esac

said="$(steamship --version)"
test "${said}" = "steamship ${VERSION}"
test -f /usr/share/man/man1/steamship.1.gz
test -f /usr/share/man/man1/steamship-upload.1.gz
test -f /usr/share/bash-completion/completions/steamship
test -f "${zsh}"
test -f /usr/share/fish/vendor_completions.d/steamship.fish
test -f /usr/share/doc/steamship/README.md

export STEAMSHIP_HOME=/tmp/steamship
steamship install
status=0
steamship upload /spacewar/steam/app_build.vdf --version packages --account anonymous --preview \
  2> /tmp/upload.txt || status=$?
cat /tmp/upload.txt
test "${status}" -eq 1
grep -q '^  ✗ Failed to initialize build on server (Access Denied)$' /tmp/upload.txt

if [[ "${SYSTEM}" == debian ]]; then
  apt-get remove --yes steamship
else
  dnf remove --assumeyes steamship
fi
test ! -e /usr/bin/steamship
