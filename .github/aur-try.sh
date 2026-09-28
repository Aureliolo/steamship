#!/usr/bin/env bash
# Run inside a bare Arch Linux container by the aur action: builds steamship-bin from the
# PKGBUILD in /aur as the AUR would, lints it, installs it, uploads a preview of Valve's test app
# from /spacewar anonymously with it, and removes it again. An archive already in /aur is used in
# place of the release's download, as makepkg does with any source it finds beside the PKGBUILD.
set -euo pipefail

# lib32-gcc-libs, which the package depends on, is in the multilib repository, off by default.
# And the image keeps man pages and documentation out of what it installs, which Arch does not.
printf '\n[multilib]\nInclude = /etc/pacman.d/mirrorlist\n' >> /etc/pacman.conf
sed -i '/^NoExtract/d' /etc/pacman.conf
pacman -Syu --noconfirm --needed namcap git

# makepkg refuses to run as root.
useradd --create-home builder
cp -r /aur /home/builder/aur
chown -R builder: /home/builder/aur
build() {
  su builder -c "cd /home/builder/aur && $1"
}
build 'makepkg --printsrcinfo | diff -u .SRCINFO -'
build 'namcap PKGBUILD'
build 'makepkg --nodeps --noconfirm'
package="$(find /home/builder/aur -maxdepth 1 -name 'steamship-bin-*-x86_64.pkg.tar.zst' -print -quit)"
namcap "${package}"
pacman -U --noconfirm "${package}"

version="$(sed -n 's/^pkgver=//p' /home/builder/aur/PKGBUILD)"
said="$(steamship --version)"
test "${said}" = "steamship ${version}"
for file in /usr/share/man/man1/steamship.1.gz /usr/share/man/man1/steamship-upload.1.gz \
  /usr/share/bash-completion/completions/steamship /usr/share/zsh/site-functions/_steamship \
  /usr/share/fish/vendor_completions.d/steamship.fish /usr/share/licenses/steamship-bin/LICENSE-MIT; do
  if [[ ! -f "${file}" ]]; then
    echo "The package installed no ${file}." >&2
    exit 1
  fi
done

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

pacman -R --noconfirm steamship-bin
test ! -e /usr/bin/steamship
