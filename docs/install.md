# Install

steamship is one binary for Windows, Linux and macOS. Install it with your package manager, or
download a release and check its signature yourself. Either way, it sets up the steamcmd it needs
the first time it runs.

## Homebrew

On macOS and Linux:

```sh
brew tap aureliolo/steamship https://github.com/Aureliolo/steamship
brew install aureliolo/steamship/steamship
```

## Scoop

On Windows, straight from the latest release:

```powershell
scoop install https://github.com/Aureliolo/steamship/releases/latest/download/steamship.json
```

or with this repository as a bucket, which `scoop update` then follows:

```powershell
scoop bucket add aureliolo https://github.com/Aureliolo/steamship
scoop install aureliolo/steamship
```

## winget

On Windows:

```powershell
winget install Aureliolo.steamship
```

Homebrew, Scoop and winget install the release's archive only if it matches the SHA-256 the
release is signed over.

## Debian, Ubuntu and Fedora

On x86-64 Linux, the release's `.deb` or `.rpm`:

```sh
# Debian and Ubuntu
curl -fLO https://github.com/Aureliolo/steamship/releases/download/v{{version}}/steamship_{{version}}-1_amd64.deb
sudo apt install ./steamship_{{version}}-1_amd64.deb
# Fedora
sudo dnf install https://github.com/Aureliolo/steamship/releases/download/v{{version}}/steamship-{{version}}-1.x86_64.rpm
```

Each brings the 32-bit C libraries Valve's steamcmd for Linux loads and the certificates it checks
Steam against, and carries a man page per command and tab completion for bash, zsh and fish.
Check one as [Verifying a download](#verifying-a-download) shows, with the package's name in place
of the archive's.

Installed any other way on Linux, steamship needs those too: `lib32gcc-s1` and `ca-certificates`
on Debian and Ubuntu, `glibc.i686`, `libgcc.i686` and `ca-certificates` on Fedora,
`lib32-gcc-libs` and `ca-certificates` on Arch. Most systems have the certificates already; a
minimal container may not.

## Cargo

Anywhere Rust runs, built from source in a few minutes:

```sh
cargo install --locked steamship
```

With [cargo-binstall](https://github.com/cargo-bins/cargo-binstall), a Cargo add-on installed
separately, the release's own archive instead, in seconds:

```sh
cargo binstall steamship
```

## A release archive

Each [release](https://github.com/Aureliolo/steamship/releases/latest) carries an archive for
each system, with its checksum and signature. The Linux binary is linked statically, so it runs
on any x86-64 Linux, whatever its glibc. These are steamship {{version}}'s:

| System              | Archive                                                          |
| ------------------- | ---------------------------------------------------------------- |
| Linux x86-64        | [steamship-{{version}}-x86_64-linux-musl.tar.gz][linux]          |
| macOS Apple silicon | [steamship-{{version}}-aarch64-apple-darwin.tar.gz][macos-arm]   |
| macOS Intel         | [steamship-{{version}}-x86_64-apple-darwin.tar.gz][macos-intel]  |
| Windows x86-64      | [steamship-{{version}}-x86_64-pc-windows-msvc.zip][windows]      |
| Windows on Arm      | [steamship-{{version}}-aarch64-pc-windows-msvc.zip][windows-arm] |

[linux]: https://github.com/Aureliolo/steamship/releases/download/v{{version}}/steamship-{{version}}-x86_64-linux-musl.tar.gz
[macos-arm]: https://github.com/Aureliolo/steamship/releases/download/v{{version}}/steamship-{{version}}-aarch64-apple-darwin.tar.gz
[macos-intel]: https://github.com/Aureliolo/steamship/releases/download/v{{version}}/steamship-{{version}}-x86_64-apple-darwin.tar.gz
[windows]: https://github.com/Aureliolo/steamship/releases/download/v{{version}}/steamship-{{version}}-x86_64-pc-windows-msvc.zip
[windows-arm]: https://github.com/Aureliolo/steamship/releases/download/v{{version}}/steamship-{{version}}-aarch64-pc-windows-msvc.zip

From a terminal, the Linux one with curl, wget or the GitHub command line:

```sh
curl -fLO https://github.com/Aureliolo/steamship/releases/download/v{{version}}/steamship-{{version}}-x86_64-linux-musl.tar.gz
wget https://github.com/Aureliolo/steamship/releases/download/v{{version}}/steamship-{{version}}-x86_64-linux-musl.tar.gz
gh release download v{{version}} --repo Aureliolo/steamship --pattern '*linux-musl*'
```

Check it, then put `steamship` on your `PATH`.

### Verifying a download

```sh with the GitHub command line, gh
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
with it perfectly. Every release also carries an SPDX SBOM of its five binaries, attested against
each archive in the same file.

## Tab completion

Homebrew installs tab completion for bash, zsh and fish, and `man steamship`. Anywhere else,
steamship prints it for your shell to load:

```sh
steamship completions bash > ~/.local/share/bash-completion/completions/steamship
steamship completions zsh > "${fpath[1]}/_steamship"
steamship completions fish > ~/.config/fish/completions/steamship.fish
steamship completions powershell >> $PROFILE
```

Each release archive also carries them in `completions/`, and a man page for every command in
`man/`.

## Upgrading

Upgrade the way you installed:

```sh
brew upgrade steamship
scoop update steamship
winget upgrade Aureliolo.steamship
cargo install --locked steamship
cargo binstall steamship
```

or download the archive from the latest release again. Once a day, at a terminal, steamship asks
GitHub whether a newer release is out; when one is, it says so after the command, with the
command that upgrades the way you installed. It never asks in CI, or with
`STEAMSHIP_NO_UPDATE_CHECK` set.

## steamcmd

steamship runs Valve's own steamcmd, and installs it itself. The first `login` or `upload` fetches
it from Valve's signed package manifest at the version this steamship release pins, checks every
package's SHA-256, and on Windows Valve's code signature too. steamcmd is kept from updating
itself, and its files are checked again after every run: one that changed stops steamship with
exit code 4.

`steamship install` does it ahead of time, as in CI, and verifies an install that is already
there.

Valve keeps old steamcmd builds on its CDN for years, then deletes them. An install already made
keeps working after that; a new one fails, saying that Valve no longer serves the pinned steamcmd
and how to upgrade to a steamship that pins one it does.

steamcmd, its login token and the build output live in steamship's home, a per-user folder
unless `STEAMSHIP_HOME` names another. The token can do everything the build account may, so
treat that folder like a password.
