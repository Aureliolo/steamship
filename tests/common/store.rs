//! The credential store as the tests use it: a lock on it, and on Linux a Secret Service of the
//! tests' own.
//!
//! Windows' Credential Manager loses changes that different processes make at the same moment: a
//! credential one deletes can come back when another writes its own, which its own `cmdkey` shows
//! with a few processes at once. So every test that keeps or forgets a key holds [`lock`],
//! which also holds off the test runs cargo-mutants starts side by side.
//!
//! One gnome-keyring shared by every test run falls over under many at once (it has been seen to
//! fail its own assertions and exit), and would hold a person's real keys besides. So each test
//! process starts a session bus and gnome-keyring of its own, in a folder of its own, unlocked,
//! with no display for any prompt to open on, and both end when the process does: the script
//! holding them waits on its input, which closes as the process exits.

use std::env;
#[cfg(target_os = "linux")]
use std::ffi::OsString;
use std::fs::{File, OpenOptions};
#[cfg(target_os = "linux")]
use std::io::{BufRead as _, BufReader};
#[cfg(target_os = "linux")]
use std::process::{ChildStdin, Command, Stdio};
#[cfg(target_os = "linux")]
use std::sync::OnceLock;
#[cfg(target_os = "linux")]
use std::thread;
#[cfg(target_os = "linux")]
use std::time::{Duration, Instant};

#[cfg(target_os = "linux")]
use steamship::dbus::Value;
#[cfg(target_os = "linux")]
use steamship::dbus::session::Connection;

/// Starts the bus, prints its address once it listens, then gnome-keyring on it, and ends both
/// when its input ends. The bus starts nothing itself: a session bus would start a gnome-keyring
/// of its own, its keyring locked, for any call that came while the tests' own was not there,
/// and every test after would fail as if a prompt had been dismissed.
#[cfg(target_os = "linux")]
const SCRIPT: &str = r#"
set -eu
folder=$1
unset DISPLAY WAYLAND_DISPLAY
export HOME="$folder" XDG_RUNTIME_DIR="$folder" XDG_DATA_HOME="$folder/data"
mkdir -p "$XDG_DATA_HOME"
cat >"$folder/bus.conf" <<CONFIG
<busconfig>
  <type>session</type>
  <listen>unix:dir=$folder</listen>
  <auth>EXTERNAL</auth>
  <policy context="default">
    <allow send_destination="*" eavesdrop="true"/>
    <allow eavesdrop="true"/>
    <allow own="*"/>
  </policy>
</busconfig>
CONFIG
dbus-daemon --config-file="$folder/bus.conf" --nofork --print-address=3 3>"$folder/address" &
bus=$!
keyring=
trap 'kill $bus $keyring 2>/dev/null; rm -rf "$folder"' EXIT
while [ ! -s "$folder/address" ]; do sleep 0.05; done
DBUS_SESSION_BUS_ADDRESS=$(head -n 1 "$folder/address")
export DBUS_SESSION_BUS_ADDRESS
printf 'tests' | gnome-keyring-daemon --foreground --unlock --components=secrets >/dev/null 2>&1 &
keyring=$!
echo "$DBUS_SESSION_BUS_ADDRESS"
cat >/dev/null
"#;

/// Holds every other test, in this process or another, off the credential store until dropped.
///
/// # Panics
///
/// When the lock file cannot be opened or locked.
#[must_use]
pub fn lock() -> File {
    let file = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(false)
        .open(env::temp_dir().join("steamship-tests-credentials.lock"))
        .unwrap();
    file.lock().unwrap();
    file
}

/// What a `steamship` run by the tests needs set: on Linux this process's own Secret Service.
#[cfg(target_os = "linux")]
#[must_use]
pub fn environment() -> [(&'static str, &'static str); 1] {
    [("DBUS_SESSION_BUS_ADDRESS", secret_service())]
}

/// What a `steamship` run by the tests needs set: elsewhere the system's store is its own.
#[cfg(not(target_os = "linux"))]
#[must_use]
pub const fn environment() -> [(&'static str, &'static str); 0] {
    []
}

#[cfg(target_os = "linux")]
struct Held {
    address: String,
    /// Open for as long as the process runs; its end is the script's cue to stop.
    _running: ChildStdin,
}

/// The address of this process's own session bus, with gnome-keyring on it and ready.
///
/// # Panics
///
/// When they cannot be started, as when dbus-daemon or gnome-keyring-daemon is not installed.
#[cfg(target_os = "linux")]
#[expect(
    clippy::zombie_processes,
    reason = "the script is left running for the process's life, and ends on its own after"
)]
pub fn secret_service() -> &'static str {
    static HELD: OnceLock<Held> = OnceLock::new();
    &HELD
        .get_or_init(|| {
            let folder = tempfile::tempdir().unwrap().keep();
            let mut child = Command::new("sh")
                .args(["-c", SCRIPT, "sh"])
                .arg(&folder)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .spawn()
                .unwrap();
            let mut line = String::new();
            let _: usize = BufReader::new(child.stdout.take().unwrap())
                .read_line(&mut line)
                .unwrap();
            let address = line.trim().to_owned();
            assert!(
                !address.is_empty(),
                "no session bus: the Linux tests need dbus-daemon and gnome-keyring-daemon"
            );
            wait_for_keyring(&address);
            Held {
                address,
                _running: child.stdin.take().unwrap(),
            }
        })
        .address
}

/// Waits until gnome-keyring owns the Secret Service's name, so that nothing asks for it first
/// and has the bus start another; and then until its default keyring is there and unlocked.
/// gnome-keyring takes the name before it has made and unlocked that keyring, and a key kept in
/// the moments between would ask for a prompt, which has no display to open on.
#[cfg(target_os = "linux")]
fn wait_for_keyring(address: &str) {
    let deadline = Instant::now().checked_add(Duration::from_secs(20)).unwrap();
    let address = OsString::from(address);
    let mut bus =
        Connection::session(|name| (name == "DBUS_SESSION_BUS_ADDRESS").then(|| address.clone()))
            .unwrap();
    loop {
        let owned = bus
            .call(
                "org.freedesktop.DBus",
                "/org/freedesktop/DBus",
                "org.freedesktop.DBus",
                "NameHasOwner",
                &[Value::Str("org.freedesktop.secrets".to_owned())],
            )
            .unwrap();
        if owned.first() == Some(&Value::Bool(true)) && default_unlocked(&mut bus) {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "gnome-keyring never took the name, or never unlocked its default keyring"
        );
        thread::sleep(Duration::from_millis(50));
    }
}

/// Whether the keyring the `default` alias names is there and unlocked.
#[cfg(target_os = "linux")]
fn default_unlocked(bus: &mut Connection) -> bool {
    let Ok(alias) = bus.call(
        "org.freedesktop.secrets",
        "/org/freedesktop/secrets",
        "org.freedesktop.Secret.Service",
        "ReadAlias",
        &[Value::Str("default".to_owned())],
    ) else {
        return false;
    };
    let Some(Value::Path(collection)) = alias.first() else {
        return false;
    };
    if collection == "/" {
        return false;
    }
    let locked = bus.call(
        "org.freedesktop.secrets",
        collection,
        "org.freedesktop.DBus.Properties",
        "Get",
        &[
            Value::Str("org.freedesktop.Secret.Collection".to_owned()),
            Value::Str("Locked".to_owned()),
        ],
    );
    matches!(
        locked.as_deref(),
        Ok([Value::Variant(unlocked)]) if **unlocked == Value::Bool(false)
    )
}
