//! Whether Steam shows an app released, as steamcmd's `app_info_print` gives it.
//!
//! Valve sets a build live on an unreleased app's default branch for the publisher key alone,
//! but once players can buy the app it asks for a Steam Mobile confirmation, and the default
//! branch is the one they play. So steamship sets default live only for an app whose
//! `ReleaseState` Steam gives as unreleased, and takes anything else, or nothing, for released.

use crate::vdf::Block;

/// The `ReleaseState` values of an app nobody can buy yet: `unavailable` for Tidal Mill Inn,
/// Brewhouse and Falcon Ridge Overseer before their store pages, `prerelease` for apps whose
/// page says coming soon. Spacewar and Stardew Valley show `released`.
const UNRELEASED: [&str; 2] = ["unavailable", "prerelease"];

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
    use crate::{settings, vdf};

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
        let inn = settings::info_from_console(CONSOLE, 1_000_050).unwrap();
        assert_eq!(state(&inn), Some("unavailable"));
        let stardew = settings::info_from_console(CONSOLE, 413_150).unwrap();
        assert_eq!(state(&stardew), Some("released"));
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
