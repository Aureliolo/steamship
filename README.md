# steamship

[![CI](https://github.com/Aureliolo/steamship/actions/workflows/ci.yml/badge.svg)](https://github.com/Aureliolo/steamship/actions/workflows/ci.yml)
[![Scorecard](https://api.scorecard.dev/projects/github.com/Aureliolo/steamship/badge)](https://scorecard.dev/viewer/?uri=github.com/Aureliolo/steamship)
[![SLSA Build 3](https://img.shields.io/badge/SLSA-Build%20L3-2f6f4e?style=flat)](.github/release-process.md#slsa)
[![SBOM](https://img.shields.io/badge/SBOM-SPDX-2f6f4e?style=flat)](.github/release-process.md#what-a-release-carries)
[![Signed releases](https://img.shields.io/badge/releases-Sigstore%20signed-2f6f4e?style=flat)](.github/release-process.md#verifying-a-release)
[![Rust 1.98.1](https://img.shields.io/badge/rust-1.98.1-b7410e?style=flat&logo=rust)](rust-toolchain.toml)
[![Licence: MIT OR Apache-2.0](https://img.shields.io/badge/licence-MIT%20OR%20Apache--2.0-blue)](#licence)

**steamcmd uploads your build. Everything around the upload is left to you:** where steamcmd
comes from, whether it updated itself since yesterday, whether the upload really worked, and
whether your login token ends up in a log.

steamship runs Valve's own steamcmd with your own `app_build` and `depot_build` scripts, and adds
what steamcmd leaves out:

- **steamcmd set up for you, pinned and verified.** Fetched on first use from Valve's signed
  package manifest at a version fixed in each steamship release, every package's SHA-256 checked,
  and on Windows Valve's code signature too. steamcmd is kept from updating itself, and its files
  are checked again after every run.
- **A clear result.** Success is read from the exit code and Valve's build log, not from
  steamcmd's console output, which is unreliable when captured. You get the BuildID, or the
  reason it failed.
- **No passwords kept.** You log in once. Your password and Steam Guard code are passed directly
  to steamcmd, never logged or saved, and nothing steamship prints or writes contains your login
  token.
- **Checked before it uploads.** Scripts that would upload the wrong thing are refused before
  anything is sent.

It works with any engine: anything that ends in a folder of files per platform can be shipped.

## Install

Download the archive for your system from the
[latest release](https://github.com/Aureliolo/steamship/releases/latest) (Windows x86-64, Linux
x86-64, macOS arm64 and x86-64) and put `steamship` on your `PATH`, or let Cargo do it:

```sh
cargo binstall steamship          # that same archive, in seconds
cargo install --locked steamship  # or built from source
```

## Upgrade

The same commands replace the steamship you have with the newest release:

```sh
cargo binstall steamship
cargo install --locked steamship
```

or download the archive from the latest release again. Once a day, at a terminal, steamship asks
GitHub whether a newer release is out; when one is, it says so after the command, with the command
that upgrades the way you installed. It never asks in CI, or with `STEAMSHIP_NO_UPDATE_CHECK` set.

## Quick start

```sh
steamship login                                     # once
steamship status                                    # is the login still good?
steamship check steam/app_build.vdf                 # offline, no login
steamship upload steam/app_build.vdf --version 1.4.0 --preview   # uploads nothing
steamship upload steam/app_build.vdf --version 1.4.0
```

`steam/app_build.vdf` and its `depot_build` files are Valve's own format, the same ones the
Steamworks SDK's ContentBuilder uses. If you already upload with steamcmd, you already have them.

## Uploading from CI

```sh
steamship ci    # once, in your game's repository, after steamship login
```

`steamship ci` sets your login as the repository secret `STEAMSHIP_LOGIN` and shows the step to
add to your workflow, after the step that builds your content:

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
`preview: true` makes it Valve's dry run. It runs on Linux, Windows and macOS runners.

## Commands

### `login`

```sh
steamship login [--account <name>]
```

Logs the account in with steamcmd. Without `--account`, steamship uses the account you logged in
with last, or asks for its name. It then asks for the password, and for a Steam Guard code or
approval in the Steam Mobile app; what you type is passed directly to steamcmd, never logged or
saved. steamcmd keeps a token, and later runs use it without a password. When it expires, an
upload stops and tells you to run this again.

### `status`

```sh
steamship status [--account <name>]
```

Shows the home, the build account and steamcmd, then logs in with the saved login as an upload
does and says whether Steam takes it. Nothing is asked for, and the account's name is never
printed. It exits 0 when an upload would log in, and 3 when it would not.

### `logout`

```sh
steamship logout
```

Forgets the login: the token steamcmd saved and the account steamship remembered. The next upload
needs `steamship login` first.

### `check`

```sh
steamship check <app_build.vdf>
```

Reads the app script and every depot script it names, without logging in, and refuses:

- `SetLive` naming `default`, which Valve only allows from the Steamworks site;
- `Preview` or `Local` set in the file (use `--preview` instead);
- a `ContentRoot` or `LocalPath` that does not exist or matches no files;
- a `steam_appid.txt` in the content;
- a Linux executable in the content without its executable bit, where the file system has one.

Keys steamship does not know are passed to steamcmd unchanged.

### `upload`

```sh
steamship upload <app_build.vdf> --version <version> [--preview] [--account <name>]
```

Runs `check`, then the build. Build output (logs, manifests, chunk cache) goes to steamship's own
folder per app, never into your content, and is kept between runs so later uploads are faster.
The build description is `<version> <commit>`, the commit being the Git commit your scripts are
in. A failure or an expired token ends the run instead of waiting at a prompt. It prints the
BuildID and, when the script names one, the branch it was set live on; in a GitHub Actions step
it also gives the BuildID as the step's `build-id` output.

`--preview` is Valve's dry run: the whole build is computed and logged, nothing is uploaded and
nothing is set live.

### `workshop`

```sh
steamship workshop <item.vdf> [--account <name>]
```

Uploads a Workshop item from Valve's `workshopitem` script, with the saved login. It refuses,
before anything is sent, an `appid` or `publishedfileid` that is not a number, a
`contentfolder` that is missing or empty, a `previewfile` that is missing or larger than the
1 MB Steam takes, a `visibility` other than 0 (public), 1 (friends only), 2 (private) or 3
(unlisted), and a title, description or change note longer than Steam takes. Paths are relative
to the script, as for `upload`.

A script with no `publishedfileid`, or `0`, makes a new item: steamship prints its ID and the line
to add to the script, so that later uploads update the same item. Your script is never rewritten.

### `ci`

```sh
steamship ci [--script <app_build.vdf>] [--repo <owner/name>] [--account <name>]
             [--secret <name>] [--output <file>]
```

Sets up uploads from GitHub Actions, run once in your game's repository after `steamship login`.
It finds the repository from `origin` and the app build script by what it holds, checks the
script, and logs in with the saved login to be sure Steam takes it. Then it sets the login as a
repository secret, `STEAMSHIP_LOGIN` unless `--secret` names another, through the GitHub command
line ([`gh`](https://cli.github.com)), and shows the workflow step that uploads, to add after the
step that builds your content. Anything it cannot find it asks for; the options answer ahead.

The secret holds the token steamcmd saved and the account's name, never a password or a Steam
Guard secret; nothing of it is shown. When the token expires, an upload in CI exits 3; run
`steamship login` and `steamship ci` again. For another CI, `--output <file>` writes the login to
a file instead: give its contents to the CI as `STEAMSHIP_LOGIN`, then delete the file.

### `install`

Optional. `login` and `upload` install steamcmd when it is missing; this does it ahead of time,
for example in CI, and verifies an existing install.

### Exit codes

| Code | Meaning                                                                              |
| ---- | ------------------------------------------------------------------------------------ |
| 0    | Done                                                                                 |
| 1    | The upload or login failed; the reason is printed, and for an upload the logs' paths |
| 2    | `check` refused the scripts, or the command line was wrong                           |
| 3    | Not logged in, or the token expired; run `steamship login`                           |
| 4    | steamcmd is missing or has been altered                                              |

## Configuration

| Variable                    | Meaning                                                                       |
| --------------------------- | ----------------------------------------------------------------------------- |
| `STEAMSHIP_HOME`            | Where steamcmd, its token and build output live; a per-user folder by default |
| `STEAMSHIP_ACCOUNT`         | The build account, if not the one `login` remembered                          |
| `STEAMSHIP_LOGIN`           | In CI, the login `steamship ci` set as a secret; `upload` and `status` use it |
| `STEAMSHIP_NO_UPDATE_CHECK` | Set to anything to never ask GitHub for a newer release                       |

There is no password setting anywhere.

## The build account

Valve recommends a dedicated Steam account for uploads with only **Edit App Metadata** and
**Publish App Changes To Steam**, in a [permission group](https://partner.steamgames.com/pub/groups/)
holding only the apps it uploads. Its token can do everything those permissions allow, so treat
`$STEAMSHIP_HOME` like a password.

## Roadmap

Before 1.0:

- branch status and moving a build between beta branches, through the partner Web API;
- a Renovate preset that raises steamship pins, and Scoop, winget and Homebrew packages.

## Licence

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or [MIT licence](LICENSE-MIT),
at your option. Unless you say otherwise, any contribution you submit for inclusion is licensed
the same way, without any additional terms.

steamship is not affiliated with or endorsed by Valve. Steam and Steamworks are trademarks of Valve
Corporation.
