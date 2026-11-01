# docs

The source of <https://aureliolo.github.io/steamship>. `cargo run -p steamship-docs-site` renders
it into `target/site`. Each release publishes it from its tag through
`.github/workflows/docs.yml`. The site always describes the latest release.

| File           | Page                            |
| -------------- | ------------------------------- |
| `install.md`   | `install.html`                  |
| `uploading.md` | `uploading.html`                |
| `games.md`     | `games.html`                    |
| `ci.md`        | `ci.html`                       |
| `theme/`       | the template, stylesheet, icon  |

`docs-site/` makes the front page and the command reference, which have no file here. It makes
the reference from the command definitions in `src/cli.rs`. The reference then says what
`steamship --help` says. Every page is also published as markdown, beside `llms.txt` and
`llms-full.txt`.

A page added here has to be listed in `PAGES` in `docs-site/src/main.rs`. The build refuses a page
that is not, and any link or anchor that goes nowhere.
