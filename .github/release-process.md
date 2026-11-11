# Releases

## What a person does

Run **Prepare release** from the Actions tab and pick the bump. It waits for CI, CodeQL and
BinSkim to pass on `main`. It raises the version in `Cargo.toml` and `Cargo.lock` on a
`release/vX.Y.Z` branch, in a commit GitHub signs. It opens the pull request as the packaging app
with auto-merge on. The pull request merges itself once every required check has passed.

Those checks do not test, analyse or build the code again, since it differs in nothing but the
version. Nobody types a version twice or makes a tag by hand. Nothing waits on a click once the
run starts.

## What happens on the merge

`release-tag.yml` sees a new version on `main` with no tag, creates `vX.Y.Z` and starts
`release.yml` on it. `release.yml` calls `release-build.yml` for the first five steps, which
build, check and sign. Then it publishes and packages.

### Building

1. **verify** refuses to go on unless the release commit is on `main`, carries a valid
   signature, and the tag names the version in `Cargo.toml`.
2. **build** makes the five binaries, each on its own system. They are for x86-64 Linux, x86-64
   and arm64 Windows, and arm64 and x86-64 macOS.
   - The tests are not run again. `main` takes a pull request only once it is up to date and CI
     has passed on each of these systems.
   - Each binary records every crate it was built from, at its exact version
     (`cargo auditable`).
   - There is no Arm Linux build, because Valve's steamcmd for Linux is x86 only.

### Linux packages, SBOM and signing

1. **linux packages** builds the `.deb` and `.rpm` from the Linux archive with nFPM. It runs in
   the Linux build's job once the archive is packed and waits for no other system's build.
   - It lints each with its own system's linter.
   - It installs each on a bare Debian and Fedora, where it installs steamcmd and runs it.
2. **sbom** reads an SPDX SBOM from the five binaries themselves.
   - It checks both packages hold the Linux archive's binary byte for byte.
   - It checks the SBOM lists every crate linked into each binary at its locked version, and
     every file shipped with its SHA-256. An SBOM that lists nothing looks like a passing step.
3. **attest** signs everything through Sigstore and attests the SBOM against each archive and
   package. It gathers both signed attestations into one file.
   - It is the only job that can sign. It runs no code from the repository.

### Publishing

1. **publish** checks every file once more from the file alone, the way you would. Then it
   creates the GitHub release.
2. **crates** publishes the same tagged source, with no stored token, to crates.io. The registry
   trusts this workflow, with a short-lived token as the whole credential.
3. **action** uses the GitHub Action from the tag on each system, as a game's workflow would. It
   installs the release just published, checked by SHA-256 and attestation, then runs it.
4. **site** builds <https://aureliolo.github.io/steamship> from the tag and publishes it. Only
   `v*` tags can publish it, since the `github-pages` environment admits nothing else.
   - The site always describes the latest release, with its version, its archives and its
     commands.

### Packaging

1. **packages** writes the Homebrew formula, the Scoop manifest and the winget manifests from the
   release's checksums. Each is verified against the attestation first.
   - It installs them as a user would, with Homebrew on Linux and macOS. On x86-64 and Arm
     Windows it installs with Scoop, by the release's address and as a bucket.
   - It opens a pull request putting the formula and the Scoop manifest on `main`, as the
     packaging app. It merges the pull request once every required check has passed.
   - A check that fails, or anything else that keeps the pull request from merging, fails the job
     with the reason.

winget gets a release only when `winget.yml` is run on its tag, as [winget](#winget) says.

## What a release carries

- An archive per system, holding the binary, the README and both licences. The Linux binary,
  linked statically against musl, runs on any x86-64 Linux whatever its glibc.
- A `.deb` for Debian and Ubuntu and an `.rpm` for Fedora, holding the Linux binary, its man
  pages and completions. Both depend on the 32-bit C libraries Valve's steamcmd loads.
- A SHA-256 checksum for each archive and package.
- `steamship.json`, the Scoop manifest, which `scoop install` reads by the release's address.
- An SPDX SBOM of the five binaries.
- A Sigstore build-provenance attestation over all of these, and an SBOM attestation tying the
  SBOM to each archive and package, both in `steamship-X.Y.Z.intoto.jsonl`. They are keyless,
  with no signing key anywhere.

Releases are immutable. A published one cannot be edited or replaced. No release tag can be moved
or deleted.

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
GitHub-hosted runner. A checksum cannot prove that, since one made beside a tampered archive
agrees with it perfectly.

## SLSA

SLSA Build Level 3 on GitHub Actions means the build runs in a reusable workflow. The certificate
then names the build steps rather than whatever called them. `release-build.yml` is that
workflow. A verifier that passes `--signer-workflow` is requiring its exact steps at the tag.

The build and the signing are separate jobs. The runners are GitHub's and used once. The release
build restores no cache, which is the one way another run could reach into it.

## The packaging app

A pull request a workflow opens with its own token has its checks held until someone with write
access approves them. A merge made with that token starts no workflow on `main`. A GitHub App
installed on this repository avoids both, because its pull requests run their checks like
anyone's. The **packages** job opens and merges its pull request as the app. **Prepare release**
opens the release pull request as the app, with auto-merge on.

Each run makes an installation token for each change it makes, for this repository with those two
permissions. Each token lasts an hour. The waiting in between uses the job's own token, which only
reads. When main moves while the checks run, the branch is brought up to date and the checks run
again. If it moves a second time, the job fails and a re-run picks the pull request up where it
is.

### Setting it up, once

1. Create two environments under Settings, Environments, before anything goes into them:
   `packages`, limited to tags matching `v*`, and `release-prepare`, limited to the branch
   `main`. A job can then read the app's key only on a release tag or when preparing a release.
2. Create a GitHub App under Settings, Developer settings, GitHub Apps, with no webhook.
   - Its only repository permissions are **Contents: read and write** and **Pull requests: read
     and write**.
   - It subscribes to no events and installs only on this account.
   - The app can serve other repositories too, since each run's token is for this repository
     alone.
3. Install it on the repositories it serves, chosen one by one, never on all of them.
   `Aureliolo/steamship` is one.
4. Put the app's client ID in each environment's variable `PACKAGING_APP_CLIENT_ID`.
5. Generate a private key and put its contents in each environment's secret `PACKAGING_APP_KEY`.
   Then delete the key file.

## winget

winget takes packages only through a pull request to `microsoft/winget-pkgs`, which its
moderators review. So a release reaches winget only when it is chosen to:

```sh
gh workflow run winget.yml --ref v1.0.0
```

Run on a release's tag, `winget.yml` checks the manifest schema, then writes the winget manifests
from the release's signed checksums. It checks each installer against its signed archive, its
hash and the path of the program inside it. It validates them with `winget validate`, then
submits them with `.github/winget-submit.sh`. Run on a branch, it checks the schema alone.

The pull request is opened from a fork with a **classic** token. GitHub lets no fine-grained token
open a pull request on a repository its owner is not a member of. `wingetcreate` is not used,
because its newest release cannot read the manifest schema `.github/packages.sh` writes.

`.github/packages.sh` writes the manifests whole, one installer for each Windows archive. They are
not raised with `wingetcreate update`, which keeps only the installers the last version had. The
first version is submitted by hand, once, for a moderator to review.

### Setting up the token

1. Fork `microsoft/winget-pkgs` to the account that will own the token. No separate account is
   needed, and it needs no access to this repository. The fork is never synced: each submission
   builds on its newest commit, so the token needs no `workflow` scope.
2. On that account, create a classic token with only the `public_repo` scope.
3. Create an environment named `winget`, limited to tags matching `v*`. Put the token in its
   secret `WINGET_TOKEN`.

A missing or expired token fails the submit job.

The job pushes branch `Aureliolo.steamship-<version>` to the fork and opens one pull request from
it. A re-run opens nothing new when `microsoft/winget-pkgs` already holds the version or a pull
request for it is open. A branch left by a failed run is reused. A token whose account has no
fork fails the job.

## When a release fails

Re-running the release workflow on the tag is safe. A release the tag already carries, with
exactly its files, is taken as done. **Tag release** can be run by hand to start the release again
on a tag that exists. A version whose tag exists cannot be released again, since the tag cannot be
moved. Raise the version past it.
