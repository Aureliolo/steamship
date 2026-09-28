#!/usr/bin/env bash
# Writes the Homebrew formula, the Scoop manifest and the winget manifests for one release, from
# the .sha256 files its build wrote beside the archives. Whoever calls this has already checked
# those files against the release's attestation, so every hash below is one the release is signed
# over.
#
#   .github/packages.sh <version> <folder with the .sha256 files> <folder to write into>
#
# It writes <folder>/Formula/steamship.rb, <folder>/bucket/steamship.json and the three
# manifests winget takes in <folder>/winget.
set -euo pipefail

if [[ $# -ne 3 ]]; then
  echo "usage: $0 <version> <folder with the .sha256 files> <folder to write into>" >&2
  exit 2
fi
version="$1"
sums="$2"
out="$3"

if [[ ! "${version}" =~ ^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$ ]]; then
  echo "${version} is not a version of three numbers." >&2
  exit 2
fi

repository="https://github.com/Aureliolo/steamship"
download="${repository}/releases/download/v${version}"

# The one archive of this version whose name ends in $1, as the release's checksums name it: the
# names are the release's to choose, and an older release named its Linux archive differently.
archive_of() {
  local found=("${sums}/steamship-${version}-"*"$1.sha256")
  if [[ ${#found[@]} -ne 1 || ! -f "${found[0]}" ]]; then
    echo "${sums} holds no single checksum for an archive ending in $1." >&2
    exit 1
  fi
  basename "${found[0]}" .sha256
}

# The hash in an archive's .sha256, refused unless the file names that archive: a sidecar for
# another archive, renamed, would otherwise pass for this one.
hash_of() {
  local file="${sums}/$1.sha256"
  local hash name
  read -r hash name < "${file}"
  if [[ ! "${hash}" =~ ^[0-9a-f]{64}$ || "${name#\*}" != "$1" ]]; then
    echo "${file} does not hold the SHA-256 of $1." >&2
    exit 1
  fi
  printf '%s' "${hash}"
}

macos_arm_archive="$(archive_of aarch64-apple-darwin.tar.gz)"
macos_intel_archive="$(archive_of x86_64-apple-darwin.tar.gz)"
linux_archive="$(archive_of linux-musl.tar.gz)"
windows_archive="$(archive_of x86_64-pc-windows-msvc.zip)"
windows_arm_archive="$(archive_of aarch64-pc-windows-msvc.zip)"
macos_arm="$(hash_of "${macos_arm_archive}")"
macos_intel="$(hash_of "${macos_intel_archive}")"
linux="$(hash_of "${linux_archive}")"
windows="$(hash_of "${windows_archive}")"
windows_arm="$(hash_of "${windows_arm_archive}")"

mkdir -p "${out}/Formula" "${out}/bucket" "${out}/winget"

cat > "${out}/Formula/steamship.rb" << FORMULA
# Written by .github/packages.sh for each release, from the release's own signed checksums.
class Steamship < Formula
  desc "Uploads game builds to Steam with Valve's steamcmd"
  homepage "${repository}"
  license any_of: ["MIT", "Apache-2.0"]

  on_macos do
    on_arm do
      url "${download}/${macos_arm_archive}"
      sha256 "${macos_arm}"
    end
    on_intel do
      url "${download}/${macos_intel_archive}"
      sha256 "${macos_intel}"
    end
  end

  on_linux do
    on_intel do
      url "${download}/${linux_archive}"
      sha256 "${linux}"
    end
  end

  def install
    bin.install "steamship"
    man1.install Dir["man/*.1"]
    bash_completion.install "completions/steamship.bash" => "steamship"
    zsh_completion.install "completions/_steamship"
    fish_completion.install "completions/steamship.fish"
  end

  test do
    assert_equal "steamship #{version}", shell_output("#{bin}/steamship --version").strip
  end
end
FORMULA

# The autoupdate block is what `checkver.ps1 -Update` in a Scoop bucket reads to raise the
# manifest by itself; the hash comes from the .sha256 beside the archive rather than a download.
jq -n \
  --arg version "${version}" \
  --arg homepage "${repository}" \
  --arg download "${download}" \
  --arg next_download "${repository}/releases/download/v\$version" \
  --arg x64 "${windows_archive%.zip}" \
  --arg x64_hash "${windows}" \
  --arg arm64 "${windows_arm_archive%.zip}" \
  --arg arm64_hash "${windows_arm}" \
  '
  def later: split($version) | join("$version");
  def now(folder; hash): {url: "\($download)/\(folder).zip", hash: hash, extract_dir: folder};
  def next(folder): {
    url: "\($next_download)/\(folder | later).zip",
    hash: {url: "$url.sha256"},
    extract_dir: (folder | later)
  };
  {
    version: $version,
    description: "Uploads game builds to Steam with Valve'"'"'s steamcmd.",
    homepage: $homepage,
    license: "MIT|Apache-2.0",
    architecture: {"64bit": now($x64; $x64_hash), arm64: now($arm64; $arm64_hash)},
    bin: "steamship.exe",
    checkver: "github",
    autoupdate: {architecture: {"64bit": next($x64), arm64: next($arm64)}}
  }' > "${out}/bucket/steamship.json"

# Written whole rather than raised with `wingetcreate update`, which carries only the installers
# the last version had and cannot add one for a new architecture.
winget_installer() {
  local architecture="$1" archive="$2" hash="$3"
  cat << INSTALLER
- Architecture: ${architecture}
  InstallerUrl: ${download}/${archive}
  InstallerSha256: ${hash^^}
  NestedInstallerFiles:
  - RelativeFilePath: ${archive%.zip}\\steamship.exe
    PortableCommandAlias: steamship
INSTALLER
}

manifest_version=1.12.0
schema="https://aka.ms/winget-manifest"
x64_installer="$(winget_installer x64 "${windows_archive}" "${windows}")"
arm64_installer="$(winget_installer arm64 "${windows_arm_archive}" "${windows_arm}")"

cat > "${out}/winget/Aureliolo.steamship.yaml" << VERSION
# yaml-language-server: \$schema=${schema}.version.${manifest_version}.schema.json

PackageIdentifier: Aureliolo.steamship
PackageVersion: ${version}
DefaultLocale: en-GB
ManifestType: version
ManifestVersion: ${manifest_version}
VERSION

cat > "${out}/winget/Aureliolo.steamship.installer.yaml" << INSTALLER
# yaml-language-server: \$schema=${schema}.installer.${manifest_version}.schema.json

PackageIdentifier: Aureliolo.steamship
PackageVersion: ${version}
InstallerType: zip
NestedInstallerType: portable
Installers:
${x64_installer}
${arm64_installer}
ManifestType: installer
ManifestVersion: ${manifest_version}
INSTALLER

cat > "${out}/winget/Aureliolo.steamship.locale.en-GB.yaml" << LOCALE
# yaml-language-server: \$schema=${schema}.defaultLocale.${manifest_version}.schema.json

PackageIdentifier: Aureliolo.steamship
PackageVersion: ${version}
PackageLocale: en-GB
Publisher: Aureliolo
PublisherUrl: https://github.com/Aureliolo
PublisherSupportUrl: ${repository}/issues
PackageName: steamship
PackageUrl: ${repository}
License: MIT OR Apache-2.0
LicenseUrl: ${repository}#licence
ShortDescription: Uploads game builds to Steam with Valve's steamcmd.
Description: steamship runs Valve's own steamcmd with your app_build and depot_build scripts, sets steamcmd up pinned and verified, checks the scripts before anything is sent, reports the BuildID or the reason an upload failed, and never keeps a password.
Moniker: steamship
Tags:
- gamedev
- steam
- steamcmd
- steamworks
- upload
ReleaseNotesUrl: ${repository}/releases/tag/v${version}
ManifestType: defaultLocale
ManifestVersion: ${manifest_version}
LOCALE
