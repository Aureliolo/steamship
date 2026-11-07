//! The builds each upload set live, kept in the steamship home so that a rollback knows which
//! build a branch had before.

use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};

/// The folder of the home holding one file per app.
const FOLDER: &str = "recorded-builds";

/// One build set live on one branch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub app: u32,
    pub branch: String,
    pub build_id: u64,
    /// The full commit the build was made from.
    pub commit: String,
    /// Whether the commit was on the repository's `green` branch when it went up.
    pub green: bool,
    /// Whether the entry is a rollback setting an earlier build live, not an upload.
    pub rollback: bool,
}

/// The file recording `app`'s builds.
#[must_use]
pub fn file(home: &Path, app: u32) -> PathBuf {
    home.join(FOLDER).join(format!("{app}.json"))
}

/// The entries `text` holds, oldest first.
///
/// # Errors
///
/// When the text is not a JSON list of entries.
pub fn parse(text: &str) -> Result<Vec<Entry>, String> {
    let value: Value = serde_json::from_str(text).map_err(|error| error.to_string())?;
    let list = value
        .as_array()
        .ok_or_else(|| "it is not a list of builds".to_owned())?;
    list.iter().map(entry).collect()
}

fn entry(value: &Value) -> Result<Entry, String> {
    let text = |key: &str| value.get(key).and_then(Value::as_str).map(str::to_owned);
    let number = |key: &str| value.get(key).and_then(Value::as_u64);
    Ok(Entry {
        app: number("app")
            .and_then(|app| u32::try_from(app).ok())
            .ok_or("a build has no app")?,
        branch: text("branch").ok_or("a build has no branch")?,
        build_id: number("build_id").ok_or("a build has no build_id")?,
        commit: text("commit").ok_or("a build has no commit")?,
        green: value
            .get("green")
            .and_then(Value::as_bool)
            .ok_or("a build does not say whether its commit was green")?,
        rollback: value
            .get("rollback")
            .and_then(Value::as_bool)
            .ok_or("a build does not say whether it is a rollback")?,
    })
}

/// `entries` as the file holds them.
#[must_use]
pub fn render(entries: &[Entry]) -> String {
    let list: Vec<Value> = entries
        .iter()
        .map(|each| {
            json!({
                "app": each.app,
                "branch": each.branch,
                "build_id": each.build_id,
                "commit": each.commit,
                "green": each.green,
                "rollback": each.rollback,
            })
        })
        .collect();
    let mut text = serde_json::to_string_pretty(&list).unwrap_or_else(|_| "[]".to_owned());
    text.push('\n');
    text
}

/// The builds recorded for `app`, oldest first. None when it has none yet.
///
/// # Errors
///
/// When the file cannot be read or is not a list of builds.
pub fn load(home: &Path, app: u32) -> Result<Vec<Entry>, String> {
    let path = file(home, app);
    match fs::read_to_string(&path) {
        Ok(text) => parse(&text).map_err(|why| format!("{}: {why}", path.display())),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(Vec::new()),
        Err(error) => Err(format!("{}: {error}", path.display())),
    }
}

/// Adds `added` to the builds recorded for its app.
///
/// # Errors
///
/// When the file cannot be read, is not a list of builds, or cannot be written.
pub fn record(home: &Path, added: Entry) -> Result<(), String> {
    let path = file(home, added.app);
    let mut entries = load(home, added.app)?;
    entries.push(added);
    let folder = path.parent().unwrap_or(home);
    fs::create_dir_all(folder).map_err(|error| format!("{}: {error}", folder.display()))?;
    // Written whole beside the file and moved over it, so a stopped run never leaves half a list.
    let part = path.with_extension("json.part");
    fs::write(&part, render(&entries)).map_err(|error| format!("{}: {error}", part.display()))?;
    fs::rename(&part, &path).map_err(|error| format!("{}: {error}", path.display()))
}

/// The build live on `branch` now by the record, and the different one before it.
///
/// # Errors
///
/// When `branch` has no recorded build, or only one build.
pub fn previous<'entries>(
    entries: &'entries [Entry],
    app: u32,
    branch: &str,
) -> Result<(&'entries Entry, &'entries Entry), String> {
    let on_branch: Vec<&Entry> = entries
        .iter()
        .filter(|each| each.branch.eq_ignore_ascii_case(branch))
        .collect();
    let current = on_branch.last().copied().ok_or_else(|| {
        format!("steamship recorded no upload to {branch} for app {app}, so it has no build to go back to")
    })?;
    // Rollbacks are stepped over, so that going back twice goes back twice.
    let uploads: Vec<&Entry> = on_branch
        .iter()
        .copied()
        .filter(|each| !each.rollback)
        .collect();
    let before = uploads
        .iter()
        .rposition(|each| each.build_id == current.build_id)
        .and_then(|at| {
            uploads
                .get(..at)?
                .iter()
                .rev()
                .find(|each| each.build_id != current.build_id)
                .copied()
        })
        .ok_or_else(|| {
            format!(
                "steamship recorded only BuildID {} on {branch} for app {app}, so it has no earlier build to go back to",
                current.build_id
            )
        })?;
    Ok((current, before))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry_of(branch: &str, build_id: u64) -> Entry {
        Entry {
            app: 480,
            branch: branch.to_owned(),
            build_id,
            commit: format!("{build_id:040x}"),
            green: true,
            rollback: false,
        }
    }

    fn rolled_back_to(branch: &str, build_id: u64) -> Entry {
        Entry {
            rollback: true,
            ..entry_of(branch, build_id)
        }
    }

    #[test]
    fn a_second_rollback_goes_back_again_instead_of_undoing_the_first() {
        let mut entries = vec![
            entry_of("testing", 5),
            entry_of("testing", 7),
            entry_of("testing", 9),
        ];
        let step = |recorded: &[Entry]| {
            let (current, before) = previous(recorded, 480, "testing").unwrap();
            (current.build_id, before.build_id)
        };
        assert_eq!(step(&entries), (9, 7));
        entries.push(rolled_back_to("testing", 7));
        assert_eq!(step(&entries), (7, 5));
        entries.push(rolled_back_to("testing", 5));
        let why = previous(&entries, 480, "testing").unwrap_err();
        assert!(why.contains("only BuildID 5 on testing"), "{why}");
    }

    #[test]
    fn an_upload_after_a_rollback_goes_back_to_the_newest_upload_before_it() {
        let entries = vec![
            entry_of("testing", 5),
            entry_of("testing", 7),
            rolled_back_to("testing", 5),
            entry_of("testing", 9),
        ];
        let (current, before) = previous(&entries, 480, "testing").unwrap();
        assert_eq!((current.build_id, before.build_id), (9, 7));
    }

    #[test]
    fn rollback_entries_are_marked_in_the_file() {
        let entries = vec![entry_of("testing", 5), rolled_back_to("testing", 5)];
        let read = parse(&render(&entries)).unwrap();
        assert_eq!(
            read.iter().map(|each| each.rollback).collect::<Vec<_>>(),
            [false, true]
        );
    }

    #[test]
    fn entries_survive_being_written_and_read() {
        let entries = vec![entry_of("testing", 7), entry_of("default", 8)];
        assert_eq!(parse(&render(&entries)), Ok(entries));
    }

    #[test]
    fn a_file_that_is_not_a_list_is_refused() {
        assert_eq!(parse("{}"), Err("it is not a list of builds".to_owned()));
    }

    #[test]
    fn an_entry_missing_its_commit_is_refused() {
        let why = parse(
            r#"[{"app": 480, "branch": "testing", "build_id": 1, "green": true, "rollback": false}]"#,
        );
        assert_eq!(why, Err("a build has no commit".to_owned()));
    }

    #[test]
    fn the_previous_build_is_the_one_before_on_the_same_branch() {
        let entries = vec![
            entry_of("testing", 1),
            entry_of("default", 2),
            entry_of("testing", 3),
        ];
        let (current, before) = previous(&entries, 480, "testing").unwrap();
        assert_eq!((current.build_id, before.build_id), (3, 1));
    }

    #[test]
    fn a_build_recorded_twice_in_a_row_is_stepped_over() {
        let entries = vec![
            entry_of("testing", 1),
            entry_of("testing", 2),
            entry_of("testing", 2),
        ];
        let (current, before) = previous(&entries, 480, "testing").unwrap();
        assert_eq!((current.build_id, before.build_id), (2, 1));
    }

    #[test]
    fn a_branch_never_uploaded_to_has_nothing_to_go_back_to() {
        let entries = vec![entry_of("testing", 1)];
        let why = previous(&entries, 480, "default").unwrap_err();
        assert!(
            why.contains("recorded no upload to default for app 480"),
            "{why}"
        );
    }

    #[test]
    fn a_branch_with_one_build_has_nothing_to_go_back_to() {
        let entries = vec![entry_of("testing", 1)];
        let why = previous(&entries, 480, "testing").unwrap_err();
        assert!(why.contains("only BuildID 1 on testing"), "{why}");
    }
}
