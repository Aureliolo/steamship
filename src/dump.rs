//! Steam's logs, shown in a GitHub Actions log when a run fails there, since the action leaves
//! nothing behind to look at afterwards.
//!
//! Each file is one collapsed group. What it holds is shown with workflow commands stopped, so no
//! line in a log can act as one; and only its last lines, so a long log does not drown the job's.

use std::collections::BTreeMap;
use std::collections::hash_map::RandomState;
use std::fmt::{self, Write as _};
use std::fs;
use std::hash::BuildHasher as _;
use std::path::{Path, PathBuf};
use std::process;

/// The most lines of any one file shown.
pub const LINES: usize = 400;

/// Whether this is a GitHub Actions job, as its runner says in the environment.
#[must_use]
pub fn in_actions<Lookup>(lookup: Lookup) -> bool
where
    Lookup: Fn(&str) -> Option<String>,
{
    lookup("GITHUB_ACTIONS").is_some_and(|value| value == "true")
}

/// Whether steamship runs in CI, as GitHub Actions and most other systems say by setting `CI`
/// to anything but empty or `false`.
#[must_use]
pub fn in_ci<Lookup>(lookup: Lookup) -> bool
where
    Lookup: Fn(&str) -> Option<String>,
{
    in_actions(&lookup) || lookup("CI").is_some_and(|value| !value.is_empty() && value != "false")
}

/// How long each file in a folder of logs was, taken before a run, so that only what the run
/// added is shown: steamcmd adds to its logs rather than starting them afresh.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Marks(BTreeMap<PathBuf, u64>);

impl Marks {
    /// The length of every file in `folder` now; none when the folder is not there yet.
    #[must_use]
    pub fn take(folder: &Path) -> Self {
        let lengths = fs::read_dir(folder)
            .into_iter()
            .flatten()
            .filter_map(Result::ok)
            .filter_map(|entry| {
                let metadata = entry.metadata().ok()?;
                metadata.is_file().then_some((entry.path(), metadata.len()))
            })
            .collect();
        Self(lengths)
    }

    /// What each file in `folder` gained since the marks were taken, by name, the files that did
    /// not change left out.
    #[must_use]
    pub fn added(&self, folder: &Path) -> Vec<(String, Vec<u8>)> {
        let mut added = Vec::new();
        for (path, length) in Self::take(folder).0 {
            let from = self.0.get(&path).copied().unwrap_or(0);
            // A file shorter than it was has been started afresh, so all of it is this run's.
            let from = if length < from { 0 } else { from };
            if length == from {
                continue;
            }
            let Some(bytes) = fs::read(&path)
                .ok()
                .and_then(|whole| whole.get(usize::try_from(from).ok()?..).map(<[u8]>::to_vec))
            else {
                continue;
            };
            let name = path
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default();
            added.push((name, bytes));
        }
        added
    }
}

/// `text` as one collapsed group titled `title`, cut to its last [`LINES`] lines, its contents
/// shown with workflow commands stopped until `token`.
#[must_use]
pub fn group(title: &str, text: &str, token: &str) -> String {
    let lines: Vec<&str> = text.lines().collect();
    let left_out = lines.len().saturating_sub(LINES);
    let mut shown = format!("::group::{title}\n::stop-commands::{token}\n");
    if left_out > 0 {
        let _said: fmt::Result = writeln!(
            shown,
            "({left_out} earlier {} left out)",
            if left_out == 1 { "line" } else { "lines" }
        );
    }
    for line in lines.iter().skip(left_out) {
        shown.push_str(line.trim_end_matches('\r'));
        shown.push('\n');
    }
    let _said: fmt::Result = write!(shown, "::{token}::\n::endgroup::\n");
    shown
}

/// A token nobody writing a log could know ahead, to end a stretch with workflow commands
/// stopped.
#[must_use]
pub fn token() -> String {
    let random = RandomState::new().hash_one(process::id());
    format!("steamship-{random:016x}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_job_that_says_it_is_one_is_one() {
        let with = |value: Option<&str>| {
            in_actions(|name| {
                (name == "GITHUB_ACTIONS")
                    .then_some(value)
                    .flatten()
                    .map(str::to_owned)
            })
        };
        assert!(with(Some("true")));
        assert!(!with(Some("false")));
        assert!(!with(Some("")));
        assert!(!with(None));
    }

    #[test]
    fn ci_is_said_by_ci_set_to_anything_but_empty_or_false_or_by_github_actions() {
        let with = |set: &[(&str, &str)]| {
            let set: Vec<(String, String)> = set
                .iter()
                .map(|&(name, value)| (name.to_owned(), value.to_owned()))
                .collect();
            in_ci(|name| {
                set.iter()
                    .find(|(found, _)| found == name)
                    .map(|(_, value)| value.clone())
            })
        };
        assert!(with(&[("CI", "true")]));
        assert!(with(&[("CI", "1")]));
        assert!(!with(&[("CI", "false")]));
        assert!(!with(&[("CI", "")]));
        assert!(!with(&[]));
        assert!(with(&[("GITHUB_ACTIONS", "true")]));
        assert!(with(&[("GITHUB_ACTIONS", "true"), ("CI", "false")]));
    }

    #[test]
    fn a_group_shows_its_text_with_commands_stopped() {
        assert_eq!(
            group("content_log.txt", "one\r\n::add-mask::x\n", "t0k"),
            "::group::content_log.txt\n::stop-commands::t0k\none\n::add-mask::x\n::t0k::\n\
             ::endgroup::\n"
        );
    }

    /// The numbers from 1 to `last`, one to a line.
    fn numbered(last: usize) -> String {
        (1..=last).fold(String::new(), |mut all, line| {
            let _said: fmt::Result = writeln!(all, "{line}");
            all
        })
    }

    #[test]
    fn a_long_text_shows_only_its_last_lines_and_says_how_many_were_left_out() {
        let over = LINES.checked_add(2).unwrap();
        let shown = group("t", &numbered(over), "k");
        assert!(
            shown.contains("\n(2 earlier lines left out)\n3\n"),
            "{shown}"
        );
        assert!(shown.ends_with(&format!("{over}\n::k::\n::endgroup::\n")));
        let one_over = numbered(LINES.checked_add(1).unwrap());
        assert!(group("t", &one_over, "k").contains("(1 earlier line left out)"));
        assert!(!group("t", &numbered(LINES), "k").contains("left out"));
    }

    #[test]
    fn tokens_differ_between_calls() {
        assert_ne!(token(), token());
        assert!(token().starts_with("steamship-"));
    }

    #[test]
    fn only_what_a_run_added_to_each_log_is_shown() {
        let folder = tempfile::tempdir().unwrap();
        let path = |name: &str| folder.path().join(name);
        fs::write(path("content_log.txt"), "old\n").unwrap();
        fs::write(path("stderr.txt"), "long before\n").unwrap();
        fs::write(path("quiet.txt"), "same\n").unwrap();
        let marks = Marks::take(folder.path());
        fs::write(path("content_log.txt"), "old\nnew\n").unwrap();
        fs::write(path("stderr.txt"), "fresh\n").unwrap();
        fs::write(path("workshop_log.txt"), "made\n").unwrap();
        assert_eq!(
            marks.added(folder.path()),
            [
                ("content_log.txt".to_owned(), b"new\n".to_vec()),
                ("stderr.txt".to_owned(), b"fresh\n".to_vec()),
                ("workshop_log.txt".to_owned(), b"made\n".to_vec()),
            ]
        );
        assert_eq!(Marks::take(&path("missing")), Marks::default());
    }
}
