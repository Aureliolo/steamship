//! What `steamship check` refuses: scripts that would upload the wrong thing, or the right thing
//! somewhere it should not go.

use std::collections::BTreeSet;
use std::fs::{self, File};
use std::io;
use std::path::{Path, PathBuf, is_separator};

use crate::scripts::{self, AppScript, DepotScript, FileMapping, Problem};
#[cfg(unix)]
use crate::unix;
use crate::{pattern, pe};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Report {
    /// Present when the scripts could be read at all.
    pub app_id: Option<u32>,
    /// The branch `SetLive` names, which only Steam can say whether the app has.
    pub set_live: Option<String>,
    pub depots: Vec<DepotFiles>,
    pub problems: Vec<Problem>,
    /// Whether the content the scripts map was checked too, and the depots' files counted.
    pub content: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DepotFiles {
    pub depot_id: u32,
    pub files: BTreeSet<PathBuf>,
    /// How many of the files are debug symbols, which some teams ship on purpose.
    pub debug_symbols: usize,
    /// Each of the files that is a Windows program or library, with the DLLs it imports.
    pub windows: Vec<(PathBuf, pe::Imports)>,
    /// Whether the depot runs an install script, which can install anything.
    pub install_scripts: bool,
}

/// The folders Unity writes beside a build and names as never to be shipped.
const DO_NOT_SHIP: [&str; 2] = [
    "*_BurstDebugInformation_DoNotShip",
    "*_BackUpThisFolder_ButDontShipItWithYourGame",
];

/// Checks the app script at `path`, its depot scripts, and the content they map.
#[must_use]
pub fn check(path: &Path) -> Report {
    checked(path, true)
}

/// Checks the app script at `path` and its depot scripts alone, as can be done before the build
/// they map exists.
#[must_use]
pub fn scripts_only(path: &Path) -> Report {
    checked(path, false)
}

/// Checks every file in the build folder `path`, as before a script maps it: for app `app`, when
/// it is known, which says whose redistributables count.
#[must_use]
pub fn folder(path: &Path, app: Option<u32>) -> Report {
    let mut problems = Vec::new();
    let mut found = Vec::new();
    match walk(path, "*", true, &mut found) {
        Ok(()) if !path.is_dir() => problems.push(Problem::new(path, "is not a folder")),
        Ok(()) => {}
        Err(error) => problems.push(Problem::new(path, format!("cannot be read: {error}"))),
    }
    let files: BTreeSet<PathBuf> = found.into_iter().collect();
    for file in &files {
        content_problems("the build", file, &mut problems);
    }
    for folder in not_to_ship(path, &files) {
        problems.push(Problem::new(
            &folder,
            "is in the build; Unity names this folder as one not to ship",
        ));
    }
    let build = DepotFiles {
        depot_id: 0,
        debug_symbols: files.iter().filter(|file| is_debug_symbols(file)).count(),
        windows: windows_files(&files, &mut problems),
        files,
        install_scripts: false,
    };
    Report {
        app_id: app,
        set_live: None,
        depots: vec![build],
        problems,
        content: true,
    }
}

fn checked(path: &Path, content: bool) -> Report {
    match scripts::load(path) {
        Ok(app) => check_loaded(&app, content),
        Err(problems) => Report {
            app_id: None,
            set_live: None,
            depots: Vec::new(),
            problems,
            content,
        },
    }
}

fn check_loaded(app: &AppScript, content: bool) -> Report {
    let mut problems = Vec::new();
    match app
        .set_live
        .as_deref()
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("default") => problems.push(Problem::new(
            &app.path,
            "\"SetLive\" names \"default\", which steamcmd never sets live; upload --default sets \
             it live while the app is unreleased",
        )),
        Some("public") => problems.push(Problem::new(
            &app.path,
            "\"SetLive\" names \"public\", the Web API's name for the default branch, which \
             steamcmd never sets live; upload --default sets it live while the app is unreleased",
        )),
        _ => {}
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
        .map(|depot| {
            let files = if content {
                files(depot, &mut problems)
            } else {
                mappings(depot, &mut problems);
                BTreeSet::new()
            };
            DepotFiles {
                depot_id: depot.depot_id,
                debug_symbols: files.iter().filter(|file| is_debug_symbols(file)).count(),
                windows: windows_files(&files, &mut problems),
                files,
                install_scripts: !depot.install_scripts.is_empty(),
            }
        })
        .collect();
    Report {
        app_id: Some(app.app_id),
        set_live: app
            .set_live
            .clone()
            .filter(|branch| !branch.trim().is_empty()),
        depots,
        problems,
        content,
    }
}

/// A problem for each of `depot`'s mappings that no content could make right.
fn mappings(depot: &DepotScript, problems: &mut Vec<Problem>) {
    for mapping in &depot.mappings {
        if let Some(message) = unmappable(mapping) {
            problems.push(Problem::new(
                &depot.path,
                format!(
                    "depot {}: LocalPath \"{}\" {message}",
                    depot.depot_id, mapping.local_path
                ),
            ));
        }
    }
}

/// The local path's folder and file name, split where steamcmd splits them.
fn split(mapping: &FileMapping) -> (String, String) {
    let local = scripts::native(&mapping.local_path);
    let (folder, name) = local
        .rsplit_once(is_separator)
        .unwrap_or(("", local.as_str()));
    (folder.to_owned(), name.to_owned())
}

/// Why `mapping` can never be mapped, if it cannot.
fn unmappable(mapping: &FileMapping) -> Option<&'static str> {
    split(mapping).0.contains(['*', '?']).then_some(
        "has a wildcard in a folder name, which steamship cannot map the way steamcmd does",
    )
}

/// The files `depot` maps, after its exclusions, with a problem for everything wrong in them.
fn files(depot: &DepotScript, problems: &mut Vec<Problem>) -> BTreeSet<PathBuf> {
    let root = &depot.content_root;
    if !root.is_dir() {
        let what = if matches!(root.try_exists(), Ok(false)) {
            "does not exist; put the depot's files there, or point ContentRoot at the folder they \
             are in"
        } else {
            "is not a folder; point ContentRoot at the folder the depot's files are in"
        };
        problems.push(Problem::new(
            &depot.path,
            format!(
                "depot {}: ContentRoot {} {what}",
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
        content_problems(&format!("depot {}", depot.depot_id), file, problems);
    }
    for folder in not_to_ship(root, &mapped) {
        problems.push(Problem::new(
            &folder,
            format!(
                "is in depot {}; Unity names this folder as one not to ship",
                depot.depot_id
            ),
        ));
    }
    for script in &depot.install_scripts {
        let path = root.join(scripts::native(script));
        let message = if !path.is_file() {
            format!("is not a file in {}", root.display())
        } else if !mapped.iter().any(|file| same_file(file, &path)) {
            "is not in the depot; map it with a FileMapping too".to_owned()
        } else {
            continue;
        };
        problems.push(Problem::new(
            &depot.path,
            format!(
                "depot {}: InstallScript \"{script}\" {message}",
                depot.depot_id
            ),
        ));
    }
    mapped
}

/// Each folder under `root`, holding one of `files`, that Unity names as not to be shipped.
fn not_to_ship(root: &Path, files: &BTreeSet<PathBuf>) -> BTreeSet<PathBuf> {
    let mut folders = BTreeSet::new();
    for file in files {
        let mut folder = root.to_path_buf();
        let relative = file.strip_prefix(root).unwrap_or(file);
        for part in relative.parent().into_iter().flat_map(Path::components) {
            folder.push(part);
            let name = part.as_os_str().to_string_lossy();
            if DO_NOT_SHIP
                .iter()
                .any(|pattern| pattern::matches(pattern, &name))
            {
                let _new: bool = folders.insert(folder.clone());
                break;
            }
        }
    }
    folders
}

/// Symbols for a debugger: Windows' program databases, a macOS symbol bundle's contents, and
/// Linux's separate debug files.
fn is_debug_symbols(file: &Path) -> bool {
    let extension = |path: &Path, wanted: &str| {
        path.extension()
            .is_some_and(|found| found.eq_ignore_ascii_case(wanted))
    };
    extension(file, "pdb")
        || extension(file, "debug")
        || file.ancestors().any(|folder| extension(folder, "dSYM"))
}

/// Whether two paths name the same file, however each was spelt.
fn same_file(one: &Path, other: &Path) -> bool {
    match (fs::canonicalize(one), fs::canonicalize(other)) {
        (Ok(one), Ok(other)) => one == other,
        _ => false,
    }
}

/// Whether `depot` ships `file`, as its mappings and exclusions take files from its content root,
/// whether or not the file is there yet.
#[must_use]
pub fn ships(depot: &DepotScript, file: &Path) -> bool {
    let root = &depot.content_root;
    let (Some(parent), Some(file_name)) = (file.parent(), file.file_name()) else {
        return false;
    };
    let file_name = file_name.to_string_lossy();
    !excluded(root, file, &depot.exclusions)
        && depot.mappings.iter().any(|mapping| {
            let (folder, name) = split(mapping);
            let folder = root.join(folder);
            unmappable(mapping).is_none()
                && pattern::matches(&name, &file_name)
                && (parent == folder || (mapping.recursive && parent.starts_with(&folder)))
        })
}

fn mapped_by(root: &Path, mapping: &FileMapping) -> Result<Vec<PathBuf>, String> {
    if let Some(message) = unmappable(mapping) {
        return Err(message.to_owned());
    }
    let (folder, name) = split(mapping);
    let mut found = Vec::new();
    walk(&root.join(folder), &name, mapping.recursive, &mut found)
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

/// What is wrong with `file`, which ships in `place`, a depot or a build's folder.
fn content_problems(place: &str, file: &Path, problems: &mut Vec<Problem>) {
    let name = file
        .file_name()
        .map(|name| name.to_string_lossy())
        .unwrap_or_default();
    if name.eq_ignore_ascii_case("steam_appid.txt") {
        problems.push(Problem::new(
            file,
            format!("is in {place}; it is for development only and must not ship"),
        ));
    }
    // Windows file systems have no executable bit, so there the question cannot be asked.
    #[cfg(unix)]
    if let Some(message) = unix::missing_executable_bit(file) {
        problems.push(Problem::new(file, message));
    }
    if name.to_ascii_lowercase().ends_with(".runtimeconfig.json")
        && let Some(message) = runtime_installed_apart(file)
    {
        problems.push(Problem::new(file, message));
    }
}

/// Each of `files` that is a Windows program or library, with the DLLs it imports, and a problem
/// for each that says it is one but cannot be read as one.
fn windows_files(
    files: &BTreeSet<PathBuf>,
    problems: &mut Vec<Problem>,
) -> Vec<(PathBuf, pe::Imports)> {
    let mut found = Vec::new();
    for file in files {
        match File::open(file).and_then(|mut opened| pe::imports(&mut opened)) {
            Ok(Some(imports)) => found.push((file.clone(), imports)),
            Ok(None) => {}
            Err(error) => problems.push(Problem::new(
                file,
                format!("cannot be read as a Windows program or library: {error}"),
            )),
        }
    }
    found
}

/// Why the .NET program whose runtime settings are at `file` cannot run on its own, if it cannot:
/// it names a shared runtime to run on rather than shipping its own.
fn runtime_installed_apart(file: &Path) -> Option<String> {
    let settings = fs::read(file)
        .map_err(|error| error.to_string())
        .and_then(|bytes| {
            serde_json::from_slice::<serde_json::Value>(&bytes).map_err(|error| error.to_string())
        });
    match settings {
        Ok(settings) => {
            let options = settings.get("runtimeOptions")?;
            (options.get("framework").is_some() || options.get("frameworks").is_some()).then(|| {
                "runs on a .NET runtime installed apart from it, which neither Windows nor a \
                 Steamworks Common Redistributable installs; publish the program self-contained"
                    .to_owned()
            })
        }
        Err(error) => Some(format!(
            "cannot be read as .NET's runtime settings: {error}"
        )),
    }
}

#[cfg(test)]
mod tests {
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
        assert!(ships(&depot(&[("windows/*", false)], &[]), &windows));
        assert!(!ships(&depot(&[("windows/*", false)], &[]), &demo));
        assert!(ships(&depot(&[("*", true)], &[]), &demo));
        assert!(!ships(&depot(&[("*", false)], &[]), &demo));
        assert!(ships(&depot(&[("demo/windows/*.exe", false)], &[]), &demo));
        assert!(!ships(&depot(&[("demo/windows/*.pck", false)], &[]), &demo));
        assert!(!ships(&depot(&[("*/windows/*", true)], &[]), &demo));
        assert!(!ships(&depot(&[("*", true)], &["demo/windows/*"]), &demo));
        assert!(!ships(&depot(&[("*", true)], &[]), Path::new("")));
    }
}
