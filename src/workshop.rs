//! Workshop items: Valve's `workshopitem` script checked, and the copy steamcmd is given.
//!
//! steamcmd's `workshop_build_item` creates an item when the script names no `publishedfileid`,
//! or `0`, and then writes the new item's ID into the script it was given. steamship gives it a
//! copy in the home with every path made absolute, so the ID is read back from the copy, and the
//! original is left for its author to change.

use std::collections::BTreeMap;
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

/// Why a new item is refused without a preview: Steam makes it anyway, and says nothing.
const GREY: &str = ", as Steam gives an item made without one its grey default picture";

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
    let files = content.as_deref().map_or(0, |content| {
        files_in(content, "contentfolder").unwrap_or_else(|problem| {
            refuse(problem);
            0
        })
    });
    match item.text("previewfile") {
        Some(preview) => {
            if let Some(problem) = unpreviewable(&scripts::resolve(&folder, preview), "previewfile")
            {
                refuse(problem);
            }
        }
        None if published.is_none() => refuse(format!("a new item needs a \"previewfile\"{GREY}")),
        None => {}
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
        if let Some(problem) = item
            .text(key)
            .and_then(|text| too_long(text, &format!("\"{key}\""), longest))
        {
            refuse(problem);
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

/// How many files the item's content folder `content` holds, or why it cannot go up, the folder
/// `said` as it was given.
fn files_in(content: &Path, said: &str) -> Result<usize, String> {
    match count(content) {
        Ok(0) => Err(format!("{said} {} holds no files", content.display())),
        Ok(files) => Ok(files),
        Err(error) => Err(format!(
            "{said} {} cannot be read: {error}",
            content.display()
        )),
    }
}

/// Why the preview image at `preview` cannot go up, if it cannot, the image `said` as it was
/// given.
fn unpreviewable(preview: &Path, said: &str) -> Option<String> {
    match fs::metadata(preview) {
        Ok(metadata) if !metadata.is_file() => {
            Some(format!("{said} {} is not a file", preview.display()))
        }
        Ok(metadata) if metadata.len() > PREVIEW => Some(format!(
            "{said} {} is larger than the 1 MB Steam takes",
            preview.display()
        )),
        Ok(_) => None,
        Err(error) => Some(format!(
            "{said} {} cannot be read: {error}",
            preview.display()
        )),
    }
}

/// That `text`, `said` as it was given, is longer than the `longest` Steam takes, if it is.
fn too_long(text: &str, said: &str, longest: usize) -> Option<String> {
    (text.len() > longest).then(|| format!("{said} is longer than the {longest} bytes Steam takes"))
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
    let slot = item
        .text("publishedfileid")
        .map(str::trim)
        .filter(|published| !matches!(*published, "" | "0"))
        .unwrap_or(NEW);
    write_copy(home, app_id, slot, rewritten).map_err(|error| match error {
        upload::Error::Script { reason, .. } => problem(reason),
        other @ (upload::Error::Io { .. } | upload::Error::Description(_)) => other,
    })
}

/// The folder a script's copy, and steamcmd's console beside it, go into for an item the script
/// gives no ID: one item a run makes, which is then given one.
const NEW: &str = "new";

/// Writes `item`, the inside of a `workshopitem` block, as the script steamcmd is given for
/// `app_id` in `home`, in the item's own folder `slot`, and says where it is. Each item has one,
/// so that uploading several in a row leaves each one's console to read.
fn write_copy(home: &Path, app_id: u32, slot: &str, item: Block) -> Result<PathBuf, upload::Error> {
    let at = |path: &Path| {
        let path = path.to_path_buf();
        move |error| upload::Error::Io { path, error }
    };
    let whole = Block {
        pairs: vec![Pair {
            key: "workshopitem".to_owned(),
            value: Value::Block(item),
        }],
    };
    let items = home.join(ITEMS).join(app_id.to_string()).join(slot);
    fs::create_dir_all(&items).map_err(at(&items))?;
    let copy = items.join("workshop_item.vdf");
    let text = vdf::write(&whole).map_err(|error| upload::Error::Script {
        path: copy.clone(),
        reason: error.to_string(),
    })?;
    fs::write(&copy, text).map_err(at(&copy))?;
    Ok(copy)
}

/// Who may see an item on Steam, as `workshopitem` numbers it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Visibility {
    Public,
    FriendsOnly,
    Private,
    Unlisted,
}

/// The visibilities by the word each is given as, in Valve's order.
const VISIBILITIES: [(&str, Visibility); 4] = [
    ("public", Visibility::Public),
    ("friends-only", Visibility::FriendsOnly),
    ("private", Visibility::Private),
    ("unlisted", Visibility::Unlisted),
];

/// Reads a visibility given as its word, such as `private`.
///
/// # Errors
///
/// When `text` is none of the words.
pub fn visibility(text: &str) -> Result<Visibility, String> {
    VISIBILITIES
        .iter()
        .find(|(word, _)| *word == text)
        .map(|(_, visibility)| *visibility)
        .ok_or_else(|| "it is public, friends-only, private or unlisted".to_owned())
}

impl Visibility {
    /// The number `workshopitem` gives it as.
    const fn number(self) -> &'static str {
        match self {
            Self::Public => "0",
            Self::FriendsOnly => "1",
            Self::Private => "2",
            Self::Unlisted => "3",
        }
    }
}

/// An item given by its fields rather than by a script.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fields {
    pub app_id: u32,
    /// The item it updates; none when it makes a new one.
    pub published: Option<u64>,
    pub content: PathBuf,
    pub preview: Option<PathBuf>,
    pub title: String,
    pub description: Option<String>,
    pub change_note: Option<String>,
    /// Who may see it; none leaves an existing item as it is on Steam.
    pub visibility: Option<Visibility>,
}

/// Checks the item `fields` give, and says every problem at once.
///
/// # Errors
///
/// Every problem found.
pub fn check_fields(fields: &Fields) -> Result<Item, Vec<String>> {
    let mut problems = Vec::new();
    let files = files_in(&fields.content, "--content").unwrap_or_else(|problem| {
        problems.push(problem);
        0
    });
    if let Some(problem) = fields
        .preview
        .as_deref()
        .and_then(|preview| unpreviewable(preview, "--preview"))
    {
        problems.push(problem);
    }
    if fields.published.is_none() && fields.preview.is_none() {
        problems.push(format!("a new item needs --preview{GREY}"));
    }
    if fields.published.is_none() && fields.visibility.is_none() {
        problems
            .push("a new item needs --visibility, which steamship never chooses for it".to_owned());
    }
    for (text, said, longest) in [
        (Some(&fields.title), "--title", TITLE),
        (fields.description.as_ref(), "--description", TEXT),
        (fields.change_note.as_ref(), "--change-note", TEXT),
    ] {
        if let Some(problem) = text.and_then(|text| too_long(text, said, longest)) {
            problems.push(problem);
        }
    }
    if problems.is_empty() {
        Ok(Item {
            app_id: fields.app_id,
            published: fields.published,
            content: Some(fields.content.clone()),
            files,
        })
    } else {
        Err(problems)
    }
}

/// Whether `name`, an item's name in a game's record, may name the item's folder.
///
/// It may hold letters, digits, `-`, `_` and `.`, not first, so that it stays one folder inside
/// the app's in steamship's home, whatever the system.
///
/// # Errors
///
/// When it may not, saying why.
pub fn named(name: &str) -> Result<(), String> {
    if name.is_empty() {
        return Err("--name is empty".to_owned());
    }
    let kept = name
        .chars()
        .all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.'));
    if !kept || name.starts_with('.') {
        return Err(format!(
            "--name {name} may hold only letters, digits, '-', '_' and '.', and not begin with '.'"
        ));
    }
    Ok(())
}

/// Writes the script steamcmd is given for the item `fields` give, named `name`, into its folder
/// in `home`, with its paths made absolute, and says where it is.
///
/// # Errors
///
/// When a path is not valid Unicode, or the script cannot be written.
pub fn prepare_fields(home: &Path, fields: &Fields, name: &str) -> Result<PathBuf, upload::Error> {
    let path = |given: &Path| -> Result<String, upload::Error> {
        let absolute = path::absolute(given).map_err(|error| upload::Error::Io {
            path: given.to_path_buf(),
            error,
        })?;
        upload::written(&absolute).map_err(|reason| upload::Error::Script {
            path: given.to_path_buf(),
            reason,
        })
    };
    let mut item = Block::default();
    let mut put = |key: &str, value: String| {
        item.pairs.push(Pair {
            key: key.to_owned(),
            value: Value::Text(value),
        });
    };
    put("appid", fields.app_id.to_string());
    if let Some(published) = fields.published {
        put("publishedfileid", published.to_string());
    }
    put("contentfolder", path(&fields.content)?);
    if let Some(preview) = &fields.preview {
        put("previewfile", path(preview)?);
    }
    if let Some(visibility) = fields.visibility {
        put("visibility", visibility.number().to_owned());
    }
    put("title", fields.title.clone());
    if let Some(description) = &fields.description {
        put("description", description.clone());
    }
    if let Some(change_note) = &fields.change_note {
        put("changenote", change_note.clone());
    }
    write_copy(home, fields.app_id, name, item)
}

/// A game's items by the name it gives each, kept in a file in its repository that steamship
/// writes, so that a run from any machine updates the item an earlier one made.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Record {
    pub app_id: u32,
    pub items: BTreeMap<String, u64>,
}

impl Record {
    /// Reads a record from its `text`: an object with the `app` and the `items`, each name's
    /// item ID.
    ///
    /// # Errors
    ///
    /// When the text is not such an object.
    pub fn read(text: &str) -> Result<Self, String> {
        let file: serde_json::Value =
            serde_json::from_str(text).map_err(|error| format!("not JSON: {error}"))?;
        let app_id = file
            .get("app")
            .and_then(serde_json::Value::as_u64)
            .and_then(|app| u32::try_from(app).ok())
            .filter(|app| *app > 0)
            .ok_or("\"app\" is not an app ID")?;
        let listed = file
            .get("items")
            .and_then(serde_json::Value::as_object)
            .ok_or("there is no \"items\" object")?;
        let mut items = BTreeMap::new();
        for (name, published) in listed {
            let published = published
                .as_u64()
                .filter(|published| *published > 0)
                .ok_or_else(|| format!("{name} is not given an item's ID"))?;
            let _: Option<u64> = items.insert(name.clone(), published);
        }
        Ok(Self { app_id, items })
    }

    /// The record as its file holds it.
    #[must_use]
    pub fn text(&self) -> String {
        let file = serde_json::json!({"app": self.app_id, "items": self.items});
        let mut text = serde_json::to_string_pretty(&file).unwrap_or_default();
        text.push('\n');
        text
    }
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
        let script = folder.script(r#""appid" "0" "contentfolder" "hat" "publishedfileid" "5674""#);
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
    fn a_new_items_script_needs_a_preview_and_an_updates_does_not() {
        let folder = sound();
        let new = folder.script(r#""appid" "1" "contentfolder" "hat""#);
        assert_eq!(
            messages(&check(&new).unwrap_err()),
            [format!("a new item needs a \"previewfile\"{GREY}").as_str()]
        );
        let update = folder.script(r#""appid" "1" "contentfolder" "hat" "publishedfileid" "5674""#);
        assert_eq!(check(&update).map(|item| item.published), Ok(Some(5674)));
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
                .join("new")
                .join("workshop_item.vdf")
        );
        let update = folder.script(&format!("{SOUND} \"publishedfileid\" \"5674\""));
        assert_eq!(
            prepare(home.path(), &update, 480).unwrap(),
            home.path().join("workshop/480/5674/workshop_item.vdf"),
            "an item with an ID has a folder of its own"
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

    fn fields(folder: &Folder) -> Fields {
        Fields {
            app_id: 480,
            published: None,
            content: folder.root.join("workshop").join("hat"),
            preview: Some(folder.root.join("workshop").join("hat.png")),
            title: "A green hat".to_owned(),
            description: Some("Green.".to_owned()),
            change_note: Some("1.2".to_owned()),
            visibility: Some(Visibility::Private),
        }
    }

    #[test]
    fn a_visibility_is_given_by_its_word_and_written_as_valves_number() {
        for (word, number) in [
            ("public", "0"),
            ("friends-only", "1"),
            ("private", "2"),
            ("unlisted", "3"),
        ] {
            assert_eq!(visibility(word).map(Visibility::number), Ok(number));
        }
        assert_eq!(
            visibility("Public"),
            Err("it is public, friends-only, private or unlisted".to_owned())
        );
    }

    #[test]
    fn an_item_given_by_its_fields_is_checked_as_a_script_is() {
        let folder = sound();
        let item = check_fields(&fields(&folder)).unwrap();
        assert_eq!(
            item,
            Item {
                app_id: 480,
                published: None,
                content: Some(folder.root.join("workshop").join("hat")),
                files: 2,
            }
        );
        let at_most = Fields {
            title: "t".repeat(128),
            description: Some("d".repeat(8000)),
            change_note: Some("c".repeat(8000)),
            ..fields(&folder)
        };
        assert_eq!(check_fields(&at_most).map(|taken| taken.files), Ok(2));
        let updated = Fields {
            published: Some(5674),
            preview: None,
            description: None,
            change_note: None,
            visibility: None,
            ..fields(&folder)
        };
        assert_eq!(check_fields(&updated).unwrap().published, Some(5674));
    }

    #[test]
    fn a_new_item_is_refused_without_a_preview_and_an_update_keeps_the_one_it_has() {
        let folder = sound();
        let unpictured = Fields {
            preview: None,
            ..fields(&folder)
        };
        assert_eq!(
            check_fields(&unpictured).unwrap_err(),
            [format!("a new item needs --preview{GREY}")]
        );
        let updated = Fields {
            published: Some(5674),
            ..unpictured
        };
        assert_eq!(check_fields(&updated).unwrap().published, Some(5674));
    }

    #[test]
    fn every_problem_with_an_items_fields_is_named_at_once() {
        let folder = sound();
        drop(folder.put("workshop/empty/.keep", b""));
        fs::remove_file(folder.root.join("workshop/empty/.keep")).unwrap();
        let big = folder.put("workshop/big.png", &vec![0_u8; 1024 * 1024 + 1]);
        let empty = folder.root.join("workshop").join("empty");
        let wrong = Fields {
            content: empty.clone(),
            preview: Some(big.clone()),
            title: "t".repeat(129),
            description: Some("d".repeat(8001)),
            change_note: Some("c".repeat(8001)),
            visibility: None,
            ..fields(&folder)
        };
        assert_eq!(
            check_fields(&wrong).unwrap_err(),
            [
                format!("--content {} holds no files", empty.display()),
                format!(
                    "--preview {} is larger than the 1 MB Steam takes",
                    big.display()
                ),
                "a new item needs --visibility, which steamship never chooses for it".to_owned(),
                "--title is longer than the 128 bytes Steam takes".to_owned(),
                "--description is longer than the 8000 bytes Steam takes".to_owned(),
                "--change-note is longer than the 8000 bytes Steam takes".to_owned(),
            ]
        );
    }

    #[test]
    fn the_script_written_from_fields_has_each_one_given_and_absolute_paths() {
        let folder = sound();
        let home = tempfile::tempdir().unwrap();
        let copy = prepare_fields(home.path(), &fields(&folder), "green-hat").unwrap();
        assert_eq!(
            copy,
            home.path().join("workshop/480/green-hat/workshop_item.vdf")
        );
        let absolute = |relative: &str| {
            upload::written(&path::absolute(folder.root.join("workshop").join(relative)).unwrap())
                .unwrap()
        };
        let document = scripts::read(&copy).unwrap();
        let item = document.block("workshopitem").unwrap();
        let pairs: Vec<(&str, &str)> = item
            .pairs
            .iter()
            .map(|pair| (pair.key.as_str(), item.text(&pair.key).unwrap()))
            .collect();
        assert_eq!(
            pairs,
            [
                ("appid", "480"),
                ("contentfolder", absolute("hat").as_str()),
                ("previewfile", absolute("hat.png").as_str()),
                ("visibility", "2"),
                ("title", "A green hat"),
                ("description", "Green."),
                ("changenote", "1.2"),
            ]
        );
        let updated = Fields {
            published: Some(5674),
            preview: None,
            description: None,
            change_note: None,
            visibility: None,
            ..fields(&folder)
        };
        let again = prepare_fields(home.path(), &updated, "green-hat").unwrap();
        assert_eq!(again, copy, "an item keeps its folder");
        let rewritten = scripts::read(&again).unwrap();
        let keys: Vec<&str> = rewritten
            .block("workshopitem")
            .unwrap()
            .pairs
            .iter()
            .map(|pair| pair.key.as_str())
            .collect();
        assert_eq!(keys, ["appid", "publishedfileid", "contentfolder", "title"]);
        assert_eq!(published(&again), Some(5674));
    }

    #[test]
    fn an_item_is_named_by_letters_digits_dashes_underscores_and_dots_not_first() {
        for taken in ["example", "Green-hat_2", "mod.v2", "7"] {
            assert_eq!(named(taken), Ok(()), "{taken}");
        }
        assert_eq!(named(""), Err("--name is empty".to_owned()));
        for refused in ["../hats", "a/b", "a\\b", ".hidden", "two words", "h\u{e4}t"] {
            assert_eq!(
                named(refused),
                Err(format!(
                    "--name {refused} may hold only letters, digits, '-', '_' and '.', and not \
                     begin with '.'"
                )),
                "{refused}"
            );
        }
    }

    #[test]
    fn an_items_file_is_read_and_written_by_name() {
        let text = "{\n  \"app\": 480,\n  \"items\": {\n    \"example\": 777,\n    \"hats\": 5674\n  }\n}\n";
        let record = Record::read(text).unwrap();
        assert_eq!(record.app_id, 480);
        assert_eq!(
            record.items,
            BTreeMap::from([("example".to_owned(), 777), ("hats".to_owned(), 5674)])
        );
        assert_eq!(record.text(), text);
        for (unread, why) in [
            ("{", "not JSON"),
            (r#"{"items": {}}"#, "\"app\" is not an app ID"),
            (r#"{"app": 0, "items": {}}"#, "\"app\" is not an app ID"),
            (
                r#"{"app": 4294967296, "items": {}}"#,
                "\"app\" is not an app ID",
            ),
            (r#"{"app": 480}"#, "there is no \"items\" object"),
            (
                r#"{"app": 480, "items": {"a": "777"}}"#,
                "a is not given an item's ID",
            ),
            (
                r#"{"app": 480, "items": {"a": 0}}"#,
                "a is not given an item's ID",
            ),
        ] {
            let error = Record::read(unread).unwrap_err();
            assert!(error.starts_with(why), "{unread}: {error}");
        }
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
