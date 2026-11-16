#!/usr/bin/env bash
# Finishes the SHA-256 of each release-attachment pin in .github/tools.env whose tag a branch
# raised. Renovate hashes the checksum file a release ships when that file holds only a bare
# hash, which is not the archive's digest. GitHub lists the archive's own digest on the release,
# so the asset the base's pin names is found by its digest, renamed for the new tag and read from
# the new release.
#
# A pin is matched to the base's by its name, which is unique in the file. It is left as it is
# when anything is missing: no digest, no single matching asset, no such pin on the base, or a
# release GitHub or jq cannot read. The download's own checksum check still refuses a wrong
# digest. Nothing from the branch runs: the pins are read as text, and each value is checked
# before it reaches an address or a file.
#
#   .github/attachment-digests.sh <base-ref>
#
# Needs gh, jq and git. Prints the changed file to $GITHUB_OUTPUT as `files=` when it is set,
# whatever way the script ends.
set -euo pipefail

file=.github/tools.env
changed=""
failed=0

GITHUB_OUTPUT="${GITHUB_OUTPUT:-}"
trap '[[ -z "${GITHUB_OUTPUT}" ]] || echo "files=${changed}" >> "${GITHUB_OUTPUT}"' EXIT

if [[ $# -ne 1 ]]; then
  echo "usage: $0 <base-ref>" >&2
  exit 2
fi
# A commit id rather than a ref name: git for Windows' shell rewrites `origin/main:path` as a path.
base="$(git rev-parse --verify "$1^{commit}")"

# Prints one line per pin: <line of the SHA-256> <name> <repository> <tag> <digest>. The name is
# the key without its _TAG or _SHA256.
pins() {
  awk '
    /^# renovate: datasource=github-release-attachments depName=[^[:space:]]+ versioning=[^[:space:]]+$/ {
      split($0, parts, "depName=")
      split(parts[2], fields, " ")
      repository = fields[1]
      state = 1
      next
    }
    state == 1 && /^[A-Z0-9_]+_TAG=[^[:space:]]+$/ {
      split($0, pair, "=")
      name = pair[1]
      sub(/_TAG$/, "", name)
      tag = pair[2]
      state = 2
      next
    }
    state == 2 && /^[A-Z0-9_]+_SHA256=[0-9a-f]+$/ {
      split($0, pair, "=")
      if (pair[1] == name "_SHA256" && length(pair[2]) == 64) {
        print NR, name, repository, tag, pair[2]
      }
    }
    { state = 0 }
  '
}

# Prints the text with the one whole-token occurrence of $2 swapped for $3, and nothing unless
# there is exactly one. A token is not preceded by a letter, digit or dot, and not followed by a letter,
# digit or a dot and digit, so a version inside a longer one never matches.
replace_token() {
  local rest="$1" needle="$2" replacement="$3" head="" before after prefix last next next_two
  local count=0 result=""
  while [[ "${rest}" == *"${needle}"* ]]; do
    before="${rest%%"${needle}"*}"
    after="${rest#*"${needle}"}"
    prefix="${head}${before}"
    last="${prefix: -1}"
    next="${after:0:1}"
    next_two="${after:1:1}"
    if [[ ! "${last}" =~ [A-Za-z0-9.] && ! "${next}" =~ [A-Za-z0-9] ]] \
      && ! [[ "${next}" == . && "${next_two}" =~ [0-9] ]]; then
      count=$((count + 1))
      result="${prefix}${replacement}${after}"
    fi
    head="${prefix}${needle}"
    rest="${after}"
  done
  if ((count == 1)); then
    printf '%s' "${result}"
  fi
}

# Each list is made into a variable before it is read into an array, so that a command failing in
# it stops the script under `set -e`. An empty text gives an empty array.
current="$(pins < "${file}")"
pinned=()
[[ -z "${current}" ]] || mapfile -t pinned <<< "${current}"

previous=""
if git cat-file -e "${base}:${file}" 2> /dev/null; then
  previous="$(git show "${base}:${file}")"
  previous="$(pins <<< "${previous}")"
fi
earlier=()
[[ -z "${previous}" ]] || mapfile -t earlier <<< "${previous}"

declare -A base_repository=() base_tag=() base_digest=() base_count=() branch_count=()
for pin in "${earlier[@]}"; do
  read -r _ name repository tag digest <<< "${pin}"
  base_count["${name}"]=$((${base_count["${name}"]:-0} + 1))
  base_repository["${name}"]="${repository}"
  base_tag["${name}"]="${tag}"
  base_digest["${name}"]="${digest}"
done
for pin in "${pinned[@]}"; do
  read -r _ name _ _ _ <<< "${pin}"
  branch_count["${name}"]=$((${branch_count["${name}"]:-0} + 1))
done
for name in "${!base_tag[@]}"; do
  if [[ -z "${branch_count["${name}"]:-}" ]]; then
    echo "::notice file=${file}::${name} is on main but not on this branch."
  fi
done

for pin in "${pinned[@]}"; do
  read -r line name repository tag digest <<< "${pin}"
  label="${name} (${repository})"
  where="file=${file},line=${line}"

  if [[ -z "${base_tag["${name}"]:-}" ]]; then
    echo "::notice ${where}::${label} is new on this branch, so its SHA-256 is left as written."
    continue
  fi
  if ((branch_count["${name}"] > 1 || base_count["${name}"] > 1)); then
    echo "::notice ${where}::${name} is named more than once in ${file}, so ${label} keeps its SHA-256."
    continue
  fi
  old_tag="${base_tag["${name}"]}"
  old_digest="${base_digest["${name}"]}"
  if [[ "${old_tag}" == "${tag}" ]]; then
    continue
  fi
  if [[ "${repository}" != "${base_repository["${name}"]}" ]]; then
    echo "::notice ${where}::${label} names another repository than main's, so it keeps its SHA-256."
    continue
  fi
  if [[ ! "${repository}" =~ ^[A-Za-z0-9._-]+/[A-Za-z0-9._-]+$ ]] \
    || [[ ! "${tag}" =~ ^[A-Za-z0-9._+-]+$ ]] || [[ ! "${old_tag}" =~ ^[A-Za-z0-9._+-]+$ ]]; then
    echo "::error ${where}::${label} has a repository or tag outside letters, digits and . _ + -. Put back the one main has."
    failed=1
    continue
  fi

  if ! old_release="$(gh api "repos/${repository}/releases/tags/${old_tag}")" \
    || ! new_release="$(gh api "repos/${repository}/releases/tags/${tag}")"; then
    echo "::notice ${where}::GitHub did not list ${repository}'s releases ${old_tag} and ${tag}, so ${label} keeps its SHA-256."
    continue
  fi
  if ! found="$(jq -r --arg digest "sha256:${old_digest}" \
    '.assets[] | select(.digest == $digest) | .name' <<< "${old_release}")"; then
    echo "::notice ${where}::The ${old_tag} release of ${repository} could not be read, so ${label} keeps its SHA-256."
    continue
  fi
  names=()
  [[ -z "${found}" ]] || mapfile -t names <<< "${found//$'\r'/}"
  if ((${#names[@]} != 1)); then
    echo "::notice ${where}::${repository} ${old_tag} lists ${#names[@]} assets with the digest of ${label}, so it keeps its SHA-256."
    continue
  fi

  # Asset names carry the tag as written, without its v, or not at all. The first form the old
  # name holds is the one swapped.
  name_old="${names[0]}"
  name_new="${name_old}"
  for form in "${old_tag}" "${old_tag#v}"; do
    [[ "${name_old}" == *"${form}"* ]] || continue
    replacement="${tag}"
    [[ "${form}" == "${old_tag}" ]] || replacement="${tag#v}"
    name_new="$(replace_token "${name_old}" "${form}" "${replacement}")"
    break
  done
  if [[ -z "${name_new}" ]]; then
    echo "::notice ${where}::${name_old} holds ${old_tag} other than once as a whole version, so ${label} keeps its SHA-256."
    continue
  fi

  if ! found="$(jq -r --arg name "${name_new}" \
    '.assets[] | select(.name == $name) | .digest // ""' <<< "${new_release}")"; then
    echo "::notice ${where}::The ${tag} release of ${repository} could not be read, so ${label} keeps its SHA-256."
    continue
  fi
  digests=()
  [[ -z "${found}" ]] || mapfile -t digests <<< "${found//$'\r'/}"
  if ((${#digests[@]} != 1)) || [[ ! "${digests[0]}" =~ ^sha256:[0-9a-f]{64}$ ]]; then
    echo "::notice ${where}::${repository} ${tag} has no single ${name_new} with a digest, so ${label} keeps its SHA-256."
    continue
  fi
  live="${digests[0]#sha256:}"
  if [[ "${live}" != "${digest}" ]]; then
    sed -i "${line}s/[0-9a-f]\{64\}/${live}/" "${file}"
    echo "${label}: ${digest} is now ${live}, the digest of ${name_new}."
    changed="${file}"
  fi
done

exit "${failed}"
