//! Builds steamship's documentation site into `target/site`: the pages in `docs/`, a command
//! reference and front page rendered from the command line itself, a markdown copy of every
//! page, and `llms.txt`. The build refuses a page nothing links to, a link or anchor that goes
//! nowhere, and code too wide to read, so what is published is whole.
//!
//! A generator of its own rather than a site framework: the site is small, the repository is
//! Rust, and the command reference comes from the same clap definitions `--help` prints.

pub mod check;
pub mod commands;
pub mod home;
pub mod markdown;

use std::fmt::{self, Write as _};
use std::fs;
use std::io::{self, Write as _};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::CommandFactory as _;
use markdown::escaped;
use steamship::cli::Cli;
use steamship::show::SHIP;

/// Where the site is published, for the canonical links and llms.txt.
const SITE_URL: &str = "https://aureliolo.github.io/steamship";

/// How a page is made.
#[derive(Debug, Clone, Copy)]
enum Source {
    /// Written in `docs/`, as markdown.
    File(&'static str),
    /// The front page, made here.
    Home,
    /// The command reference, made from the command line.
    Commands,
}

#[derive(Debug, Clone, Copy)]
struct Page {
    source: Source,
    /// Where the page lands, which is also its link; its markdown copy has the same name.
    name: &'static str,
    /// In the browser's tab, and in the navigation unless `nav` says otherwise.
    title: &'static str,
    nav: &'static str,
    /// One line, for the page's description and for llms.txt.
    summary: &'static str,
}

const PAGES: [Page; 5] = [
    Page {
        source: Source::Home,
        name: "index",
        title: "steamship",
        // The wordmark beside it already says steamship.
        nav: "Overview",
        summary: "What steamship does, what it guarantees, and how to install it.",
    },
    Page {
        source: Source::File("install.md"),
        name: "install",
        title: "Install",
        nav: "Install",
        summary: "Installing, verifying and upgrading steamship, and the steamcmd it pins.",
    },
    Page {
        source: Source::File("uploading.md"),
        name: "uploading",
        title: "Uploading",
        nav: "Uploading",
        summary: "Logging in, checking scripts, uploading builds and Workshop items, and branches.",
    },
    Page {
        source: Source::File("ci.md"),
        name: "ci",
        title: "Uploading from CI",
        nav: "CI",
        summary: "Uploads from GitHub Actions or any other CI, and keeping steamship current.",
    },
    Page {
        source: Source::Commands,
        name: "commands",
        title: "Commands",
        nav: "Commands",
        summary: "Every command and option, the exit codes and the environment, from the CLI.",
    },
];

fn main() -> ExitCode {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    match build(&root.join("docs"), &root.join("target").join("site")) {
        Ok(published) => {
            let said = writeln!(io::stdout(), "built {}", published.display());
            if said.is_ok() {
                ExitCode::SUCCESS
            } else {
                ExitCode::FAILURE
            }
        }
        Err(why) => {
            let _said: io::Result<()> = writeln!(io::stderr(), "the site was not built: {why}");
            ExitCode::FAILURE
        }
    }
}

/// Renders every page from `docs` into `out`, which is emptied first, and checks the result.
fn build(docs: &Path, out: &Path) -> Result<PathBuf, String> {
    refuse_orphans(docs)?;
    let template = read(&docs.join("theme").join("page.html"))?;
    let version = Cli::command().get_version().unwrap_or_default().to_owned();
    if out.exists() {
        fs::remove_dir_all(out).map_err(|error| format!("{}: {error}", out.display()))?;
    }
    fs::create_dir_all(out).map_err(|error| format!("{}: {error}", out.display()))?;

    let mut texts = Vec::with_capacity(PAGES.len());
    for page in PAGES {
        let (body, text) = match page.source {
            Source::File(file) => {
                let written = read(&docs.join(file))?.replace("{{version}}", &version);
                (markdown::to_html(&written), written)
            }
            Source::Home => (home::html(&version), home::text(&version)),
            Source::Commands => (commands::html(), commands::text()),
        };
        write(
            &out.join(format!("{}.html", page.name)),
            &fill(&template, page, &body, &version),
        )?;
        write(&out.join(format!("{}.md", page.name)), &text)?;
        texts.push(text);
    }
    let _bytes: u64 = fs::copy(docs.join("theme").join("site.css"), out.join("site.css"))
        .map_err(|error| format!("site.css: {error}"))?;
    write(&out.join("llms.txt"), &llms())?;
    write(&out.join("llms-full.txt"), &texts.join("\n---\n\n"))?;
    // Pages would otherwise run the site through Jekyll, which drops some files.
    write(&out.join(".nojekyll"), "")?;
    check::site(out)?;
    Ok(out.to_path_buf())
}

/// A page's HTML, the template filled in around `body`.
fn fill(template: &str, page: Page, body: &str, version: &str) -> String {
    let title = if matches!(page.source, Source::Home) {
        page.title.to_owned()
    } else {
        format!("{} \u{b7} steamship", page.title)
    };
    let mut links = String::new();
    for other in PAGES {
        let current = if other.name == page.name {
            r#" class="on" aria-current="page""#
        } else {
            ""
        };
        let _written: fmt::Result = write!(
            links,
            r#"<a href="{}.html"{current}>{}</a>"#,
            other.name, other.nav
        );
    }
    let class = if matches!(page.source, Source::Home) {
        r#" class="home""#
    } else {
        ""
    };
    template
        .replace("{{title}}", &escaped(&title))
        .replace("{{description}}", &escaped(page.summary))
        .replace("{{canonical}}", &format!("{SITE_URL}/{}.html", page.name))
        .replace("{{bodyclass}}", class)
        .replace("{{toplinks}}", &links)
        .replace("{{ship}}", &SHIP.map(escaped).join("\n"))
        .replace("{{version}}", version)
        .replace("{{content}}", body)
}

/// The index an agent reads first: every page, as the markdown copy.
fn llms() -> String {
    let mut index = String::from(
        "# steamship\n\n> Uploads game builds to Steam with Valve's steamcmd, set up pinned and \
         verified, with a clear result and no passwords.\n\n## Pages\n\n",
    );
    for page in PAGES {
        let _written: fmt::Result = writeln!(
            index,
            "- [{}]({SITE_URL}/{}.md): {}",
            page.title, page.name, page.summary
        );
    }
    index
}

/// Refuses a page in `docs` that no entry here publishes, which would otherwise sit there with
/// nothing linking to it.
fn refuse_orphans(docs: &Path) -> Result<(), String> {
    let entries = fs::read_dir(docs).map_err(|error| format!("{}: {error}", docs.display()))?;
    for entry in entries {
        let name = entry
            .map_err(|error| format!("{}: {error}", docs.display()))?
            .file_name();
        let name = name.to_string_lossy();
        let published = PAGES
            .iter()
            .any(|page| matches!(page.source, Source::File(file) if file == name));
        if name.ends_with(".md") && name != "README.md" && !published {
            return Err(format!(
                "docs/{name} is published by no page; add it to PAGES in docs-site/src/main.rs"
            ));
        }
    }
    Ok(())
}

fn read(path: &Path) -> Result<String, String> {
    fs::read_to_string(path).map_err(|error| format!("{}: {error}", path.display()))
}

fn write(path: &Path, contents: &str) -> Result<(), String> {
    fs::write(path, contents).map_err(|error| format!("{}: {error}", path.display()))
}

#[cfg(test)]
mod tests {
    use std::{env, process};

    use super::*;

    #[test]
    fn a_page_is_filled_into_the_template_with_its_own_link_marked() {
        let template = "{{title}}|{{description}}|{{canonical}}|{{bodyclass}}|{{toplinks}}|{{ship}}|{{version}}|{{content}}";
        let install = PAGES.get(1).copied().unwrap_or(PAGES[0]);
        let filled = fill(template, install, "<p>body</p>", "1.2.3");
        assert!(filled.starts_with("Install \u{b7} steamship|"), "{filled}");
        assert!(
            filled.contains(&format!("{SITE_URL}/install.html")),
            "{filled}"
        );
        assert!(
            filled.contains(r#"<a href="install.html" class="on" aria-current="page">Install</a>"#),
            "{filled}"
        );
        assert!(
            filled.contains(r#"<a href="index.html">Overview</a>"#),
            "{filled}"
        );
        assert!(
            filled.ends_with(&format!(
                "|{}|1.2.3|<p>body</p>",
                SHIP.map(escaped).join("\n")
            )),
            "the ship the terminal draws, then the version: {filled}"
        );
        let home = fill(template, PAGES[0], "", "1.2.3");
        assert!(home.starts_with("steamship|"), "{home}");
        assert!(home.contains(r#"| class="home"|"#), "{home}");
    }

    #[test]
    fn llms_txt_lists_every_page_by_its_markdown_copy() {
        let index = llms();
        for page in PAGES {
            assert!(
                index.contains(&format!("({SITE_URL}/{}.md): {}", page.name, page.summary)),
                "{index}"
            );
        }
    }

    #[test]
    fn a_page_in_docs_that_nothing_publishes_is_refused() {
        let docs = env::temp_dir().join(format!("steamship-docs-{}", process::id()));
        fs::create_dir_all(&docs).unwrap_or_default();
        for name in ["install.md", "README.md", "notes.txt"] {
            fs::write(docs.join(name), "").unwrap_or_default();
        }
        assert_eq!(refuse_orphans(&docs), Ok(()));
        fs::write(docs.join("forgotten.md"), "").unwrap_or_default();
        let refused = refuse_orphans(&docs);
        fs::remove_dir_all(&docs).unwrap_or_default();
        assert!(
            matches!(&refused, Err(why) if why.starts_with("docs/forgotten.md is published by no page")),
            "{refused:?}"
        );
    }
}
