//! The default branch's names. It is `default` on the Steamworks site and in every message of
//! steamship's, and `public` to the Web API and steamcmd.

/// The names the default branch goes by: `default` on the Steamworks site, `public` in the Web
/// API.
pub const DEFAULT: [&str; 2] = ["default", "public"];

/// The default branch's name in the Web API, which `SetAppBuildLive` takes for it.
pub const DEFAULT_KEY: &str = "public";

/// Whether `branch` names the default branch.
#[must_use]
pub fn is_default(branch: &str) -> bool {
    DEFAULT
        .iter()
        .any(|default| branch.eq_ignore_ascii_case(default))
}

/// The name Steamworks shows for `branch`: the Web API calls the default branch `public`, which
/// the site, and every message of steamship's, calls `default`.
#[must_use]
pub fn shown_name(branch: &str) -> &str {
    if is_default(branch) {
        "default"
    } else {
        branch
    }
}

/// The name the Web API and steamcmd take for `branch`: `public` for the default branch, in either
/// of its names, and any other as given.
#[must_use]
pub fn key(branch: &str) -> &str {
    if is_default(branch) {
        DEFAULT_KEY
    } else {
        branch
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_branch_is_known_by_both_its_names() {
        for default in ["default", "public", "Public"] {
            assert!(is_default(default), "{default}");
        }
        assert!(!is_default("testing"));
    }

    #[test]
    fn steam_takes_the_default_branch_as_public_and_any_other_as_named() {
        assert_eq!(key("default"), "public");
        assert_eq!(key("Public"), "public");
        assert_eq!(key("testing"), "testing");
    }
}
