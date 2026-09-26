# steamship

[![CI](https://github.com/Aureliolo/steamship/actions/workflows/ci.yml/badge.svg)](https://github.com/Aureliolo/steamship/actions/workflows/ci.yml)
[![Licence: MIT OR Apache-2.0](https://img.shields.io/badge/licence-MIT%20OR%20Apache--2.0-blue)](#licence)

**steamcmd uploads your build. Everything around the upload is left to you:** where steamcmd
comes from, whether it updated itself since yesterday, whether the upload really worked, and
whether your login token ends up in a log.

steamship runs Valve's own steamcmd with your own `app_build` and `depot_build` scripts, and adds
what steamcmd leaves out:

- **steamcmd set up for you, pinned and verified.** Fetched on first use from Valve's signed
  package manifest at a version fixed in each steamship release, every package's SHA-256 checked.
  steamcmd updates itself, so its files are checked again after every run.
- **A clear result.** Success is read from the exit code and Valve's build log, not from
  steamcmd's console output, which is unreliable when captured. You get the BuildID, or the
  reason it failed.
- **No passwords.** You log in once, yourself, into steamcmd. steamship never sees a password or
  Steam Guard code, and nothing it prints or writes contains your login token.
- **Checked before it uploads.** Scripts that would upload the wrong thing are refused before
  anything is sent.

It works with any engine: anything that ends in a folder of files per platform can be shipped.

> **Status:** in development, nothing released yet. This README is the contract for 1.0.

## Install

Download the binary for your system from
[Releases](https://github.com/Aureliolo/steamship/releases) (Windows x86-64, Linux x86-64, macOS
arm64 and x86-64), or build it from source:

```sh
cargo install --locked steamship@<version>
```

## Quick start

```sh
steamship login --account <build-account>           # once; you type into steamcmd yourself
steamship check steam/app_build.vdf                 # offline, no login
steamship upload steam/app_build.vdf --version 1.4.0 --preview   # uploads nothing
steamship upload steam/app_build.vdf --version 1.4.0
```

`steam/app_build.vdf` and its `depot_build` files are Valve's own format, the same ones the
Steamworks SDK's ContentBuilder uses. If you already upload with steamcmd, you already have them.

## Commands

### `login`

```sh
steamship login --account <build-account>
```

Runs `steamcmd +login <build-account> +quit` in your terminal. You type the password and Steam
Guard code into steamcmd, or approve in the Steam Mobile app; steamship does not read them.
steamcmd keeps a token, and later runs use it without a password. When it expires, an upload
stops and tells you to run this again.

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
BuildID and, when the script names one, the branch it was set live on.

`--preview` is Valve's dry run: the whole build is computed and logged, nothing is uploaded and
nothing is set live.

### `install`

Optional. `login` and `upload` install steamcmd when it is missing; this does it ahead of time,
for example in CI, and verifies an existing install.

### Exit codes

| Code | Meaning                                                          |
| ---- | ---------------------------------------------------------------- |
| 0    | Done                                                             |
| 1    | The upload failed; the reason and the log's path are printed     |
| 2    | `check` refused the scripts, or the command line was wrong       |
| 3    | Not logged in, or the token expired; run `steamship login`       |
| 4    | steamcmd is missing, altered, or has updated itself past the pin |

## Configuration

| Variable            | Meaning                                                               |
| ------------------- | --------------------------------------------------------------------- |
| `STEAMSHIP_HOME`    | Where steamcmd, its token and build output live; a per-user folder by default |
| `STEAMSHIP_ACCOUNT` | The build account, if not the one `login` remembered                  |

There is no password setting anywhere.

## The build account

Valve recommends a dedicated Steam account for uploads with only **Edit App Metadata** and
**Publish App Changes To Steam**, in a [permission group](https://partner.steamgames.com/pub/groups/)
holding only the apps it uploads. Its token can do everything those permissions allow, so treat
`$STEAMSHIP_HOME` like a password.

## Roadmap

After 1.0:

- a GitHub Action;
- Workshop items (`workshop_build_item`);
- branch status and moving a build between beta branches, through the partner Web API;
- a Renovate preset that raises steamship pins, and Scoop, winget and Homebrew packages.

## Licence

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or [MIT licence](LICENSE-MIT),
at your option. Unless you say otherwise, any contribution you submit for inclusion is licensed
the same way, without any additional terms.

steamship is not affiliated with or endorsed by Valve. Steam and Steamworks are trademarks of Valve
Corporation.
