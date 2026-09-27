//! The front page: what steamship is for, the three things it guarantees, and the way in.
//!
//! It is looked at rather than read, so it is made here rather than written as markdown, and its
//! markdown copy says the same in prose.

use std::fmt::{self, Write as _};

use crate::markdown::{highlighted, inline};

struct Card {
    kicker: &'static str,
    title: &'static str,
    body: &'static str,
    link: &'static str,
    link_text: &'static str,
}

const CARDS: [Card; 3] = [
    Card {
        kicker: "steamcmd",
        title: "Pinned and verified",
        body: "Fetched from Valve's signed manifest at a version fixed in each release, every \
               package checked by SHA-256, kept from updating itself, and checked again after \
               every run.",
        link: "install.html#steamcmd",
        link_text: "The steamcmd it pins",
    },
    Card {
        kicker: "upload",
        title: "A clear result",
        body: "Read from the exit code and Valve's build log, never from console output: the \
               BuildID, or the reason it failed. Scripts that would upload the wrong thing are \
               refused before anything is sent.",
        link: "uploading.html",
        link_text: "Uploading",
    },
    Card {
        kicker: "login",
        title: "No passwords kept",
        body: "You log in once. What you type goes straight to steamcmd, never logged or saved, \
               and nothing steamship prints or writes contains your login token.",
        link: "ci.html",
        link_text: "Uploading from CI",
    },
];

/// Where a way's command, note or link names the version, filled in with the one being built.
const VERSION: &str = "VERSION";

/// Where a way names the release the version is, as its archives' address begins.
const RELEASE: &str = "RELEASE";

/// The release's address, with the version to fill in.
const RELEASE_URL: &str = "https://github.com/Aureliolo/steamship/releases/download/vVERSION";

struct Way {
    id: &'static str,
    name: &'static str,
    command: &'static str,
    note: &'static str,
    /// The archive itself, for a way that is downloading one.
    link: Option<&'static str>,
}

impl Way {
    /// `text` with the release and the version it names filled in.
    fn filled(text: &str, version: &str) -> String {
        text.replace(RELEASE, RELEASE_URL).replace(VERSION, version)
    }
}

const WAYS: [Way; 7] = [
    Way {
        id: "homebrew",
        name: "Homebrew",
        command: "brew tap aureliolo/steamship https://github.com/Aureliolo/steamship\n\
                  brew install aureliolo/steamship/steamship",
        note: "macOS and Linux. Homebrew installs the release's archive only if it matches the \
               SHA-256 the release is signed over.",
        link: None,
    },
    Way {
        id: "scoop",
        name: "Scoop",
        command: "scoop bucket add aureliolo https://github.com/Aureliolo/steamship\n\
                  scoop install aureliolo/steamship",
        note: "Windows, with this repository as a bucket. Scoop installs the release's archive \
               only if it matches the SHA-256 the release is signed over.",
        link: None,
    },
    Way {
        id: "winget",
        name: "winget",
        command: "winget install Aureliolo.steamship",
        note: "Windows. winget installs the release's archive only if it matches the SHA-256 in \
               its manifest.",
        link: None,
    },
    Way {
        id: "cargo",
        name: "Cargo",
        command: "# the release's archive, in seconds\n\
                  cargo binstall steamship\n\
                  # or built from source\n\
                  cargo install --locked steamship",
        note: "Anywhere Rust runs.",
        link: None,
    },
    Way {
        id: "linux",
        name: "Linux archive",
        command: "curl -fLO RELEASE/steamship-VERSION-x86_64-linux-musl.tar.gz\n\
                  wget RELEASE/steamship-VERSION-x86_64-linux-musl.tar.gz\n\
                  gh release download vVERSION --repo Aureliolo/steamship --pattern '*linux-musl*'",
        note: "Any x86-64 Linux, whatever its glibc: steamship vVERSION, with curl, wget or the \
               GitHub command line. Check it as Install shows, then put `steamship` on your \
               `PATH`.",
        link: Some("RELEASE/steamship-VERSION-x86_64-linux-musl.tar.gz"),
    },
    Way {
        id: "macos",
        name: "macOS archive",
        command: "curl -fLO RELEASE/steamship-VERSION-aarch64-apple-darwin.tar.gz\n\
                  gh release download vVERSION --repo Aureliolo/steamship --pattern '*aarch64-apple*'",
        note: "Apple silicon: steamship vVERSION, with curl or the GitHub command line; on an \
               Intel Mac, `aarch64` becomes `x86_64`. Check it as Install shows, then put \
               `steamship` on your `PATH`.",
        link: Some("RELEASE/steamship-VERSION-aarch64-apple-darwin.tar.gz"),
    },
    Way {
        id: "windows",
        name: "Windows archive",
        command: "curl.exe -fLO RELEASE/steamship-VERSION-x86_64-pc-windows-msvc.zip\n\
                  gh release download vVERSION --repo Aureliolo/steamship --pattern '*windows*'",
        note: "Windows x86-64: steamship vVERSION, with the curl Windows ships or the GitHub \
               command line. Check it as Install shows, then put `steamship.exe` on your `PATH`.",
        link: Some("RELEASE/steamship-VERSION-x86_64-pc-windows-msvc.zip"),
    },
];

const FIRST_UPLOAD: &str = "\
# once
steamship login
# is the login still good?
steamship status
# offline, no login
steamship check steam/app_build.vdf
# a rehearsal: builds everything and sends nothing to Steam
steamship upload steam/app_build.vdf --version 1.4.0 --preview
# the real thing
steamship upload steam/app_build.vdf --version 1.4.0";

const SCRIPTS: &str = "`steam/app_build.vdf` and its depot scripts are Valve's own format, the \
                       same ones the Steamworks SDK's ContentBuilder uses. If you already upload \
                       with steamcmd, you already have them.";

const SUB: &str = "steamship runs Valve's own steamcmd with your own build scripts, and adds what \
                   steamcmd leaves out. It works with any engine: anything that ends in a folder \
                   of files per platform can be shipped.";

const SMALLPRINT: &str = "MIT or Apache-2.0, at your option. Not affiliated with or endorsed by \
                          Valve; Steam and Steamworks are trademarks of Valve Corporation.";

/// The front page, as the site's HTML.
#[must_use]
pub fn html(version: &str) -> String {
    let mut page = format!(
        "<section class=\"hero\">\n<h1 id=\"steamship\">Upload to Steam, and know it \
         worked.</h1>\n<p class=\"sub\">{SUB}</p>\n</section>\n<div class=\"cards\">"
    );
    for card in CARDS {
        let _card: fmt::Result = write!(
            page,
            "<div class=\"card\"><span class=\"kicker\">{}</span><h3>{}</h3><p>{}</p>\
             <a class=\"go\" href=\"{}\">{}</a></div>",
            card.kicker,
            inline(card.title),
            inline(card.body),
            card.link,
            inline(card.link_text)
        );
    }
    let _install: fmt::Result = write!(
        page,
        "</div>\n<section class=\"install\">\n<h2 class=\"step\" id=\"install\">Install</h2>\n\
         {}\n<h2 class=\"step\" id=\"first-upload\">First upload</h2>\n<figure class=\"code\">\
         <pre><code>{}</code></pre></figure>\n<p class=\"note\">{}</p>\n</section>\n\
         <p class=\"smallprint\">{SMALLPRINT}</p>\n",
        picker(version),
        highlighted(FIRST_UPLOAD),
        inline(SCRIPTS)
    );
    page
}

/// The way chosen when the reader's system is not known: Cargo installs on every one.
const ANYWHERE: &str = "cargo";

/// Chooses the way for the reader's system, where the browser says what it is. Without it, or on
/// a system it cannot place, the page stays on Cargo.
const BY_SYSTEM: &str = "<script>(function(){var n=navigator,s=((n.userAgentData&&\
                         n.userAgentData.platform)||n.platform||n.userAgent||'').toLowerCase(),\
                         w=/win/.test(s)?'winget':/mac|linux|x11/.test(s)?'homebrew':'',\
                         e=w&&document.getElementById('pick-'+w);if(e){e.checked=true;}})();\
                         </script>";

/// The ways to install, one chip each, and the command for the one chosen: radio inputs and
/// sibling selectors, so choosing needs no script, and a few lines of it only choose first for
/// the reader's system. The rules tying each input to its chip and its panel are made here with
/// them, so a way added above cannot outrun the stylesheet.
fn picker(version: &str) -> String {
    let mut inputs = String::new();
    let mut chips = String::new();
    let mut panels = String::new();
    let mut rules = String::new();
    for way in WAYS {
        let checked = if way.id == ANYWHERE { " checked" } else { "" };
        let id = way.id;
        let _input: fmt::Result = writeln!(
            inputs,
            "<input type=\"radio\" name=\"way\" id=\"pick-{id}\"{checked} />"
        );
        let _chip: fmt::Result = write!(chips, "<label for=\"pick-{id}\">{}</label>", way.name);
        let link = way.link.map_or_else(String::new, |link| {
            format!(
                " <a href=\"{}\">Download it directly.</a>",
                Way::filled(link, version)
            )
        });
        let _panel: fmt::Result = write!(
            panels,
            "<div class=\"panel panel-{id}\"><pre><code>{}</code></pre><p>{}{link}</p></div>",
            highlighted(&Way::filled(way.command, version)),
            inline(&Way::filled(way.note, version))
        );
        let _rule: fmt::Result = write!(
            rules,
            "#pick-{id}:checked ~ .panels .panel-{id}{{display:block}}\
             #pick-{id}:checked ~ .chips label[for=\"pick-{id}\"]{{color:var(--signal-ink);\
             background:var(--signal);border-color:var(--signal)}}\
             #pick-{id}:focus-visible ~ .chips label[for=\"pick-{id}\"]{{outline:2px solid \
             var(--signal);outline-offset:2px}}"
        );
    }
    format!(
        "<div class=\"picker\">\n<style>{rules}</style>\n{inputs}<div class=\"chips\">{chips}\
         </div>\n<div class=\"panels\">{panels}</div>\n</div>\n{BY_SYSTEM}"
    )
}

/// The front page as markdown.
#[must_use]
pub fn text(version: &str) -> String {
    let mut page =
        format!("# steamship\n\nUpload to Steam, and know it worked. {SUB}\n\n## What it adds\n\n");
    for card in CARDS {
        let _card: fmt::Result = write!(
            page,
            "**{}.** {} See [{}]({}).\n\n",
            card.title,
            card.body,
            card.link_text,
            card.link.replace(".html", ".md")
        );
    }
    page.push_str("## Install\n");
    for way in WAYS {
        let link = way.link.map_or_else(String::new, |link| {
            format!(" [Download it directly]({}).", Way::filled(link, version))
        });
        let _way: fmt::Result = write!(
            page,
            "\n{}: {}{link}\n\n```sh\n{}\n```\n",
            way.name,
            Way::filled(way.note, version),
            Way::filled(way.command, version)
        );
    }
    let _rest: fmt::Result = write!(
        page,
        "\n## First upload\n\n```sh\n{FIRST_UPLOAD}\n```\n\n{SCRIPTS}\n\n{SMALLPRINT}\n"
    );
    page
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_way_to_install_has_a_chip_a_panel_and_its_rules() {
        let picker = picker("9.8.7");
        for way in WAYS {
            let id = way.id;
            assert!(picker.contains(&format!("id=\"pick-{id}\"")), "{id}");
            assert!(
                picker.contains(&format!("<label for=\"pick-{id}\">{}</label>", way.name)),
                "{id}"
            );
            assert!(
                picker.contains(&format!("<div class=\"panel panel-{id}\">")),
                "{id}"
            );
            assert!(
                picker.contains(&format!(
                    "#pick-{id}:checked ~ .panels .panel-{id}{{display:block}}"
                )),
                "{id}"
            );
        }
        assert_eq!(
            picker.matches(" checked />").count(),
            1,
            "one way is chosen to start with"
        );
        assert!(
            picker.contains("id=\"pick-cargo\" checked />"),
            "Cargo, which runs anywhere"
        );
        assert!(
            picker.ends_with("</script>")
                && ["winget", "homebrew"]
                    .iter()
                    .all(|way| picker.contains(&format!("'{way}'"))),
            "the reader's system chooses first: {picker}"
        );
        let linux = "https://github.com/Aureliolo/steamship/releases/download/v9.8.7/\
                     steamship-9.8.7-x86_64-linux-musl.tar.gz";
        assert!(
            picker.contains(&format!(
                "curl -fLO {linux}\nwget {linux}\ngh release download v9.8.7 "
            )) && picker.contains(&format!("<a href=\"{linux}\">Download it directly.</a>")),
            "each archive by its whole address, at the version it was built at: {picker}"
        );
        assert!(
            !picker.contains(VERSION) && !picker.contains(RELEASE),
            "{picker}"
        );
    }

    #[test]
    fn the_front_page_links_onward_and_its_copy_says_the_same() {
        let page = html("9.8.7");
        for card in CARDS {
            assert!(
                page.contains(&format!("href=\"{}\"", card.link)),
                "{}",
                card.link
            );
        }
        assert!(page.contains("<code>steam/app_build.vdf</code>"), "{page}");
        let text = text("9.8.7");
        assert!(text.starts_with("# steamship\n\n"), "{text}");
        assert!(text.contains("See [Uploading](uploading.md)."), "{text}");
        assert!(
            text.contains("winget install Aureliolo.steamship"),
            "{text}"
        );
    }
}
