//! What `steamship check` refuses: scripts that would upload the wrong thing, or the right thing
//! somewhere it should not go.

use std::collections::BTreeSet;
use std::fs::{self, File};
use std::io;
use std::path::{Path, PathBuf, is_separator};

use crate::scripts::{self, AppScript, DepotScript, FileMapping, Problem};
#[cfg(unix)]
use crate::unix;
use crate::vdf::Block;
use crate::{magic, pattern, pe, settings, vdf};

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
    checked(path, true, None)
}

/// Checks as [`check`] does, and the content against `settings`, the app's settings snapshot,
/// when there is one.
#[must_use]
pub fn against(path: &Path, settings: Option<&Block>) -> Report {
    checked(path, true, settings)
}

/// Checks the app script at `path` and its depot scripts alone, as can be done before the build
/// they map exists.
#[must_use]
pub fn scripts_only(path: &Path) -> Report {
    checked(path, false, None)
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

fn checked(path: &Path, content: bool, settings: Option<&Block>) -> Report {
    match scripts::load(path) {
        Ok(app) => check_loaded(&app, content, settings),
        Err(problems) => Report {
            app_id: None,
            set_live: None,
            depots: Vec::new(),
            problems,
            content,
        },
    }
}

fn check_loaded(app: &AppScript, content: bool, settings: Option<&Block>) -> Report {
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
        .collect::<Vec<_>>();
    if content {
        unshipped_configurations(&depots, &mut problems);
        if let Some(settings) = settings {
            unshipped_manifest(app, settings, &mut problems);
            unlaunchable(app, &depots, settings, &mut problems);
        }
    }
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

/// The size a VDF file in a build has to stay under to be read as Steam Input's Action Manifest,
/// which runs to tens of kilobytes even with every language's strings.
const MANIFEST_LIMIT: u64 = 4 * 1024 * 1024;

/// A problem for each controller configuration that an Action Manifest in `depots` lists and the
/// build does not ship where the manifest says, which is beside it.
fn unshipped_configurations(depots: &[DepotFiles], problems: &mut Vec<Problem>) {
    let mut shipped: Option<BTreeSet<PathBuf>> = None;
    for depot in depots {
        for file in &depot.files {
            for (controller, path) in configurations(file) {
                let wanted = file
                    .parent()
                    .unwrap_or_else(|| Path::new(""))
                    .join(scripts::native(&path));
                let shipped = shipped.get_or_insert_with(|| {
                    depots
                        .iter()
                        .flat_map(|each| &each.files)
                        .filter_map(|shipping| fs::canonicalize(shipping).ok())
                        .collect()
                });
                if fs::canonicalize(&wanted).is_ok_and(|wanted| shipped.contains(&wanted)) {
                    continue;
                }
                problems.push(Problem::new(
                    file,
                    format!(
                        "is Steam Input's Action Manifest in depot {}, and lists \"{path}\" for \
                         {controller}, which the build does not ship beside it: put the \
                         configuration there, or take it out of the manifest",
                        depot.depot_id
                    ),
                ));
            }
        }
    }
}

/// What to do when a bundled Steam Input configuration has no action manifest named.
const NAME_THE_MANIFEST: &str = "enter its path from the install folder on the app's Steam Input \
                                 page in Steamworks, then save the settings again";

/// A problem for each system the build installs on without the action manifest of the Steam
/// Input configuration that `settings`, `app`'s, say the game bundles; one for the snapshot when
/// they name none. Steam reads the manifest from the install folder, where the depots' mappings
/// put each file, so a depot for one system shipping it leaves the others without.
fn unshipped_manifest(app: &AppScript, settings: &Block, problems: &mut Vec<Problem>) {
    let Some(manifest) = settings::action_manifest(settings) else {
        return;
    };
    let snapshot = app
        .path
        .parent()
        .unwrap_or_else(|| Path::new(""))
        .join(settings::SNAPSHOT);
    if manifest.is_empty() {
        problems.push(Problem::new(
            &snapshot,
            format!(
                "chooses a Custom Configuration as app {}'s Default Controller Configuration and \
                 names no action manifest: {NAME_THE_MANIFEST}",
                app.app_id
            ),
        ));
        return;
    }
    let mut systems: Vec<String> = Vec::new();
    let mut shipping: Vec<String> = Vec::new();
    for depot in &app.depots {
        let on = settings::depot_systems(settings, depot.depot_id);
        let ships = installs(depot, manifest);
        if ships && on.is_empty() {
            return;
        }
        for system in on {
            if ships && !shipping.contains(&system) {
                shipping.push(system.clone());
            }
            if !systems.contains(&system) {
                systems.push(system);
            }
        }
    }
    let unshipped = |on: &str| {
        Problem::new(
            &snapshot,
            format!(
                "names \"{manifest}\" as app {}'s Steam Input action manifest, and the build \
                 installs no file there{on}: ship the manifest at that path in the install \
                 folder, or change the path on the app's Steam Input page in Steamworks and save \
                 the settings again",
                app.app_id
            ),
        )
    };
    if systems.is_empty() {
        problems.push(unshipped(""));
    }
    for system in systems.iter().filter(|system| !shipping.contains(system)) {
        problems.push(unshipped(&format!(" on {system}")));
    }
}

/// A problem when the build in `folder`, app `app`'s, lacks the action manifest of the Steam
/// Input configuration that `settings` say the game bundles, or when they name none.
#[must_use]
pub fn folder_manifest(folder: &Path, app: u32, settings: &Block) -> Option<Problem> {
    let manifest = settings::action_manifest(settings)?;
    if manifest.is_empty() {
        return Some(Problem::new(
            folder,
            format!(
                "is a build of app {app}, whose settings choose a Custom Configuration as its \
                 Default Controller Configuration and name no action manifest: \
                 {NAME_THE_MANIFEST}"
            ),
        ));
    }
    (!folder.join(scripts::native(manifest)).is_file()).then(|| {
        Problem::new(
            folder,
            format!(
                "has no \"{manifest}\", which app {app}'s settings name as its Steam Input action \
                 manifest: put the manifest there, or change the path on the app's Steam Input \
                 page in Steamworks and save the settings again"
            ),
        )
    })
}

/// The folders and file name of `written`, a path from the install folder or a depot's.
fn parts(written: &str) -> Vec<String> {
    scripts::native(written)
        .split(is_separator)
        .filter(|part| !part.is_empty() && *part != ".")
        .map(str::to_owned)
        .collect()
}

/// Where `depot`'s mappings install `file`, one of its content's, each as its folders and name
/// from the install folder.
fn installed(depot: &DepotScript, file: &Path) -> Vec<Vec<String>> {
    depot
        .mappings
        .iter()
        .filter(|mapping| unmappable(mapping).is_none())
        .filter_map(|mapping| {
            let (folder, pattern) = split(mapping);
            let rest: Vec<String> = file
                .strip_prefix(depot.content_root.join(folder))
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

/// None when `on`, depots with their files, install `wanted`, a path's folders and name from the
/// install folder, matched by case or, with `any_case`, in any; otherwise the programs and macOS
/// apps they install in its folder instead.
fn programs_beside(
    on: &[(&DepotScript, &DepotFiles)],
    wanted: &[String],
    any_case: bool,
) -> Option<BTreeSet<String>> {
    let (_, folder) = wanted.split_last()?;
    let same = |one: &String, other: &String| {
        if any_case {
            one.eq_ignore_ascii_case(other)
        } else {
            one == other
        }
    };
    let mut beside: BTreeSet<String> = BTreeSet::new();
    for (depot, built) in on {
        for file in &built.files {
            for at in installed(depot, file) {
                // A macOS app is a folder, which a launch option names whole.
                if at.len() >= wanted.len()
                    && at.iter().zip(wanted).all(|(one, other)| same(one, other))
                {
                    return None;
                }
                match at.strip_prefix(folder) {
                    Some([name]) if is_program(file, name) => {
                        let _new: bool = beside.insert(name.clone());
                    }
                    Some([bundle, _, ..]) if is_bundle(bundle) => {
                        let _new: bool = beside.insert(bundle.clone());
                    }
                    _ => {}
                }
            }
        }
    }
    Some(beside)
}

/// A problem for each launch option in `settings`, `app`'s, whose program the build does not
/// install on a system the option runs on.
///
/// Steam starts the program from the install folder, where the depots' mappings put each file,
/// so a build whose program was renamed without its launch option goes up unable to start.
/// Linux alone tells names apart by case. A system whose content comes from a depot the build
/// leaves out is not judged, nor an option offered only on a beta branch the build does not set
/// live.
fn unlaunchable(
    app: &AppScript,
    depots: &[DepotFiles],
    settings: &Block,
    problems: &mut Vec<Problem>,
) {
    let snapshot = app
        .path
        .parent()
        .unwrap_or_else(|| Path::new(""))
        .join(settings::SNAPSHOT);
    let uploaded: Vec<u32> = app.depots.iter().map(|depot| depot.depot_id).collect();
    let mut all: Vec<Option<String>> = Vec::new();
    for depot in &uploaded {
        let on = settings::depot_systems(settings, *depot);
        if on.is_empty() {
            all = vec![None];
            break;
        }
        for system in on {
            if !all.contains(&Some(system.clone())) {
                all.push(Some(system));
            }
        }
    }
    for option in settings::launch_options(settings) {
        let offered = option.branch.as_deref().is_none_or(|branch| {
            app.set_live
                .as_deref()
                .is_some_and(|live| live.trim().eq_ignore_ascii_case(branch))
        });
        if !offered {
            continue;
        }
        let systems: Vec<Option<String>> = if option.systems.is_empty() {
            all.clone()
        } else {
            option.systems.iter().cloned().map(Some).collect()
        };
        for system in systems {
            let system = system.as_deref();
            if settings::installs_beside(settings, system, &uploaded) {
                continue;
            }
            let on: Vec<(&DepotScript, &DepotFiles)> = app
                .depots
                .iter()
                .zip(depots)
                .filter(|(depot, _)| {
                    let installs_on = settings::depot_systems(settings, depot.depot_id);
                    installs_on.is_empty()
                        || system.is_none_or(|system| installs_on.iter().any(|each| each == system))
                })
                .collect();
            if on.is_empty() {
                continue;
            }
            let wanted = parts(&option.executable);
            let Some((_, folder)) = wanted.split_last() else {
                continue;
            };
            let Some(beside) = programs_beside(&on, &wanted, system != Some("linux")) else {
                continue;
            };
            let on_system = system.map_or_else(String::new, |system| format!(" on {system}"));
            let place = if folder.is_empty() {
                "the install folder".to_owned()
            } else {
                format!("\"{}\"", folder.join("/"))
            };
            let instead = if beside.is_empty() {
                format!("no program in {place}")
            } else {
                format!("only {} in {place}", quoted(&beside))
            };
            problems.push(Problem::new(
                &snapshot,
                format!(
                    "starts \"{}\" in app {}'s launch option {}{on_system}, which the build does not \
                     install: it installs {instead}. Name the build's program in the launch \
                     option, under Installation, General Installation in Steamworks, and save \
                     the settings again, or ship the program under that name",
                    option.executable, app.app_id, option.key
                ),
            ));
        }
    }
}

/// Whether `file`, by its name `name`, is a program a launch option could start: one that starts
/// as a program does and is not named as a library, which starts the same way.
fn is_program(file: &Path, name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    let library = [".dll", ".so", ".dylib"]
        .iter()
        .any(|ending| name.ends_with(ending))
        || name.contains(".so.");
    !library && magic::looks_like_program(file).unwrap_or(false)
}

/// `items` as a list read aloud: a, b and c.
fn listed(mut items: Vec<String>) -> String {
    match items.pop() {
        Some(last) if !items.is_empty() => format!("{} and {last}", items.join(", ")),
        Some(last) => last,
        None => String::new(),
    }
}

/// `names`, each quoted, as a list read aloud: "a", "b" and "c".
fn quoted<'name>(names: impl IntoIterator<Item = &'name String>) -> String {
    listed(
        names
            .into_iter()
            .map(|name| format!("\"{name}\""))
            .collect(),
    )
}

/// Whether `name` is a macOS app's folder.
fn is_bundle(name: &str) -> bool {
    Path::new(name)
        .extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("app"))
}

/// Whether `folder` holds `wanted`, a path's folders and name, matching each by case or, with
/// `any_case`, in any case.
fn holds(folder: &Path, wanted: &[String], any_case: bool) -> bool {
    let mut at = folder.to_path_buf();
    for part in wanted {
        let exact = at.join(part);
        at = if exact.exists() {
            exact
        } else if any_case {
            let Some(found) = fs::read_dir(&at)
                .into_iter()
                .flatten()
                .flatten()
                .find(|entry| {
                    entry
                        .file_name()
                        .to_string_lossy()
                        .eq_ignore_ascii_case(part)
                })
            else {
                return false;
            };
            found.path()
        } else {
            return false;
        };
    }
    true
}

/// A problem when the build in `folder`, app `app`'s, holds a program and none of the programs
/// `settings` say its launch options start.
///
/// A build's folder is one system's, which the folder does not say, so any option's program will
/// do; a folder holding no program, such as a depot of data alone, is not judged.
#[must_use]
pub fn folder_launch(folder: &Path, app: u32, settings: &Block) -> Option<Problem> {
    let options: Vec<settings::Launch> = settings::launch_options(settings)
        .into_iter()
        .filter(|option| option.branch.is_none())
        .collect();
    let mut programs: BTreeSet<String> = BTreeSet::new();
    for option in &options {
        let wanted = parts(&option.executable);
        let Some((_, inside)) = wanted.split_last() else {
            continue;
        };
        let any_case =
            option.systems.is_empty() || option.systems.iter().any(|system| system != "linux");
        if holds(folder, &wanted, any_case) {
            return None;
        }
        let place = inside
            .iter()
            .fold(folder.to_path_buf(), |place, part| place.join(part));
        for entry in fs::read_dir(&place).into_iter().flatten().flatten() {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().into_owned();
            if (path.is_dir() && is_bundle(&name)) || (path.is_file() && is_program(&path, &name)) {
                let mut found = inside.to_vec();
                found.push(name);
                let _new: bool = programs.insert(found.join("/"));
            }
        }
    }
    if programs.is_empty() {
        return None;
    }
    let started: Vec<String> = options
        .iter()
        .map(|option| {
            let on = if option.systems.is_empty() {
                String::new()
            } else {
                format!(" on {}", option.systems.join(", "))
            };
            format!("\"{}\"{on}", option.executable)
        })
        .collect();
    Some(Problem::new(
        folder,
        format!(
            "is a build of app {app} holding {}, and none of the programs its launch options \
             start: {}. Name the build's program in a launch option, under Installation, General \
             Installation in Steamworks, and save the settings again, or ship the program under \
             that name",
            quoted(&programs),
            listed(started)
        ),
    ))
}

/// Whether `depot` installs a file of its content at `path`, a path from the install folder.
fn installs(depot: &DepotScript, path: &str) -> bool {
    let wanted = parts(path);
    depot.mappings.iter().any(|mapping| {
        let Some(rest) = wanted.strip_prefix(parts(&mapping.depot_path).as_slice()) else {
            return false;
        };
        let Some((name, folders)) = rest.split_last() else {
            return false;
        };
        let (folder, pattern) = split(mapping);
        let file = folders
            .iter()
            .fold(depot.content_root.join(folder), |file, part| {
                file.join(part)
            })
            .join(name);
        (folders.is_empty() || mapping.recursive)
            && pattern::matches(&pattern, name)
            && !excluded(&depot.content_root, &file, &depot.exclusions)
            && file.is_file()
    })
}

/// Each controller and the configuration file `file` lists for it, when `file` is Steam Input's
/// Action Manifest; none when it is any other file.
fn configurations(file: &Path) -> Vec<(String, String)> {
    let vdf = file
        .extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("vdf"));
    let small = || fs::metadata(file).is_ok_and(|metadata| metadata.len() < MANIFEST_LIMIT);
    if !vdf || !small() {
        return Vec::new();
    }
    let Some(root) = fs::read_to_string(file)
        .ok()
        .and_then(|text| vdf::parse(&text).ok())
    else {
        return Vec::new();
    };
    let Some(listed) = root
        .block("Action Manifest")
        .and_then(|manifest| manifest.block("configurations"))
    else {
        return Vec::new();
    };
    let mut found = Vec::new();
    for controller in &listed.pairs {
        let vdf::Value::Block(entries) = &controller.value else {
            continue;
        };
        for entry in &entries.pairs {
            if let vdf::Value::Block(entry) = &entry.value
                && let Some(path) = entry.text("path")
            {
                found.push((controller.key.clone(), path.to_owned()));
            }
        }
    }
    found
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

    #[test]
    fn a_file_is_installed_where_each_mapping_taking_it_puts_it() {
        let top = Path::new("export").join("windows").join("sample.exe");
        let inside = Path::new("export")
            .join("windows")
            .join("bin")
            .join("sample.exe");
        let at = |path: &[&str]| path.iter().map(|&part| part.to_owned()).collect::<Vec<_>>();
        assert_eq!(
            installed(&depot(&[("windows/*", false)], &[]), &top),
            [at(&["sample.exe"])]
        );
        assert_eq!(
            installed(&depot(&[("windows/*", false)], &[]), &inside),
            Vec::<Vec<String>>::new()
        );
        assert_eq!(
            installed(&depot(&[("windows/*", true)], &[]), &inside),
            [at(&["bin", "sample.exe"])]
        );
        assert_eq!(
            installed(&depot(&[("windows/*.pck", true)], &[]), &top),
            Vec::<Vec<String>>::new()
        );
    }
}
