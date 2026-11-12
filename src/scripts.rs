//! Valve's app and depot build scripts, read into what steamship needs from them.
//!
//! Paths follow steamcmd, as tried: the app script's `ContentRoot` and depot script names are
//! relative to the app script, and `LocalPath` is relative to the content root. A depot script's
//! own `ContentRoot` is relative to the app's, or to the app script when it has none, never to
//! the depot script. It also says which files a depot's mappings take, and where they install
//! them.

use std::fmt;
use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf, is_separator};

use crate::pattern;
use crate::vdf::{self, Block, Value};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Problem {
    pub file: PathBuf,
    pub message: String,
}

impl Problem {
    pub fn new<Message>(file: &Path, message: Message) -> Self
    where
        Message: Into<String>,
    {
        Self {
            file: file.to_path_buf(),
            message: message.into(),
        }
    }
}

impl fmt::Display for Problem {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.file.display(), self.message)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppScript {
    pub path: PathBuf,
    pub app_id: u32,
    pub set_live: Option<String>,
    pub preview: Option<String>,
    pub local: Option<String>,
    pub depots: Vec<DepotScript>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DepotScript {
    /// The file the depot is described in: its own script, or the app script when inline.
    pub path: PathBuf,
    pub depot_id: u32,
    pub content_root: PathBuf,
    pub mappings: Vec<FileMapping>,
    pub exclusions: Vec<String>,
    /// Each `InstallScript`, a path from the content root to a file the depot also maps.
    pub install_scripts: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileMapping {
    pub local_path: String,
    pub depot_path: String,
    pub recursive: bool,
}

impl DepotScript {
    /// Whether the depot ships `file`, as its mappings and exclusions take files from its content
    /// root, whether or not the file is there yet.
    #[must_use]
    pub fn ships(&self, file: &Path) -> bool {
        let root = &self.content_root;
        let (Some(parent), Some(file_name)) = (file.parent(), file.file_name()) else {
            return false;
        };
        let file_name = file_name.to_string_lossy();
        !self.excludes(file)
            && self.mappings.iter().any(|mapping| {
                let (folder, name) = mapping.split();
                let folder = root.join(folder);
                mapping.unmappable().is_none()
                    && pattern::matches(&name, &file_name)
                    && (parent == folder || (mapping.recursive && parent.starts_with(&folder)))
            })
    }

    /// Whether the depot installs a file of its content at `path`, a path from the install folder.
    pub(crate) fn installs(&self, path: &str) -> bool {
        let wanted = parts(path);
        self.mappings.iter().any(|mapping| {
            let Some(rest) = wanted.strip_prefix(parts(&mapping.depot_path).as_slice()) else {
                return false;
            };
            let Some((name, folders)) = rest.split_last() else {
                return false;
            };
            let (folder, pattern) = mapping.split();
            let file = folders
                .iter()
                .fold(self.content_root.join(folder), |file, part| file.join(part))
                .join(name);
            (folders.is_empty() || mapping.recursive)
                && pattern::matches(&pattern, name)
                && !self.excludes(&file)
                && file.is_file()
        })
    }

    /// Where the depot's mappings install `file`, one of its content's, each as its folders and
    /// name from the install folder.
    pub(crate) fn installed(&self, file: &Path) -> Vec<Vec<String>> {
        self.mappings
            .iter()
            .filter(|mapping| mapping.unmappable().is_none())
            .filter_map(|mapping| {
                let (folder, pattern) = mapping.split();
                let rest: Vec<String> = file
                    .strip_prefix(self.content_root.join(folder))
                    .ok()?
                    .components()
                    .map(|part| part.as_os_str().to_string_lossy().into_owned())
                    .collect();
                let (name, folders) = rest.split_last()?;
                if !(folders.is_empty() || mapping.recursive) || !pattern::matches(&pattern, name) {
                    return None;
                }
                let mut at = parts(&mapping.depot_path);
                at.extend(rest);
                Some(at)
            })
            .collect()
    }

    /// A `FileExclusion` with a folder in it is matched against the path from the content root. One
    /// without is matched against the file name at any depth, as Valve's `"*.pdb"` example is.
    pub(crate) fn excludes(&self, file: &Path) -> bool {
        let relative = file.strip_prefix(&self.content_root).unwrap_or(file);
        let relative = relative.to_string_lossy().replace('\\', "/");
        let name = file
            .file_name()
            .map(|name| name.to_string_lossy())
            .unwrap_or_default();
        self.exclusions.iter().any(|exclusion| {
            let exclusion = exclusion.replace('\\', "/");
            if exclusion.contains('/') {
                pattern::matches(&exclusion, &relative)
            } else {
                pattern::matches(&exclusion, &name)
            }
        })
    }
}

impl FileMapping {
    /// Why the mapping can never be mapped, if it cannot.
    pub(crate) fn unmappable(&self) -> Option<&'static str> {
        self.split().0.contains(['*', '?']).then_some(
            "has a wildcard in a folder name, which steamship cannot map as steamcmd does. Name the \
             folder without one",
        )
    }

    /// The files the mapping takes under `root`.
    pub(crate) fn files_under(&self, root: &Path) -> Result<Vec<PathBuf>, String> {
        if let Some(message) = self.unmappable() {
            return Err(format!(" {message}"));
        }
        let (folder, name) = self.split();
        let mut found = Vec::new();
        walk(&root.join(folder), &name, self.recursive, &mut found)
            .map_err(|error| format!(": {error}. Fix or remove that path"))?;
        Ok(found)
    }

    /// The local path's folder and file name, split where steamcmd splits them.
    fn split(&self) -> (String, String) {
        let local = native(&self.local_path);
        let (folder, name) = local
            .rsplit_once(is_separator)
            .unwrap_or(("", local.as_str()));
        (folder.to_owned(), name.to_owned())
    }
}

/// The app the app script at `path` names, or none while it names none Steamworks gave, as before
/// the app exists, or cannot be read.
#[must_use]
pub fn app_named(path: &Path) -> Option<u32> {
    read(path)
        .ok()?
        .block("AppBuild")?
        .text("AppID")?
        .trim()
        .parse()
        .ok()
        .filter(|app| *app != 0)
}

/// Reads the app script at `path` and every depot script it names.
///
/// # Errors
///
/// Every problem that stops the scripts from being understood, all of them rather than the first.
pub fn load(path: &Path) -> Result<AppScript, Vec<Problem>> {
    if matches!(fs::metadata(path), Err(error) if error.kind() == io::ErrorKind::NotFound) {
        return Err(vec![Problem::new(
            path,
            "does not exist. Name the app's build script, such as steam/app_build.vdf",
        )]);
    }
    let document = read(path).map_err(|problem| vec![problem])?;
    let Some(app) = document.block("AppBuild") else {
        return Err(vec![Problem::new(path, "there is no \"AppBuild\" block")]);
    };
    let mut problems = Vec::new();
    let app_id = number(app, "AppID", path, &mut problems);
    let folder = parent(path);
    let content_root = app.text("ContentRoot").map(|root| resolve(&folder, root));
    let mut depots = Vec::new();
    match app.block("Depots") {
        Some(listed) if !listed.pairs.is_empty() => {
            for pair in &listed.pairs {
                match depot(
                    path,
                    &folder,
                    &pair.key,
                    &pair.value,
                    content_root.as_deref(),
                ) {
                    Ok(script) => depots.push(script),
                    Err(mut found) => problems.append(&mut found),
                }
            }
        }
        _ => problems.push(Problem::new(path, "\"Depots\" is missing or empty")),
    }
    match app_id {
        Some(app_id) if problems.is_empty() => Ok(AppScript {
            path: path.to_path_buf(),
            app_id,
            set_live: app.text("SetLive").map(str::to_owned),
            preview: app.text("Preview").map(str::to_owned),
            local: app.text("Local").map(str::to_owned),
            depots,
        }),
        _ => Err(problems),
    }
}

/// The script at `path`, parsed.
///
/// # Errors
///
/// When it does not exist, cannot be read, or is not `KeyValues` text.
pub fn read(path: &Path) -> Result<Block, Problem> {
    let text = fs::read_to_string(path).map_err(|error| {
        Problem::new(
            path,
            if error.kind() == io::ErrorKind::NotFound {
                "does not exist".to_owned()
            } else {
                format!("cannot be read: {error}")
            },
        )
    })?;
    vdf::parse(&text).map_err(|error| Problem::new(path, error.to_string()))
}

fn depot(
    app_path: &Path,
    app_folder: &Path,
    listed_id: &str,
    value: &Value,
    app_root: Option<&Path>,
) -> Result<DepotScript, Vec<Problem>> {
    let (path, block) = match value {
        Value::Block(block) => (app_path.to_path_buf(), block.clone()),
        Value::Text(name) => {
            let path = resolve(app_folder, name);
            if !path.is_file() {
                return Err(vec![Problem::new(
                    app_path,
                    format!(
                        "depot {listed_id} names {}, which does not exist",
                        path.display()
                    ),
                )]);
            }
            let document = read(&path).map_err(|problem| vec![problem])?;
            let Some(block) = document
                .block("DepotBuild")
                .or_else(|| document.block("DepotBuildConfig"))
            else {
                return Err(vec![Problem::new(
                    &path,
                    "there is no \"DepotBuild\" block",
                )]);
            };
            (path, block.clone())
        }
    };
    let mut problems = Vec::new();
    let Ok(listed) = listed_id.parse::<u32>() else {
        return Err(vec![Problem::new(
            app_path,
            format!("depot ID \"{listed_id}\" in \"Depots\" is not a number"),
        )]);
    };
    let depot_id = if block.get("DepotID").is_some() {
        number(&block, "DepotID", &path, &mut problems)
    } else {
        Some(listed)
    };
    if let Some(own) = depot_id.filter(|&own| own != listed) {
        problems.push(Problem::new(
            &path,
            format!("says DepotID {own}, but the app script lists it as {listed}"),
        ));
    }
    let content_root = match (block.text("ContentRoot"), app_root) {
        (Some(own), _) => Some(resolve(app_root.unwrap_or(app_folder), own)),
        (None, Some(root)) => Some(root.to_path_buf()),
        (None, None) => None,
    };
    if content_root.is_none() {
        problems.push(Problem::new(
            &path,
            format!("depot {listed} has no ContentRoot, in the app script or its own"),
        ));
    }
    let mappings = mappings(&block, &path, &mut problems);
    let mut exclusions = Vec::new();
    for exclusion in block.all("FileExclusion") {
        match exclusion {
            Value::Text(pattern) => exclusions.push(pattern.clone()),
            // Ignoring it would upload the very files it was written to keep out.
            Value::Block(_) => problems.push(Problem::new(
                &path,
                "a \"FileExclusion\" is a block, where a pattern belongs",
            )),
        }
    }
    let mut install_scripts = Vec::new();
    for script in block.all("InstallScript") {
        match script {
            Value::Text(file) => install_scripts.push(file.clone()),
            Value::Block(_) => problems.push(Problem::new(
                &path,
                "an \"InstallScript\" is a block, where a path belongs",
            )),
        }
    }
    match (depot_id, content_root) {
        (Some(depot_id), Some(content_root)) if problems.is_empty() => Ok(DepotScript {
            path,
            depot_id,
            content_root,
            mappings,
            exclusions,
            install_scripts,
        }),
        _ => Err(problems),
    }
}

fn mappings(block: &Block, path: &Path, problems: &mut Vec<Problem>) -> Vec<FileMapping> {
    let mut found = Vec::new();
    let earlier = problems.len();
    for value in block.all("FileMapping") {
        let Value::Block(mapping) = value else {
            problems.push(Problem::new(path, "a \"FileMapping\" is not a block"));
            continue;
        };
        match (mapping.text("LocalPath"), mapping.text("DepotPath")) {
            (Some(local_path), Some(depot_path)) => found.push(FileMapping {
                local_path: local_path.to_owned(),
                depot_path: depot_path.to_owned(),
                recursive: mapping.text("Recursive") == Some("1"),
            }),
            _ => problems.push(Problem::new(
                path,
                "a \"FileMapping\" needs both \"LocalPath\" and \"DepotPath\"",
            )),
        }
    }
    if found.is_empty() && problems.len() == earlier {
        problems.push(Problem::new(path, "the depot has no \"FileMapping\""));
    }
    found
}

fn number(block: &Block, key: &str, path: &Path, problems: &mut Vec<Problem>) -> Option<u32> {
    let Some(text) = block.text(key) else {
        problems.push(Problem::new(path, format!("\"{key}\" is missing")));
        return None;
    };
    let parsed = text.trim().parse().ok();
    match parsed {
        None => problems.push(Problem::new(
            path,
            format!("\"{key}\" \"{text}\" is not a number"),
        )),
        Some(0) => problems.push(Problem::new(
            path,
            format!("\"{key}\" is 0, which Steamworks never gives. Put in the ID it shows for it"),
        )),
        Some(_) => return parsed,
    }
    None
}

fn parent(path: &Path) -> PathBuf {
    path.parent()
        .map_or_else(|| PathBuf::from("."), Path::to_path_buf)
}

/// Joins a path written in a script onto `base`.
///
/// Scripts are usually written on Windows, so elsewhere a backslash is read as the separator it
/// was meant as. The result is rebuilt from its parts, so it is shown with the system's own
/// separator throughout, however `base` and the script were written. A trailing separator, which
/// marks a folder in `ContentRoot`, is kept.
///
/// A `..` goes with the folder before it wherever the system would take both out anyway, so a
/// person reads `export` rather than `steam/../export`.
#[must_use]
pub fn resolve(base: &Path, written: &str) -> PathBuf {
    let native = native(written);
    let path = Path::new(&native);
    let joined = if path.is_absolute() {
        path.to_path_buf()
    } else {
        base.join(path)
    };
    let mut rebuilt = PathBuf::new();
    for component in joined.components() {
        let folded = component == Component::ParentDir && cancels(&rebuilt) && rebuilt.pop();
        if !folded {
            rebuilt.push(component);
        }
    }
    if rebuilt.as_os_str().is_empty() {
        rebuilt.push(".");
    }
    if native.ends_with(['/', '\\']) {
        rebuilt.join("")
    } else {
        rebuilt
    }
}

/// Whether a `..` after `folder` leads where `folder`'s own parent is.
///
/// Windows takes a `..` out with the name before it, whatever is on disk. Elsewhere `..` leads
/// to the parent of where a symbolic link points, so it cancels only a real folder.
fn cancels(folder: &Path) -> bool {
    matches!(folder.components().next_back(), Some(Component::Normal(_)))
        && (cfg!(windows) || fs::symlink_metadata(folder).is_ok_and(|found| found.is_dir()))
}

/// A script path with the host's separator. On Windows both already work.
#[must_use]
pub fn native(written: &str) -> String {
    if cfg!(windows) {
        written.to_owned()
    } else {
        written.replace('\\', "/")
    }
}

/// The folders and file name of `written`, a path from the install folder or a depot's.
pub(crate) fn parts(written: &str) -> Vec<String> {
    native(written)
        .split(is_separator)
        .filter(|part| !part.is_empty() && *part != ".")
        .map(str::to_owned)
        .collect()
}

pub(crate) fn walk(
    folder: &Path,
    name: &str,
    recursive: bool,
    found: &mut Vec<PathBuf>,
) -> io::Result<()> {
    if !folder.is_dir() {
        return Ok(());
    }
    for entry in fs::read_dir(folder).map_err(naming(folder))? {
        let entry = entry.map_err(naming(folder))?;
        let path = entry.path();
        if entry.file_type().map_err(naming(&path))?.is_dir() {
            if recursive {
                walk(&path, name, recursive, found)?;
            }
            continue;
        }
        // The kind above is the entry's own, so a link to a folder is never walked into and a
        // loop of links cannot make the walk endless. A link to a file counts as that file.
        if path.metadata().map_err(naming(&path))?.is_file()
            && pattern::matches(name, &entry.file_name().to_string_lossy())
        {
            found.push(path);
        }
    }
    Ok(())
}

/// An error that names `path` as what cannot be read, since the system's reason alone does not.
fn naming(path: &Path) -> impl FnOnce(io::Error) -> io::Error {
    move |error| {
        io::Error::new(
            error.kind(),
            format!("{} cannot be read: {error}", path.display()),
        )
    }
}

#[cfg(test)]
mod tests {
    use std::path::MAIN_SEPARATOR;

    #[cfg(unix)]
    use crate::unix::make_link;

    use super::*;

    fn depot(mappings: &[(&str, bool)], exclusions: &[&str]) -> DepotScript {
        DepotScript {
            path: PathBuf::from("steam/depot.vdf"),
            depot_id: 1_000_051,
            content_root: PathBuf::from("export"),
            mappings: mappings
                .iter()
                .map(|(local_path, recursive)| FileMapping {
                    local_path: (*local_path).to_owned(),
                    depot_path: ".".to_owned(),
                    recursive: *recursive,
                })
                .collect(),
            exclusions: exclusions
                .iter()
                .map(|&exclusion| exclusion.to_owned())
                .collect(),
            install_scripts: Vec::new(),
        }
    }

    #[test]
    fn a_depot_ships_a_file_its_mappings_take_and_its_exclusions_leave() {
        let windows = Path::new("export").join("windows").join("sample.exe");
        let demo = Path::new("export")
            .join("demo")
            .join("windows")
            .join("sample.exe");
        assert!(depot(&[("windows/*", false)], &[]).ships(&windows));
        assert!(!depot(&[("windows/*", false)], &[]).ships(&demo));
        assert!(depot(&[("*", true)], &[]).ships(&demo));
        assert!(!depot(&[("*", false)], &[]).ships(&demo));
        assert!(depot(&[("demo/windows/*.exe", false)], &[]).ships(&demo));
        assert!(!depot(&[("demo/windows/*.pck", false)], &[]).ships(&demo));
        assert!(!depot(&[("*/windows/*", true)], &[]).ships(&demo));
        assert!(!depot(&[("*", true)], &["demo/windows/*"]).ships(&demo));
        assert!(!depot(&[("*", true)], &[]).ships(Path::new("")));
    }

    #[test]
    fn a_file_is_installed_where_each_mapping_taking_it_puts_it() {
        let top = Path::new("export").join("windows").join("sample.exe");
        let inside = Path::new("export")
            .join("windows")
            .join("bin")
            .join("sample.exe");
        let at = |path: &[&str]| path.iter().map(|&part| part.to_owned()).collect::<Vec<_>>();
        assert_eq!(
            depot(&[("windows/*", false)], &[]).installed(&top),
            [at(&["sample.exe"])]
        );
        assert_eq!(
            depot(&[("windows/*", false)], &[]).installed(&inside),
            Vec::<Vec<String>>::new()
        );
        assert_eq!(
            depot(&[("windows/*", true)], &[]).installed(&inside),
            [at(&["bin", "sample.exe"])]
        );
        assert_eq!(
            depot(&[("windows/*.pck", true)], &[]).installed(&top),
            Vec::<Vec<String>>::new()
        );
    }

    fn separators(path: &Path) -> Vec<char> {
        let mut found: Vec<char> = path
            .to_string_lossy()
            .chars()
            .filter(|character| ['/', '\\'].contains(character))
            .collect();
        found.dedup();
        found
    }

    #[test]
    fn a_resolved_path_has_the_systems_separator_throughout() {
        for (base, written) in [
            ("scratch/steam", "depot_build_windows.vdf"),
            ("scratch/steam", "..\\export\\windows"),
            ("scratch/./steam", "./depot.vdf"),
        ] {
            let resolved = resolve(Path::new(base), written);
            assert_eq!(
                separators(&resolved),
                [MAIN_SEPARATOR],
                "{}",
                resolved.display()
            );
        }
        assert_eq!(
            resolve(Path::new("scratch/./steam"), "./depot.vdf"),
            ["scratch", "steam", "depot.vdf"]
                .iter()
                .collect::<PathBuf>()
        );
    }

    #[test]
    fn a_folder_written_with_a_trailing_separator_keeps_it() {
        for written in ["../export/", "..\\export\\"] {
            let resolved = resolve(Path::new("steam"), written);
            assert!(
                resolved.to_string_lossy().ends_with(MAIN_SEPARATOR),
                "{}",
                resolved.display()
            );
            assert_eq!(separators(&resolved), [MAIN_SEPARATOR]);
        }
        assert!(
            !resolve(Path::new("steam"), "../export")
                .to_string_lossy()
                .ends_with(MAIN_SEPARATOR)
        );
    }

    #[test]
    fn the_app_a_script_names_is_read_alone_and_none_while_it_names_none_steam_gave() {
        let folder = tempfile::tempdir().unwrap();
        let named = |text: &str| {
            let path = folder.path().join("app_build.vdf");
            fs::write(&path, text).unwrap();
            app_named(&path)
        };
        assert_eq!(
            named(r#""AppBuild" { "AppID" " 480 " "Depots" { "0" "depot.vdf" } }"#),
            Some(480)
        );
        for text in [
            r#""AppBuild" { "AppID" "0" }"#,
            r#""AppBuild" { "AppID" "Spacewar" }"#,
            r#""AppBuild" { "Desc" "no app" }"#,
            r#""DepotBuild" { "AppID" "480" }"#,
            "\"AppBuild\" {",
        ] {
            assert_eq!(named(text), None, "{text}");
        }
        assert_eq!(app_named(&folder.path().join("missing.vdf")), None);
    }

    #[test]
    fn a_parent_after_a_real_folder_is_shown_as_where_it_leads() {
        let project = tempfile::tempdir().unwrap();
        let steam = project.path().join("steam");
        fs::create_dir_all(&steam).unwrap();
        assert_eq!(
            resolve(&steam, "..\\export\\"),
            project.path().join("export").join("")
        );
        assert_eq!(resolve(&steam, "../.."), project.path().parent().unwrap());
        assert_eq!(resolve(Path::new("src"), "..").as_os_str(), ".");
    }

    #[cfg(unix)]
    #[test]
    fn a_parent_after_a_symbolic_link_is_kept_since_it_leads_past_where_the_link_points() {
        let project = tempfile::tempdir().unwrap();
        let elsewhere = tempfile::tempdir().unwrap();
        let steam = project.path().join("steam");
        make_link(&elsewhere.path().to_string_lossy(), &steam).unwrap();
        assert_eq!(
            resolve(&steam, "../export"),
            steam.join("..").join("export")
        );
    }
}
