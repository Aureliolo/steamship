# Working on steamship

How changes are made here, for people and coding agents alike. CI enforces most of it; the rest
is what review asks for.

These instructions live in this file alone. Claude Code reads AGENTS.md only where no CLAUDE.md
sits beside it, so CI fails on a CLAUDE.md anywhere in the tree.

## Writing

Everything a person reads counts: messages, help text, docs, comments, commit messages.

- British English: colour, behaviour, licence as the noun. `typos` checks it with the en-gb
  locale.
- No em or en dashes anywhere. Use a colon, semicolon, comma, full stop or brackets. The
  `dashes` job fails on either.
- No marketing filler. The words in `.github/hype-words.txt` fail the `hype` job. Say what
  steamship does, and give a number only when it was measured.
- A message says what happened and what to do next, naming the file, app or branch it is about.
- Never print, log or write in plain text a password, a Steam Guard code, a login token or a Web
  API key, nor anything that holds one.

## Code

- Comments say why, never what. One that restates the line below it goes. A comment speaks to
  the next reader: no "changed to", "now uses", "previously" or issue numbers. The history
  records what changed.
- Tests are named as sentences that say what holds, and assert on what a person sees.
- `Cargo.toml` turns on clippy's pedantic, nursery, cargo and restriction groups. The few lints
  allowed are listed there with the reason. Fix a lint rather than silence it; an `#[expect]`
  says why in its `reason`.
- `unsafe` stays in `src/unix.rs` and `src/windows.rs`, each block with a `SAFETY:` comment.

## Layout

- `src/` is one crate, library and binary. The command line is the supported interface; the
  library's modules are public only for the tests and fuzz targets.
- `docs/` is the site's source. `docs-site/` renders it, with every command's help, into
  `target/site`.
- `fuzz/` holds a cargo-fuzz target per parser, a workspace member so it shares the lint table.
- `pins/` holds steamcmd's manifests, compiled into steamship. Renovate raises them;
  `cargo run --example pins` rewrites them by hand.
- `.github/packages.sh` writes `Formula/` and `bucket/` at each release; leave them to it.
- Releases start from the Actions tab, as `.github/release-process.md` describes.

## Steam

- Credentials reach steamcmd through its environment or the system's credential store, never
  its arguments.
- steamship never sets the default branch live and never describes it: Valve keeps both to the
  Steamworks site, and `check` refuses a script that tries.
- In scope: getting a game's builds, content and configuration into Steamworks, and checking
  them there. Out of scope: financial reports and live operations.

## Supply chain

Every action is pinned by commit SHA with its version beside it, and every tool a workflow
downloads by version and SHA-256. Renovate raises them; nothing is fetched at "latest".

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
start their own D-Bus and GNOME Keyring, so `dbus-daemon` and `gnome-keyring-daemon` must be
installed.

`cargo test` covers the steamship crate alone. When `docs/`, `docs-site/` or a command's help
changes, also run `cargo test --locked -p steamship-docs-site` and
`cargo run --locked -p steamship-docs-site`; the build refuses a dead link or anchor and code
too wide for the page.

CI runs everything else on the pull request: the tests on Linux, macOS and Windows, line
coverage of at least 97 %, cargo-mutants on the changed lines, a fuzzing pass over each parser,
cargo-deny, actionlint and zizmor on the workflows, the documentation site against WCAG 2.2 AA,
and the Linux packages built and installed on Debian and Fedora. Leave those to CI rather than
running them locally. Links to other sites are checked weekly.

A commit message is one capitalised sentence in the imperative saying what the change does, such
as "Count the drift an achievements check finds", with no prefix. `main` takes signed commits
only, and pull requests are squash-merged.
