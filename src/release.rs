//! An app's info as steamcmd's `app_info_print` prints it, and whether it shows the app released.
//!
//! Valve sets a build live on an unreleased app's default branch for the publisher key alone,
//! but once players can buy the app it asks for a Steam Mobile confirmation, and the default
//! branch is the one they play. So steamship sets default live only for an app whose
//! `ReleaseState` Steam gives as unreleased, and takes anything else, or nothing, for released.

use std::fmt;
use std::ops::Range;

use crate::vdf::{self, Block};

/// Why an app's info could not be read out of steamcmd's console.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Unread {
    /// steamcmd printed none, or none that can be read. Another run may print it whole.
    Printed(String),
    /// Steam showed the build account nothing for the app, which another run would show again.
    Hidden(String),
}

impl Unread {
    /// What happened, and running the command again where that may help.
    #[must_use]
    pub fn and_again(&self) -> String {
        match self {
            Self::Printed(why) => format!("{why}. Run the command again"),
            Self::Hidden(why) => why.clone(),
        }
    }
}

impl fmt::Display for Unread {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Printed(why) | Self::Hidden(why) => formatter.write_str(why),
        }
    }
}

/// The `ReleaseState` values of an app nobody can buy yet: `unavailable` for Tidal Mill Inn,
/// Brewhouse and Falcon Ridge Overseer before their store pages, `prerelease` for apps whose
/// page says coming soon. Spacewar and Stardew Valley show `released`.
const UNRELEASED: [&str; 2] = ["unavailable", "prerelease"];

/// All of `app`'s info in steamcmd's `console`, where `app_info_print` printed it, Steam's own
/// bookkeeping and release state among it.
///
/// # Errors
///
/// When steamcmd printed none, what it printed cannot be read, as when its closing brace is
/// missing, or Steam showed the build account nothing, as for an app it cannot see. Each says
/// what happened, and the command that asked adds what to do next.
pub fn info_from_console(console: &str, app: u32) -> Result<Block, Unread> {
    let Some(cut) = info_at(console, app).and_then(|at| console.get(at)) else {
        return Err(Unread::Printed(format!(
            "steamcmd printed no app info for app {app}"
        )));
    };
    // A carriage return before a line's end goes, so a value over several lines reads alike on
    // every system.
    let printed: String = cut.lines().flat_map(|line| [line, "\n"]).collect();
    let document = vdf::parse(&printed).map_err(|error| {
        Unread::Printed(format!(
            "steamcmd printed app info for app {app} that cannot be read: {error}"
        ))
    })?;
    let shown = document.block(&app.to_string()).unwrap_or(&document);
    if shown.pairs.is_empty() {
        return Err(Unread::Hidden(format!(
            "Steam shows the build account no app info for app {app}: it cannot see the app, or \
             there is no such app. Check the app ID, and that the account is in a permission \
             group that holds the app"
        )));
    }
    Ok(shown.clone())
}

/// Where in steamcmd's `console` the info `app_info_print` printed for `app` lies, so what the
/// other commands of the same run printed is read apart from it.
///
/// It runs from the line that names the app to its closing brace. When that never comes, it ends
/// before the first line that is neither indented nor its opening brace, so that what steamcmd
/// printed next stays out of it. None when steamcmd printed no info.
#[must_use]
pub fn info_at(console: &str, app: u32) -> Option<Range<usize>> {
    let named = format!("\"{app}\"");
    let mut at = 0_usize;
    // Where the info starts, and where its lines stop being a block's.
    let mut open: Option<(usize, Option<usize>)> = None;
    for line in console.split_inclusive('\n') {
        let end = at.saturating_add(line.len());
        let trimmed = line.trim_end();
        match open {
            // Its closing brace is the only one at the start of a line: those within are indented.
            Some((start, _)) if trimmed == "}" => return Some(start..end),
            Some((start, None)) if !line.starts_with('\t') && trimmed != "{" => {
                open = Some((start, Some(at)));
            }
            None if trimmed == named => open = Some((at, None)),
            _ => {}
        }
        at = end;
    }
    open.map(|(start, stop)| start..stop.unwrap_or(at))
}

/// The `ReleaseState` in `info`, an app's info as steamcmd printed it, if it holds one.
#[must_use]
pub fn state(info: &Block) -> Option<&str> {
    info.block("common")?.text("ReleaseState")
}

/// Whether `state`, a `ReleaseState`, is that of an app nobody can buy yet.
#[must_use]
pub fn unreleased(state: &str) -> bool {
    UNRELEASED
        .iter()
        .any(|unreleased| state.eq_ignore_ascii_case(unreleased))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Tidal Mill Inn's and Stardew Valley's app info, from steamcmd 1791918946 on 2026-10-18,
    /// each common block cut short.
    const CONSOLE: &str = "Waiting for user info...OK\n\
        AppID : 1000050, change number : 39949502/39949502, last change : Sun Oct 18 00:07:38 2026 \n\
        \"1000050\"\n\
        {\n\
        \t\"common\"\n\
        \t{\n\
        \t\t\"name\"\t\t\"Tidal Mill Inn\"\n\
        \t\t\"section_type\"\t\t\"ownersonly\"\n\
        \t\t\"type\"\t\t\"Game\"\n\
        \t\t\"ReleaseState\"\t\t\"unavailable\"\n\
        \t}\n\
        }\n\
        AppID : 413150, change number : 39132990/39132990, last change : Thu Oct 15 16:52:32 2026 \n\
        \"413150\"\n\
        {\n\
        \t\"common\"\n\
        \t{\n\
        \t\t\"name\"\t\t\"Stardew Valley\"\n\
        \t\t\"type\"\t\t\"Game\"\n\
        \t\t\"ReleaseState\"\t\t\"released\"\n\
        \t}\n\
        }\n\
        Unloading Steam API...OK\n";

    #[test]
    fn the_release_state_is_read_for_the_app_asked_about() {
        let inn = info_from_console(CONSOLE, 1_000_050).unwrap();
        assert_eq!(state(&inn), Some("unavailable"));
        let stardew = info_from_console(CONSOLE, 413_150).unwrap();
        assert_eq!(state(&stardew), Some("released"));
    }

    #[test]
    fn a_run_that_printed_the_info_then_built_is_cut_after_the_info() {
        let built = "Building depot 1000011...\nSuccessfully finished AppID 1000010 build \
                     (BuildID 4242).\n";
        let console = format!("{CONSOLE}{built}");
        let mut rest = console.clone();
        let info: String = rest.drain(info_at(&console, 1_000_050).unwrap()).collect();
        assert_eq!(
            info,
            "\"1000050\"\n{\n\t\"common\"\n\t{\n\t\t\"name\"\t\t\"Tidal Mill Inn\"\n\
             \t\t\"section_type\"\t\t\"ownersonly\"\n\t\t\"type\"\t\t\"Game\"\n\
             \t\t\"ReleaseState\"\t\t\"unavailable\"\n\t}\n}\n"
        );
        assert_eq!(
            state(&info_from_console(&info, 1_000_050).unwrap()),
            Some("unavailable")
        );
        assert_eq!(info_at(built, 1_000_050), None);
    }

    #[test]
    fn info_whose_closing_brace_is_lost_ends_before_what_steamcmd_printed_next() {
        let lost = CONSOLE.replacen("\t}\n}\nUnloading", "\t}\nUnloading", 1);
        assert_eq!(
            info_at(&lost, 413_150).and_then(|at| lost.get(at)),
            Some(
                "\"413150\"\n{\n\t\"common\"\n\t{\n\t\t\"name\"\t\t\"Stardew Valley\"\n\
                 \t\t\"type\"\t\t\"Game\"\n\t\t\"ReleaseState\"\t\t\"released\"\n\t}\n"
            )
        );
        assert_eq!(
            info_from_console(&lost, 413_150).unwrap_err().and_again(),
            "steamcmd printed app info for app 413150 that cannot be read: line 9, column 1: a \
             block is missing its closing }. Run the command again"
        );
    }

    #[test]
    fn info_with_no_common_block_or_no_release_state_has_none() {
        for text in [
            "\"config\" { }",
            "\"common\" \"x\"",
            "\"common\" { \"name\" \"x\" }",
        ] {
            let info = vdf::parse(text).unwrap();
            assert_eq!(state(&info), None, "{text}");
        }
    }

    #[test]
    fn only_unavailable_and_prerelease_are_unreleased() {
        assert!(unreleased("unavailable"));
        assert!(unreleased("prerelease"));
        assert!(unreleased("Prerelease"));
        for released in ["released", "unavailable soon", ""] {
            assert!(!unreleased(released), "{released}");
        }
    }
}
