//! `check` against real script and content trees, in a folder whose name has spaces and brackets
//! the way a Steam library's or a build worktree's can.
#![expect(
    clippy::unwrap_used,
    reason = "a test reports failure by panicking, its helpers included"
)]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use steamship::check::{Report, check};

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

    fn file(&self, relative: &str, contents: &[u8]) -> PathBuf {
        let path = self.root.join(relative);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, contents).unwrap();
        path
    }

    /// Two depots in files of their own, content for both, and a debug file to exclude.
    fn shipping(app_extra: &str) -> (Self, PathBuf) {
        let project = Self::new();
        project.file("export/windows/game.exe", b"MZ");
        project.file("export/windows/game.pck", b"pack");
        project.file("export/windows/game.pdb", b"symbols");
        project.file("export/linux/game.x86_64", b"not really elf");
        project.file("export/linux/data/game.pck", b"pack");
        project.file(
            "steam/depot_windows.vdf",
            br#""DepotBuild"
{
	"DepotID" "1001"
	"FileMapping" { "LocalPath" "windows\*" "DepotPath" "." "Recursive" "1" }
	"FileExclusion" "*.pdb"
}
"#,
        );
        project.file(
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

fn names(report: &Report, depot: usize) -> Vec<String> {
    report.depots[depot]
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
    assert_eq!(names(&report, 0), ["game.exe", "game.pck"]);
    assert_eq!(names(&report, 1), ["game.pck", "game.x86_64"]);
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
        ["\"SetLive\" names \"default\", which Valve only allows from the Steamworks site"]
    );
}

#[test]
fn no_set_live_is_fine() {
    let (_project, app) = Project::shipping("");
    let text = fs::read_to_string(&app)
        .unwrap()
        .replace("\"SetLive\" \"testing\"", "");
    fs::write(&app, text).unwrap();
    assert!(check(&app).problems.is_empty());
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
fn a_shipped_steam_appid_txt_is_refused() {
    let (project, app) = Project::shipping("");
    let planted = project.file("export/windows/Steam_AppID.txt", b"1000");
    let report = check(&app);
    assert_eq!(report.problems.len(), 1);
    let reported = fs::canonicalize(&report.problems[0].file).unwrap();
    assert_eq!(reported, fs::canonicalize(planted).unwrap());
}

#[test]
fn a_mapping_that_matches_nothing_is_refused() {
    let (project, app) = Project::shipping("");
    fs::remove_dir_all(project.root.join("export/linux")).unwrap();
    let report = check(&app);
    assert_eq!(report.problems.len(), 1);
    assert!(
        report.problems[0]
            .message
            .contains(r#"LocalPath "linux\\*" matches no files"#)
    );
}

#[test]
fn a_missing_content_root_is_refused() {
    let (project, app) = Project::shipping("");
    fs::remove_dir_all(project.root.join("export")).unwrap();
    let report = check(&app);
    assert_eq!(report.problems.len(), 2);
    assert!(
        report
            .problems
            .iter()
            .all(|p| p.message.contains("is not a folder"))
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
    assert_eq!(found.len(), 2, "{found:?}");
    assert!(found[0].contains("says DepotID 1003, but the app script lists it as 1001"));
    assert!(found[1].contains("depot 1002 names"));
}

#[test]
fn an_inline_depot_uses_the_app_scripts_folder() {
    let project = Project::new();
    project.file("content/game.exe", b"MZ");
    let app = project.file(
        "scripts/app.vdf",
        br#""AppBuild" { "AppID" "1000" "ContentRoot" "../content/" "Depots" { "1001" {
            "FileMapping" { "LocalPath" "*" "DepotPath" "." "recursive" "1" } } } }"#,
    );
    let report = check(&app);
    assert!(report.problems.is_empty(), "{:?}", messages(&report));
    assert_eq!(names(&report, 0), ["game.exe"]);
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
    use std::os::unix::fs::PermissionsExt;

    let (project, app) = Project::shipping("");
    let program = project.file("export/linux/game.x86_64", &elf_program());
    fs::set_permissions(&program, fs::Permissions::from_mode(0o644)).unwrap();
    assert_eq!(
        messages(&check(&app)),
        ["is a Linux program without its executable bit; run chmod +x on it"]
    );
    fs::set_permissions(&program, fs::Permissions::from_mode(0o755)).unwrap();
    assert!(check(&app).problems.is_empty());
}

#[cfg(unix)]
fn elf_program() -> Vec<u8> {
    let mut bytes = vec![0u8; 64 + 56];
    bytes[..4].copy_from_slice(b"\x7fELF");
    bytes[4] = 2;
    bytes[5] = 1;
    bytes[16..18].copy_from_slice(&3u16.to_le_bytes());
    bytes[32..40].copy_from_slice(&64u64.to_le_bytes());
    bytes[54..56].copy_from_slice(&56u16.to_le_bytes());
    bytes[56..58].copy_from_slice(&1u16.to_le_bytes());
    bytes[64..68].copy_from_slice(&3u32.to_le_bytes());
    bytes
}

fn run(script: &Path) -> (Option<i32>, String, String) {
    let output = Command::new(env!("CARGO_BIN_EXE_steamship"))
        .arg("check")
        .arg(script)
        .output();
    let output = output.unwrap();
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
    assert_eq!((code, stderr.as_str()), (Some(0), ""));
    assert!(
        stdout.ends_with("app 1000, depot 1001 (2 files), depot 1002 (2 files); nothing refused\n")
    );
}

#[test]
fn the_command_lists_every_refusal_and_exits_2() {
    let (_project, app) = Project::shipping(r#""Preview" "1""#);
    let (code, stdout, stderr) = run(&app);
    assert_eq!((code, stdout.as_str()), (Some(2), ""));
    assert!(stderr.starts_with("refused: "));
    assert!(stderr.contains("\"Preview\" is set in the script; pass --preview"));
}
