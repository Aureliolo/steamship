//! The system's real credential store, as the library keeps a key in it. Each test keeps its key
//! for a home of its own, so it never meets a person's real key, and forgets it again. On Linux the
//! store is a gnome-keyring of this process's own.
//!
//! A key the `steamship` command keeps, shows and forgets is tested in `steamship.rs`, by the
//! command itself: on macOS a Keychain item answers the program that made it, so a key this test
//! program kept is not one a `steamship` it runs may remove.
#![expect(
    clippy::unwrap_used,
    reason = "a test reports failure by panicking, its helpers included"
)]

pub mod common;

#[cfg(target_os = "linux")]
use std::env;
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

#[test]
fn a_key_is_kept_replaced_read_back_and_forgotten_for_its_home_alone() {
    store();
    let _store = common::store::lock();
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
    let _store = common::store::lock();
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
