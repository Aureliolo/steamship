//! What `steamship check` refuses: scripts that would upload the wrong thing, or the right thing
//! somewhere it should not go.

use std::collections::BTreeSet;
use std::fs;
use std::io;
use std::path::{Path, PathBuf, is_separator};

use crate::pattern;
use crate::scripts::{self, AppScript, DepotScript, FileMapping, Problem};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Report {
    /// Present when the scripts could be read at all.
    pub app_id: Option<u32>,
    pub depots: Vec<DepotFiles>,
    pub problems: Vec<Problem>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DepotFiles {
    pub depot_id: u32,
    pub files: BTreeSet<PathBuf>,
}

/// Checks the app script at `path`, its depot scripts, and the content they map.
#[must_use]
pub fn check(path: &Path) -> Report {
    match scripts::load(path) {
        Ok(app) => check_loaded(&app),
        Err(problems) => Report {
            app_id: None,
            depots: Vec::new(),
            problems,
        },
    }
}

fn check_loaded(app: &AppScript) -> Report {
    let mut problems = Vec::new();
    if app
        .set_live
        .as_deref()
        .is_some_and(|branch| branch.eq_ignore_ascii_case("default"))
    {
        problems.push(Problem::new(
            &app.path,
            "\"SetLive\" names \"default\", which Valve only allows from the Steamworks site",
        ));
    }
    if app
        .preview
        .as_deref()
        .is_some_and(|preview| !matches!(preview.trim(), "" | "0"))
    {
        problems.push(Problem::new(
            &app.path,
            "\"Preview\" is set in the script; pass --preview",
        ));
    }
    if app
        .local
        .as_deref()
        .is_some_and(|local| !local.trim().is_empty())
    {
        problems.push(Problem::new(
            &app.path,
            "\"Local\" is set, which sends the build to a local content server, not to Steam",
        ));
    }
    let depots = app
        .depots
        .iter()
        .map(|depot| DepotFiles {
            depot_id: depot.depot_id,
            files: files(depot, &mut problems),
        })
        .collect();
    Report {
        app_id: Some(app.app_id),
        depots,
        problems,
    }
}

/// The files `depot` maps, after its exclusions, with a problem for everything wrong in them.
fn files(depot: &DepotScript, problems: &mut Vec<Problem>) -> BTreeSet<PathBuf> {
    let root = &depot.content_root;
    if !root.is_dir() {
        problems.push(Problem::new(
            &depot.path,
            format!(
                "depot {}: ContentRoot {} is not a folder",
                depot.depot_id,
                root.display()
            ),
        ));
        return BTreeSet::new();
    }
    let mut mapped = BTreeSet::new();
    for mapping in &depot.mappings {
        match mapped_by(root, mapping) {
            Ok(found) if found.is_empty() => problems.push(Problem::new(
                &depot.path,
                format!(
                    "depot {}: LocalPath \"{}\" matches no files in {}",
                    depot.depot_id,
                    mapping.local_path,
                    root.display()
                ),
            )),
            Ok(found) => mapped.extend(found),
            Err(message) => problems.push(Problem::new(
                &depot.path,
                format!(
                    "depot {}: LocalPath \"{}\" {message}",
                    depot.depot_id, mapping.local_path
                ),
            )),
        }
    }
    mapped.retain(|file| !excluded(root, file, &depot.exclusions));
    for file in &mapped {
        content_problems(depot.depot_id, file, problems);
    }
    mapped
}

fn mapped_by(root: &Path, mapping: &FileMapping) -> Result<Vec<PathBuf>, String> {
    let local = scripts::native(&mapping.local_path);
    let (folder, name) = local
        .rsplit_once(is_separator)
        .unwrap_or(("", local.as_str()));
    if folder.contains(['*', '?']) {
        return Err(
            "has a wildcard in a folder name, which steamship cannot map the way steamcmd \
                    does"
                .to_owned(),
        );
    }
    let mut found = Vec::new();
    walk(&root.join(folder), name, mapping.recursive, &mut found)
        .map_err(|error| format!("cannot be read: {error}"))?;
    Ok(found)
}

fn walk(folder: &Path, name: &str, recursive: bool, found: &mut Vec<PathBuf>) -> io::Result<()> {
    if !folder.is_dir() {
        return Ok(());
    }
    for entry in fs::read_dir(folder)? {
        let entry = entry?;
        let path = entry.path();
        if entry.file_type()?.is_dir() {
            if recursive {
                walk(&path, name, recursive, found)?;
            }
            continue;
        }
        // The kind above is the entry's own, so a link to a folder is never walked into and a
        // loop of links cannot make the walk endless; a link to a file counts as that file.
        if path.metadata()?.is_file()
            && pattern::matches(name, &entry.file_name().to_string_lossy())
        {
            found.push(path);
        }
    }
    Ok(())
}

/// A `FileExclusion` with a folder in it is matched against the path from the content root; one
/// without is matched against the file name at any depth, as Valve's `"*.pdb"` example is.
fn excluded(root: &Path, file: &Path, exclusions: &[String]) -> bool {
    let relative = file.strip_prefix(root).unwrap_or(file);
    let relative = relative.to_string_lossy().replace('\\', "/");
    let name = file
        .file_name()
        .map(|name| name.to_string_lossy())
        .unwrap_or_default();
    exclusions.iter().any(|exclusion| {
        let exclusion = exclusion.replace('\\', "/");
        if exclusion.contains('/') {
            pattern::matches(&exclusion, &relative)
        } else {
            pattern::matches(&exclusion, &name)
        }
    })
}

fn content_problems(depot_id: u32, file: &Path, problems: &mut Vec<Problem>) {
    let name = file
        .file_name()
        .map(|name| name.to_string_lossy())
        .unwrap_or_default();
    if name.eq_ignore_ascii_case("steam_appid.txt") {
        problems.push(Problem::new(
            file,
            format!("is in depot {depot_id}; it is for development only and must not ship"),
        ));
    }
    // Windows file systems have no executable bit, so there the question cannot be asked.
    #[cfg(unix)]
    if let Some(message) = crate::unix::missing_executable_bit(file) {
        problems.push(Problem::new(file, message));
    }
}
