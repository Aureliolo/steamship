//! Which commits an upload may ship: those on origin's `green` branch.
#![expect(
    clippy::unwrap_used,
    reason = "a test reports failure by panicking, its helpers included"
)]

use std::fs;
use std::path::{Path, PathBuf};

use steamship::green::{self, Policy, Standing};
use steamship::project::short;
use steamship::{ledger, run};

struct Repository {
    _temp: tempfile::TempDir,
    root: PathBuf,
}

fn git(folder: &Path, args: &[&str]) {
    let status = run::spawn(
        run::command("git")
            .args([
                "-c",
                "user.name=steamship",
                "-c",
                "user.email=steamship@example.invalid",
            ])
            .args(["-c", "commit.gpgsign=false"])
            // A hooks folder set in the machine's own git config would otherwise run here.
            .args(["-c", "core.hooksPath=.git/hooks"])
            .args(args)
            .current_dir(folder),
    )
    .and_then(|mut git| git.wait())
    .unwrap();
    assert!(status.success(), "git {args:?}");
}

fn head(root: &Path) -> String {
    let output = run::output(
        run::command("git")
            .args(["rev-parse", "HEAD"])
            .current_dir(root),
    )
    .unwrap();
    String::from_utf8(output.stdout).unwrap().trim().to_owned()
}

/// A repository with one commit and a bare origin that holds no `green` yet.
fn repository() -> Repository {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("game");
    fs::create_dir_all(&root).unwrap();
    git(
        temp.path(),
        &[
            "init",
            "--quiet",
            "--bare",
            "--initial-branch=main",
            "origin.git",
        ],
    );
    git(&root, &["init", "--quiet", "--initial-branch=main"]);
    git(
        &root,
        &[
            "remote",
            "add",
            "origin",
            &temp.path().join("origin.git").to_string_lossy(),
        ],
    );
    commit(&root, "first");
    Repository { _temp: temp, root }
}

fn commit(root: &Path, message: &str) {
    git(root, &["commit", "--quiet", "--allow-empty", "-m", message]);
}

fn move_green(root: &Path) {
    git(
        root,
        &["push", "--quiet", "origin", "+HEAD:refs/heads/green"],
    );
}

#[test]
fn a_commit_at_the_tip_of_green_is_let_through_as_green() {
    let game = repository();
    move_green(&game.root);
    let shipped = green::require(&game.root, Policy::Required).unwrap();
    assert_eq!(shipped.commit, head(&game.root));
    assert!(shipped.green);
}

#[test]
fn a_commit_green_has_moved_past_is_still_on_green() {
    let game = repository();
    move_green(&game.root);
    let earlier = head(&game.root);
    commit(&game.root, "second");
    move_green(&game.root);
    git(&game.root, &["reset", "--quiet", "--hard", &earlier]);
    let shipped = green::require(&game.root, Policy::Required).unwrap();
    assert_eq!(shipped.commit, earlier);
    assert!(shipped.green);
}

#[test]
fn a_commit_ahead_of_green_is_refused_naming_it_green_and_the_fix() {
    let game = repository();
    move_green(&game.root);
    let tip = head(&game.root);
    commit(&game.root, "second");
    let why = green::require(&game.root, Policy::Required).unwrap_err();
    let ahead = head(&game.root);
    assert!(
        why.starts_with(&format!(
            "commit {} is not on origin's green branch (green is at {})",
            short(&ahead),
            short(&tip)
        )),
        "{why}"
    );
    assert!(why.contains("Run a round"), "{why}");
}

#[test]
fn a_commit_on_another_line_than_green_is_refused() {
    let game = repository();
    git(&game.root, &["checkout", "--quiet", "-b", "side"]);
    commit(&game.root, "side");
    move_green(&game.root);
    git(&game.root, &["checkout", "--quiet", "main"]);
    commit(&game.root, "main");
    let why = green::require(&game.root, Policy::Required).unwrap_err();
    assert!(why.contains("is not on origin's green branch"), "{why}");
}

#[test]
fn an_origin_with_no_green_branch_refuses_every_commit() {
    let game = repository();
    let why = green::require(&game.root, Policy::Required).unwrap_err();
    assert!(why.contains("origin has no green branch"), "{why}");
    assert_eq!(
        green::standing(&game.root, &head(&game.root)).unwrap(),
        Standing::Off { tip: None }
    );
}

#[test]
fn allowing_ungreen_lets_a_commit_through_and_records_that_it_was_not_green() {
    let game = repository();
    let shipped = green::require(&game.root, Policy::Allowed).unwrap();
    assert_eq!(shipped.commit, head(&game.root));
    assert!(!shipped.green);

    move_green(&game.root);
    commit(&game.root, "second");
    assert!(!green::require(&game.root, Policy::Allowed).unwrap().green);
}

#[test]
fn allowing_ungreen_still_marks_a_green_commit_green() {
    let game = repository();
    move_green(&game.root);
    assert!(green::require(&game.root, Policy::Allowed).unwrap().green);
}

#[test]
fn a_folder_outside_a_repository_is_refused_even_when_ungreen_is_allowed() {
    let temp = tempfile::tempdir().unwrap();
    let why = green::require(temp.path(), Policy::Allowed).unwrap_err();
    assert!(why.contains("rev-parse HEAD failed"), "{why}");
}

#[test]
fn builds_recorded_in_the_home_are_read_back_in_order() {
    let home = tempfile::tempdir().unwrap();
    assert_eq!(ledger::load(home.path(), 480).unwrap(), Vec::new());
    let entry = |build_id: u64, green: bool| ledger::Entry {
        app: 480,
        branch: "testing".to_owned(),
        build_id,
        commit: format!("{build_id:040x}"),
        green,
    };
    ledger::record(home.path(), entry(1, true)).unwrap();
    ledger::record(home.path(), entry(2, false)).unwrap();
    assert_eq!(
        ledger::load(home.path(), 480).unwrap(),
        vec![entry(1, true), entry(2, false)]
    );
    assert_eq!(ledger::load(home.path(), 481).unwrap(), Vec::new());
}

#[test]
fn a_record_that_is_not_a_list_is_refused_naming_the_file() {
    let home = tempfile::tempdir().unwrap();
    let file = ledger::file(home.path(), 480);
    fs::create_dir_all(file.parent().unwrap()).unwrap();
    fs::write(&file, "{}").unwrap();
    let why = ledger::load(home.path(), 480).unwrap_err();
    assert!(why.ends_with("it is not a list of builds"), "{why}");
    assert!(why.contains("480.json"), "{why}");
}
