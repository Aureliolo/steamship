#!/usr/bin/env bash
# Run inside a bare Debian or Fedora container by the linux-packages action: lints the package
# for $SYSTEM from /packages with that system's own linter, installs it, checks what it put where,
# uploads a preview of Valve's test app from /spacewar anonymously, and removes it again. Neither
# linter nor Git brings any 32-bit library, so the install still starts from none.
set -euo pipefail
: "${SYSTEM:?}" "${VERSION:?}"

case "${SYSTEM}" in
  debian)
    export DEBIAN_FRONTEND=noninteractive
    deb="/packages/steamship_${VERSION}-1_amd64.deb"
    apt-get update
    apt-get install --yes --no-install-recommends lintian git
    lintian --fail-on error,warning --display-info "${deb}"
    apt-get install --yes --no-install-recommends "${deb}"
    dpkg --verify steamship
    zsh=/usr/share/zsh/vendor-completions/_steamship
    ;;
  fedora)
    rpm="/packages/steamship-${VERSION}-1.x86_64.rpm"
    dnf install --assumeyes --setopt=install_weak_deps=False rpmlint git-core
    rpmlint --strict --config /rpmlint.toml "${rpm}"
    dnf install --assumeyes --setopt=install_weak_deps=False "${rpm}"
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
# An upload names the commit its scripts are at, so they are given one.
cp -r /spacewar /tmp/spacewar
git -C /tmp/spacewar init --quiet
git -C /tmp/spacewar add .
git -C /tmp/spacewar -c user.name=packages -c user.email=packages@localhost commit --quiet \
  --message 'The scripts under test'
status=0
steamship upload /tmp/spacewar/steam/app_build.vdf --version packages --account anonymous --preview \
  2> /tmp/upload.txt || status=$?
cat /tmp/upload.txt
if [[ "${status}" -ne 1 ]] ||
  ! grep -q '^  ✗ Failed to initialize build on server (Access Denied)$' /tmp/upload.txt; then
  echo "The upload did not reach Steam's refusal; steamcmd's console:" >&2
  cat /tmp/steamship/apps/480/output/steamcmd.log >&2 || true
  exit 1
fi

if [[ "${SYSTEM}" == debian ]]; then
  apt-get remove --yes steamship
else
  dnf remove --assumeyes steamship
fi
test ! -e /usr/bin/steamship
