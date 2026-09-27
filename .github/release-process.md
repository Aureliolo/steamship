# Releases

## What a person does

Run **Prepare release** from the Actions tab and pick the bump. It raises the version in
`Cargo.toml` and `Cargo.lock` on a `release/vX.Y.Z` branch, in a commit GitHub signs, opens the
pull request, and links it in the run summary.

That pull request's checks wait for **Approve workflows to run** in the merge box, which is
GitHub's rule for anything a workflow opens itself. Approve them, and merge once they are green.
Everything after the merge is automatic. Nobody types a version twice and nobody makes a tag by
hand.

## What happens on the merge

`release-tag.yml` sees a new version on `main` with no tag, creates `vX.Y.Z`, and starts
`release.yml` on it. `release.yml` calls `release-build.yml`, which does the first four steps,
and then publishes:

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

## What a release carries

- An archive per system, holding the binary, the README and both licences. The Linux binary is
  linked statically against musl, so it runs on any x86-64 Linux, whatever its glibc.
- A SHA-256 checksum for each archive.
- An SPDX SBOM of the four binaries.
- A Sigstore build-provenance attestation over all of these, and an SBOM attestation tying the
  SBOM to each archive, both in `steamship-X.Y.Z.intoto.jsonl`. They are keyless: there is no
  signing key anywhere.

Releases are immutable: a published one cannot be edited or replaced, and no release tag can be
moved or deleted.

## Verifying a release

```sh
VERSION=X.Y.Z
ARCHIVE=steamship-${VERSION}-x86_64-unknown-linux-musl.tar.gz
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

## When a release fails

Re-running the release workflow on the tag is safe: a release the tag already carries, with
exactly its files, is taken as done. **Tag release** can be run by hand to start the release
again on a tag that exists. A version whose tag exists cannot be released again, since the tag
cannot be moved; raise the version past it.
