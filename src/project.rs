//! A game's folder as steamship finds it, so that a command run in the game needs nothing named.
//!
//! Its build scripts, records, catalogues and example mods are where steamship expects them, or
//! where its `steamship.toml` says they are. The layout steamship expects is godot-kit's: the build scripts in `steam/`, the base mod's
//! records in `mods/base/data`, every language's catalogue in `game/locale`, the example mods in
//! `modding/`, and the version in `project.godot`. `steamship.toml` names whatever a game keeps
//! elsewhere, the command that builds it, and the one steamship it ships with.

use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};

use serde_json::Value;
use toml::de::Error as TomlError;

use crate::{run, workshop};

/// The file a game names its own layout in, at its root.
pub const FILE: &str = "steamship.toml";

/// Where a game keeps its app build script when its `steamship.toml` names none.
const SCRIPT: &str = "steam/app_build.vdf";

/// Every setting `steamship.toml` takes at its top, and under `[workshop]`.
const SETTINGS: [&str; 7] = [
    "steamship",
    "script",
    "build",
    "records",
    "locale",
    "import",
    "workshop",
];
const WORKSHOP: [&str; 3] = ["mods", "items", "footer"];

/// godot-kit's record types `achievements` reads, and the list a file of steamship's own holds
/// them in.
pub const ACHIEVEMENTS: Records = Records {
    types: &["achievement", "stat", "achievement_set"],
    list: "achievements",
};

/// godot-kit's record type `leaderboards` reads, and the list a file of steamship's own holds them
/// in.
pub const LEADERBOARDS: Records = Records {
    types: &["leaderboard"],
    list: "leaderboards",
};

/// Which of the record files a command reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Records {
    types: &'static [&'static str],
    list: &'static str,
}

/// A path the game gives, and whether `steamship.toml` named it rather than the layout: one the
/// file names must be there.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Given {
    pub path: PathBuf,
    pub named: bool,
}

impl Given {
    const fn laid_out(path: PathBuf) -> Self {
        Self { path, named: false }
    }
}

/// A game, found from a folder inside it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Project {
    /// The game's folder.
    pub root: PathBuf,
    /// Whether the game has a `steamship.toml`, rather than the layout alone.
    pub configured: bool,
    /// The one steamship version the game ships with, when it pins one.
    pub pin: Option<String>,
    /// The app build script.
    pub script: PathBuf,
    /// The program and arguments that build the game into the folders its depots ship; empty
    /// when the content is built some other way.
    pub build: Vec<String>,
    /// The folders and files of records: godot-kit's, or steamship's own.
    pub records: Vec<Given>,
    /// The folder of every language's gettext catalogue.
    pub locale: Given,
    /// Where the achievement editor's import is written, and Steam's icon receipt beside it.
    pub import: PathBuf,
    /// The folder of example mods, each a Workshop item.
    pub mods: Given,
    /// The file keeping each Workshop item's ID by its mod's name.
    pub items: PathBuf,
    /// Text set below every example mod's own description on the Workshop.
    pub footer: Option<String>,
}

/// An example mod, as its Workshop item.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mod {
    /// Its folder's name, which is its name in the items file.
    pub name: String,
    pub folder: PathBuf,
    /// `preview.png` in its folder, when it has one.
    pub preview: Option<PathBuf>,
    pub title: String,
    pub description: String,
    pub version: String,
}

/// The commit a game is shipped from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pushed {
    pub commit: String,
    /// origin's default branch, which is at the same commit.
    pub branch: String,
}

impl Project {
    /// The game `start` is in: the nearest folder holding a `steamship.toml`, or else
    /// `steam/app_build.vdf`.
    ///
    /// # Errors
    ///
    /// When the `steamship.toml` found cannot be read, or says what steamship does not take.
    pub fn find(start: &Path) -> Result<Option<Self>, String> {
        for folder in start.ancestors() {
            let file = folder.join(FILE);
            if file.is_file() {
                let text = fs::read_to_string(&file)
                    .map_err(|error| format!("{}: {error}", file.display()))?;
                return Self::configured(folder, &text)
                    .map(Some)
                    .map_err(|why| format!("{}: {why}", file.display()));
            }
            let script = under(folder, SCRIPT);
            if script.is_file() {
                return Ok(Some(Self::laid_out(folder, script)));
            }
        }
        Ok(None)
    }

    fn laid_out(root: &Path, script: PathBuf) -> Self {
        let scripts = script
            .parent()
            .map_or_else(|| root.to_path_buf(), Path::to_path_buf);
        Self {
            root: root.to_path_buf(),
            configured: false,
            pin: None,
            build: Vec::new(),
            records: vec![Given::laid_out(under(root, "mods/base/data"))],
            locale: Given::laid_out(under(root, "game/locale")),
            import: scripts.join("achievements"),
            mods: Given::laid_out(root.join("modding")),
            items: scripts.join("workshop_items.json"),
            footer: None,
            script,
        }
    }

    /// The game at `root`, as the `steamship.toml` `text` lays it out.
    ///
    /// # Errors
    ///
    /// When the text is not TOML, or names a setting steamship does not take, one of the wrong
    /// kind, or a path outside the game.
    pub fn configured(root: &Path, text: &str) -> Result<Self, String> {
        let table: toml::Table = text
            .parse()
            .map_err(|error: TomlError| error.message().to_owned())?;
        known(&table, &SETTINGS, "")?;
        let script = table
            .get("script")
            .map(|value| inside(root, value, "script"))
            .transpose()?;
        let mut project = Self::laid_out(root, script.unwrap_or_else(|| under(root, SCRIPT)));
        project.configured = true;
        if let Some(value) = table.get("steamship") {
            project.pin = Some(text_of(value, "steamship")?.to_owned());
        }
        if let Some(value) = table.get("build") {
            project.build = texts(value, "build")?;
            if project.build.is_empty() {
                return Err(
                    "`build` is empty: name the program and its arguments, such as \
                            [\"python\", \"tools/build.py\"], or leave it out"
                        .to_owned(),
                );
            }
        }
        if let Some(value) = table.get("records") {
            project.records = paths(root, value, "records")?;
        }
        if let Some(value) = table.get("locale") {
            project.locale = named(inside(root, value, "locale")?);
        }
        if let Some(value) = table.get("import") {
            project.import = inside(root, value, "import")?;
        }
        if let Some(section) = table.get("workshop") {
            let workshop = section
                .as_table()
                .ok_or("`workshop` is not a table: write it as a [workshop] section")?;
            known(workshop, &WORKSHOP, "workshop.")?;
            if let Some(mods) = workshop.get("mods") {
                project.mods = named(inside(root, mods, "workshop.mods")?);
            }
            if let Some(items) = workshop.get("items") {
                project.items = inside(root, items, "workshop.items")?;
            }
            if let Some(footer) = workshop.get("footer") {
                project.footer = Some(text_of(footer, "workshop.footer")?.to_owned());
            }
        }
        Ok(project)
    }

    /// Refuses a steamship other than the one the game pins.
    ///
    /// # Errors
    ///
    /// When the game pins a version and `running` is another.
    pub fn pinned(&self, running: &str) -> Result<(), String> {
        match &self.pin {
            Some(pin) if pin != running => Err(format!(
                "{} pins steamship {pin}, and this is {running}: install {pin}, or change the pin",
                self.root.join(FILE).display()
            )),
            _ => Ok(()),
        }
    }

    /// The folder the build scripts are in.
    #[must_use]
    pub fn scripts(&self) -> &Path {
        self.script.parent().unwrap_or(&self.root)
    }

    /// The snapshot of the app's settings in Steamworks, beside the build scripts.
    #[must_use]
    pub fn snapshot(&self) -> PathBuf {
        self.scripts().join("settings.vdf")
    }

    /// The version the game ships as, from `project.godot`'s `config/version`.
    ///
    /// # Errors
    ///
    /// When there is no `project.godot`, or it names no version.
    pub fn version(&self) -> Result<String, String> {
        let path = self.root.join("project.godot");
        let text = fs::read_to_string(&path).map_err(|error| {
            if error.kind() == io::ErrorKind::NotFound {
                format!(
                    "{} has no project.godot to take the version from: give it with --version",
                    self.root.display()
                )
            } else {
                format!("{}: {error}", path.display())
            }
        })?;
        godot_version(&text).ok_or_else(|| {
            format!(
                "{} names no config/version under [application]: set the game's version there, or \
                 give it with --version",
                path.display()
            )
        })
    }

    /// The record files holding what `kind` reads: each JSON file in a records folder, and each
    /// file named, that holds any, or that cannot be read as JSON, for the command to refuse.
    ///
    /// # Errors
    ///
    /// When `steamship.toml` names records that are not there.
    pub fn records(&self, kind: Records) -> Result<Vec<PathBuf>, String> {
        let mut found = Vec::new();
        for given in &self.records {
            let files = if given.path.is_dir() {
                listed(&given.path, |name| {
                    Path::new(name)
                        .extension()
                        .is_some_and(|extension| extension.eq_ignore_ascii_case("json"))
                })?
            } else if given.path.is_file() {
                vec![given.path.clone()]
            } else if given.named {
                return Err(missing(&given.path, "records"));
            } else {
                Vec::new()
            };
            found.extend(files.into_iter().filter(|file| holds(file, kind)));
        }
        Ok(found)
    }

    /// Every language's gettext catalogue.
    ///
    /// # Errors
    ///
    /// When `steamship.toml` names a locale folder that is not there, or it cannot be read.
    pub fn catalogues(&self) -> Result<Vec<PathBuf>, String> {
        if !self.locale.path.is_dir() {
            return if self.locale.named {
                Err(missing(&self.locale.path, "locale"))
            } else {
                Ok(Vec::new())
            };
        }
        listed(&self.locale.path, |name| {
            Path::new(name)
                .extension()
                .is_some_and(|extension| extension.eq_ignore_ascii_case("po"))
        })
    }

    /// Every language's rich presence file, `rich_presence_<language>.vdf` beside the scripts.
    ///
    /// # Errors
    ///
    /// When the scripts' folder cannot be read.
    pub fn presence(&self) -> Result<Vec<PathBuf>, String> {
        listed(self.scripts(), |name| {
            name.starts_with("rich_presence_")
                && Path::new(name)
                    .extension()
                    .is_some_and(|extension| extension.eq_ignore_ascii_case("vdf"))
        })
    }

    /// Every example mod, a folder holding a godot-kit `manifest.json`, as its Workshop item.
    ///
    /// # Errors
    ///
    /// When `steamship.toml` names a mods folder that is not there, or a manifest cannot be read,
    /// lacks a name, description or version, or a mod's folder name is not one an item may have.
    pub fn mods(&self) -> Result<Vec<Mod>, String> {
        if !self.mods.path.is_dir() {
            return if self.mods.named {
                Err(missing(&self.mods.path, "workshop.mods"))
            } else {
                Ok(Vec::new())
            };
        }
        let mut found = Vec::new();
        for folder in listed(&self.mods.path, |_| true)? {
            let manifest = folder.join("manifest.json");
            if !manifest.is_file() {
                continue;
            }
            let name = folder
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default();
            workshop::named(&name)
                .map_err(|why| format!("{}: the mod's folder {why}", folder.display()))?;
            let text = fs::read_to_string(&manifest)
                .map_err(|error| format!("{}: {error}", manifest.display()))?;
            let read: Value = serde_json::from_str(&text)
                .map_err(|error| format!("{}: not JSON: {error}", manifest.display()))?;
            let field = |key: &str, why: &str| {
                read.get(key)
                    .and_then(Value::as_str)
                    .filter(|value| !value.trim().is_empty())
                    .map(str::to_owned)
                    .ok_or_else(|| format!("{}: gives no \"{key}\", {why}", manifest.display()))
            };
            let title = field("name", "which is the item's title")?;
            let own = field("description", "which is the item's description")?;
            let version = field("version", "which is the item's change note")?;
            let description = match &self.footer {
                Some(footer) => format!("{own}\n\n{footer}"),
                None => own,
            };
            let preview = folder.join("preview.png");
            found.push(Mod {
                name,
                preview: preview.is_file().then_some(preview),
                folder,
                title,
                description,
                version,
            });
        }
        Ok(found)
    }
}

/// Refuses any key of `table` not among `settings`, each shown after `prefix`.
fn known(table: &toml::Table, settings: &[&str], prefix: &str) -> Result<(), String> {
    table
        .keys()
        .find(|key| !settings.contains(&key.as_str()))
        .map_or(Ok(()), |key| {
            Err(format!(
                "`{prefix}{key}` is no setting steamship takes; it takes {}",
                settings
                    .iter()
                    .map(|setting| format!("`{prefix}{setting}`"))
                    .collect::<Vec<_>>()
                    .join(", ")
            ))
        })
}

fn text_of<'value>(value: &'value toml::Value, key: &str) -> Result<&'value str, String> {
    value
        .as_str()
        .ok_or_else(|| format!("`{key}` is not text: write it in quotes"))
}

/// A list of text, as `build` and `records` take it.
fn texts(value: &toml::Value, key: &str) -> Result<Vec<String>, String> {
    let wrong = || format!("`{key}` is not a list of text: write it as [\"one\", \"two\"]");
    value
        .as_array()
        .ok_or_else(wrong)?
        .iter()
        .map(|each| each.as_str().map(str::to_owned).ok_or_else(wrong))
        .collect()
}

/// One path or a list of them, each inside `root`.
fn paths(root: &Path, value: &toml::Value, key: &str) -> Result<Vec<Given>, String> {
    let written = if value.is_str() {
        vec![text_of(value, key)?.to_owned()]
    } else {
        texts(value, key)?
    };
    written
        .iter()
        .map(|each| inside(root, &toml::Value::String(each.clone()), key).map(named))
        .collect()
}

const fn named(path: PathBuf) -> Given {
    Given { path, named: true }
}

/// The path `value` names, relative to `root`, which it must stay inside: a game's settings travel
/// with it to every machine it is checked out on.
fn inside(root: &Path, value: &toml::Value, key: &str) -> Result<PathBuf, String> {
    let written = text_of(value, key)?;
    let path = Path::new(written);
    if written.trim().is_empty() {
        return Err(format!(
            "`{key}` is empty: name a path inside the game's folder"
        ));
    }
    if path
        .components()
        .any(|part| !matches!(part, Component::Normal(_) | Component::CurDir))
        || written.split(['/', '\\']).any(|part| part == "..")
    {
        return Err(format!(
            "`{key}` is \"{written}\", which is not inside the game's folder: name a path \
             relative to {FILE}, without .."
        ));
    }
    Ok(under(root, written))
}

/// `relative` under `root`, split at either slash, so that it is shown with the system's own
/// separator throughout however it was written.
fn under(root: &Path, relative: &str) -> PathBuf {
    relative
        .split(['/', '\\'])
        .filter(|part| !part.is_empty() && *part != ".")
        .fold(root.to_path_buf(), |path, part| path.join(part))
}

fn missing(path: &Path, key: &str) -> String {
    format!(
        "`{key}` names {}, which is not there: make it, or change `{key}` in {FILE}",
        path.display()
    )
}

/// The entries of `folder` whose names `wanted` takes, in order of name.
fn listed(folder: &Path, wanted: impl Fn(&str) -> bool) -> Result<Vec<PathBuf>, String> {
    let entries = fs::read_dir(folder).map_err(|error| format!("{}: {error}", folder.display()))?;
    let mut found = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|error| format!("{}: {error}", folder.display()))?;
        if wanted(&entry.file_name().to_string_lossy()) {
            found.push(entry.path());
        }
    }
    found.sort();
    Ok(found)
}

/// Whether the JSON file at `path` holds records `kind` reads, either as godot-kit's list of them
/// or as steamship's own file. One that cannot be read is kept, for the command to name why.
fn holds(path: &Path, kind: Records) -> bool {
    let Ok(text) = fs::read_to_string(path) else {
        return true;
    };
    match serde_json::from_str::<Value>(&text) {
        Ok(Value::Array(records)) => records.iter().any(|record| {
            record
                .get("type")
                .and_then(Value::as_str)
                .is_some_and(|kind_of| kind.types.contains(&kind_of))
        }),
        Ok(Value::Object(file)) => file.contains_key(kind.list),
        Ok(_) => false,
        Err(_) => true,
    }
}

/// The `config/version` a `project.godot`'s `text` gives under `[application]`.
#[must_use]
pub fn godot_version(text: &str) -> Option<String> {
    let mut section = "";
    for line in text.lines().map(str::trim) {
        if line.starts_with('[') {
            section = line;
            continue;
        }
        if section == "[application]"
            && let Some(value) = line.strip_prefix("config/version=")
        {
            let version = value.strip_prefix('"')?.strip_suffix('"')?;
            return (!version.is_empty()).then(|| version.to_owned());
        }
    }
    None
}

/// The commit the game at `root` is at, refused unless its tree holds nothing that commit does
/// not and origin's default branch is at it too: so that what goes up is what was pushed.
///
/// # Errors
///
/// When Git cannot be run, the game is not in a repository, its tree holds changes, or origin
/// cannot be asked or is at another commit.
pub fn pushed(root: &Path) -> Result<Pushed, String> {
    unchanged(root)?;
    let commit = git(root, &["rev-parse", "HEAD"])?;
    let remote = git(root, &["ls-remote", "--symref", "origin", "HEAD"])?;
    let (branch, at) = remote_head(&remote).ok_or_else(|| {
        "origin names no default branch, which is the one a game is shipped from".to_owned()
    })?;
    if at != commit {
        return Err(format!(
            "HEAD is at {}, and origin's {branch} at {}: push or pull until they are one, so what \
             goes up is the commit origin's {branch} holds",
            short(&commit),
            short(&at)
        ));
    }
    Ok(Pushed { commit, branch })
}

/// Refuses a tree at `root` holding anything its commit does not, untracked files among it; what
/// Git ignores, such as the build, is left out.
///
/// # Errors
///
/// When Git cannot be run, `root` is in no repository, or the tree holds changes.
pub fn unchanged(root: &Path) -> Result<(), String> {
    let changed = git(root, &["status", "--porcelain"])?;
    if changed.is_empty() {
        return Ok(());
    }
    let files: Vec<&str> = changed
        .lines()
        .map(|line| line.get(3..).unwrap_or(line))
        .collect();
    Err(format!(
        "the game's folder holds what its commit does not ({}): commit it or put it away, so \
         what goes up is a pushed commit and the build it makes",
        files.join(", ")
    ))
}

/// The first twelve digits of a commit, as Git shows one.
#[must_use]
pub fn short(commit: &str) -> &str {
    commit.get(..12).unwrap_or(commit)
}

/// The default branch and its commit, from what `git ls-remote --symref origin HEAD` printed.
fn remote_head(printed: &str) -> Option<(String, String)> {
    let mut branch = None;
    let mut commit = None;
    for line in printed.lines() {
        if let Some(target) = line.strip_prefix("ref: ") {
            let (reference, _) = target.split_once('\t')?;
            branch = Some(reference.strip_prefix("refs/heads/")?.to_owned());
            continue;
        }
        if let Some((sha, "HEAD")) = line.split_once('\t') {
            commit = Some(sha.to_owned());
        }
    }
    Some((branch?, commit?))
}

/// What Git printed for `args` in `root`, without the line ending it closed with; the start is
/// kept, as `status --porcelain` begins a line with a space for a file changed but not staged.
fn git(root: &Path, args: &[&str]) -> Result<String, String> {
    let output = run::output(
        run::command("git")
            .args(args)
            .current_dir(root)
            // A remote asked for a password would wait for one; the credential helper answers.
            .env("GIT_TERMINAL_PROMPT", "0"),
    )
    .map_err(|error| {
        if error.kind() == io::ErrorKind::NotFound {
            "Git is not installed, or not on the PATH, and steamship ships only a pushed commit"
                .to_owned()
        } else {
            format!("Git could not be run ({error}), and steamship ships only a pushed commit")
        }
    })?;
    if !output.status.success() {
        let said = String::from_utf8_lossy(&output.stderr);
        let said = said
            .lines()
            .find(|line| !line.trim().is_empty())
            .unwrap_or("");
        return Err(format!(
            "git {} failed in {}: {}",
            args.join(" "),
            root.display(),
            said.trim()
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout)
        .trim_end()
        .to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn configured(text: &str) -> Result<Project, String> {
        Project::configured(Path::new("game"), text)
    }

    #[test]
    fn a_game_with_no_file_is_laid_out_as_godot_kit_lays_one_out() {
        let project = Project::laid_out(
            Path::new("game"),
            Path::new("game/steam/app_build.vdf").to_path_buf(),
        );
        assert!(!project.configured);
        assert_eq!(project.snapshot(), Path::new("game/steam/settings.vdf"));
        assert_eq!(project.import, Path::new("game/steam/achievements"));
        assert_eq!(project.items, Path::new("game/steam/workshop_items.json"));
        assert_eq!(project.build, Vec::<String>::new());
        assert_eq!(project.pin, None);
    }

    #[test]
    fn the_file_names_what_the_game_keeps_elsewhere() {
        let project = configured(
            r#"
            steamship = "1.2.3"
            script = "ship/app.vdf"
            build = ["python", "tools/build.py"]
            records = ["data", "steam/achievements.json"]
            locale = "mods/base/locale"
            import = "ship/editor"

            [workshop]
            mods = "examples"
            items = "ship/items.json"
            footer = "Free to copy."
            "#,
        )
        .unwrap();
        assert!(project.configured);
        assert_eq!(project.pin.as_deref(), Some("1.2.3"));
        assert_eq!(project.script, Path::new("game/ship/app.vdf"));
        assert_eq!(project.snapshot(), Path::new("game/ship/settings.vdf"));
        assert_eq!(project.build, ["python", "tools/build.py"]);
        assert_eq!(project.import, Path::new("game/ship/editor"));
        assert_eq!(project.items, Path::new("game/ship/items.json"));
        assert_eq!(project.footer.as_deref(), Some("Free to copy."));
        assert_eq!(
            project.records,
            [
                named(PathBuf::from("game/data")),
                named(PathBuf::from("game/steam/achievements.json"))
            ]
        );
        assert_eq!(
            project.locale,
            named(PathBuf::from("game/mods/base/locale"))
        );
        assert_eq!(project.mods, named(PathBuf::from("game/examples")));
    }

    #[test]
    fn a_file_saying_what_steamship_does_not_take_is_refused_naming_it() {
        for (text, said) in [
            (
                "builds = []",
                "`builds` is no setting steamship takes; it takes `steamship`, `script`, `build`, \
                 `records`, `locale`, `import`, `workshop`",
            ),
            (
                "[workshop]\nmod = \"x\"",
                "`workshop.mod` is no setting steamship takes; it takes `workshop.mods`, \
                 `workshop.items`, `workshop.footer`",
            ),
            (
                "steamship = 1",
                "`steamship` is not text: write it in quotes",
            ),
            (
                "build = \"python tools/build.py\"",
                "`build` is not a list of text: write it as [\"one\", \"two\"]",
            ),
            (
                "build = []",
                "`build` is empty: name the program and its arguments, such as [\"python\", \
                 \"tools/build.py\"], or leave it out",
            ),
            (
                "records = [\"../other/data\"]",
                "`records` is \"../other/data\", which is not inside the game's folder: name a \
                 path relative to steamship.toml, without ..",
            ),
            (
                "script = \"\"",
                "`script` is empty: name a path inside the game's folder",
            ),
            (
                "workshop = \"modding\"",
                "`workshop` is not a table: write it as a [workshop] section",
            ),
        ] {
            assert_eq!(configured(text).unwrap_err(), said, "{text}");
        }
        assert_eq!(
            configured("records = [\"/data\"]").map(|_| ()),
            Err(
                "`records` is \"/data\", which is not inside the game's folder: name a path \
                 relative to steamship.toml, without .."
                    .to_owned()
            )
        );
        assert_ne!(configured("steamship = ").map(|_| ()), Ok(()));
    }

    #[test]
    fn a_pinned_game_refuses_any_other_steamship() {
        let project = configured("steamship = \"1.2.3\"").unwrap();
        assert_eq!(project.pinned("1.2.3"), Ok(()));
        assert_eq!(
            project.pinned("1.2.4").unwrap_err(),
            format!(
                "{} pins steamship 1.2.3, and this is 1.2.4: install 1.2.3, or change the pin",
                Path::new("game").join("steamship.toml").display()
            )
        );
        assert_eq!(configured("").unwrap().pinned("9.9.9"), Ok(()));
    }

    #[test]
    fn the_version_is_project_godots_under_application() {
        let text = "[gd_resource]\nconfig/version=\"9\"\n\n[application]\nconfig/name=\"Inn\"\n\
                    config/version=\"0.3.1\"\n\n[display]\n";
        assert_eq!(godot_version(text).as_deref(), Some("0.3.1"));
        assert_eq!(godot_version("[application]\nconfig/version=\"\"\n"), None);
        assert_eq!(godot_version("[application]\nconfig/name=\"Inn\"\n"), None);
        assert_eq!(godot_version("[display]\nconfig/version=\"1\"\n"), None);
    }

    #[test]
    fn origins_default_branch_is_read_from_ls_remote() {
        assert_eq!(
            remote_head("ref: refs/heads/main\tHEAD\nabc123\tHEAD\n"),
            Some(("main".to_owned(), "abc123".to_owned()))
        );
        assert_eq!(remote_head("abc123\tHEAD\n"), None);
        assert_eq!(remote_head(""), None);
    }

    #[test]
    fn a_commit_is_shown_by_its_first_twelve_digits() {
        assert_eq!(short("0123456789abcdef"), "0123456789ab");
        assert_eq!(short("abc"), "abc");
    }

    /// A game laid out in a folder of its own, with `files` written into it.
    fn game(files: &[(&str, &str)], toml: Option<&str>) -> (tempfile::TempDir, Project) {
        let folder = tempfile::tempdir().unwrap();
        for (relative, contents) in files {
            let path = under(folder.path(), relative);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, contents).unwrap();
        }
        let project = toml.map_or_else(
            || Project::laid_out(folder.path(), under(folder.path(), SCRIPT)),
            |text| Project::configured(folder.path(), text).unwrap(),
        );
        (folder, project)
    }

    fn names(paths: &[PathBuf]) -> Vec<String> {
        paths
            .iter()
            .map(|path| path.file_name().unwrap().to_string_lossy().into_owned())
            .collect()
    }

    #[test]
    fn only_the_record_files_holding_what_a_command_reads_are_given_it() {
        let (_folder, project) = game(
            &[
                (
                    "mods/base/data/achievements.json",
                    r#"[{"type": "achievement"}, {"type": "stat"}]"#,
                ),
                ("mods/base/data/boards.json", r#"[{"type": "leaderboard"}]"#),
                (
                    "mods/base/data/items.json",
                    r#"[{"type": "item"}, "loose"]"#,
                ),
                ("mods/base/data/options.json", r#"{"volume": 1}"#),
                ("mods/base/data/number.json", "3"),
                ("mods/base/data/broken.json", "[{"),
                ("mods/base/data/notes.txt", "[]"),
            ],
            None,
        );
        assert_eq!(
            names(&project.records(ACHIEVEMENTS).unwrap()),
            ["achievements.json", "broken.json"],
            "and one that does not read, for the command to say why"
        );
        assert_eq!(
            names(&project.records(LEADERBOARDS).unwrap()),
            ["boards.json", "broken.json"]
        );
    }

    #[test]
    fn a_record_file_that_cannot_be_read_is_kept_for_the_command_to_say_why() {
        assert!(holds(Path::new("no-such-records.json"), ACHIEVEMENTS));
        assert!(holds(Path::new("no-such-records.json"), LEADERBOARDS));
    }

    #[test]
    fn steamships_own_files_named_in_the_file_are_given_by_the_list_they_hold() {
        let (_folder, project) = game(
            &[
                (
                    "steam/achievements.json",
                    r#"{"app": 1, "achievements": []}"#,
                ),
                ("steam/boards.json", r#"{"leaderboards": []}"#),
            ],
            Some(r#"records = ["steam/achievements.json", "steam/boards.json"]"#),
        );
        assert_eq!(
            names(&project.records(ACHIEVEMENTS).unwrap()),
            ["achievements.json"]
        );
        assert_eq!(
            names(&project.records(LEADERBOARDS).unwrap()),
            ["boards.json"]
        );
    }

    #[test]
    fn a_folder_the_file_names_must_be_there_and_one_the_layout_expects_need_not() {
        let (_folder, laid_out) = game(&[], None);
        assert_eq!(laid_out.records(ACHIEVEMENTS), Ok(Vec::new()));
        assert_eq!(laid_out.catalogues(), Ok(Vec::new()));
        assert_eq!(laid_out.mods(), Ok(Vec::new()));

        let (folder, named) = game(
            &[],
            Some("records = \"data\"\nlocale = \"po\"\n[workshop]\nmods = \"examples\"\n"),
        );
        for (found, key, path) in [
            (named.records(ACHIEVEMENTS).map(|_| ()), "records", "data"),
            (named.catalogues().map(|_| ()), "locale", "po"),
            (named.mods().map(|_| ()), "workshop.mods", "examples"),
        ] {
            assert_eq!(
                found,
                Err(format!(
                    "`{key}` names {}, which is not there: make it, or change `{key}` in \
                     steamship.toml",
                    folder.path().join(path).display()
                ))
            );
        }
    }

    #[test]
    fn every_catalogue_and_rich_presence_file_is_found_in_order_of_name() {
        let (_folder, project) = game(
            &[
                ("game/locale/fr.po", ""),
                ("game/locale/de.po", ""),
                ("game/locale/notes.md", ""),
                ("steam/app_build.vdf", ""),
                ("steam/rich_presence_german.vdf", ""),
                ("steam/rich_presence_english.vdf", ""),
                ("steam/rich_presence_notes.txt", ""),
            ],
            None,
        );
        assert_eq!(names(&project.catalogues().unwrap()), ["de.po", "fr.po"]);
        assert_eq!(
            names(&project.presence().unwrap()),
            ["rich_presence_english.vdf", "rich_presence_german.vdf"]
        );
    }

    #[test]
    fn each_example_mod_is_its_manifests_item_with_the_footer_and_its_own_preview() {
        let (folder, project) = game(
            &[
                (
                    "modding/example/manifest.json",
                    r#"{"name": "Example", "description": "Shows it all.", "version": "1.0.0"}"#,
                ),
                ("modding/example/preview.png", "picture"),
                (
                    "modding/plain/manifest.json",
                    r#"{"name": "Plain", "description": "Shows little.", "version": "2"}"#,
                ),
                ("modding/notes/readme.md", "no manifest, no item"),
            ],
            Some("[workshop]\nfooter = \"Free to copy.\"\n"),
        );
        let modding = folder.path().join("modding");
        assert_eq!(
            project.mods().unwrap(),
            [
                Mod {
                    name: "example".to_owned(),
                    folder: modding.join("example"),
                    preview: Some(modding.join("example").join("preview.png")),
                    title: "Example".to_owned(),
                    description: "Shows it all.\n\nFree to copy.".to_owned(),
                    version: "1.0.0".to_owned(),
                },
                Mod {
                    name: "plain".to_owned(),
                    folder: modding.join("plain"),
                    preview: None,
                    title: "Plain".to_owned(),
                    description: "Shows little.\n\nFree to copy.".to_owned(),
                    version: "2".to_owned(),
                },
            ]
        );
    }

    #[test]
    fn a_manifest_lacking_what_its_item_needs_is_refused_naming_it() {
        for (manifest, said) in [
            (
                r#"{"description": "D", "version": "1"}"#,
                "gives no \"name\", which is the item's title",
            ),
            (
                r#"{"name": "N", "description": " ", "version": "1"}"#,
                "gives no \"description\", which is the item's description",
            ),
            (
                r#"{"name": "N", "description": "D", "version": 1}"#,
                "gives no \"version\", which is the item's change note",
            ),
            ("{", "not JSON: "),
        ] {
            let (_folder, project) = game(&[("modding/example/manifest.json", manifest)], None);
            let refused = project.mods().unwrap_err();
            assert!(refused.contains(said), "{manifest}: {refused}");
        }
        let (_folder, badly_named) = game(
            &[(
                "modding/my mod/manifest.json",
                r#"{"name": "N", "description": "D", "version": "1"}"#,
            )],
            None,
        );
        assert!(
            badly_named
                .mods()
                .unwrap_err()
                .ends_with("the mod's folder --name my mod may hold only letters, digits, '-', '_' and '.', and not begin with '.'"),
        );
    }

    #[test]
    fn the_version_comes_from_project_godot_and_its_absence_says_what_to_do() {
        let (folder, project) = game(
            &[("project.godot", "[application]\nconfig/version=\"1.2\"\n")],
            None,
        );
        assert_eq!(project.version(), Ok("1.2".to_owned()));
        fs::write(folder.path().join("project.godot"), "[application]\n").unwrap();
        assert!(
            project
                .version()
                .unwrap_err()
                .ends_with("names no config/version under [application]: set the game's version there, or give it with --version")
        );
        fs::remove_file(folder.path().join("project.godot")).unwrap();
        fs::create_dir_all(folder.path().join("project.godot")).unwrap();
        assert!(project.version().unwrap_err().starts_with(&format!(
            "{}: ",
            folder.path().join("project.godot").display()
        )));
        fs::remove_dir(folder.path().join("project.godot")).unwrap();
        assert_eq!(
            project.version(),
            Err(format!(
                "{} has no project.godot to take the version from: give it with --version",
                folder.path().display()
            ))
        );
    }

    #[test]
    fn a_game_is_found_from_any_folder_inside_it_its_file_before_its_layout() {
        let (folder, _) = game(
            &[("steam/app_build.vdf", ""), ("mods/base/data/a.json", "[]")],
            None,
        );
        let inner = folder.path().join("mods").join("base");
        let found = Project::find(&inner).unwrap().unwrap();
        assert_eq!(found.root, folder.path());
        assert!(!found.configured);

        fs::write(folder.path().join(FILE), "script = \"ship/app.vdf\"\n").unwrap();
        let configured = Project::find(&inner).unwrap().unwrap();
        assert!(configured.configured);
        assert_eq!(
            configured.script,
            folder.path().join("ship").join("app.vdf")
        );

        fs::write(folder.path().join(FILE), "nonsense = 1\n").unwrap();
        assert!(
            Project::find(&inner)
                .unwrap_err()
                .starts_with(&format!("{}: ", folder.path().join(FILE).display()))
        );

        let elsewhere = tempfile::tempdir().unwrap();
        assert_eq!(Project::find(elsewhere.path()), Ok(None));
    }
}
