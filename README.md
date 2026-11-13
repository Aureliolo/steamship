# <img src="docs/theme/ship.svg" width="32" align="absmiddle" alt=""> steamship

[![CI](https://github.com/Aureliolo/steamship/actions/workflows/ci.yml/badge.svg)](https://github.com/Aureliolo/steamship/actions/workflows/ci.yml)
[![crates.io](https://img.shields.io/crates/v/steamship?style=flat)](https://crates.io/crates/steamship)
[![Latest release](https://img.shields.io/github/v/release/Aureliolo/steamship?style=flat)](https://github.com/Aureliolo/steamship/releases/latest)
[![Cargo downloads](https://img.shields.io/crates/d/steamship?style=flat&label=cargo%20downloads)](https://crates.io/crates/steamship)
[![Docs](https://img.shields.io/badge/docs-aureliolo.github.io-0e7490?style=flat)](https://aureliolo.github.io/steamship/)
[![Scorecard](https://api.scorecard.dev/projects/github.com/Aureliolo/steamship/badge)](https://scorecard.dev/viewer/?uri=github.com/Aureliolo/steamship)
[![SLSA Build 3](https://img.shields.io/badge/SLSA-Build%20L3-2f6f4e?style=flat)](.github/release-process.md#slsa)
[![SBOM](https://img.shields.io/badge/SBOM-SPDX-2f6f4e?style=flat)](.github/release-process.md#what-a-release-carries)
[![Signed releases](https://img.shields.io/badge/releases-Sigstore%20signed-2f6f4e?style=flat)](https://aureliolo.github.io/steamship/install.html#verifying-a-download)
[![Rust 1.100.0](https://img.shields.io/badge/rust-1.100.0-b7410e?style=flat&logo=rust)](rust-toolchain.toml)
[![Licence: MIT OR Apache-2.0](https://img.shields.io/badge/licence-MIT%20OR%20Apache--2.0-blue)](#licence)

steamship is a command-line tool that uploads your game's builds to Steam through Valve's own
steamcmd. It runs steamcmd with your own `app_build` and `depot_build` scripts. It adds what
steamcmd leaves out:

- It sets steamcmd up for you, pinned and verified. It checks steamcmd again after every run.
- Each upload ends in its BuildID, or the reason it failed, read from Valve's build log rather than
  from console output.
- You log in once and steamship keeps no password. It redacts the login token from everything it
  prints. The token stays in [steamship's home](docs/install.md#the-home-folder) on your
  machine. `steamship ci` copies it into a CI secret or a file you name.
- A script that would upload the wrong content, such as a depot of another app, is refused before
  anything is sent.

It works with any engine that exports a folder of files per platform.

## Install

```sh
# macOS and Linux
brew tap aureliolo/steamship https://github.com/Aureliolo/steamship
brew install aureliolo/steamship/steamship
# Windows
winget install Aureliolo.steamship
# anywhere Rust is installed
cargo install --locked steamship
```

Every way to install, and how to verify a download, is on the
[install page](https://aureliolo.github.io/steamship/install.html).

## Quick start

```sh
steamship login
steamship check steam/app_build.vdf
steamship upload steam/app_build.vdf --version 1.4.0 --dry-run
steamship upload steam/app_build.vdf --version 1.4.0
```

`steam/app_build.vdf` and its depot scripts are Valve's own format. If you already upload with
steamcmd, you already have them. If not, `steamship init <app id>` writes a starting set.
`--dry-run` is Valve's dry run. It computes the whole build and uploads nothing.

`steamship ci` sets up the same upload from GitHub Actions.

In a game's folder, every command finds the game's files itself. There `steamship upload` alone
builds, checks and ships the pushed commit, as
[Shipping a game](https://aureliolo.github.io/steamship/games.html) describes.

## Documentation

The site is <https://aureliolo.github.io/steamship>:

- [Uploading](https://aureliolo.github.io/steamship/uploading.html)
- [Shipping a game](https://aureliolo.github.io/steamship/games.html)
- [Uploading from CI](https://aureliolo.github.io/steamship/ci.html)
- [Commands](https://aureliolo.github.io/steamship/commands.html)

Security issues go through [SECURITY.md](.github/SECURITY.md).

## Licence

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or
[MIT licence](LICENSE-MIT), at your option. Unless you say otherwise, any contribution you submit
for inclusion is licensed the same way, without any additional terms.

steamship is not affiliated with or endorsed by Valve. Steam and Steamworks are trademarks of Valve
Corporation.
