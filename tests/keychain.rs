//! The system's real credential store, as the library keeps a key in it. Each test keeps its key
//! for a home of its own, so it never meets a person's real key, and forgets it again. On Linux the
//! store is a gnome-keyring of this process's own.
//!
//! A key the `steamship` command keeps, shows and forgets is tested in `steamship/commands/`, by the
//! command itself: on macOS a Keychain item answers the program that made it, so a key this test
//! program kept is not one a `steamship` it runs may remove.

pub mod common;

use std::env;
use std::fs;
use std::panic::{self, AssertUnwindSafe};
use std::path::{Path, PathBuf};
#[cfg(target_os = "linux")]
use std::sync::Once;

#[cfg(target_os = "macos")]
use steamship::macos as platform;
#[cfg(target_os = "linux")]
use steamship::secret_service as platform;
use steamship::webapi::Key;
#[cfg(windows)]
use steamship::windows as platform;
use steamship::{keychain, run};

const FIRST: &str = "0123456789ABCDEF0123456789ABCDEF";
const SECOND: &str = "fedcba9876543210fedcba9876543210";

/// Points this process at its own store before anything here reads the environment. Every test
/// calls it first.
#[cfg(target_os = "linux")]
#[expect(
    unsafe_code,
    reason = "the library finds the session bus in the environment, as steamship does"
)]
fn store() {
    static SET: Once = Once::new();
    SET.call_once(|| {
        let address = common::store::secret_service();
        // SAFETY: every test here calls this before anything else, and `Once` holds each one
        // until the first has set it, so no thread of this process reads the environment while
        // it changes.
        unsafe {
            env::set_var("DBUS_SESSION_BUS_ADDRESS", address);
        }
    });
}

#[cfg(not(target_os = "linux"))]
const fn store() {}

/// Forgets the key and every branch password kept for `home`.
fn forget_all(home: &Path) {
    drop(keychain::forget(home));
    drop(keychain::forget_branch_passwords(home));
}

/// A home whose key and branch passwords the library forgets once the test is done with them.
fn forgetting_home() -> common::store::Home {
    common::store::Home::new(forget_all)
}

#[test]
fn a_key_is_kept_replaced_read_back_and_forgotten_for_its_home_alone() {
    store();
    let home = forgetting_home();
    let other = tempfile::tempdir().unwrap();
    let home = home.path();
    assert!(!keychain::has(home).unwrap());
    assert!(keychain::kept(home).unwrap().is_none());
    assert!(!keychain::forget(home).unwrap());

    keychain::keep(home, &Key::parse(FIRST).unwrap()).unwrap();
    assert!(keychain::has(home).unwrap());
    assert_eq!(keychain::kept(home).unwrap().unwrap().text(), FIRST);
    assert!(
        !keychain::has(other.path()).unwrap(),
        "another home keeps a key of its own"
    );

    keychain::keep(home, &Key::parse(SECOND).unwrap()).unwrap();
    assert_eq!(keychain::kept(home).unwrap().unwrap().text(), SECOND);

    assert!(keychain::forget(home).unwrap());
    assert!(!keychain::has(home).unwrap());
    assert!(keychain::kept(home).unwrap().is_none());
    assert!(!keychain::forget(home).unwrap());
}

#[test]
fn what_is_kept_that_is_not_a_key_is_refused_not_used() {
    store();
    let home = forgetting_home();
    let item = keychain::Item::key(home.path());
    for kept in [&b"not a key"[..], &[0xff, 0xfe][..]] {
        platform::keep_secret(&item, kept).unwrap();
        assert_eq!(
            keychain::kept(home.path()).unwrap_err(),
            keychain::Error::NotAKey
        );
    }
    assert!(keychain::forget(home.path()).unwrap());
}

#[test]
fn a_branch_password_is_kept_for_its_app_and_branch_apart_from_the_key() {
    store();
    let home = common::store::Home::new(|home| {
        drop(keychain::forget(home));
        drop(keychain::forget_branch_password(home, 480, "testing"));
    });
    let home = home.path();
    assert!(
        keychain::branch_password(home, 480, "testing")
            .unwrap()
            .is_none()
    );
    keychain::keep_branch_password(home, 480, "testing", "open sesame").unwrap();
    assert_eq!(
        keychain::branch_password(home, 480, "testing")
            .unwrap()
            .unwrap()
            .as_str(),
        "open sesame"
    );
    for (app, branch) in [(480, "staging"), (481, "testing")] {
        assert!(
            keychain::branch_password(home, app, branch)
                .unwrap()
                .is_none(),
            "{app} {branch}"
        );
    }
    assert!(!keychain::has(home).unwrap(), "the key is kept apart");
    assert!(keychain::forget_branch_password(home, 480, "testing").unwrap());
    assert!(
        keychain::branch_password(home, 480, "testing")
            .unwrap()
            .is_none()
    );
}

/// Homes that only start like another's, which forget everything kept for them when dropped.
struct Neighbours(Vec<PathBuf>);

impl Drop for Neighbours {
    fn drop(&mut self) {
        for home in &self.0 {
            forget_all(home);
        }
    }
}

#[test]
fn every_branch_password_of_a_home_is_forgotten_and_those_of_no_other_home() {
    store();
    let home = forgetting_home();
    let path = home.path();
    let neighbours = Neighbours(
        ["x", " b", ":1000:friends"]
            .map(|tail| PathBuf::from(format!("{}{tail}", path.display())))
            .to_vec(),
    );
    for (app, branch) in [(480, "testing"), (480, "staging"), (1000, "friends")] {
        keychain::keep_branch_password(path, app, branch, "open-sesame").unwrap();
    }
    keychain::keep(path, &Key::parse(FIRST).unwrap()).unwrap();
    for other in &neighbours.0 {
        keychain::keep(other, &Key::parse(SECOND).unwrap()).unwrap();
        keychain::keep_branch_password(other, 1000, "friends", "open-sesame").unwrap();
    }
    assert_eq!(keychain::branch_passwords(path).unwrap().len(), 3);

    assert_eq!(keychain::forget_branch_passwords(path).unwrap(), 3);
    assert_eq!(keychain::forget_branch_passwords(path).unwrap(), 0);
    assert_eq!(keychain::branch_passwords(path).unwrap(), Vec::new());
    assert!(
        keychain::has(path).unwrap(),
        "the key is not a branch password"
    );
    for other in &neighbours.0 {
        assert!(keychain::has(other).unwrap(), "{}", other.display());
        assert_eq!(
            keychain::branch_passwords(other).unwrap(),
            [keychain::Item::branch(other, 1000, "friends")],
            "{}",
            other.display()
        );
    }
}

#[test]
fn a_home_that_keeps_a_branch_password_after_its_forget_ran_fails_its_test() {
    store();
    let home = common::store::Home::new(|home| drop(keychain::forget(home)));
    let path = home.path().to_path_buf();
    keychain::keep_branch_password(&path, 480, "testing", "open-sesame").unwrap();
    let caught = panic::catch_unwind(AssertUnwindSafe(|| drop(home)));
    forget_locked(&path);
    let payload = caught.unwrap_err();
    let message = payload.downcast_ref::<String>().unwrap();
    assert!(
        message.contains(&format!("branch-password {}:480:testing", path.display())),
        "{message}"
    );
}

/// Forgets the key and every branch password kept for `home`, holding the store's lock.
fn forget_locked(home: &Path) {
    let _store = common::store::lock();
    forget_all(home);
}

/// Forgets everything kept for its homes once dropped, holding the store's lock to do it.
struct Remains(Vec<PathBuf>);

impl Drop for Remains {
    fn drop(&mut self) {
        for home in &self.0 {
            forget_locked(home);
        }
    }
}

#[test]
fn a_new_home_forgets_what_is_kept_for_temp_homes_that_are_gone_and_for_no_other() {
    store();
    let gone = tempfile::tempdir().unwrap();
    let standing = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir_in(env!("CARGO_TARGET_TMPDIR")).unwrap();
    let homes = [gone.path(), standing.path(), outside.path()];
    let _remains = Remains(homes.map(Path::to_path_buf).to_vec());
    {
        let _store = common::store::lock();
        for home in homes {
            keychain::keep(home, &Key::parse(FIRST).unwrap()).unwrap();
            keychain::keep_branch_password(home, 480, "testing", "open-sesame").unwrap();
        }
    }
    let gone_home = gone.path().to_path_buf();
    drop(gone);
    assert!(!gone_home.exists());

    let _fresh = forgetting_home();

    assert!(
        !keychain::has(&gone_home).unwrap(),
        "the key of the gone home"
    );
    assert_eq!(keychain::branch_passwords(&gone_home).unwrap(), Vec::new());
    for home in [standing.path(), outside.path()] {
        assert!(keychain::has(home).unwrap(), "{}", home.display());
        assert_eq!(
            keychain::branch_passwords(home).unwrap().len(),
            1,
            "{}",
            home.display()
        );
    }
}

/// Set for the copy of this test program that
/// `a_failing_test_that_leaves_a_branch_password_kept_fails_for_its_own_reason` starts, in which
/// that test fails for real.
const FAILING: &str = "STEAMSHIP_TEST_FAILING_WITH_A_LEAK";

// A panic while a panic unwinds ends the whole program, so the failing test runs in a copy of
// this one, where that would show as the copy's crash rather than end this run.
#[test]
fn a_failing_test_that_leaves_a_branch_password_kept_fails_for_its_own_reason() {
    store();
    if env::var_os(FAILING).is_some() {
        let home = common::store::Home::new(|home| drop(keychain::forget(home)));
        keychain::keep_branch_password(home.path(), 480, "testing", "open-sesame").unwrap();
        // libtest prints the test's name on the same line, so the path follows a marker of its own.
        println!("leaky home {}", home.path().display());
        panic!("the test's own failure");
    }
    let output = run::output(
        run::command(env::current_exe().unwrap())
            .args([
                "--exact",
                "a_failing_test_that_leaves_a_branch_password_kept_fails_for_its_own_reason",
                "--nocapture",
                "--test-threads=1",
            ])
            .env(FAILING, "1"),
    )
    .unwrap();
    let said = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let home = said
        .lines()
        .find_map(|line| line.split_once("leaky home ").map(|(_, path)| path))
        .map(PathBuf::from);
    if let Some(home) = &home {
        forget_locked(home);
        if home.exists() {
            fs::remove_dir_all(home).unwrap();
        }
    }
    assert_eq!(output.status.code(), Some(101_i32), "{said}");
    assert!(said.contains("the test's own failure"), "{said}");
    assert!(!said.contains("the credential store still keeps"), "{said}");
    let home = home.unwrap();
    assert_eq!(common::store::kept_for(&home).unwrap(), Vec::new());
}

#[test]
fn a_gone_home_outside_the_swept_folder_keeps_its_secrets_through_a_sweep() {
    store();
    // A folder of the test's own is swept, since the system's temp folder may hold the whole
    // build, as a mutation run's copy of the repository does.
    let swept = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let path = outside.path().to_path_buf();
    let _remains = Remains(vec![path.clone()]);
    let held = common::store::lock();
    keychain::keep(&path, &Key::parse(FIRST).unwrap()).unwrap();
    keychain::keep_branch_password(&path, 480, "testing", "open-sesame").unwrap();
    drop(outside);
    assert!(!path.exists());

    common::store::sweep(swept.path());

    assert!(keychain::has(&path).unwrap(), "{}", path.display());
    assert_eq!(
        keychain::branch_passwords(&path).unwrap(),
        [keychain::Item::branch(&path, 480, "testing")]
    );
    drop(held);
    forget_locked(&path);
    assert_eq!(common::store::kept_for(&path).unwrap(), Vec::new());
}
