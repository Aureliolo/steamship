# Working on steamship

How changes are made here, for people and coding agents alike. CI enforces most of it. Review asks
for the rest.

## Writing

Everything a person reads counts, from messages and help text to docs, comments and commit
messages.

- Write British English (colour, behaviour, licence as the noun). `typos` checks it with the en-gb
  locale.
- No em or en dashes, and no semicolons in prose: use a colon, comma, full stop or brackets. The
  static checks fail on a dash.
- Code keeps the semicolons its language needs.
- No marketing filler. The static checks fail on the words in `.github/hype-words.txt`.
- Say what steamship does, with a number only when it was measured.
- Describe steamship as it is meant to be. Leave out status lines, "not yet" and caveats for a
  gap that is about to close.
- A message says what happened and what to do next. It names the file, app or branch it is about.

## Secrets and private details

- Never print, log or write in plain text a password, Steam Guard code, login token or Web API
  key. The same holds for anything that contains one.
- Nothing private goes into the repository or onto its GitHub pages. That covers any local path,
  machine or account, and the name of any private repository.
- Nor does anything about a game that is not public, such as its name, Steam ID or content.
  Fixtures and examples use sample names and IDs, such as Spacewar's 480.

## Code

- Comments say why, never what. One that restates the line below it goes.
- A comment speaks to the next reader, without "changed to", "now uses", "previously" or issue
  numbers. The history records what changed.
- Tests are named as sentences that say what holds, and assert on what a person sees.
- `Cargo.toml` turns on clippy's pedantic, nursery, cargo and restriction groups. The few lints
  allowed are listed there with the reason.
- Fix a lint rather than silence it. An `#[expect]` says why in its `reason`.
- `unsafe` stays in `src/unix.rs` and `src/windows.rs`, each block with a `SAFETY:` comment.

## Layout

- `src/` is one crate, library and binary.
- The command line is the supported interface. The library's modules are public only for the
  tests and fuzz targets.
- `docs/` is the site's source. `docs-site/` renders it, with every command's help, into
  `target/site`.
- `fuzz/` holds a cargo-fuzz target per parser, a workspace member so it shares the lint table.
- `pins/` holds steamcmd's manifests, compiled into steamship. Renovate raises them, or
  `cargo run --example pins` rewrites them by hand.
- `.github/packages.sh` writes `Formula/` and `bucket/` at each release. Leave them to it.
- Releases start from the Actions tab, as `.github/release-process.md` describes.

## Steam

- Credentials reach steamcmd through its environment or the system's credential store, never
  its arguments.
- steamship sets the default branch live only through the Web API and only while steamcmd's
  app info shows the build account the app unreleased. It never sends the SteamID a released
  app's default needs.
- steamship never describes the default branch, which Valve keeps to the Steamworks site.
  `check` refuses a script whose `SetLive` names it, which steamcmd cannot do.
- steamship deletes a leaderboard and its scores only while steamcmd's app info shows the build
  account the app unreleased. It never changes a leaderboard's settings.
- Every line steamcmd or Steam is expected to print, in code or a test fixture, is copied from a
  real run, never guessed.

## Scope

In scope is getting a game's builds, content and configuration into Steamworks and checking
them there. Out of scope are financial reports (sales, wishlists) and live operations (Workshop
moderation, game server tokens, cheat reports).

## Supply chain

Every action is pinned by commit SHA with its version beside it. Every tool a workflow downloads
is pinned by version and SHA-256. Renovate raises them, apart from winget, whose newer release
fails `winget.yml`'s weekly run. Nothing is fetched at "latest".

## Before a pull request

Run these, which are fast:

```sh
cargo fmt --check
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo test --locked
markdownlint-cli2
typos
```

`typos` comes from the `typos-cli` crate and `markdownlint-cli2` from npm. On Linux the tests
start their own D-Bus and GNOME Keyring. They need `dbus-daemon` and `gnome-keyring-daemon`
installed. On Windows they run their fake steamcmd with Git's `sh`, through the `fake-steamcmd`
feature that test builds alone turn on. `.github/pack.sh` refuses a binary built with it.

`cargo test` covers the steamship crate alone. When `docs/`, `docs-site/` or a command's help
changes, also run `cargo test --locked -p steamship-docs-site` and
`cargo run --locked -p steamship-docs-site`. The build refuses a dead link or anchor and code too
wide for the page.

## What CI runs

CI runs everything else on the pull request:

- The tests on Linux, macOS and Windows
- Line coverage of at least 97 %
- cargo-mutants on the changed lines
- A fuzzing pass over each parser
- cargo-deny
- actionlint and zizmor on the workflows
- The documentation site against WCAG 2.2 AA
- The Linux packages, built and installed on Debian and Fedora

Leave those to CI rather than running them locally. Links to other sites are checked weekly.

## Commits

A commit message is one capitalised sentence in the imperative, with no prefix. It says what the
change does, such as "Count the drift an achievements check finds". `main` takes signed commits
only. Pull requests are squash-merged.
