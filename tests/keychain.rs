//! The system's real credential store, as `login --web-api-key`, `builds`, `status` and `logout`
//! use it. Each test keeps its key for a home of its own, so it never meets a person's real key,
//! and forgets it again. On Linux the store is a gnome-keyring of this process's own.
#![expect(
    clippy::unwrap_used,
    reason = "a test reports failure by panicking, its helpers included"
)]

pub mod common;

#[cfg(target_os = "linux")]
use std::env;
use std::path::Path;
use std::process::Command;
#[cfg(target_os = "linux")]
use std::sync::Once;

use steamship::keychain;
#[cfg(target_os = "macos")]
use steamship::macos as platform;
#[cfg(target_os = "linux")]
use steamship::secret_service as platform;
use steamship::webapi::Key;
#[cfg(windows)]
use steamship::windows as platform;

const FIRST: &str = "0123456789ABCDEF0123456789ABCDEF";
const SECOND: &str = "fedcba9876543210fedcba9876543210";

/// Points this process, and what it runs, at its own store before anything here reads the
/// environment. Every test calls it first.
#[cfg(target_os = "linux")]
#[expect(
    unsafe_code,
    reason = "the library finds the session bus in the environment, as steamship does"
)]
fn store() {
    static SET: Once = Once::new();
    SET.call_once(|| {
        let address = common::secret_service();
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

fn steamship(args: &[&str], home: &Path, variables: &[(&str, &str)]) -> String {
    let output = Command::new(env!("CARGO_BIN_EXE_steamship"))
        .args(args)
        .env("STEAMSHIP_HOME", home)
        .env("STEAMSHIP_NO_UPDATE_CHECK", "1")
        .env_remove("STEAMSHIP_WEB_API_KEY")
        .envs(variables.iter().copied())
        .output()
        .unwrap();
    String::from_utf8_lossy(&output.stdout).into_owned()
}

#[test]
fn a_key_is_kept_replaced_read_back_and_forgotten_for_its_home_alone() {
    store();
    let _store = common::store_lock();
    let home = tempfile::tempdir().unwrap();
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
    let _store = common::store_lock();
    let home = tempfile::tempdir().unwrap();
    let account = home.path().display().to_string();
    for kept in [&b"not a key"[..], &[0xff, 0xfe][..]] {
        platform::keep_secret(&account, kept).unwrap();
        assert_eq!(
            keychain::kept(home.path()).unwrap_err(),
            keychain::Error::NotAKey
        );
    }
    assert!(keychain::forget(home.path()).unwrap());
}

#[test]
fn status_shows_a_kept_key_and_one_set_and_logout_forgets_the_kept_one() {
    store();
    let _store = common::store_lock();
    let home = tempfile::tempdir().unwrap();
    let kept = Key::parse("0123456789abcdef0123456789abcdef").unwrap();
    keychain::keep(home.path(), &kept).unwrap();
    let shown = steamship(&["status"], home.path(), &[]);
    let set = steamship(
        &["status"],
        home.path(),
        &[("STEAMSHIP_WEB_API_KEY", "fedcba9876543210fedcba9876543210")],
    );
    let forgot = steamship(&["logout"], home.path(), &[]);
    let still = keychain::has(home.path()).unwrap();
    let _: bool = keychain::forget(home.path()).unwrap();
    assert!(
        shown.contains(&format!("  api key   kept in {}\n", keychain::STORE)),
        "{shown}"
    );
    assert!(
        set.contains("  api key   from STEAMSHIP_WEB_API_KEY\n"),
        "the variable comes first: {set}"
    );
    assert!(!set.contains("fedcba"), "{set}");
    assert!(
        forgot.contains("  api key   \u{2713} forgotten\n"),
        "{forgot}"
    );
    assert!(!still, "logout forgets the kept key");
}
