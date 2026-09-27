//! Workshop items: Valve's `workshopitem` script checked, and the copy steamcmd is given.
//!
//! steamcmd's `workshop_build_item` creates an item when the script names no `publishedfileid`,
//! or `0`, and then writes the new item's ID into the script it was given. steamship gives it a
//! copy in the home with every path made absolute, so the ID is read back from the copy, and the
//! original is left for its author to change.

use std::fs;
use std::io;
use std::path::{self, Path, PathBuf};

use crate::scripts::{self, Problem};
use crate::upload;
use crate::vdf::{self, Block, Pair, Value};

/// The folder in the home that holds each app's copy of its item scripts.
pub const ITEMS: &str = "workshop";

/// The longest title Steam takes for an item, in bytes.
const TITLE: usize = 128;

/// The longest description or change note Steam takes, in bytes.
const TEXT: usize = 8000;

/// The largest preview image Steam takes, in bytes.
const PREVIEW: u64 = 1024 * 1024;

/// A checked item script.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Item {
    pub app_id: u32,
    /// The item it updates; none when it makes a new one.
    pub published: Option<u64>,
    /// The folder uploaded as the item's content, if it names one.
    pub content: Option<PathBuf>,
    /// How many files the content folder holds.
    pub files: usize,
}

/// Checks the item script at `path` and what it names, and says every problem at once.
///
/// # Errors
///
/// Every problem found, the script unreadable being the only one then.
pub fn check(path: &Path) -> Result<Item, Vec<Problem>> {
    let document = scripts::read(path).map_err(|problem| vec![problem])?;
    let Some(item) = document.block("workshopitem") else {
        return Err(vec![Problem::new(
            path,
            "there is no \"workshopitem\" block",
        )]);
    };
    let mut problems = Vec::new();
    let mut refuse = |message: String| problems.push(Problem::new(path, message));
    let app_id = match item.text("appid").map(str::parse::<u32>) {
        Some(Ok(app_id)) if app_id > 0 => Some(app_id),
        Some(_) => {
            refuse("\"appid\" is not an app's ID".to_owned());
            None
        }
        None => {
            refuse("there is no \"appid\"".to_owned());
            None
        }
    };
    let published = match item.text("publishedfileid").map(str::parse::<u64>) {
        None | Some(Ok(0)) => None,
        Some(Ok(published)) => Some(published),
        Some(Err(_)) => {
            refuse("\"publishedfileid\" is not an item's ID".to_owned());
            None
        }
    };
    let folder = folder(path);
    let content = item
        .text("contentfolder")
        .map(|written| scripts::resolve(&folder, written));
    let files = content
        .as_deref()
        .map_or(0, |content| match count(content) {
            Ok(0) => {
                refuse(format!(
                    "contentfolder {} holds no files",
                    content.display()
                ));
                0
            }
            Ok(files) => files,
            Err(error) => {
                refuse(format!(
                    "contentfolder {} cannot be read: {error}",
                    content.display()
                ));
                0
            }
        });
    if let Some(preview) = item.text("previewfile") {
        let preview = scripts::resolve(&folder, preview);
        match fs::metadata(&preview) {
            Ok(metadata) if !metadata.is_file() => {
                refuse(format!("previewfile {} is not a file", preview.display()));
            }
            Ok(metadata) if metadata.len() > PREVIEW => refuse(format!(
                "previewfile {} is larger than the 1 MB Steam takes",
                preview.display()
            )),
            Ok(_) => {}
            Err(error) => refuse(format!(
                "previewfile {} cannot be read: {error}",
                preview.display()
            )),
        }
    }
    match item.text("visibility").map(str::parse::<u8>) {
        None | Some(Ok(0..=3)) => {}
        Some(_) => refuse(
            "\"visibility\" is not 0 (public), 1 (friends only), 2 (private) or 3 (unlisted)"
                .to_owned(),
        ),
    }
    for (key, longest) in [
        ("title", TITLE),
        ("description", TEXT),
        ("changenote", TEXT),
    ] {
        if item.text(key).is_some_and(|text| text.len() > longest) {
            refuse(format!(
                "\"{key}\" is longer than the {longest} bytes Steam takes"
            ));
        }
    }
    match (app_id, problems.is_empty()) {
        (Some(app_id), true) => Ok(Item {
            app_id,
            published,
            content,
            files,
        }),
        _ => Err(problems),
    }
}

fn folder(path: &Path) -> PathBuf {
    path.parent()
        .filter(|folder| !folder.as_os_str().is_empty())
        .map_or_else(|| PathBuf::from("."), Path::to_path_buf)
}

/// How many files `folder` holds, in it and every folder under it.
fn count(folder: &Path) -> io::Result<usize> {
    let mut files = 0_usize;
    for entry in fs::read_dir(folder)? {
        let entry = entry?;
        files = files.saturating_add(if entry.file_type()?.is_dir() {
            count(&entry.path())?
        } else {
            1
        });
    }
    Ok(files)
}

/// Writes the copy of the item script at `original` that steamcmd is given into the app's folder
/// in `home`, with its paths made absolute, and says where it is.
///
/// # Errors
///
/// When the script cannot be read or written, or a path in it is not valid Unicode.
pub fn prepare(home: &Path, original: &Path, app_id: u32) -> Result<PathBuf, upload::Error> {
    let problem = |reason: String| upload::Error::Script {
        path: original.to_path_buf(),
        reason,
    };
    let at = |path: &Path| {
        let path = path.to_path_buf();
        move |error| upload::Error::Io { path, error }
    };
    let document = scripts::read(original).map_err(|found| problem(found.message))?;
    let item = document
        .block("workshopitem")
        .ok_or_else(|| problem("there is no \"workshopitem\" block".to_owned()))?;
    let absolute = path::absolute(original).map_err(at(original))?;
    let folder = folder(&absolute);
    let mut rewritten = Block::default();
    for pair in &item.pairs {
        let path = ["contentfolder", "previewfile"]
            .iter()
            .any(|key| pair.key.eq_ignore_ascii_case(key));
        let value = match &pair.value {
            Value::Text(written) if path => {
                Value::Text(upload::written(&scripts::resolve(&folder, written)).map_err(problem)?)
            }
            value @ (Value::Text(_) | Value::Block(_)) => value.clone(),
        };
        rewritten.pairs.push(Pair {
            key: pair.key.clone(),
            value,
        });
    }
    let whole = Block {
        pairs: vec![Pair {
            key: "workshopitem".to_owned(),
            value: Value::Block(rewritten),
        }],
    };
    let items = home.join(ITEMS).join(app_id.to_string());
    fs::create_dir_all(&items).map_err(at(&items))?;
    let copy = items.join("workshop_item.vdf");
    let text = vdf::write(&whole).map_err(|error| problem(error.to_string()))?;
    fs::write(&copy, text).map_err(at(&copy))?;
    Ok(copy)
}

/// The item's ID in the copy steamcmd was given, which steamcmd writes when it made the item.
#[must_use]
pub fn published(copy: &Path) -> Option<u64> {
    let document = scripts::read(copy).ok()?;
    document
        .block("workshopitem")?
        .text("publishedfileid")?
        .parse()
        .ok()
        .filter(|published| *published > 0)
}

/// What a run of `workshop_build_item` came to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// Uploaded, as the item with this ID.
    Published { published: u64 },
    /// steamcmd could not log in with what it has saved, for this reason.
    NotLoggedIn(String),
    /// Anything else, with every reason steamcmd gave.
    Failed(Vec<String>),
}

/// Reads the result of a run from steamcmd's exit `code`, its `console`, and the item's ID in
/// the copy it was given afterwards.
#[must_use]
pub fn judge(code: Option<i32>, console: &str, published: Option<u64>) -> Outcome {
    if let Some(reason) = upload::refused_login(console) {
        return Outcome::NotLoggedIn(reason);
    }
    match (code, published) {
        (Some(0_i32), Some(published)) => Outcome::Published { published },
        _ => Outcome::Failed(upload::reasons(code, console, None)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Folder {
        _temp: tempfile::TempDir,
        root: PathBuf,
    }

    impl Folder {
        fn new() -> Self {
            let temp = tempfile::tempdir().unwrap();
            let root = temp.path().join("my game (x86)");
            fs::create_dir_all(&root).unwrap();
            Self { _temp: temp, root }
        }

        fn put(&self, relative: &str, contents: &[u8]) -> PathBuf {
            let path = self.root.join(relative);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(&path, contents).unwrap();
            path
        }

        fn script(&self, keys: &str) -> PathBuf {
            self.put(
                "workshop/item.vdf",
                format!("\"workshopitem\" {{ {keys} }}").as_bytes(),
            )
        }
    }

    const SOUND: &str = r#""appid" "480" "contentfolder" "hat" "previewfile" "hat.png"
        "visibility" "2" "title" "A green hat" "changenote" "1.2""#;

    fn sound() -> Folder {
        let folder = Folder::new();
        drop(folder.put("workshop/hat/hat.mdl", b"model"));
        drop(folder.put("workshop/hat/textures/green.png", b"png"));
        drop(folder.put("workshop/hat.png", b"png"));
        folder
    }

    fn messages(problems: &[Problem]) -> Vec<&str> {
        problems
            .iter()
            .map(|problem| problem.message.as_str())
            .collect()
    }

    #[test]
    fn a_sound_new_item_is_checked_with_its_files_counted() {
        let folder = sound();
        let item = check(&folder.script(SOUND)).unwrap();
        assert_eq!(item.app_id, 480);
        assert_eq!(item.published, None);
        assert_eq!(item.files, 2);
        assert_eq!(
            item.content,
            Some(
                folder
                    .root
                    .join("workshop")
                    .join("hat")
                    .components()
                    .collect()
            )
        );
    }

    #[test]
    fn what_is_at_each_limit_is_taken() {
        let folder = sound();
        drop(folder.put("workshop/hat.png", &vec![0_u8; 1024 * 1024]));
        let script = folder.script(&format!(
            r#""appid" "1" "contentfolder" "hat" "previewfile" "hat.png" "visibility" "3"
            "title" "{}" "description" "{}" "changenote" "{}""#,
            "t".repeat(128),
            "d".repeat(8000),
            "c".repeat(8000)
        ));
        assert_eq!(check(&script).map(|item| item.app_id), Ok(1));
    }

    #[test]
    fn app_zero_is_no_app() {
        let folder = sound();
        let script = folder.script(r#""appid" "0" "contentfolder" "hat""#);
        assert_eq!(
            messages(&check(&script).unwrap_err()),
            ["\"appid\" is not an app's ID"]
        );
    }

    #[test]
    fn an_item_to_update_is_named_by_its_id_and_zero_makes_a_new_one() {
        let folder = sound();
        let update = check(&folder.script(&format!("{SOUND} \"publishedfileid\" \"5674\"")));
        assert_eq!(update.unwrap().published, Some(5674));
        let new = check(&folder.script(&format!("{SOUND} \"publishedfileid\" \"0\"")));
        assert_eq!(new.unwrap().published, None);
    }

    #[test]
    fn every_problem_is_named_at_once() {
        let folder = sound();
        drop(folder.put("workshop/empty/.keep", b""));
        fs::remove_file(folder.root.join("workshop/empty/.keep")).unwrap();
        drop(folder.put("workshop/big.png", &vec![0_u8; 1024 * 1024 + 1]));
        let script = folder.script(&format!(
            r#""appid" "x" "publishedfileid" "-1" "contentfolder" "empty" "previewfile" "big.png"
            "visibility" "4" "title" "{}" "description" "{}" "changenote" "{}""#,
            "t".repeat(129),
            "d".repeat(8001),
            "c".repeat(8001)
        ));
        let problems = check(&script).unwrap_err();
        let empty = folder.root.join("workshop").join("empty");
        let big = folder.root.join("workshop").join("big.png");
        assert_eq!(
            messages(&problems),
            [
                "\"appid\" is not an app's ID".to_owned(),
                "\"publishedfileid\" is not an item's ID".to_owned(),
                format!("contentfolder {} holds no files", empty.display()),
                format!(
                    "previewfile {} is larger than the 1 MB Steam takes",
                    big.display()
                ),
                "\"visibility\" is not 0 (public), 1 (friends only), 2 (private) or 3 (unlisted)"
                    .to_owned(),
                "\"title\" is longer than the 128 bytes Steam takes".to_owned(),
                "\"description\" is longer than the 8000 bytes Steam takes".to_owned(),
                "\"changenote\" is longer than the 8000 bytes Steam takes".to_owned(),
            ]
        );
    }

    #[test]
    fn what_is_missing_is_named() {
        let folder = Folder::new();
        let script = folder.script(r#""contentfolder" "gone" "previewfile" "gone.png""#);
        let problems = check(&script).unwrap_err();
        let found = messages(&problems);
        assert_eq!(found.first(), Some(&"there is no \"appid\""));
        assert!(
            found
                .iter()
                .any(|message| message.starts_with("contentfolder ")
                    && message.contains(" cannot be read: "))
        );
        assert!(
            found
                .iter()
                .any(|message| message.starts_with("previewfile ")
                    && message.contains(" cannot be read: "))
        );
        let other = folder.put("workshop/other.vdf", br#""AppBuild" { "AppID" "480" }"#);
        assert_eq!(
            messages(&check(&other).unwrap_err()),
            ["there is no \"workshopitem\" block"]
        );
        drop(folder.put("workshop/folder.png/inside", b""));
        let foldered = folder.script(r#""appid" "480" "previewfile" "folder.png""#);
        let not_file = check(&foldered).unwrap_err();
        assert!(
            messages(&not_file)
                .iter()
                .any(|message| message.ends_with(" is not a file")),
            "{not_file:?}"
        );
    }

    #[test]
    fn the_copy_has_absolute_paths_and_the_rest_as_written() {
        let folder = sound();
        let home = tempfile::tempdir().unwrap();
        let script = folder.script(&format!("{SOUND} \"description\" \"Green.\""));
        let copy = prepare(home.path(), &script, 480).unwrap();
        assert_eq!(
            copy,
            home.path()
                .join("workshop")
                .join("480")
                .join("workshop_item.vdf")
        );
        let document = scripts::read(&copy).unwrap();
        let item = document.block("workshopitem").unwrap();
        let absolute = |relative: &str| {
            upload::written(&path::absolute(folder.root.join("workshop").join(relative)).unwrap())
                .unwrap()
        };
        assert_eq!(item.text("contentfolder"), Some(absolute("hat").as_str()));
        assert_eq!(item.text("previewfile"), Some(absolute("hat.png").as_str()));
        assert_eq!(item.text("title"), Some("A green hat"));
        assert_eq!(item.text("description"), Some("Green."));
        assert_eq!(published(&copy), None);
    }

    #[test]
    fn a_copy_that_cannot_be_made_says_why() {
        let folder = sound();
        let home = tempfile::tempdir().unwrap();
        let other = folder.put("workshop/other.vdf", br#""AppBuild" { "AppID" "480" }"#);
        let error = prepare(home.path(), &other, 480).unwrap_err();
        assert!(
            error
                .to_string()
                .ends_with(": there is no \"workshopitem\" block"),
            "{error}"
        );
        fs::write(home.path().join("workshop"), "a file where the folder goes").unwrap();
        let blocked = prepare(home.path(), &folder.script(SOUND), 480).unwrap_err();
        assert!(matches!(blocked, upload::Error::Io { .. }), "{blocked}");
    }

    #[test]
    fn the_id_steamcmd_writes_into_the_copy_is_read_back() {
        let home = tempfile::tempdir().unwrap();
        let copy = home.path().join("workshop_item.vdf");
        fs::write(
            &copy,
            r#""workshopitem" { "appid" "480" "publishedfileid" "5674" }"#,
        )
        .unwrap();
        assert_eq!(published(&copy), Some(5674));
        fs::write(
            &copy,
            r#""workshopitem" { "appid" "480" "publishedfileid" "0" }"#,
        )
        .unwrap();
        assert_eq!(published(&copy), None);
        assert_eq!(published(&home.path().join("missing.vdf")), None);
    }

    #[test]
    fn a_run_is_published_refused_or_failed() {
        assert_eq!(
            judge(Some(0_i32), "Success.\r\n", Some(5674)),
            Outcome::Published { published: 5674 }
        );
        assert_eq!(
            judge(Some(5_i32), "Cached credentials not found.\r\n", None),
            Outcome::NotLoggedIn("Cached credentials not found.".to_owned())
        );
        assert_eq!(
            judge(Some(0_i32), "", None),
            Outcome::Failed(upload::reasons(Some(0_i32), "", None))
        );
        assert_eq!(
            judge(
                Some(8_i32),
                "ERROR! Failed to update workshop item (Access Denied).\r\n",
                Some(1)
            ),
            Outcome::Failed(vec![
                "Failed to update workshop item (Access Denied).".to_owned()
            ])
        );
    }
}
