#!/usr/bin/env bash
# Builds the .deb and the .rpm for one release from its Linux archive, with nFPM and
# .github/nfpm.yaml, and writes a .sha256 beside each.
#
#   .github/linux-packages.sh <version> <Linux archive> <folder to write into>
#
# SOURCE_DATE_EPOCH, when set, dates the Debian changelog and every file in both packages; the
# release sets it to the tagged commit's time, so the same tag builds the same packages.
set -euo pipefail

if [[ $# -ne 3 ]]; then
  echo "usage: $0 <version> <Linux archive> <folder to write into>" >&2
  exit 2
fi
version="$1"
archive="$2"
out="$3"

if [[ ! "${version}" =~ ^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$ ]]; then
  echo "${version} is not a version of three numbers." >&2
  exit 2
fi

config="$(dirname "$0")/nfpm.yaml"
repository="https://github.com/Aureliolo/steamship"
maintainer="Aurelio Amoroso <19254254+Aureliolo@users.noreply.github.com>"
export SOURCE_DATE_EPOCH="${SOURCE_DATE_EPOCH:-$(date +%s)}"

stage="$(mktemp -d)"
trap 'rm -rf "${stage}"' EXIT
tar -xzf "${archive}" -C "${stage}"
folder="${stage}/$(basename "${archive}" .tar.gz)"
if [[ ! -x "${folder}/steamship" ]]; then
  echo "${archive} holds no steamship program in $(basename "${folder}")." >&2
  exit 1
fi

# Man pages are installed compressed, and without a name or time inside that would make two
# builds of one release differ.
gzip -9n "${folder}"/man/*.1

changed="$(date -u -R -d "@${SOURCE_DATE_EPOCH}")"
printf '%s\n' \
  "steamship (${version}-1) unstable; urgency=medium" \
  "" \
  "  * steamship ${version}: ${repository}/releases/tag/v${version}" \
  "" \
  " -- ${maintainer}  ${changed}" |
  gzip -9n > "${folder}/changelog.Debian.gz"

# Debian's machine-readable form. Apache-2.0 is among the licences every Debian system carries;
# MIT is not, so its text is given in full, indented as the format asks.
holder="$(grep -m1 '^Copyright (c) ' "${folder}/LICENSE-MIT")"
mit="$(sed -e 's/^$/./' -e 's/^/ /' "${folder}/LICENSE-MIT")"
cat > "${folder}/copyright" << COPYRIGHT
Format: https://www.debian.org/doc/packaging-manuals/copyright-format/1.0/
Upstream-Name: steamship
Upstream-Contact: ${repository}/issues
Source: ${repository}

Files: *
Copyright: ${holder#Copyright (c) }
License: MIT or Apache-2.0

License: MIT
${mit}

License: Apache-2.0
 On Debian systems, the full text of the Apache License, Version 2.0, is in
 /usr/share/common-licenses/Apache-2.0.
COPYRIGHT

mkdir -p "${out}"
VERSION="${version}" STAGE="${folder}" nfpm package --config "${config}" --packager deb --target "${out}/"
VERSION="${version}" STAGE="${folder}" nfpm package --config "${config}" --packager rpm --target "${out}/"

for package in "${out}/steamship_${version}-1_amd64.deb" "${out}/steamship-${version}-1.x86_64.rpm"; do
  if [[ ! -f "${package}" ]]; then
    echo "nFPM wrote no $(basename "${package}")." >&2
    exit 1
  fi
  (cd "${out}" && sha256sum "$(basename "${package}")" > "$(basename "${package}").sha256")
done
