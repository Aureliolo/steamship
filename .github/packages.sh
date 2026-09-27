#!/usr/bin/env bash
# Writes the Homebrew formula and the Scoop manifest for one release, from the .sha256 files its
# build wrote beside the archives. Whoever calls this has already checked those files against the
# release's attestation, so every hash below is one the release is signed over.
#
#   .github/packages.sh <version> <folder with the .sha256 files> <folder to write into>
#
# It writes <folder>/Formula/steamship.rb and <folder>/bucket/steamship.json.
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

# The hash in a target's .sha256, refused unless the file names that target's archive: a sidecar
# for another archive, renamed, would otherwise pass for this one.
hash_of() {
  local archive="steamship-${version}-$1.$2"
  local file="${sums}/${archive}.sha256"
  local hash name
  if [[ ! -f "${file}" ]]; then
    echo "${file} does not exist." >&2
    exit 1
  fi
  read -r hash name < "${file}"
  if [[ ! "${hash}" =~ ^[0-9a-f]{64}$ || "${name#\*}" != "${archive}" ]]; then
    echo "${file} does not hold the SHA-256 of ${archive}." >&2
    exit 1
  fi
  printf '%s' "${hash}"
}

macos_arm="$(hash_of aarch64-apple-darwin tar.gz)"
macos_intel="$(hash_of x86_64-apple-darwin tar.gz)"
linux="$(hash_of x86_64-unknown-linux-musl tar.gz)"
windows="$(hash_of x86_64-pc-windows-msvc zip)"

mkdir -p "${out}/Formula" "${out}/bucket"

cat > "${out}/Formula/steamship.rb" << FORMULA
# Written by .github/packages.sh for each release, from the release's own signed checksums.
class Steamship < Formula
  desc "Uploads game builds to Steam with Valve's steamcmd"
  homepage "${repository}"
  license any_of: ["MIT", "Apache-2.0"]

  on_macos do
    on_arm do
      url "${download}/steamship-${version}-aarch64-apple-darwin.tar.gz"
      sha256 "${macos_arm}"
    end
    on_intel do
      url "${download}/steamship-${version}-x86_64-apple-darwin.tar.gz"
      sha256 "${macos_intel}"
    end
  end

  on_linux do
    on_intel do
      url "${download}/steamship-${version}-x86_64-unknown-linux-musl.tar.gz"
      sha256 "${linux}"
    end
  end

  def install
    bin.install "steamship"
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
  --arg url "${download}/steamship-${version}-x86_64-pc-windows-msvc.zip" \
  --arg hash "${windows}" \
  --arg folder "steamship-${version}-x86_64-pc-windows-msvc" \
  --arg next "${repository}/releases/download/v\$version/steamship-\$version-x86_64-pc-windows-msvc.zip" \
  '{
    version: $version,
    description: "Uploads game builds to Steam with Valve'"'"'s steamcmd.",
    homepage: $homepage,
    license: "MIT|Apache-2.0",
    architecture: {"64bit": {url: $url, hash: $hash, extract_dir: $folder}},
    bin: "steamship.exe",
    checkver: "github",
    autoupdate: {
      architecture: {"64bit": {
        url: $next,
        hash: {url: "$url.sha256"},
        extract_dir: "steamship-$version-x86_64-pc-windows-msvc"
      }}
    }
  }' > "${out}/bucket/steamship.json"
