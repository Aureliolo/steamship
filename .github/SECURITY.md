# Security

## Reporting

Report a vulnerability privately through
[GitHub's private reporting](https://github.com/Aureliolo/steamship/security/advisories/new),
not in a public issue. You will get an answer within a week.

Anything that could leak a Steam login token, or run a steamcmd steamship did not verify, is in
scope.

## Supported versions

Only the latest release gets fixes.

## What steamship handles

- **No stored password.** steamship has no option, variable or file for a password or Steam Guard
  code. What you type at `login` is passed directly to steamcmd, never logged or saved. steamcmd
  keeps a token in its `config/config.vdf`.
- **The token file.** steamship restricts steamcmd's `config` folder to your user. It reads
  `config.vdf` to remove the token's values from everything it prints or writes, and a test
  plants a token and checks every output path; `ci` reads it to set it as a secret.
- **The CI secret.** `steamship ci` packs `config.vdf` and the account's name into one value and
  hands it to `gh` on its input, never on a command line or the screen, or writes it to a file
  readable by its owner alone. In CI, `upload` and `status` write it back where steamcmd reads it,
  and a value that is not one is refused without being repeated.
- **The Web API key.** `builds`, `promote`, `branch`, `achievements`, `leaderboards` and
  `rich-presence` take the publisher key from `STEAMSHIP_WEB_API_KEY`, or else from the system's credential
  store, and send it in the `x-webapi-key` header, never in an address, so that no error which
  names an address can name the key; a value that is not a key is refused without being
  repeated. A typed key is shown as dots and kept only after Steam has accepted it, in Windows
  Credential Manager (this machine only, never roaming), the macOS Keychain (never synchronised
  to iCloud) or the Linux Secret Service, one for each steamship home; `logout` removes it. On
  Linux steamship reaches the Secret Service with its own small D-Bus client, fuzzed, rather than
  a large third-party stack; the key crosses only the local socket to the session bus, which the
  kernel keeps to your user.
- **Nothing secret on the command line.** A password or Steam Guard code reaches steamship only as
  typed at `login`, or piped to it, never as a command-line argument, which other users of the
  machine can list.
- **steamcmd itself.** Installed from Valve's signed manifest at a pinned version, every package
  checked by SHA-256 and, on Windows, every program and library by Valve's code signature.
  steamcmd is kept from updating itself, and re-checked after every run.
- **Releases.** Built by GitHub Actions from a signed, tagged commit on main, at SLSA Build
  Level 3, with a SHA-256, an SPDX SBOM and Sigstore attestations for every file. A published
  release and its tag cannot be changed.
- **Packages.** The Homebrew formula and the Scoop manifest are written by the release from its
  checksums, each verified against the release's attestation first, and installed on each system
  before they reach main; winget's manifest is sent only once its hash matches too. The GitHub
  App that lands them on main can change this repository's contents and pull requests and
  nothing else, and the winget token belongs to an account that owns nothing but a fork of
  winget-pkgs. Both are secrets of an environment only release tags can use.
