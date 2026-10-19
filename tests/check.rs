//! `check` against real script and content trees, in a folder whose name has spaces and brackets
//! the way a Steam library's or a build worktree's can.
#![expect(
    clippy::unwrap_used,
    reason = "a test reports failure by panicking, its helpers included"
)]

use std::fs;
use std::path::{Path, PathBuf};

use steamship::check::{Report, check, scripts_only};
use steamship::run;
use steamship::scripts::Problem;

pub mod common;

struct Project {
    _temp: tempfile::TempDir,
    root: PathBuf,
}

impl Project {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("gate (x86) 1a2b");
        fs::create_dir_all(&root).unwrap();
        Self { _temp: temp, root }
    }

    /// Writes `contents` at `relative`, making its folders, and answers where.
    fn file(&self, relative: &str, contents: &[u8]) -> PathBuf {
        let path = self.root.join(relative);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, contents).unwrap();
        path
    }

    /// The same, for a file nothing refers to by path afterwards.
    fn put(&self, relative: &str, contents: &[u8]) {
        drop(self.file(relative, contents));
    }

    /// Two depots in files of their own, content for both, and a debug file to exclude.
    fn shipping(app_extra: &str) -> (Self, PathBuf) {
        let project = Self::new();
        project.put("export/windows/game.exe", b"MZ");
        project.put("export/windows/game.pck", b"pack");
        project.put("export/windows/game.pdb", b"symbols");
        project.put("export/linux/game.x86_64", b"not really elf");
        project.put("export/linux/data/game.pck", b"pack");
        project.put(
            "steam/depot_windows.vdf",
            br#""DepotBuild"
{
	"DepotID" "1001"
	"FileMapping" { "LocalPath" "windows\*" "DepotPath" "." "Recursive" "1" }
	"FileExclusion" "*.pdb"
}
"#,
        );
        project.put(
            "steam/depot_linux.vdf",
            br#""DepotBuild"
{
	"DepotID" "1002"
	"FileMapping" { "LocalPath" "linux\*" "DepotPath" "." "Recursive" "1" }
}
"#,
        );
        let app = project.file(
            "steam/app_build.vdf",
            format!(
                r#""AppBuild"
{{
	"AppID" "1000"
	"ContentRoot" "..\export\"
	"SetLive" "testing"
	{app_extra}
	"Depots"
	{{
		"1001" "depot_windows.vdf"
		"1002" "depot_linux.vdf"
	}}
}}
"#
            )
            .as_bytes(),
        );
        (project, app)
    }
}

fn messages(report: &Report) -> Vec<String> {
    report
        .problems
        .iter()
        .map(|problem| problem.message.clone())
        .collect()
}

fn names(report: &Report, depot_id: u32) -> Vec<String> {
    report
        .depots
        .iter()
        .find(|depot| depot.depot_id == depot_id)
        .unwrap()
        .files
        .iter()
        .map(|file| file.file_name().unwrap().to_string_lossy().into_owned())
        .collect()
}

#[test]
fn a_sound_build_is_mapped_and_nothing_is_refused() {
    let (_project, app) = Project::shipping("");
    let report = check(&app);
    assert_eq!(messages(&report), Vec::<String>::new());
    assert_eq!(report.app_id, Some(1000));
    assert_eq!(names(&report, 1001), ["game.exe", "game.pck"]);
    assert_eq!(names(&report, 1002), ["game.pck", "game.x86_64"]);
}

#[test]
fn setting_default_live_is_refused() {
    let (_project, app) = Project::shipping("");
    let text = fs::read_to_string(&app)
        .unwrap()
        .replace("\"testing\"", "\"Default\"");
    fs::write(&app, text).unwrap();
    assert_eq!(
        messages(&check(&app)),
        [
            "\"SetLive\" names \"default\", which steamcmd never sets live; upload --default sets \
             it live while the app is unreleased"
        ]
    );
}

#[test]
fn setting_public_live_is_refused_as_the_default_branch() {
    let (_project, app) = Project::shipping("");
    let text = fs::read_to_string(&app)
        .unwrap()
        .replace("\"testing\"", "\"Public\"");
    fs::write(&app, text).unwrap();
    assert_eq!(
        messages(&check(&app)),
        [
            "\"SetLive\" names \"public\", the Web API's name for the default branch, which \
             steamcmd never sets live; upload --default sets it live while the app is unreleased"
        ]
    );
}

#[test]
fn a_folder_unity_says_not_to_ship_is_refused_once() {
    let (project, app) = Project::shipping("");
    project.put(
        "export/linux/Game_BurstDebugInformation_DoNotShip/a.txt",
        b"a",
    );
    project.put(
        "export/linux/Game_BurstDebugInformation_DoNotShip/deep/b.txt",
        b"b",
    );
    project.put(
        "export/linux/data/Game_BackUpThisFolder_ButDontShipItWithYourGame/c.txt",
        b"c",
    );
    let report = check(&app);
    let refused: Vec<(String, String)> = report
        .problems
        .iter()
        .map(|problem| {
            (
                problem
                    .file
                    .file_name()
                    .unwrap()
                    .to_string_lossy()
                    .into_owned(),
                problem.message.clone(),
            )
        })
        .collect();
    assert_eq!(
        refused,
        [
            (
                "Game_BurstDebugInformation_DoNotShip".to_owned(),
                "is in depot 1002; Unity names this folder as one not to ship".to_owned()
            ),
            (
                "Game_BackUpThisFolder_ButDontShipItWithYourGame".to_owned(),
                "is in depot 1002; Unity names this folder as one not to ship".to_owned()
            ),
        ]
    );
}

#[test]
fn debug_symbols_are_counted_without_being_refused() {
    let (project, app) = Project::shipping("");
    project.put("export/linux/game.debug", b"symbols");
    project.put(
        "export/linux/Game.dSYM/Contents/Resources/DWARF/Game",
        b"dwarf",
    );
    project.put("export/linux/tools.PDB", b"symbols");
    let report = check(&app);
    assert_eq!(messages(&report), Vec::<String>::new());
    let symbols = |depot_id: u32| {
        report
            .depots
            .iter()
            .find(|depot| depot.depot_id == depot_id)
            .unwrap()
            .debug_symbols
    };
    assert_eq!(symbols(1001), 0, "excluded by its own FileExclusion");
    assert_eq!(symbols(1002), 3);
}

/// The Linux depot script with `extra` lines added to it.
fn with_linux_depot(project: &Project, extra: &str) {
    project.put(
        "steam/depot_linux.vdf",
        format!(
            r#""DepotBuild"
{{
	"DepotID" "1002"
	"FileMapping" {{ "LocalPath" "linux\*" "DepotPath" "." "Recursive" "1" }}
	{extra}
}}
"#
        )
        .as_bytes(),
    );
}

#[test]
fn an_install_script_must_be_a_file_the_depot_maps() {
    let (project, app) = Project::shipping("");
    project.put("export/linux/installscript.vdf", b"\"InstallScript\" {}");
    with_linux_depot(&project, r#""InstallScript" "linux\installscript.vdf""#);
    assert_eq!(messages(&check(&app)), Vec::<String>::new());

    with_linux_depot(&project, r#""InstallScript" "linux\missing.vdf""#);
    let report = check(&app);
    let [problem] = report.problems.as_slice() else {
        panic!("one problem: {:?}", report.problems);
    };
    assert!(
        problem
            .message
            .starts_with("depot 1002: InstallScript \"linux\\missing.vdf\" is not a file in "),
        "{}",
        problem.message
    );

    project.put(
        "export/elsewhere/installscript.vdf",
        b"\"InstallScript\" {}",
    );
    with_linux_depot(&project, r#""InstallScript" "elsewhere\installscript.vdf""#);
    assert_eq!(
        messages(&check(&app)),
        [
            "depot 1002: InstallScript \"elsewhere\\installscript.vdf\" is not in the depot; map \
             it with a FileMapping too"
        ]
    );
}

#[test]
fn no_set_live_is_fine() {
    let (_project, app) = Project::shipping("");
    let text = fs::read_to_string(&app)
        .unwrap()
        .replace("\"SetLive\" \"testing\"", "");
    fs::write(&app, text).unwrap();
    assert_eq!(check(&app).problems, Vec::<Problem>::new());
}

#[test]
fn preview_and_local_in_the_script_are_refused() {
    let (_project, app) = Project::shipping(r#""Preview" "1" "Local" "..\htdocs""#);
    assert_eq!(
        messages(&check(&app)),
        [
            "\"Preview\" is set in the script; pass --preview",
            "\"Local\" is set, which sends the build to a local content server, not to Steam",
        ]
    );
}

#[test]
fn the_scripts_alone_are_checked_without_the_content_they_map() {
    let (project, app) = Project::shipping("");
    fs::remove_dir_all(project.root.join("export")).unwrap();
    let report = scripts_only(&app);
    assert_eq!(messages(&report), Vec::<String>::new());
    assert!(!report.content, "the content was not checked");
    assert_eq!(report.app_id, Some(1000));
    assert_eq!(
        report
            .depots
            .iter()
            .map(|depot| (depot.depot_id, depot.files.len()))
            .collect::<Vec<_>>(),
        [(1001, 0), (1002, 0)]
    );
    assert!(check(&app).content, "a full check counts the content");
}

#[test]
fn the_scripts_alone_still_refuse_preview_and_local() {
    let (project, app) = Project::shipping(r#""Preview" "1" "Local" "..\htdocs""#);
    fs::remove_dir_all(project.root.join("export")).unwrap();
    assert_eq!(
        messages(&scripts_only(&app)),
        [
            "\"Preview\" is set in the script; pass --preview",
            "\"Local\" is set, which sends the build to a local content server, not to Steam",
        ]
    );
}

#[test]
fn a_shipped_steam_appid_txt_is_refused() {
    let (project, app) = Project::shipping("");
    let planted = project.file("export/windows/Steam_AppID.txt", b"1000");
    let report = check(&app);
    let reported: Vec<PathBuf> = report
        .problems
        .iter()
        .map(|problem| fs::canonicalize(&problem.file).unwrap())
        .collect();
    assert_eq!(reported, [fs::canonicalize(planted).unwrap()]);
    assert_eq!(
        messages(&report),
        ["is in depot 1001; it is for development only and must not ship"]
    );
}

/// A 64-bit Windows library importing `dlls`, laid out as a linker lays one out: the headers, then
/// one section holding the import table and the names it points to.
fn windows_library(dlls: &[&str]) -> Vec<u8> {
    fn put(bytes: &mut Vec<u8>, at: usize, value: &[u8]) {
        bytes.resize(at, 0);
        bytes.extend_from_slice(value);
    }
    let mut bytes = Vec::new();
    put(&mut bytes, 0, b"MZ");
    put(&mut bytes, 0x3C, &64_u32.to_le_bytes());
    put(&mut bytes, 64, b"PE\0\0");
    put(&mut bytes, 68, &0x8664_u16.to_le_bytes());
    put(&mut bytes, 70, &1_u16.to_le_bytes());
    put(&mut bytes, 84, &240_u16.to_le_bytes());
    put(&mut bytes, 88, &0x20B_u16.to_le_bytes());
    put(&mut bytes, 196, &16_u32.to_le_bytes());
    put(&mut bytes, 208, &0x1000_u32.to_le_bytes());
    put(&mut bytes, 340, &0x1000_u32.to_le_bytes());
    put(&mut bytes, 344, &0x1000_u32.to_le_bytes());
    put(&mut bytes, 348, &0x200_u32.to_le_bytes());
    put(&mut bytes, 0x200, &[]);
    let descriptors = dlls.len().checked_add(1).unwrap().checked_mul(20).unwrap();
    let mut names = Vec::new();
    for dll in dlls {
        let name = descriptors.checked_add(names.len()).unwrap();
        let address = 0x1000_u32
            .checked_add(u32::try_from(name).unwrap())
            .unwrap();
        bytes.extend_from_slice(&[0; 12]);
        bytes.extend_from_slice(&address.to_le_bytes());
        bytes.extend_from_slice(&[0; 4]);
        names.extend_from_slice(dll.as_bytes());
        names.push(0);
    }
    bytes.extend_from_slice(&[0; 20]);
    bytes.extend(names);
    bytes
}

#[test]
fn a_windows_file_importing_what_only_a_common_redistributable_installs_is_refused() {
    let (project, app) = Project::shipping("");
    let library = project.file(
        "export/windows/plugins/game.dll",
        &windows_library(&[
            "KERNEL32.dll",
            "api-ms-win-crt-runtime-l1-1-0.dll",
            "steam_api64.dll",
            "VCRUNTIME140.dll",
            "vcruntime140.dll",
            "XINPUT1_3",
            "winspool.drv",
        ]),
    );
    project.put(
        "export/windows/STEAM_API64.DLL",
        &windows_library(&["advapi32.dll"]),
    );
    let report = check(&app);
    assert_eq!(
        messages(&report),
        [
            "imports VCRUNTIME140.dll, which Windows does not have and depot 1001 does not ship, \
             so the game would need a Steamworks Common Redistributable; link its library in \
             statically, use one Windows has, or ship the DLL in the depot",
            "imports XINPUT1_3, which Windows does not have and depot 1001 does not ship, so the \
             game would need a Steamworks Common Redistributable; link its library in \
             statically, use one Windows has, or ship the DLL in the depot",
        ]
    );
    assert!(
        report
            .problems
            .iter()
            .all(|problem| fs::canonicalize(&problem.file).unwrap()
                == fs::canonicalize(&library).unwrap())
    );
}

#[test]
fn a_dll_counts_as_shipped_only_in_the_depot_of_the_file_importing_it() {
    let (project, app) = Project::shipping("");
    project.put(
        "export/windows/game.dll",
        &windows_library(&["kernel32.dll", "steam_api64.dll"]),
    );
    project.put("export/linux/steam_api64.dll", b"not for Windows");
    assert_eq!(
        messages(&check(&app)),
        [
            "imports steam_api64.dll, which Windows does not have and depot 1001 does not ship, \
             so the game would need a Steamworks Common Redistributable; link its library in \
             statically, use one Windows has, or ship the DLL in the depot"
        ]
    );
}

#[test]
fn a_windows_file_whose_imports_cannot_be_read_is_refused() {
    let (project, app) = Project::shipping("");
    let mut cut = windows_library(&["kernel32.dll"]);
    cut.truncate(80);
    project.put("export/windows/game.dll", &cut);
    assert_eq!(
        messages(&check(&app)),
        ["cannot be read as a Windows program or library: its headers are cut short"]
    );
}

#[test]
fn a_dotnet_program_on_a_runtime_installed_apart_is_refused() {
    let (project, app) = Project::shipping("");
    project.put(
        "export/windows/Tool.runtimeconfig.json",
        br#"{"runtimeOptions": {"tfm": "net8.0", "framework": {"name": "Microsoft.NETCore.App", "version": "8.0.0"}}}"#,
    );
    project.put(
        "export/windows/Editor.RuntimeConfig.json",
        br#"{"runtimeOptions": {"tfm": "net8.0", "frameworks": [{"name": "Microsoft.NETCore.App", "version": "8.0.0"}, {"name": "Microsoft.WindowsDesktop.App", "version": "8.0.0"}]}}"#,
    );
    project.put(
        "export/windows/Game.runtimeconfig.json",
        br#"{"runtimeOptions": {"tfm": "net8.0", "includedFrameworks": [{"name": "Microsoft.NETCore.App", "version": "8.0.11"}]}}"#,
    );
    project.put("export/linux/Broken.runtimeconfig.json", b"{");
    project.put("export/linux/Bare.runtimeconfig.json", b"{}");
    let mut found = messages(&check(&app));
    found.sort();
    let apart = "runs on a .NET runtime the player would have to install, which on Windows takes a \
                 Steamworks Common Redistributable; publish the program self-contained";
    assert_eq!(
        found,
        [
            "cannot be read as .NET's runtime settings: EOF while parsing an object at line 1 \
             column 1",
            apart,
            apart,
        ]
    );
}

#[test]
fn a_mapping_that_matches_nothing_is_refused() {
    let (project, app) = Project::shipping("");
    fs::remove_dir_all(project.root.join("export/linux")).unwrap();
    let found = messages(&check(&app));
    assert_eq!(found.len(), 1, "{found:?}");
    assert!(found.iter().all(|message| {
        message.starts_with(r#"depot 1002: LocalPath "linux\*" matches no files in "#)
    }));
}

#[test]
fn a_missing_content_root_is_refused() {
    let (project, app) = Project::shipping("");
    fs::remove_dir_all(project.root.join("export")).unwrap();
    let found = messages(&check(&app));
    assert_eq!(found.len(), 2, "{found:?}");
    for (message, depot) in found.iter().zip(["1001", "1002"]) {
        assert!(
            message.starts_with(&format!("depot {depot}: ContentRoot "))
                && message.ends_with(
                    " does not exist; put the depot's files there, or point ContentRoot at the \
                     folder they are in"
                ),
            "{message}"
        );
    }
}

#[test]
fn a_content_root_that_is_a_file_is_refused() {
    let (project, app) = Project::shipping("");
    fs::remove_dir_all(project.root.join("export")).unwrap();
    project.put("export", b"not a folder");
    let found = messages(&check(&app));
    assert_eq!(found.len(), 2, "{found:?}");
    assert!(
        found.iter().all(|message| message.ends_with(
            " is not a folder; point ContentRoot at the folder the depot's files are in"
        )),
        "{found:?}"
    );
}

#[test]
fn every_script_problem_is_reported_at_once() {
    let (project, app) = Project::shipping("");
    fs::remove_file(project.root.join("steam/depot_linux.vdf")).unwrap();
    let depot = project.root.join("steam/depot_windows.vdf");
    let text = fs::read_to_string(&depot)
        .unwrap()
        .replace("\"1001\"", "\"1003\"");
    fs::write(&depot, text).unwrap();
    let found = messages(&check(&app));
    let [mismatch, missing] = found.as_slice() else {
        panic!("expected two problems, found {found:?}");
    };
    assert_eq!(
        mismatch,
        "says DepotID 1003, but the app script lists it as 1001"
    );
    assert!(missing.starts_with("depot 1002 names "), "{missing}");
    assert!(missing.ends_with(", which does not exist"), "{missing}");
}

#[test]
fn an_inline_depot_uses_the_app_scripts_folder() {
    let project = Project::new();
    project.put("content/game.exe", b"MZ");
    let app = project.file(
        "scripts/app.vdf",
        br#""AppBuild" { "AppID" "1000" "ContentRoot" "../content/" "Depots" { "1001" {
            "FileMapping" { "LocalPath" "*" "DepotPath" "." "recursive" "1" } } } }"#,
    );
    let report = check(&app);
    assert!(report.problems.is_empty(), "{:?}", messages(&report));
    assert_eq!(names(&report, 1001), ["game.exe"]);
}

#[test]
fn a_script_that_is_not_keyvalues_is_refused_with_its_place() {
    let project = Project::new();
    let app = project.file("app.vdf", b"\"AppBuild\"\n{\n  \"AppID\"\n}\n");
    assert_eq!(
        messages(&check(&app)),
        ["line 3, column 3: key \"AppID\" has no value"]
    );
}

#[cfg(unix)]
#[test]
fn a_linux_program_needs_its_executable_bit() {
    use std::os::unix::fs::PermissionsExt as _;

    let (project, app) = Project::shipping("");
    let program = project.file("export/linux/game.x86_64", &elf_program());
    fs::set_permissions(&program, fs::Permissions::from_mode(0o644)).unwrap();
    assert_eq!(
        messages(&check(&app)),
        ["is a Linux program without its executable bit; run chmod +x on it"]
    );
    fs::set_permissions(&program, fs::Permissions::from_mode(0o755)).unwrap();
    assert_eq!(check(&app).problems, Vec::<Problem>::new());
}

#[cfg(unix)]
#[test]
fn a_macos_program_or_a_script_needs_its_executable_bit_and_a_library_does_not() {
    use std::os::unix::fs::PermissionsExt as _;

    let (project, app) = Project::shipping("");
    let mut program = vec![0xCF, 0xFA, 0xED, 0xFE, 0, 0, 0, 0, 0, 0, 0, 0];
    let mut library = program.clone();
    program.extend_from_slice(&2_u32.to_le_bytes());
    library.extend_from_slice(&6_u32.to_le_bytes());
    for (relative, contents) in [
        (
            "export/linux/Game.app/Contents/MacOS/Game",
            program.as_slice(),
        ),
        (
            "export/linux/start.sh",
            b"#!/bin/sh\nexec ./game\n".as_slice(),
        ),
        ("export/linux/libgame.dylib", library.as_slice()),
    ] {
        let path = project.file(relative, contents);
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
    }
    assert_eq!(
        messages(&check(&app)),
        [
            "is a macOS program without its executable bit; run chmod +x on it",
            "is a script (it starts with #!) without its executable bit; run chmod +x on it",
        ]
    );
}

/// A 64-bit position-independent ELF program: `ET_DYN` with a `PT_INTERP` segment.
#[cfg(unix)]
fn elf_program() -> Vec<u8> {
    let mut bytes = Vec::new();
    for (at, value) in [
        (0, &b"\x7fELF\x02\x01"[..]),
        (16, &3_u16.to_le_bytes()[..]),
        (32, &64_u64.to_le_bytes()[..]),
        (54, &56_u16.to_le_bytes()[..]),
        (56, &1_u16.to_le_bytes()[..]),
        (64, &3_u32.to_le_bytes()[..]),
        (120, &[][..]),
    ] {
        bytes.resize(at, 0);
        bytes.extend_from_slice(value);
    }
    bytes
}

/// `steamship check` on `script`, in a home of its own and with no key set: a key kept for the
/// home of whoever runs the tests would otherwise have `check` ask Steam about the test's app.
fn run(script: &Path) -> (Option<i32>, String, String) {
    run_with(script, &[])
}

/// [`run`], with `extra` arguments after the script.
fn run_with(script: &Path, extra: &[&str]) -> (Option<i32>, String, String) {
    let home = tempfile::tempdir().unwrap();
    let output = run::output(
        run::command(env!("CARGO_BIN_EXE_steamship"))
            .arg("check")
            .arg(script)
            .args(extra)
            .env("STEAMSHIP_HOME", home.path())
            .env_remove("STEAMSHIP_WEB_API_KEY")
            .envs(common::store::environment()),
    )
    .unwrap();
    (
        output.status.code(),
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

#[test]
fn the_command_says_what_it_mapped_and_exits_0() {
    let (_project, app) = Project::shipping("");
    let (code, stdout, stderr) = run(&app);
    assert_eq!((code, stderr.as_str()), (Some(0_i32), ""));
    assert_eq!(
        stdout,
        format!(
            "steamship check\n  \
             script    {}\n  \
             app       \u{2713} 1000, 2 depots, 4 files, checked\n  \
             branch    testing, not checked: no Web API key at hand\n    \
             a branch the app does not have leaves the build uploaded but not live; steamship \
             login --web-api-key keeps a key to check it with\n  \
             depot     1001, 2 files\n  \
             depot     1002, 2 files\n  \
             \u{2713} nothing refused\n",
            app.display()
        )
    );
}

#[test]
fn the_command_checks_the_scripts_alone_before_the_build_and_says_the_content_was_not() {
    let (project, app) = Project::shipping("");
    fs::remove_dir_all(project.root.join("export")).unwrap();
    let (code, stdout, stderr) = run_with(&app, &["--scripts-only"]);
    assert_eq!((code, stderr.as_str()), (Some(0_i32), ""));
    assert_eq!(
        stdout,
        format!(
            "steamship check\n  \
             script    {}\n  \
             app       \u{2713} 1000, 2 depots, scripts checked\n  \
             branch    testing, not checked: no Web API key at hand\n    \
             a branch the app does not have leaves the build uploaded but not live; steamship \
             login --web-api-key keeps a key to check it with\n  \
             content   not checked (--scripts-only)\n  \
             \u{2713} nothing refused\n",
            app.display()
        )
    );
    let (full, _, _) = run(&app);
    assert_eq!(
        full,
        Some(2_i32),
        "without the flag, the missing build is refused"
    );
}

#[test]
fn the_command_says_a_script_that_is_not_there_does_not_exist_and_exits_2() {
    let project = Project::new();
    for missing in ["app_build.vdf", "steam/app_build.vdf"] {
        let script = project.root.join(missing);
        let (code, _, stderr) = run(&script);
        assert_eq!(code, Some(2_i32), "{missing}");
        assert_eq!(
            stderr,
            format!("  \u{2717} refused: {}: does not exist\n", script.display()),
            "{missing}"
        );
    }
}

#[test]
fn the_command_lists_every_refusal_and_exits_2() {
    let (_project, app) = Project::shipping(r#""Preview" "1""#);
    let (code, stdout, stderr) = run(&app);
    assert_eq!(code, Some(2_i32));
    assert!(!stdout.contains('\u{2713}'), "{stdout}");
    assert!(
        stderr
            .lines()
            .all(|line| line.starts_with("  \u{2717} refused: ")),
        "{stderr}"
    );
    assert!(stderr.contains("\"Preview\" is set in the script; pass --preview"));
}

/// An app script in `scripts/`, with content beside it, and what `check` says about it.
fn refusals(app: &str, depot: Option<&str>) -> Vec<String> {
    let project = Project::new();
    project.put("content/game.exe", b"MZ");
    project.put("content/bin/tool.exe", b"MZ");
    if let Some(depot) = depot {
        project.put("scripts/depot.vdf", depot.as_bytes());
    }
    let app = project.file("scripts/app.vdf", app.as_bytes());
    messages(&check(&app))
}

/// The same scripts with no content anywhere, and what `check` says about the scripts alone.
fn script_refusals(app: &str, depot: Option<&str>) -> Vec<String> {
    let project = Project::new();
    if let Some(depot) = depot {
        project.put("scripts/depot.vdf", depot.as_bytes());
    }
    let app = project.file("scripts/app.vdf", app.as_bytes());
    messages(&scripts_only(&app))
}

const MAPPING: &str = r#""FileMapping" { "LocalPath" "*" "DepotPath" "." }"#;

#[test]
fn every_malformed_script_is_refused_with_what_is_wrong() {
    let cases = [
        (r#""Other" {}"#.to_owned(), None, "there is no \"AppBuild\" block"),
        (
            r#""AppBuild" { "AppID" "1" "ContentRoot" "../content" }"#.to_owned(),
            None,
            "\"Depots\" is missing or empty",
        ),
        (
            r#""AppBuild" { "AppID" "1" "ContentRoot" "../content" "Depots" { } }"#.to_owned(),
            None,
            "\"Depots\" is missing or empty",
        ),
        (
            format!(r#""AppBuild" {{ "ContentRoot" "../content" "Depots" {{ "1" {{ {MAPPING} }} }} }}"#),
            None,
            "\"AppID\" is missing",
        ),
        (
            format!(
                r#""AppBuild" {{ "AppID" "one" "ContentRoot" "../content" "Depots" {{ "1" {{ {MAPPING} }} }} }}"#
            ),
            None,
            "\"AppID\" \"one\" is not a number",
        ),
        (
            format!(
                r#""AppBuild" {{ "AppID" "0" "ContentRoot" "../content" "Depots" {{ "1" {{ {MAPPING} }} }} }}"#
            ),
            None,
            "\"AppID\" is 0, which Steamworks never gives: put in the ID it shows for it",
        ),
        (
            format!(
                r#""AppBuild" {{ "AppID" "1" "ContentRoot" "../content" "Depots" {{ "x" {{ {MAPPING} }} }} }}"#
            ),
            None,
            "depot ID \"x\" in \"Depots\" is not a number",
        ),
        (
            format!(r#""AppBuild" {{ "AppID" "1" "Depots" {{ "1" {{ {MAPPING} }} }} }}"#),
            None,
            "depot 1 has no ContentRoot, in the app script or its own",
        ),
        (
            r#""AppBuild" { "AppID" "1" "ContentRoot" "../content" "Depots" { "1" { "FileMapping" "*" } } }"#
                .to_owned(),
            None,
            "a \"FileMapping\" is not a block",
        ),
        (
            r#""AppBuild" { "AppID" "1" "ContentRoot" "../content" "Depots" { "1" { "FileMapping" { "LocalPath" "*" } } } }"#
                .to_owned(),
            None,
            "a \"FileMapping\" needs both \"LocalPath\" and \"DepotPath\"",
        ),
        (
            r#""AppBuild" { "AppID" "1" "ContentRoot" "../content" "Depots" { "1" { } } }"#.to_owned(),
            None,
            "the depot has no \"FileMapping\"",
        ),
        (
            format!(
                r#""AppBuild" {{ "AppID" "1" "ContentRoot" "../content" "Depots" {{ "1" {{ {MAPPING} "FileExclusion" {{ }} }} }} }}"#
            ),
            None,
            "a \"FileExclusion\" is a block, where a pattern belongs",
        ),
        (
            r#""AppBuild" { "AppID" "1" "ContentRoot" "../content" "Depots" { "1" "depot.vdf" } }"#
                .to_owned(),
            Some(r#""Something" { }"#),
            "there is no \"DepotBuild\" block",
        ),
        (
            r#""AppBuild" { "AppID" "1" "ContentRoot" "../content" "Depots" { "1" "depot.vdf" } }"#
                .to_owned(),
            Some(r#""DepotBuild" { "DepotID" "two" "FileMapping" { "LocalPath" "*" "DepotPath" "." } }"#),
            "\"DepotID\" \"two\" is not a number",
        ),
        (
            r#""AppBuild" { "AppID" "1" "ContentRoot" "../content" "Depots" { "1" { "FileMapping" { "LocalPath" "b*/tool.exe" "DepotPath" "." } } } }"#
                .to_owned(),
            None,
            "depot 1: LocalPath \"b*/tool.exe\" has a wildcard in a folder name, which steamship cannot map the way steamcmd does",
        ),
    ];
    for (app, depot, expected) in cases {
        assert_eq!(refusals(&app, depot), [expected], "{app}");
        assert_eq!(
            script_refusals(&app, depot),
            [expected],
            "the scripts alone: {app}"
        );
    }
}

#[test]
fn the_legacy_depot_block_name_is_read() {
    let depot =
        r#""DepotBuildConfig" { "DepotID" "1" "FileMapping" { "LocalPath" "*" "DepotPath" "." } }"#;
    let app =
        r#""AppBuild" { "AppID" "1" "ContentRoot" "../content" "Depots" { "1" "depot.vdf" } }"#;
    assert_eq!(refusals(app, Some(depot)), Vec::<String>::new());
}

/// A depot script one folder below the app script, with a `ContentRoot` of its own that names a
/// folder existing under the app's content root, under the app script's folder and under the
/// depot script's folder, each holding a different file.
fn nested_depot(app_content_root: &str) -> Report {
    let project = Project::new();
    project.put("content/windows/under_the_app_root.txt", b"x");
    project.put("steam/windows/beside_the_app_script.txt", b"x");
    project.put("steam/depots/windows/beside_the_depot_script.txt", b"x");
    project.put(
        "steam/depots/depot.vdf",
        br#""DepotBuild" { "DepotID" "1" "ContentRoot" "windows" "FileMapping" { "LocalPath" "*" "DepotPath" "." } }"#,
    );
    let app = project.file(
        "steam/app.vdf",
        format!(
            r#""AppBuild" {{ "AppID" "1" {app_content_root} "Depots" {{ "1" "depots/depot.vdf" }} }}"#
        )
        .as_bytes(),
    );
    check(&app)
}

#[test]
fn a_depots_own_content_root_is_relative_to_the_apps() {
    let report = nested_depot(r#""ContentRoot" "../content""#);
    assert_eq!(messages(&report), Vec::<String>::new());
    assert_eq!(names(&report, 1), ["under_the_app_root.txt"]);
}

#[test]
fn a_depots_own_content_root_is_relative_to_the_app_script_when_the_app_has_none() {
    let report = nested_depot("");
    assert_eq!(messages(&report), Vec::<String>::new());
    assert_eq!(names(&report, 1), ["beside_the_app_script.txt"]);
}

#[test]
fn an_absolute_content_root_is_taken_as_written() {
    let project = Project::new();
    project.put("content/game.exe", b"MZ");
    let absolute = project.root.join("content");
    let app = project.file(
        "scripts/app.vdf",
        format!(
            r#""AppBuild" {{ "AppID" "1" "ContentRoot" "{}" "Depots" {{ "1" {{ {MAPPING} }} }} }}"#,
            absolute.display()
        )
        .as_bytes(),
    );
    let report = check(&app);
    assert_eq!(messages(&report), Vec::<String>::new());
    assert_eq!(names(&report, 1), ["game.exe"]);
}

#[test]
fn a_mapping_descends_into_folders_only_when_recursive() {
    let project = Project::new();
    project.put("content/game.exe", b"MZ");
    project.put("content/bin/tool.exe", b"MZ");
    project.put("content/bin/tools/debug.exe", b"MZ");
    project.put("content/data.pck", b"pack");
    let script = |recursive: &str, exclusion: &str| {
        format!(
            r#""AppBuild" {{ "AppID" "1" "ContentRoot" "../content" "Depots" {{ "1" {{
                "FileMapping" {{ "LocalPath" "*.exe" "DepotPath" "." "Recursive" "{recursive}" }}
                "FileExclusion" "{exclusion}" }} }} }}"#
        )
    };
    let flat = project.file("scripts/flat.vdf", script("0", "none").as_bytes());
    assert_eq!(names(&check(&flat), 1), ["game.exe"]);
    let deep = project.file("scripts/deep.vdf", script("1", "bin/tools*").as_bytes());
    // In path order: bin/tool.exe sorts before game.exe.
    assert_eq!(names(&check(&deep), 1), ["tool.exe", "game.exe"]);
}
