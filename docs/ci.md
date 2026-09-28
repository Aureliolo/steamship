# Uploading from CI

`steamship ci` sets up uploads from GitHub Actions once, from your own machine: the login goes
into a repository secret, and a step in your workflow uploads each build. No password ever
reaches the CI.

## GitHub Actions

```sh in your game's repository, after steamship login
steamship ci
```

`ci` finds the repository from `origin` and the app build script by what it holds, checks the
script, and logs in with the saved login to be sure Steam takes it. Then it sets the login as the
repository secret `STEAMSHIP_LOGIN` through the GitHub command line,
[`gh`](https://cli.github.com), and shows the step to add to your workflow, after the step that
builds your content:

```yaml
- name: Upload to Steam
  uses: Aureliolo/steamship@<commit> # vX.Y.Z
  with:
    script: steam/app_build.vdf
    version: ${{ github.ref_name }}
    login: ${{ secrets.STEAMSHIP_LOGIN }}
```

The action installs the steamship release it is pinned to, refusing it unless its SHA-256 and
its attestation check out, uploads, and gives the BuildID as its `build-id` output;
`preview: true` makes it Valve's dry run. It runs on Linux, Windows and macOS runners, and
Renovate raises its pin by itself.

When your content is built in another job, hand it over as a tar archive:
`actions/upload-artifact` does not keep file permissions, so a Linux or macOS program would
arrive without its executable bit, which `check` refuses.

A Workshop item goes up the same way, with its `workshopitem` script in place of the build's:

```yaml
- name: Upload the Workshop item
  uses: Aureliolo/steamship@<commit> # vX.Y.Z
  with:
    workshop: workshop/item.vdf
    login: ${{ secrets.STEAMSHIP_LOGIN }}
```

The item's ID is its `published-file-id` output. A script with no `publishedfileid` is refused
there, since every run would make another item; make the item once, add its ID to the script,
or set `new-item: true` for the one run that makes it.

When an upload or a Workshop item fails there, steamship prints Steam's own logs after the
reasons, each as a collapsed group: steamcmd's console, the app and depot build logs, and what
steamcmd added to its own logs during the run. All of it is redacted as steamship's logs always
are, and the account's name is masked.

Anything `ci` cannot find it asks for; `--script`, `--repo`, `--account` and `--secret` answer
ahead. The secret holds the token steamcmd saved and the account's name, never a password or a
Steam Guard secret, and nothing of it is shown.

## When the token expires

An upload in CI exits 3 when the token has expired. Run `steamship login` and `steamship ci`
again, and the next run uploads.

## Another CI

```sh
steamship ci --output login.txt
```

`--output` writes the login to a file instead of a secret. Give its contents to your CI as the
variable `STEAMSHIP_LOGIN`, then delete the file. `steamship upload` reads it from there, and
nothing else about uploading changes.

## Keeping steamship current

A tool that installs steamship from the release archives can pin it in any JSON file as a
`"steamship"` object holding, for each target, the release's tag and the archive's SHA-256:

```json
"steamship": {
  "x86_64-pc-windows-msvc": { "tag": "v{{version}}", "sha256": "<the .sha256 beside it>" },
  "x86_64-linux-musl": { "tag": "v{{version}}", "sha256": "<the .sha256 beside it>" }
}
```

The archive is `steamship-<version>-<target>.tar.gz`, or `.zip` for Windows, from the tag's
release. With this in your Renovate configuration, Renovate raises the tag and the digest
together:

```json
"extends": ["github>Aureliolo/steamship//.github/renovate-preset"]
```
