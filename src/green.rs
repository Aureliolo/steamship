//! A game that names a branch as `green` in its `steamship.toml` ships only a commit on that
//! branch on origin, which the game's tests move to each commit they pass.

use std::path::Path;

use crate::project::{git, git_run, short};

/// The commit an upload is made from, and whether the green branch held it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Shipped {
    pub commit: String,
    pub green: bool,
}

/// Where a commit stands against origin's green branch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Standing {
    /// The commit is the branch's tip or one of its ancestors.
    On,
    /// The commit is not on the branch. `tip` is where it is, when origin has the branch.
    Off { tip: Option<String> },
}

/// Whether a commit that is not green is let through.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Policy {
    Required,
    Allowed,
}

impl Policy {
    /// `Allowed` when `allow_ungreen`, as `--allow-ungreen` asks.
    #[must_use]
    pub const fn from_flag(allow_ungreen: bool) -> Self {
        if allow_ungreen {
            Self::Allowed
        } else {
            Self::Required
        }
    }
}

/// The commit at `root`'s HEAD, refused unless origin's `branch` holds it. Under `Allowed` it is
/// let through, and `green` says which it was.
///
/// # Errors
///
/// When Git cannot be run, `root` is in no repository, or the commit is not on `branch` and the
/// policy is `Required`.
pub fn require(root: &Path, branch: &str, policy: Policy) -> Result<Shipped, String> {
    let commit = git(root, &["rev-parse", "HEAD"])?;
    let required = policy == Policy::Required;
    match standing(root, branch, &commit) {
        Ok(Standing::On) => Ok(Shipped {
            commit,
            green: true,
        }),
        Ok(Standing::Off { tip }) if required => Err(not_green(&commit, branch, tip.as_deref())),
        Err(why) if required => Err(why),
        Ok(Standing::Off { .. }) | Err(_) => Ok(Shipped {
            commit,
            green: false,
        }),
    }
}

/// Whether `commit` is on origin's `branch`, asked of the remote.
///
/// # Errors
///
/// When Git cannot be run or origin cannot be asked.
pub fn standing(root: &Path, branch: &str, commit: &str) -> Result<Standing, String> {
    let reference = format!("refs/heads/{branch}");
    let printed = git(root, &["ls-remote", "origin", &reference])?;
    let Some(tip) = tip_of(&printed) else {
        return Ok(Standing::Off { tip: None });
    };
    if tip == commit {
        return Ok(Standing::On);
    }
    // Fetched into FETCH_HEAD alone, so no branch of the game's own moves.
    drop(git(root, &["fetch", "--quiet", "origin", &reference])?);
    let ancestry = git_run(root, &["merge-base", "--is-ancestor", commit, "FETCH_HEAD"])?;
    match ancestry.status.code() {
        Some(0) => Ok(Standing::On),
        Some(1) => Ok(Standing::Off { tip: Some(tip) }),
        _ => Err(format!(
            "git could not tell whether {} is on origin's {branch}: {}",
            short(commit),
            String::from_utf8_lossy(&ancestry.stderr).trim()
        )),
    }
}

/// The commit `git ls-remote` printed for the branch, when it printed one.
fn tip_of(printed: &str) -> Option<String> {
    printed
        .lines()
        .find_map(|line| line.split_once('\t'))
        .map(|(sha, _)| sha.to_owned())
}

fn not_green(commit: &str, branch: &str, tip: Option<&str>) -> String {
    let at = tip.map_or_else(
        || format!("origin has no {branch} branch"),
        |tip| format!("{branch} is at {}", short(tip)),
    );
    format!(
        "commit {} is not on origin's {branch} branch ({at}). Upload it once your tests have \
         moved {branch} to it, or give --allow-ungreen to upload it as it is",
        short(commit)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_tip_is_the_first_column_of_the_remote_listing() {
        let printed = "0123456789abcdef0123456789abcdef01234567\trefs/heads/green";
        assert_eq!(
            tip_of(printed).as_deref(),
            Some("0123456789abcdef0123456789abcdef01234567")
        );
    }

    #[test]
    fn a_remote_without_the_branch_prints_no_tip() {
        assert_eq!(tip_of(""), None);
    }

    #[test]
    fn the_refusal_names_the_commit_the_branch_where_it_is_and_the_fix() {
        let why = not_green(
            "aaaaaaaaaaaabbbbbbbbbbbbbbbbbbbbbbbbbbbb",
            "passed",
            Some("ccccccccccccdddddddddddddddddddddddddddd"),
        );
        assert!(
            why.starts_with("commit aaaaaaaaaaaa is not on origin's passed branch"),
            "{why}"
        );
        assert!(why.contains("passed is at cccccccccccc"), "{why}");
        assert!(
            why.ends_with(
                "Upload it once your tests have moved passed to it, or give --allow-ungreen to \
                 upload it as it is"
            ),
            "{why}"
        );
    }

    #[test]
    fn the_refusal_says_when_origin_has_no_such_branch() {
        let why = not_green("aaaaaaaaaaaabbbbbbbbbbbbbbbbbbbbbbbbbbbb", "passed", None);
        assert!(why.contains("origin has no passed branch"), "{why}");
    }
}
