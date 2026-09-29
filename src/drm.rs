//! Steam DRM, which Valve's servers add to a Windows executable through steamcmd's `drm_wrap`.

use crate::upload;

/// How the wrap is asked for: Valve's flags for the tool, `drmtoolp`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// The default wrap.
    Default,
    /// Valve's compatibility mode, for an executable the default wrap breaks.
    Compatibility,
}

impl Mode {
    /// The mode asked for: compatibility mode or the default.
    #[must_use]
    pub const fn chosen(compatibility: bool) -> Self {
        if compatibility {
            Self::Compatibility
        } else {
            Self::Default
        }
    }

    /// The flags steamcmd passes Valve's tool.
    #[must_use]
    pub const fn flags(self) -> &'static str {
        match self {
            Self::Default => "0",
            Self::Compatibility => "6",
        }
    }
}

/// What came of a wrap.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// The wrapped executable was written.
    Wrapped,
    /// Steam refused the saved login, for this reason.
    NotLoggedIn(String),
    /// The wrap failed, for every reason given.
    Failed(Vec<String>),
}

/// Reads a wrap from steamcmd's exit `code`, its `console`, and whether the wrapped executable
/// was `written`.
#[must_use]
pub fn judge(code: Option<i32>, console: &str, written: bool) -> Outcome {
    if let Some(reason) = upload::refused_login(console) {
        return Outcome::NotLoggedIn(reason);
    }
    let completed = console
        .lines()
        .any(|line| line.starts_with("DRM wrap completed"));
    if code == Some(0_i32) && completed && written {
        return Outcome::Wrapped;
    }
    Outcome::Failed(refusal(console).unwrap_or_else(|| upload::reasons(code, console, None)))
}

/// Why Valve's tool refused the executable, as steamcmd 1788292693 printed it: the tool's own
/// words between `Error result: 8 (Invalid Parameter - Valve Portable DRM Tool (…)` and a closing
/// parenthesis alone on a line, after its copyright line.
fn refusal(console: &str) -> Option<Vec<String>> {
    let said: Vec<&str> = console
        .lines()
        .skip_while(|line| !line.starts_with("Error result:"))
        .skip(1)
        .take_while(|line| line.trim() != ")")
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with("(C) Copyright"))
        .collect();
    if said.is_empty() {
        return None;
    }
    let mut reasons = vec![format!(
        "Valve's DRM tool refused the executable: {}",
        said.join("; ")
    )];
    if said
        .iter()
        .any(|line| line.starts_with("Unable to add import"))
    {
        reasons.push(
            "the tool adds Steam DRM only to an executable that already imports that function; \
             Godot's and Rust's executables do not"
                .to_owned(),
        );
    }
    Some(reasons)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// steamcmd 1788292693's console wrapping an executable that imports `GetModuleHandleA`.
    const WRAPPED: &str = "Logging in user 'build_bot' [U:1:1] to Steam Public...OK\n\
        Uploading C:\\game\\probe.exe to https://partnerupload.steampowered.com/upload/1 \
        (230912 bytes)\n\
        Upload complete.\n\
        Requesting server side DRM wrap\n\
        Requesting download URL\n\
        Downloading wrapped file\n\
        DRM wrap completed; output is in C:\\game\\probe.drm.exe\n\
        Unloading Steam API...OK\n";

    /// Its console wrapping Fantasy Guild Manager's Godot executable.
    const REFUSED: &str = "Upload complete.\n\
        Requesting server side DRM wrap\n\
        Finish method returned eresult 8\n\
        Error result: 8 (Invalid Parameter - Valve Portable DRM Tool (Build: Sep 22 2026 \
        14:00:22)\n\
        (C) Copyright 2012-2025, Valve Corporation, All rights reserved.\n\
        \n\
        PE module implementation does not currently support adding imports\n\
        Unable to add import of kernel32.dll:GetModuleHandleA\n\
        )\n\
        DRM wrap failed with EResult 8 (Invalid Parameter)\n\
        Unloading Steam API...OK\n";

    #[test]
    fn a_wrap_is_done_only_when_steamcmd_says_so_exits_0_and_wrote_the_file() {
        assert_eq!(judge(Some(0_i32), WRAPPED, true), Outcome::Wrapped);
        for (code, written) in [(Some(0_i32), false), (Some(1_i32), true), (None, true)] {
            assert!(
                matches!(judge(code, WRAPPED, written), Outcome::Failed(_)),
                "{code:?} {written}"
            );
        }
        assert!(matches!(
            judge(Some(0_i32), "Unloading Steam API...OK\n", true),
            Outcome::Failed(_)
        ));
    }

    #[test]
    fn a_refused_executable_is_said_in_the_tools_own_words_with_why() {
        assert_eq!(
            judge(Some(11_i32), REFUSED, false),
            Outcome::Failed(vec![
                "Valve's DRM tool refused the executable: PE module implementation does not \
                 currently support adding imports; Unable to add import of \
                 kernel32.dll:GetModuleHandleA"
                    .to_owned(),
                "the tool adds Steam DRM only to an executable that already imports that \
                 function; Godot's and Rust's executables do not"
                    .to_owned(),
            ])
        );
        let other = "Error result: 2 (Failure - Valve Portable DRM Tool\nno such app\n)\n";
        assert_eq!(
            judge(Some(11_i32), other, false),
            Outcome::Failed(vec![
                "Valve's DRM tool refused the executable: no such app".to_owned()
            ])
        );
        assert_eq!(
            judge(Some(11_i32), "ERROR! Failed to upload\n", false),
            Outcome::Failed(vec!["Failed to upload".to_owned()])
        );
    }

    #[test]
    fn a_refused_login_is_not_a_failed_wrap() {
        assert!(matches!(
            judge(
                Some(5_i32),
                "Logging in user 'build_bot' to Steam Public...FAILED (Invalid Password)\n",
                false
            ),
            Outcome::NotLoggedIn(_)
        ));
    }

    #[test]
    fn each_mode_has_valves_flags() {
        assert_eq!(Mode::chosen(false), Mode::Default);
        assert_eq!(Mode::chosen(true), Mode::Compatibility);
        assert_eq!(Mode::Default.flags(), "0");
        assert_eq!(Mode::Compatibility.flags(), "6");
    }
}
