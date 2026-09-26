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

- **No password.** steamship has no option, variable or file for a password or Steam Guard code.
  You type them into steamcmd once, and steamcmd keeps a token in its `config/config.vdf`.
- **The token file.** steamship restricts steamcmd's `config` folder to your user. It reads
  `config.vdf` only to remove the token's values from everything it prints or writes, and a test
  plants a token and checks every output path.
- **Credentials through the environment.** Anything secret reaches steamship through environment
  variables, never command-line arguments, which other users of the machine can list.
- **steamcmd itself.** Installed from Valve's signed manifest at a pinned version, every package
  checked by SHA-256 and, on Windows, every program and library by Valve's code signature.
  steamcmd is kept from updating itself, and re-checked after every run.
- **Releases.** Built by GitHub Actions from a tagged commit, with a SHA-256 and a signed build
  provenance attestation for each binary.
