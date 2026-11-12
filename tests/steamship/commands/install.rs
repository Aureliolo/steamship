//! `steamship install`.

use std::fs::File;
use std::path::Path;
use std::thread;
use std::time::Duration;

use steamship::install;

use crate::{failure, recorded, steamship};

/// `steamship install` with `STEAMSHIP_HOME` set to `home`, or with no environment at all.
fn run(home: Option<&Path>) -> (Option<i32>, String, String) {
    steamship(&["install"], home, &[])
}

#[test]
fn install_says_when_the_install_is_as_pinned() {
    let home = recorded("");
    let (code, stdout, stderr) = run(Some(home.path()));
    assert_eq!((code, stderr.as_str()), (Some(0_i32), ""));
    assert!(stdout.starts_with("steamship install\n"), "{stdout}");
    assert!(stdout.contains(", pinned and verified\n"), "{stdout}");
    let folder = home.path().join(install::FOLDER);
    assert!(
        stdout.ends_with(&format!("  folder    {}\n", folder.display())),
        "{stdout}"
    );
}

#[test]
fn with_times_each_step_ends_with_how_long_it_took() {
    let home = recorded("");
    let (code, stdout, stderr) = steamship(&["install", "--times"], Some(home.path()), &[]);
    assert_eq!((code, stderr.as_str()), (Some(0_i32), ""));
    let verified = stdout
        .lines()
        .find(|line| line.contains(", pinned and verified"))
        .unwrap_or_else(|| panic!("{stdout}"));
    let took = verified
        .strip_suffix(" s)")
        .and_then(|line| line.rsplit_once(", pinned and verified ("))
        .map(|(_, took)| took);
    assert!(
        took.is_some_and(|took| took.parse::<f64>().is_ok() && took.contains('.')),
        "{verified}"
    );
}

#[test]
fn install_names_what_changed_and_exits_4() {
    let home = recorded(&format!("file {} steamcmd.exe\n", "0".repeat(64)));
    let (code, stdout, stderr) = run(Some(home.path()));
    assert_eq!(code, Some(4_i32));
    assert!(
        stdout.ends_with("\u{2717} differs from the pinned version\n"),
        "{stdout}"
    );
    assert!(
        stderr.trim_end().ends_with("steamcmd.exe is missing"),
        "{stderr}"
    );
}

#[test]
fn install_waits_for_another_steamship_using_the_home_then_goes_on() {
    let home = recorded("");
    let held = File::create(home.path().join("steamship.lock")).unwrap();
    held.try_lock().unwrap();
    let path = home.path().to_path_buf();
    let second = thread::spawn(move || run(Some(&path)));
    thread::sleep(Duration::from_secs(2));
    assert!(!second.is_finished(), "it waits while the home is held");
    held.unlock().unwrap();
    let (code, stdout, stderr) = second.join().unwrap();
    assert_eq!((code, stderr.as_str()), (Some(0_i32), ""));
    assert!(
        stdout.contains(&format!(
            "  steamcmd  waiting for another steamship using {} to finish...\n",
            home.path().display()
        )) && stdout.contains("\u{2713} free after ")
            && stdout.contains(", pinned and verified\n"),
        "{stdout}"
    );
}

#[test]
fn install_says_where_it_could_not_find_a_home_and_exits_1() {
    let (code, _, stderr) = run(None);
    assert_eq!(code, Some(1_i32));
    assert!(
        failure(&stderr).starts_with("neither STEAMSHIP_HOME nor "),
        "{stderr}"
    );
}
