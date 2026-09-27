//! Valve's app and depot build scripts, read into what steamship needs from them.
//!
//! Paths follow steamcmd, as tried: the app script's `ContentRoot` and depot script names are
//! relative to the app script, and `LocalPath` is relative to the content root. A depot script's
//! own `ContentRoot` is relative to the app's, or to the app script when it has none, never to
//! the depot script.

use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

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
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileMapping {
    pub local_path: String,
    pub depot_path: String,
    pub recursive: bool,
}

/// Reads the app script at `path` and every depot script it names.
///
/// # Errors
///
/// Every problem that stops the scripts from being understood, all of them rather than the first.
pub fn load(path: &Path) -> Result<AppScript, Vec<Problem>> {
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
    match (depot_id, content_root) {
        (Some(depot_id), Some(content_root)) if problems.is_empty() => Ok(DepotScript {
            path,
            depot_id,
            content_root,
            mappings,
            exclusions,
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
    if parsed.is_none() {
        problems.push(Problem::new(
            path,
            format!("\"{key}\" \"{text}\" is not a number"),
        ));
    }
    parsed
}

fn parent(path: &Path) -> PathBuf {
    path.parent()
        .map_or_else(|| PathBuf::from("."), Path::to_path_buf)
}

/// Joins a path written in a script onto `base`.
///
/// Scripts are usually written on Windows, so elsewhere a backslash is read as the separator it
/// was meant as. The result is rebuilt from its parts, so it is shown with the system's own
/// separator throughout, however `base` and the script were written; a trailing separator, which
/// marks a folder in `ContentRoot`, is kept.
#[must_use]
pub fn resolve(base: &Path, written: &str) -> PathBuf {
    let native = native(written);
    let path = Path::new(&native);
    let joined = if path.is_absolute() {
        path.to_path_buf()
    } else {
        base.join(path)
    };
    let rebuilt: PathBuf = joined.components().collect();
    if native.ends_with(['/', '\\']) {
        rebuilt.join("")
    } else {
        rebuilt
    }
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

#[cfg(test)]
mod tests {
    use std::path::MAIN_SEPARATOR;

    use super::*;

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
}
