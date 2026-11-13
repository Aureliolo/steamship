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

To have `scoop update` follow new releases, add this repository as a bucket:

```powershell
scoop bucket add aureliolo https://github.com/Aureliolo/steamship
scoop install aureliolo/steamship
```

## winget

On Windows:

```powershell
winget install Aureliolo.steamship
```

Microsoft reviews each winget version. winget therefore lags behind some releases. Scoop and the
release archives carry every release.

Homebrew, Scoop and winget install an archive only when its SHA-256 matches the one the signed
release lists.

## Debian, Ubuntu and Fedora

On x86-64 Linux, the release's `.deb` or `.rpm`:

```sh
# Debian and Ubuntu
curl -fLO https://github.com/Aureliolo/steamship/releases/download/v{{version}}/steamship_{{version}}-1_amd64.deb
sudo apt install ./steamship_{{version}}-1_amd64.deb
# Fedora
sudo dnf install https://github.com/Aureliolo/steamship/releases/download/v{{version}}/steamship-{{version}}-1.x86_64.rpm
```

Each package also installs the 32-bit C libraries and certificates steamcmd needs, a man page per
command and [tab completion](#tab-completion). Check one as
[Verifying a download](#verifying-a-download) shows, with the package's name in place of the
archive's.

Any other install on Linux needs these packages:

- Debian and Ubuntu: `lib32gcc-s1` and `ca-certificates`
- Fedora: `glibc.i686`, `libgcc.i686` and `ca-certificates`
- Arch: `lib32-gcc-libs` and `ca-certificates`

Most systems have the certificates already. A minimal container may not.

## Cargo

Anywhere Rust runs, built from source:

```sh
cargo install --locked steamship
```

[cargo-binstall](https://github.com/cargo-bins/cargo-binstall), a separate Cargo add-on,
downloads the release's own archive instead of building:

```sh
cargo binstall steamship
```

On Windows, Cargo and cargo-binstall install steamship.exe alone, and steamcmd then runs on
Windows' own console. `steamship status` names the console.

## A release archive

Each [release](https://github.com/Aureliolo/steamship/releases/latest) carries an archive for
each system, with its checksum and signature. The Linux binary is linked statically and runs on
any x86-64 Linux, whatever its glibc. These are steamship {{version}}'s:

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

The Windows archives also carry conpty.dll and OpenConsole.exe, Microsoft's console that
steamcmd runs on, with their MIT licence in LICENSE-CONPTY. Keep them beside steamship.exe.

### From a terminal

The Linux archive with curl, wget or the GitHub command line:

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
GitHub-hosted runner. Every release also carries an SPDX SBOM of its five binaries, attested
against each archive in the same file.

Once installed, `steamship self check` compares its own binary with the SHA-256 that SBOM lists
for your system, fetched from the release. When GitHub's download address fails, it reads the SBOM
through GitHub's API instead. It exits 2 if they differ. A job can run it as a check before an
upload.

A release cannot be changed once published. A binary that differs was built elsewhere or altered
since. A build from source differs from every release's.

## Tab completion

Homebrew and the Linux packages install a man page per command and tab completion for bash, zsh
and fish. Anywhere else, steamship prints the completion for your shell to load:

```sh
steamship completions bash > ~/.local/share/bash-completion/completions/steamship
steamship completions zsh > "${fpath[1]}/_steamship"
steamship completions fish > ~/.config/fish/completions/steamship.fish
steamship completions powershell >> $PROFILE
```

Each release archive also carries the completions in `completions/` and a man page for every
command in `man/`.

## Upgrading

Upgrade the way you installed:

```sh
brew upgrade steamship
scoop update steamship
winget upgrade Aureliolo.steamship
cargo install --locked steamship
cargo binstall steamship
```

Or download the archive from the latest release again.

Once a day, at a terminal, steamship asks GitHub whether a newer release is out. If one is, it
says so after the command, with the command that upgrades the way you installed. It never asks in
CI, or with `STEAMSHIP_NO_UPDATE_CHECK` set.

## steamcmd

steamship runs Valve's own steamcmd and installs it itself. The first `login` or `upload` fetches
it from Valve's signed package manifest, at the version this steamship release pins. It checks
every package's SHA-256. On Windows it checks Valve's code signature too.

steamcmd is kept from updating itself. Its files are checked again after every run. A changed
file or a failed install stops steamship with exit code 4. When the run has left a build on Steam,
or a new Workshop item whose ID no file holds, the same check exits 5, since a second run would
make another.

`steamship install` installs it ahead of time, as a CI job would. It also verifies an install
that is already there.

Valve eventually deletes old steamcmd builds from its CDN. An existing install keeps working. A new
install fails. It says that Valve no longer serves the pinned steamcmd and how to upgrade to a
steamship that pins one it does.

### The home folder

steamcmd, its login token and the build output live in steamship's home. That is a per-user
folder unless `STEAMSHIP_HOME` names another. The token can do everything the build account may,
so treat that folder like a password.

The default is `%LOCALAPPDATA%\steamship` on Windows, `$XDG_DATA_HOME/steamship` on Linux (or
`~/.local/share/steamship` when that is unset) and `~/Library/Application Support/steamship` on
macOS. `steamship status` prints the folder in use.

| Entry                                           | Holds                                                           |
| ----------------------------------------------- | --------------------------------------------------------------- |
| `steamcmd/` and `steamcmd.inventory`            | The pinned steamcmd and the record it is verified against       |
| `steamcmd.new/`, `steamcmd.old/`, `downloads/`  | An install's working folders, removed when the install finishes |
| `steamship.lock`                                | What two runs take turns on                                     |
| `account`                                       | The build account `login` remembered                            |
| `latest-release`                                | The update check's last answer                                  |
| `Steam/` on Linux, `Library/` on macOS          | steamcmd's own state, which on Windows is inside `steamcmd/`    |
| `apps/<app>/`                                   | Build output, with the chunk cache that makes uploads faster    |
| `recorded-builds/<app>.json`                    | The builds uploads set live, which `rollback` reads             |
| `workshop/<app>/<item>/`                        | The script steamcmd was given and its console                   |
| `fetched/<app>/<branch>-<system>/`              | What `fetch` downloaded                                         |

The token is `steamcmd/config/config.vdf` on Windows, `Steam/config/config.vdf` on Linux and
`Library/Application Support/Steam/config/config.vdf` on macOS. The `logs/` folder beside `config/`
holds the account's name. An install that stopped midway can leave the token's file in
`steamcmd.new/`.

Once steamcmd has run, each `.log` under `apps/<app>/output` shows `[redacted]` where the token or
the account's name stood, or is gone. The `log` line names any log that is neither. A run that was
killed leaves steamcmd's own logs as steamcmd wrote them.

To cache between CI jobs, keep `apps` and `recorded-builds` from a job that ended well, without
the logs `apps/*/output/*.log`. Never keep `account`, the folders that hold the token, or an
install's working folders.

Every steamship on the machine shares that home, whichever release each repository pins. Two runs
that use steamcmd take turns. The second waits, and says so, until the first has checked steamcmd
again after its run. A release that pins another steamcmd installs its own over the one there and
keeps the login.
