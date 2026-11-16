#!/usr/bin/env bash
# Finishes the SHA-256 of each release-attachment pin whose tag a branch raised. Renovate hashes
# the checksum file a release ships when that file holds only a bare hash, which is not the
# archive's digest. GitHub lists the archive's own digest on the release, so the asset the base's
# pin names is found by its digest, renamed for the new tag and read from the new release.
#
# A pin is left as it is when anything is missing: no digest, no matching asset, several assets
# with the base digest, or no such pin on the base. The download's own checksum check still
# refuses a wrong digest. Nothing from the branch runs: the pins are read as text, and each value
# is checked before it reaches an address or a file.
#
#   .github/attachment-digests.sh <base-ref>
#
# Needs gh, jq and git. Prints the changed files to $GITHUB_OUTPUT as `files=` when it is set.
set -euo pipefail

if [[ $# -ne 1 ]]; then
  echo "usage: $0 <base-ref>" >&2
  exit 2
fi
# A commit id rather than a ref name: git for Windows' shell rewrites `origin/main:path` as a path.
base="$(git rev-parse --verify "$1^{commit}")"

# Prints one line per pin: <line of the SHA-256> <repository> <tag variable> <tag> <digest>.
pins() {
  awk '
    /^[[:space:]]*# renovate: datasource=github-release-attachments depName=[^[:space:]]+ versioning=[^[:space:]]+$/ {
      split($0, parts, "depName=")
      split(parts[2], name, " ")
      repository = name[1]
      state = 1
      next
    }
    state == 1 && /^[[:space:]]*[A-Z0-9_]+_TAG: [^[:space:]]+$/ {
      variable = $1
      sub(/:$/, "", variable)
      tag = $2
      state = 2
      next
    }
    state == 2 && /^[[:space:]]*[A-Z0-9_]+_SHA256: [0-9a-f]+$/ && length($2) == 64 {
      print NR, repository, variable, tag, $2
    }
    { state = 0 }
  '
}

changed=()
failed=0
for file in .github/workflows/*.yml .github/actions/*/action.yml; do
  [[ -f "${file}" ]] || continue
  mapfile -t pinned < <(pins < "${file}")
  ((${#pinned[@]} > 0)) || continue
  if git cat-file -e "${base}:${file}" 2> /dev/null; then
    mapfile -t earlier < <(git show "${base}:${file}" | pins)
  else
    earlier=()
  fi

  for pin in "${pinned[@]}"; do
    read -r line repository variable tag digest <<< "${pin}"
    # The same pin on the base is the one with the same repository and variable, counted in order
    # for a file that holds several.
    seen=0
    for earlier_pin in "${pinned[@]}"; do
      read -r earlier_line earlier_repository earlier_variable _ _ <<< "${earlier_pin}"
      [[ "${earlier_repository} ${earlier_variable}" == "${repository} ${variable}" ]] || continue
      [[ "${earlier_line}" -lt "${line}" ]] && seen=$((seen + 1))
    done
    base_tag=""
    base_digest=""
    matches=0
    for earlier_pin in "${earlier[@]}"; do
      read -r _ earlier_repository earlier_variable earlier_tag earlier_digest <<< "${earlier_pin}"
      [[ "${earlier_repository} ${earlier_variable}" == "${repository} ${variable}" ]] || continue
      if ((matches == seen)); then
        base_tag="${earlier_tag}"
        base_digest="${earlier_digest}"
      fi
      matches=$((matches + 1))
    done
    label="${repository} ${variable} in ${file}"

    if [[ -z "${base_tag}" ]]; then
      echo "::notice file=${file},line=${line}::${label} is new on this branch; its SHA-256 is left as written."
      continue
    fi
    if [[ "${base_tag}" == "${tag}" ]]; then
      continue
    fi
    if [[ ! "${repository}" =~ ^[A-Za-z0-9._-]+/[A-Za-z0-9._-]+$ ]] \
      || [[ ! "${tag}" =~ ^[A-Za-z0-9._+-]+$ ]] || [[ ! "${base_tag}" =~ ^[A-Za-z0-9._+-]+$ ]]; then
      echo "::error file=${file},line=${line}::${label} has a repository or tag outside letters, digits and . _ + -. Put back the one main has."
      failed=1
      continue
    fi

    if ! old_release="$(gh api "repos/${repository}/releases/tags/${base_tag}")" \
      || ! new_release="$(gh api "repos/${repository}/releases/tags/${tag}")"; then
      echo "::notice file=${file},line=${line}::GitHub did not list ${repository}'s releases ${base_tag} and ${tag}, so ${label} keeps its SHA-256."
      continue
    fi
    mapfile -t names < <(jq -r --arg digest "sha256:${base_digest}" \
      '.assets[] | select(.digest == $digest) | .name' <<< "${old_release}" | tr -d '\r')
    if ((${#names[@]} != 1)); then
      echo "::notice file=${file},line=${line}::${repository} ${base_tag} lists ${#names[@]} assets with the digest of ${label}, so it keeps its SHA-256."
      continue
    fi
    name="${names[0]}"
    name="${name//"${base_tag}"/"${tag}"}"
    name="${name//"${base_tag#v}"/"${tag#v}"}"
    mapfile -t digests < <(jq -r --arg name "${name}" \
      '.assets[] | select(.name == $name) | .digest // ""' <<< "${new_release}" | tr -d '\r')
    if ((${#digests[@]} != 1)) || [[ ! "${digests[0]}" =~ ^sha256:[0-9a-f]{64}$ ]]; then
      echo "::notice file=${file},line=${line}::${repository} ${tag} has no single ${name} with a digest, so ${label} keeps its SHA-256."
      continue
    fi
    live="${digests[0]#sha256:}"
    if [[ "${live}" != "${digest}" ]]; then
      sed -i "${line}s/[0-9a-f]\{64\}/${live}/" "${file}"
      echo "${label}: ${digest} is now ${live}, the digest of ${name}."
      changed+=("${file}")
    fi
  done
done

if [[ -n "${GITHUB_OUTPUT:-}" ]]; then
  mapfile -t unique < <(printf '%s\n' "${changed[@]}" | sort -u)
  echo "files=${unique[*]}" >> "${GITHUB_OUTPUT}"
fi
exit "${failed}"
