# Releases

## What a person does

Run **Prepare release** from the Actions tab and pick the bump. It raises the version in
`Cargo.toml` and `Cargo.lock` on a `release/vX.Y.Z` branch, in a commit GitHub signs, and opens
the pull request as the packaging app with auto-merge on: it merges itself once every required
check has passed. Nobody types a version twice, nobody makes a tag by hand, and nothing waits on
a click once the run starts.

## What happens on the merge

`release-tag.yml` sees a new version on `main` with no tag, creates `vX.Y.Z`, and starts
`release.yml` on it. `release.yml` calls `release-build.yml`, which does the first four steps,
and then publishes and packages:

1. **verify** refuses to go on unless the release commit is on `main`, carries a valid
   signature, and the tag names the version in `Cargo.toml`.
2. **build** makes the four binaries, each on the system it is for: x86-64 Linux, x86-64
   Windows, and arm64 and x86-64 macOS. The tests run again where the system can run them.
   Each binary records every crate it was built from, at its exact version (`cargo auditable`).
3. **sbom** reads an SPDX SBOM from the four binaries themselves, then checks it lists every
   crate linked into each at its locked version and every file shipped with its SHA-256, since
   an SBOM that lists nothing looks exactly like a passing step.
4. **attest** signs everything through Sigstore, attests the SBOM against each archive, and
   gathers both signed attestations into one file. It is the only job that can sign, and it
   runs no code from the repository.
5. **publish** checks every file once more the way you would, from the file alone, and creates
   the GitHub release.
6. **crates** publishes the same tagged source to crates.io with no stored token: crates.io
   trusts this workflow, and a short-lived token is the whole credential.
7. **action** uses the GitHub Action from the tag on each system, as a game's workflow would: it
   installs the release just published, checked by SHA-256 and attestation, and runs it.
8. **site** builds <https://aureliolo.github.io/steamship> from the tag and publishes it, so the
   site always describes the latest release: its version, its archives and its commands. The
   `github-pages` environment admits `v*` tags only, so nothing else can publish it.
9. **packages** writes the Homebrew formula and the Scoop manifest from the release's checksums,
   each verified against the attestation first, and installs them as a user would: with
   Homebrew on Linux and macOS, and with Scoop on Windows, by the release's address and as a
   bucket. Then it opens a pull request putting them on `main`, as the packaging app, and merges
   it once every required check has passed; a check that fails, or anything else that keeps it
   from merging, fails the job with the reason.
10. **winget** writes the new version's manifests with Microsoft's `wingetcreate`, checks the
    installer hash in them is the one the release is signed over, and submits them to
    `microsoft/winget-pkgs`, where Microsoft's checks and moderators merge them.

## What a release carries

- An archive per system, holding the binary, the README and both licences. The Linux binary is
  linked statically against musl, so it runs on any x86-64 Linux, whatever its glibc.
- A SHA-256 checksum for each archive.
- `steamship.json`, the Scoop manifest, which `scoop install` reads by the release's address.
- An SPDX SBOM of the four binaries.
- A Sigstore build-provenance attestation over all of these, and an SBOM attestation tying the
  SBOM to each archive, both in `steamship-X.Y.Z.intoto.jsonl`. They are keyless: there is no
  signing key anywhere.

Releases are immutable: a published one cannot be edited or replaced, and no release tag can be
moved or deleted.

## Verifying a release

```sh
VERSION=X.Y.Z
ARCHIVE=steamship-${VERSION}-x86_64-linux-musl.tar.gz
gh release download "v${VERSION}" --repo Aureliolo/steamship
sha256sum -c "${ARCHIVE}.sha256"
gh attestation verify "${ARCHIVE}" --repo Aureliolo/steamship \
  --bundle "steamship-${VERSION}.intoto.jsonl" \
  --signer-workflow Aureliolo/steamship/.github/workflows/release-build.yml \
  --source-ref "refs/tags/v${VERSION}" \
  --deny-self-hosted-runners
```

The checksum proves the bytes are the ones the release lists. The attestation proves GitHub
Actions built them from this repository, by the steps in `release-build.yml` at that tag, on a
GitHub-hosted runner, which a checksum cannot: a checksum made beside a tampered archive agrees
with it perfectly.

## SLSA

SLSA Build Level 3 on GitHub Actions means the build runs in a reusable workflow, so the
certificate names the build steps rather than whatever called them. `release-build.yml` is that
workflow, and a verifier that passes `--signer-workflow` is requiring its exact steps at the tag.
The build and the signing are separate jobs, the runners are GitHub's and used once, and the
release build restores no cache, which is the one way another run could reach into it.

## The packaging app

A pull request a workflow opens with its own token has its checks held until someone with write
access approves them, and a merge made with that token starts no workflow on `main`. So the
**packages** job opens and merges its pull request, and **Prepare release** opens the release pull
request with auto-merge on, as a GitHub App installed on this repository, whose pull requests run
their checks like anyone's. Setting it up, once:

1. Create two environments under Settings, Environments, before anything goes into them:
   `packages`, limited to tags matching `v*`, and `release-prepare`, limited to the branch
   `main`. A job can then read the app's key only on a release tag, or when preparing a release
   from main.
2. Create a GitHub App under Settings, Developer settings, GitHub Apps. It needs no webhook.
   Its repository permissions are **Contents: read and write** and **Pull requests: read and
   write**, and nothing else; it subscribes to no events and installs only on this account. The
   app can serve other repositories too; each run takes a token for this repository alone, with
   those two permissions.
3. Install it on the repositories it serves, chosen one by one, never on all of them; here that
   includes `Aureliolo/steamship`.
4. Generate a private key. Put the app's client ID in each environment's variable
   `PACKAGING_APP_CLIENT_ID` and the key's contents in each one's secret `PACKAGING_APP_KEY`,
   then delete the key file.

Each run makes installation tokens for this repository with those two permissions, which last an
hour, one for each change it makes; the waiting in between uses the job's own token, which only
reads. When main moves while the checks run, the branch is brought up to date and the checks run
again; if it moves a second time, the job fails and a re-run picks the pull request up where it is.

## winget

winget takes packages only through a pull request to `microsoft/winget-pkgs`, which
`wingetcreate` opens from a fork with a **classic** token: GitHub lets no fine-grained token open
a pull request on a repository its owner is not a member of, and `wingetcreate` says so.

The first version is submitted by hand, once, and a moderator reviews it; `wingetcreate update`
only updates a package that exists. After it is accepted:

1. Create a GitHub account used for nothing else, and fork `microsoft/winget-pkgs` to it. It needs
   no access to this repository.
2. On that account, create a classic token with only the `public_repo` scope and an expiry.
3. Create an environment named `winget`, limited to tags matching `v*`, and put the token in its
   secret `WINGET_TOKEN`.
4. Set the repository variable `WINGET` to `submit`.

The job runs only with `WINGET` set, as **crates** does with `CRATES_IO`: before the first version
is in winget-pkgs, every release would fail on a step nothing could make pass. With it set, a
missing token fails the release. When the token expires, the job fails; make a new one the same
way.

## When a release fails

Re-running the release workflow on the tag is safe: a release the tag already carries, with
exactly its files, is taken as done. **Tag release** can be run by hand to start the release
again on a tag that exists. A version whose tag exists cannot be released again, since the tag
cannot be moved; raise the version past it.
