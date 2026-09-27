# docs

The source of <https://aureliolo.github.io/steamship>. `cargo run -p steamship-docs-site` renders
it into `target/site`, and each release publishes that from its tag through
`.github/workflows/docs.yml`, so the site always describes the latest release.

| File           | Page                            |
| -------------- | ------------------------------- |
| `install.md`   | `install.html`                  |
| `uploading.md` | `uploading.html`                |
| `ci.md`        | `ci.html`                       |
| `theme/`       | the template and the stylesheet |

The front page and the command reference have no file here: `docs-site/` makes them, the
reference from the command line's own definitions in `src/cli.rs`, so it says what
`steamship --help` says. Every page is also published as markdown, beside `llms.txt` and
`llms-full.txt`.

A page added here has to be listed in `PAGES` in `docs-site/src/main.rs`; the build refuses one
that is not, and any link or anchor that goes nowhere.
