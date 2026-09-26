//! The app script steamcmd is given, and the commit the build is described by.
#![expect(
    clippy::unwrap_used,
    reason = "a test reports failure by panicking, its helpers included"
)]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use steamship::upload::{self, Error};
use steamship::vdf::{self, Block};

struct Project {
    _temp: tempfile::TempDir,
    root: PathBuf,
    home: PathBuf,
}

impl Project {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("fgm gate (x86) 1a2b");
        let home = temp.path().join("home");
        fs::create_dir_all(&root).unwrap();
        Self {
            _temp: temp,
            root,
            home,
        }
    }

    fn file(&self, relative: &str, contents: &str) -> PathBuf {
        let path = self.root.join(relative);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, contents).unwrap();
        path
    }
}

fn app_block(script: &Path) -> Block {
    let document = vdf::parse(&fs::read_to_string(script).unwrap()).unwrap();
    document.block("AppBuild").unwrap().clone()
}

fn slashed(path: &Path) -> String {
    path.to_str().unwrap().replace('\\', "/")
}

const APP: &str = r#""AppBuild"
{
	"AppID" "5335950"
	"ContentRoot" "..\export\"
	"BuildOutput" "..\output\"
	"Desc" "whatever was there"
	"SetLive" "testing"
	"Depots"
	{
		"5335951" "depot_windows.vdf"
		"5335952" { "FileMapping" { "LocalPath" "linux\*" "DepotPath" "." } }
	}
}
"#;

#[test]
fn steamcmd_gets_a_copy_with_every_path_absolute_and_steamships_keys_set() {
    let project = Project::new();
    let original = project.file("steam/app_build.vdf", APP);
    let prepared = upload::prepare(&project.home, &original, "1.4.0 abc123def456", false).unwrap();
    let apps = project.home.join("apps").join("5335950");
    assert_eq!(prepared.app_id, 5_335_950);
    assert_eq!(prepared.script, apps.join("app_build.vdf"));
    assert_eq!(prepared.output, apps.join("output"));
    assert_eq!(prepared.set_live.as_deref(), Some("testing"));
    assert!(prepared.output.is_dir());
    let app = app_block(&prepared.script);
    let steam = project.root.join("steam");
    assert_eq!(
        app.text("ContentRoot"),
        Some(format!("{}/../export/", slashed(&steam)).as_str())
    );
    assert_eq!(
        app.text("BuildOutput"),
        Some(slashed(&prepared.output).as_str())
    );
    assert_eq!(app.text("Desc"), Some("1.4.0 abc123def456"));
    assert_eq!(app.get("Preview"), None);
    assert_eq!(app.text("SetLive"), Some("testing"));
    let depots = app.block("Depots").unwrap();
    assert_eq!(
        depots.text("5335951"),
        Some(format!("{}/depot_windows.vdf", slashed(&steam)).as_str())
    );
    assert!(depots.block("5335952").is_some(), "an inline depot is kept");
    assert_eq!(app.all("Desc").count(), 1);
    assert_eq!(app.all("BuildOutput").count(), 1);
}

#[test]
fn a_script_named_by_a_relative_path_gets_absolute_paths_in_its_copy() {
    let home = tempfile::tempdir().unwrap();
    let relative = Path::new("tests/fixtures/spacewar/steam/app_build.vdf");
    let prepared = upload::prepare(home.path(), relative, "1 abc", true).unwrap();
    let app = app_block(&prepared.script);
    let steam = std::path::absolute(relative.parent().unwrap()).unwrap();
    assert_eq!(
        app.text("ContentRoot"),
        Some(format!("{}/../content/", slashed(&steam)).as_str())
    );
    assert_eq!(
        app.block("Depots").unwrap().text("481"),
        Some(format!("{}/depot_build.vdf", slashed(&steam)).as_str())
    );
}

#[test]
fn a_preview_is_set_in_the_file() {
    let project = Project::new();
    let original = project.file("steam/app_build.vdf", APP);
    let prepared = upload::prepare(&project.home, &original, "1.4.0 abc", true).unwrap();
    assert_eq!(app_block(&prepared.script).text("Preview"), Some("1"));
}

#[test]
fn an_app_without_a_content_root_gets_its_scripts_folder_so_depot_roots_resolve_the_same() {
    let project = Project::new();
    let original = project.file(
        "steam/app_build.vdf",
        r#""AppBuild" { "AppID" "7" "Depots" { "8" "depot.vdf" } }"#,
    );
    let prepared = upload::prepare(&project.home, &original, "1 abc", false).unwrap();
    assert_eq!(
        app_block(&prepared.script).text("ContentRoot"),
        Some(slashed(&project.root.join("steam")).as_str())
    );
}

#[test]
fn the_last_runs_log_is_cleared_so_only_this_runs_is_read() {
    let project = Project::new();
    let original = project.file("steam/app_build.vdf", APP);
    let first = upload::prepare(&project.home, &original, "1 abc", false).unwrap();
    fs::write(
        first.log(),
        "Successfully finished AppID 5335950 build (BuildID 1).",
    )
    .unwrap();
    let chunk = first.output.join("chunkcache.bin");
    fs::write(&chunk, "kept").unwrap();
    let second = upload::prepare(&project.home, &original, "2 abc", false).unwrap();
    assert!(!second.log().exists());
    assert!(chunk.exists(), "the chunk cache is kept between runs");
}

#[test]
fn a_script_steamship_cannot_rewrite_is_refused_with_why() {
    let project = Project::new();
    for (contents, reason) in [
        (
            "\"AppBuild\" { \"Depots\" { } }",
            "there is no numeric \"AppID\"",
        ),
        ("\"DepotBuild\" { }", "there is no \"AppBuild\" block"),
        (
            "\"AppBuild\" { \"AppID\" \"1\" \"Depots\" \"x\" }",
            "\"Depots\" is not a block",
        ),
    ] {
        let original = project.file("steam/app_build.vdf", contents);
        match upload::prepare(&project.home, &original, "1 abc", false) {
            Err(Error::Script { path, reason: said }) => {
                assert_eq!((path, said.as_str()), (original, reason));
            }
            other => panic!("{contents}: {other:?}"),
        }
    }
}

#[test]
fn a_last_log_that_cannot_be_cleared_is_an_error() {
    let project = Project::new();
    let original = project.file("steam/app_build.vdf", APP);
    let first = upload::prepare(&project.home, &original, "1 abc", false).unwrap();
    fs::create_dir_all(first.log()).unwrap();
    assert!(matches!(
        upload::prepare(&project.home, &original, "1 abc", false),
        Err(Error::Io { path, .. }) if path == first.log()
    ));
}

#[test]
fn a_script_that_is_not_there_is_an_error() {
    let project = Project::new();
    let missing = project.root.join("steam/app_build.vdf");
    assert!(matches!(
        upload::prepare(&project.home, &missing, "1 abc", false),
        Err(Error::Io { .. })
    ));
}

fn git(folder: &Path, args: &[&str]) {
    let status = Command::new("git")
        .args([
            "-c",
            "user.name=steamship",
            "-c",
            "user.email=steamship@example.invalid",
        ])
        .args(["-c", "commit.gpgsign=false"])
        .args(args)
        .current_dir(folder)
        .status()
        .unwrap();
    assert!(status.success(), "git {args:?}");
}

#[test]
fn the_commit_is_the_one_the_scripts_are_in() {
    let project = Project::new();
    let original = project.file("steam/app_build.vdf", APP);
    git(&project.root, &["init", "--quiet"]);
    git(&project.root, &["add", "."]);
    git(&project.root, &["commit", "--quiet", "-m", "scripts"]);
    let output = Command::new("git")
        .args(["rev-parse", "--short=12", "HEAD"])
        .current_dir(&project.root)
        .output()
        .unwrap();
    let expected = String::from_utf8(output.stdout).unwrap();
    assert_eq!(upload::commit(&original).unwrap(), expected.trim());
    assert_eq!(upload::commit(&project.root).unwrap(), expected.trim());
}

#[test]
fn scripts_outside_a_repository_are_refused() {
    let project = Project::new();
    let original = project.file("steam/app_build.vdf", APP);
    assert!(matches!(
        upload::commit(&original),
        Err(Error::Description(reason)) if reason.ends_with("which the build description names")
    ));
}
