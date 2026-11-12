//! Which commits an upload may ship: those on origin's `green` branch.
pub mod common;

use common::git::{commit, git, head, with_origin};

use std::fs;
use std::path::{Path, PathBuf};

use steamship::green::{self, Policy, Standing};
use steamship::ledger;
use steamship::project::short;

struct Repository {
    _temp: tempfile::TempDir,
    root: PathBuf,
}

/// A repository with one commit and a bare origin that holds no `green` yet.
fn repository() -> Repository {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("game");
    fs::create_dir_all(&root).unwrap();
    with_origin(temp.path(), &root);
    commit(&root, "first");
    Repository { _temp: temp, root }
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
    let shipped = green::require(&game.root, "green", Policy::Required).unwrap();
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
    let shipped = green::require(&game.root, "green", Policy::Required).unwrap();
    assert_eq!(shipped.commit, earlier);
    assert!(shipped.green);
}

#[test]
fn a_commit_ahead_of_green_is_refused_naming_it_green_and_the_fix() {
    let game = repository();
    move_green(&game.root);
    let tip = head(&game.root);
    commit(&game.root, "second");
    let why = green::require(&game.root, "green", Policy::Required).unwrap_err();
    let ahead = head(&game.root);
    assert!(
        why.starts_with(&format!(
            "commit {} is not on origin's green branch (green is at {})",
            short(&ahead),
            short(&tip)
        )),
        "{why}"
    );
    assert!(
        why.ends_with(
            "Upload it once your tests have moved green to it, or give --allow-ungreen to upload \
             it as it is"
        ),
        "{why}"
    );
}

#[test]
fn a_commit_on_another_line_than_green_is_refused() {
    let game = repository();
    git(&game.root, &["checkout", "--quiet", "-b", "side"]);
    commit(&game.root, "side");
    move_green(&game.root);
    git(&game.root, &["checkout", "--quiet", "main"]);
    commit(&game.root, "main");
    let why = green::require(&game.root, "green", Policy::Required).unwrap_err();
    assert!(why.contains("is not on origin's green branch"), "{why}");
}

#[test]
fn an_origin_with_no_green_branch_refuses_every_commit() {
    let game = repository();
    let why = green::require(&game.root, "green", Policy::Required).unwrap_err();
    assert!(why.contains("origin has no green branch"), "{why}");
    assert_eq!(
        green::standing(&game.root, "green", &head(&game.root)).unwrap(),
        Standing::Off { tip: None }
    );
}

#[test]
fn the_branch_checked_is_the_one_named_not_one_called_green() {
    let game = repository();
    move_green(&game.root);
    let why = green::require(&game.root, "passed", Policy::Required).unwrap_err();
    assert!(why.contains("origin has no passed branch"), "{why}");
    git(
        &game.root,
        &["push", "--quiet", "origin", "HEAD:refs/heads/passed"],
    );
    let shipped = green::require(&game.root, "passed", Policy::Required).unwrap();
    assert!(shipped.green);
}

#[test]
fn allowing_ungreen_lets_a_commit_through_and_records_that_it_was_not_green() {
    let game = repository();
    let shipped = green::require(&game.root, "green", Policy::Allowed).unwrap();
    assert_eq!(shipped.commit, head(&game.root));
    assert!(!shipped.green);

    move_green(&game.root);
    commit(&game.root, "second");
    assert!(
        !green::require(&game.root, "green", Policy::Allowed)
            .unwrap()
            .green
    );
}

#[test]
fn allowing_ungreen_still_marks_a_green_commit_green() {
    let game = repository();
    move_green(&game.root);
    assert!(
        green::require(&game.root, "green", Policy::Allowed)
            .unwrap()
            .green
    );
}

#[test]
fn a_folder_outside_a_repository_is_refused_even_when_ungreen_is_allowed() {
    let temp = tempfile::tempdir().unwrap();
    let why = green::require(temp.path(), "green", Policy::Allowed).unwrap_err();
    assert!(why.contains("rev-parse HEAD failed"), "{why}");
}

#[test]
fn an_origin_that_cannot_be_reached_refuses_the_commit_unless_ungreen_is_allowed() {
    let game = repository();
    let gone = game.root.parent().unwrap().join("gone.git");
    git(
        &game.root,
        &["remote", "set-url", "origin", &gone.to_string_lossy()],
    );
    let why = green::require(&game.root, "green", Policy::Required).unwrap_err();
    assert!(why.contains("ls-remote"), "{why}");
    let shipped = green::require(&game.root, "green", Policy::Allowed).unwrap();
    assert_eq!(shipped.commit, head(&game.root));
    assert!(!shipped.green);
}

#[test]
fn a_record_that_cannot_be_read_is_an_error_not_an_empty_record() {
    let home = tempfile::tempdir().unwrap();
    fs::create_dir_all(ledger::file(home.path(), 480)).unwrap();
    let why = ledger::load(home.path(), 480).unwrap_err();
    assert!(why.contains("480.json"), "{why}");
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
        rollback: false,
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
    assert!(
        why.ends_with("it is not a list of builds. Delete the file to start a new record"),
        "{why}"
    );
    assert!(why.contains("480.json"), "{why}");
}
