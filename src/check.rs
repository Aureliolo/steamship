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
}

/// The folders Unity writes beside a build and names as never to be shipped.
const DO_NOT_SHIP: [&str; 2] = [
    "*_BurstDebugInformation_DoNotShip",
    "*_BackUpThisFolder_ButDontShipItWithYourGame",
];

/// The DLLs every edition of Windows 10 and later has, so that a game needs no installer for them,
/// and `vulkan-1.dll` and `opencl.dll`, which every graphics driver installs. Media Foundation is
/// left out, as the N editions lack it.
const WINDOWS_DLLS: [&str; 87] = [
    "advapi32.dll",
    "avrt.dll",
    "bcrypt.dll",
    "bcryptprimitives.dll",
    "cfgmgr32.dll",
    "combase.dll",
    "comctl32.dll",
    "comdlg32.dll",
    "credui.dll",
    "crypt32.dll",
    "d2d1.dll",
    "d3d9.dll",
    "d3d10.dll",
    "d3d10_1.dll",
    "d3d11.dll",
    "d3d12.dll",
    "d3dcompiler_47.dll",
    "dbghelp.dll",
    "dcomp.dll",
    "dinput8.dll",
    "dnsapi.dll",
    "dsound.dll",
    "dwmapi.dll",
    "dwrite.dll",
    "dxgi.dll",
    "dxva2.dll",
    "gdi32.dll",
    "gdiplus.dll",
    "glu32.dll",
    "hid.dll",
    "imm32.dll",
    "iphlpapi.dll",
    "kernel32.dll",
    "kernelbase.dll",
    "ksuser.dll",
    "mpr.dll",
    "msacm32.dll",
    "mscoree.dll",
    "msctf.dll",
    "msimg32.dll",
    "msvcp_win.dll",
    "msvcrt.dll",
    "mswsock.dll",
    "ncrypt.dll",
    "netapi32.dll",
    "normaliz.dll",
    "ntdll.dll",
    "ole32.dll",
    "oleacc.dll",
    "oleaut32.dll",
    "opencl.dll",
    "opengl32.dll",
    "powrprof.dll",
    "propsys.dll",
    "psapi.dll",
    "rpcrt4.dll",
    "sechost.dll",
    "secur32.dll",
    "setupapi.dll",
    "shcore.dll",
    "shell32.dll",
    "shlwapi.dll",
    "sspicli.dll",
    "ucrtbase.dll",
    "uiautomationcore.dll",
    "urlmon.dll",
    "user32.dll",
    "userenv.dll",
    "usp10.dll",
    "uxtheme.dll",
    "version.dll",
    "vulkan-1.dll",
    "windowscodecs.dll",
    "winhttp.dll",
    "wininet.dll",
    "winmm.dll",
    "winspool.drv",
    "wintrust.dll",
    "winusb.dll",
    "wldap32.dll",
    "ws2_32.dll",
    "wsock32.dll",
    "wtsapi32.dll",
    "xaudio2_8.dll",
    "xaudio2_9.dll",
    "xinput1_4.dll",
    "xinput9_1_0.dll",
];

/// Windows' API sets, which name no file: Windows itself says which DLL answers each.
const API_SETS: [&str; 2] = ["api-ms-win-", "ext-ms-win-"];

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
                files,
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
        content_problems(depot.depot_id, file, problems);
    }
    redistributables(depot.depot_id, &mapped, problems);
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
    if let Some(message) = unix::missing_executable_bit(file) {
        problems.push(Problem::new(file, message));
    }
    if name.to_ascii_lowercase().ends_with(".runtimeconfig.json")
        && let Some(message) = runtime_installed_apart(file)
    {
        problems.push(Problem::new(file, message));
    }
}

/// A problem for each DLL a Windows program or library in depot `depot_id` imports that neither
/// Windows has nor the depot ships, which only a Steamworks Common Redistributable would install.
fn redistributables(depot_id: u32, files: &BTreeSet<PathBuf>, problems: &mut Vec<Problem>) {
    let shipped: BTreeSet<String> = files
        .iter()
        .filter_map(|file| file.file_name())
        .map(|name| name.to_string_lossy().to_ascii_lowercase())
        .collect();
    for file in files {
        let imports = match File::open(file).and_then(|mut opened| pe::imports(&mut opened)) {
            Ok(Some(imports)) => imports,
            Ok(None) => continue,
            Err(error) => {
                problems.push(Problem::new(
                    file,
                    format!("cannot be read as a Windows program or library: {error}"),
                ));
                continue;
            }
        };
        let mut named = BTreeSet::new();
        for dll in imports {
            let mut name = dll.to_ascii_lowercase();
            // Windows looks for a name without an extension as a DLL.
            if !name.contains('.') {
                name.push_str(".dll");
            }
            if WINDOWS_DLLS.contains(&name.as_str())
                || API_SETS.iter().any(|set| name.starts_with(set))
                || shipped.contains(&name)
                || !named.insert(name)
            {
                continue;
            }
            problems.push(Problem::new(
                file,
                format!(
                    "imports {dll}, which Windows does not have and depot {depot_id} does not \
                     ship, so the game would need a Steamworks Common Redistributable; link its \
                     library in statically, use one Windows has, or ship the DLL in the depot"
                ),
            ));
        }
    }
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
                "runs on a .NET runtime the player would have to install, which on Windows takes \
                 a Steamworks Common Redistributable; publish the program self-contained"
                    .to_owned()
            })
        }
        Err(error) => Some(format!(
            "cannot be read as .NET's runtime settings: {error}"
        )),
    }
}
