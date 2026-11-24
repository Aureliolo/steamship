//! `steamship logout`.

use std::fs::{self, File};
use std::path::{Path, PathBuf};
use std::thread;
use std::time::Duration;

use crate::common;
use crate::{
    APPS, KEY, faked_with, fetching, forgetting_home, login_with_key, logout, saved_login,
    steamship, web_api,
};

/// Keeps a file out of a removal's reach until dropped.
///
/// Unix, for a user who is not root, does it with permissions: none on the file, and a folder
/// whose entries cannot change. Windows does it by holding the file open and sharing it with
/// nobody, which is what a virus scanner or an open editor does to a file there.
struct Kept {
    #[cfg(unix)]
    path: PathBuf,
    #[cfg(windows)]
    _open: File,
}

impl Kept {
    #[cfg(unix)]
    fn new(path: &Path) -> Self {
        use std::os::unix::fs::PermissionsExt as _;

        fs::set_permissions(path, fs::Permissions::from_mode(0o000)).unwrap();
        let folder = path.parent().unwrap();
        fs::set_permissions(folder, fs::Permissions::from_mode(0o500)).unwrap();
        Self {
            path: path.to_path_buf(),
        }
    }

    #[cfg(windows)]
    fn new(path: &Path) -> Self {
        use std::os::windows::fs::OpenOptionsExt as _;

        let open = File::options().read(true).share_mode(0).open(path).unwrap();
        Self { _open: open }
    }
}

#[cfg(unix)]
impl Drop for Kept {
    fn drop(&mut self) {
        use std::os::unix::fs::PermissionsExt as _;

        // Put back, so the temporary folder can be cleaned up after the test.
        let folder = self.path.parent().unwrap();
        fs::set_permissions(folder, fs::Permissions::from_mode(0o700)).unwrap();
        fs::set_permissions(&self.path, fs::Permissions::from_mode(0o600)).unwrap();
    }
}

/// A file that cannot be removed, in a folder of its own inside `parent`, so that clearing
/// `parent` fails.
fn stuck_in(parent: &Path, folder: &str) -> Kept {
    let file = parent.join(folder).join("file");
    fs::create_dir_all(file.parent().unwrap()).unwrap();
    fs::write(&file, "held").unwrap();
    Kept::new(&file)
}

fn write(home: &Path, file: &str, text: &str) -> PathBuf {
    let path = home.join(file);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, text).unwrap();
    path
}

const TOKEN: &str = "\"token\" \"a_saved_login_token_0123456789\"";

fn keep_a_key(home: &Path) {
    let (host, _requests) = web_api(vec![("200 OK", APPS)]);
    let (code, stdout, stderr) = login_with_key(home, &host, KEY, &[]);
    assert_eq!(code, Some(0_i32), "{stdout}{stderr}");
}

#[test]
fn logout_waits_for_another_steamship_using_the_home_and_keeps_the_login_until_then() {
    let home = forgetting_home();
    let saved = home.path().join("steamcmd/config/config.vdf");
    fs::create_dir_all(saved.parent().unwrap()).unwrap();
    fs::write(&saved, "\"token\" \"a_saved_login_token_0123456789\"").unwrap();
    let account = home.path().join("account");
    fs::write(&account, "build_bot\n").unwrap();
    let held = File::create(home.path().join("steamship.lock")).unwrap();
    held.try_lock().unwrap();
    let path = home.path().to_path_buf();
    let second = thread::spawn(move || steamship(&["logout"], Some(&path), &[]));
    thread::sleep(Duration::from_secs(2));
    assert!(!second.is_finished(), "it waits while the home is held");
    assert!(
        saved.exists() && account.exists(),
        "the login is kept while another steamship holds the home"
    );
    held.unlock().unwrap();
    let (code, stdout, stderr) = second.join().unwrap();
    assert_eq!((code, stderr.as_str()), (Some(0_i32), ""), "{stdout}");
    assert!(
        stdout.contains(&format!(
            "  steamcmd  waiting for another steamship using {} to finish...\n",
            home.path().display()
        )) && stdout.contains("\u{2713} free after ")
            && stdout.contains("  login     \u{2713} forgotten\n"),
        "{stdout}"
    );
    assert!(!saved.exists() && !account.exists());
}

#[test]
fn logout_forgets_steamcmds_saved_login_on_every_system_and_the_account() {
    let home = forgetting_home();
    let saved = [
        "steamcmd/config/config.vdf",
        "Steam/config/config.vdf",
        "Library/Application Support/Steam/config/config.vdf",
        "steamcmd/userdata/12345/config/localconfig.vdf",
    ];
    for file in saved {
        let path = home.path().join(file);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, "\"token\" \"a_saved_login_token_0123456789\"").unwrap();
    }
    fs::write(home.path().join("account"), "build_bot\n").unwrap();
    let kept = home.path().join("steamcmd/config/other.vdf");
    fs::write(&kept, "kept").unwrap();
    let (code, stdout, stderr) = steamship(&["logout"], Some(home.path()), &[]);
    assert_eq!((code, stderr.as_str()), (Some(0_i32), ""), "{stdout}");
    assert_eq!(
        stdout,
        "steamship logout\n  \
         login     \u{2713} forgotten\n  \
         account   \u{2713} forgotten\n  \
         api key   none kept\n  \
         passwords none kept\n  \
         \u{2713} logged out\n    \
         the next upload needs steamship login first\n"
    );
    for file in saved {
        assert!(!home.path().join(file).exists(), "{file}");
    }
    assert!(!home.path().join("account").exists());
    assert!(kept.exists(), "only the login is removed");
    let (again, said, _) = steamship(&["logout"], Some(home.path()), &[]);
    assert_eq!(again, Some(0_i32));
    assert!(
        said.contains("  login     none saved\n  account   none remembered\n"),
        "{said}"
    );
}

#[test]
fn logout_forgets_a_login_that_a_stopped_install_carried() {
    let home = forgetting_home();
    let carried = write(home.path(), "steamcmd.new/config/config.vdf", TOKEN);
    fs::create_dir_all(home.path().join("steamcmd.old")).unwrap();
    let (code, stdout, stderr) = steamship(&["logout"], Some(home.path()), &[]);
    assert_eq!((code, stderr.as_str()), (Some(0_i32), ""), "{stdout}");
    assert!(
        stdout.contains("  login     \u{2713} forgotten\n"),
        "{stdout}"
    );
    assert!(!carried.exists());
    assert!(!home.path().join("steamcmd.new").exists());
    assert!(!home.path().join("steamcmd.old").exists());
    drop(write(home.path(), "steamcmd.new/config/other.txt", "kept"));
    let (again, said, _) = steamship(&["logout"], Some(home.path()), &[]);
    assert_eq!(again, Some(0_i32), "{said}");
    assert!(said.contains("  login     none saved\n"), "{said}");
}

#[test]
fn a_login_file_that_cannot_be_removed_still_lets_the_rest_go() {
    let home = forgetting_home();
    let config = write(home.path(), "steamcmd/config/config.vdf", TOKEN);
    let local = write(
        home.path(),
        "steamcmd/userdata/12345/config/localconfig.vdf",
        TOKEN,
    );
    let account = write(home.path(), "account", "build_bot\n");
    let left = write(home.path(), "steamcmd.old/left", "left");
    keep_a_key(home.path());
    let held = Kept::new(&config);
    let (code, stdout, stderr) = steamship(&["logout"], Some(home.path()), &[]);
    drop(held);
    assert_eq!(code, Some(1_i32), "{stdout}{stderr}");
    assert!(
        stdout.contains("  login     \u{2717} not forgotten\n"),
        "{stdout}"
    );
    assert!(stderr.contains("config.vdf"), "{stderr}");
    assert!(
        stdout.contains("  account   \u{2713} forgotten\n"),
        "{stdout}"
    );
    assert!(
        stdout.contains("  api key   \u{2713} forgotten\n"),
        "{stdout}"
    );
    assert!(!stdout.contains("logged out"), "{stdout}");
    assert!(!local.exists() && !account.exists() && !left.exists());
}

#[test]
fn a_login_list_that_cannot_be_read_still_lets_every_step_run() {
    let home = forgetting_home();
    drop(write(home.path(), "Steam/userdata", "not a folder"));
    let tokens = [
        "steamcmd/config/config.vdf",
        "Steam/config/config.vdf",
        "Library/Application Support/Steam/config/config.vdf",
    ]
    .map(|file| write(home.path(), file, TOKEN));
    let account = write(home.path(), "account", "build_bot\n");
    let left = write(home.path(), "steamcmd.old/left", "left");
    let (code, stdout, stderr) = steamship(&["logout"], Some(home.path()), &[]);
    assert_eq!(code, Some(1_i32), "{stdout}{stderr}");
    assert!(
        stdout.contains("  login     \u{2717} not forgotten\n"),
        "{stdout}"
    );
    assert!(stderr.contains("userdata"), "{stderr}");
    assert!(
        stdout.contains("  account   \u{2713} forgotten\n"),
        "{stdout}"
    );
    assert!(stdout.contains("  api key   none kept\n"), "{stdout}");
    assert!(!stdout.contains("logged out"), "{stdout}");
    assert!(!account.exists() && !left.exists());
    for token in &tokens {
        assert!(!token.exists(), "{} is left", token.display());
    }
}

#[test]
fn a_working_folder_that_cannot_be_cleared_fails_logout_once_the_login_is_gone() {
    let home = forgetting_home();
    let saved = write(home.path(), "steamcmd/config/config.vdf", TOKEN);
    let account = write(home.path(), "account", "build_bot\n");
    keep_a_key(home.path());
    let held = stuck_in(&home.path().join("steamcmd.old"), "stuck");
    let (code, stdout, stderr) = steamship(&["logout"], Some(home.path()), &[]);
    drop(held);
    assert_eq!(code, Some(1_i32), "{stdout}{stderr}");
    assert!(
        stdout.contains("  login     \u{2713} forgotten\n"),
        "{stdout}"
    );
    assert!(
        stdout.contains("  account   \u{2713} forgotten\n"),
        "{stdout}"
    );
    assert!(
        stdout.contains("  api key   \u{2713} forgotten\n"),
        "{stdout}"
    );
    assert!(!stdout.contains("logged out"), "{stdout}");
    assert!(
        stdout.contains("  install folders \u{2717} not cleared\n"),
        "{stdout}"
    );
    assert!(stderr.contains("steamcmd.old"), "{stderr}");
    assert!(
        stderr.contains("remove what is named by hand, then run steamship logout again"),
        "{stderr}"
    );
    assert!(!saved.exists() && !account.exists());
}

/// A login `file` carried in `folder` of a home, beside a file in it that cannot be removed, and
/// `logout` run on it. Whether the login file is left, the exit code, and what it printed.
fn logout_beside_a_stuck_file(folder: &str, file: &str) -> (bool, Option<i32>, String, String) {
    let home = forgetting_home();
    let carried = write(home.path(), &format!("{folder}/{file}"), TOKEN);
    // Named to sort after the login, so a clear that ran first would still reach the login.
    let held = stuck_in(&home.path().join(folder), "zz_held");
    let (code, stdout, stderr) = steamship(&["logout"], Some(home.path()), &[]);
    drop(held);
    (carried.exists(), code, stdout, stderr)
}

#[test]
fn a_carried_login_goes_even_when_its_folder_cannot_be_cleared() {
    for (folder, file) in [
        ("steamcmd.new", "config/config.vdf"),
        ("steamcmd.old", "userdata/12345/config/localconfig.vdf"),
    ] {
        let (left, code, stdout, stderr) = logout_beside_a_stuck_file(folder, file);
        assert_eq!(code, Some(1_i32), "{folder}: {stdout}{stderr}");
        assert!(
            stdout.contains("  login     \u{2713} forgotten\n"),
            "{folder}: {stdout}"
        );
        assert!(!left, "{folder}: {file} is left");
        assert!(stderr.contains(folder), "{folder}: {stderr}");
    }
}

/// A steamcmd that logs in, waits at its console, and installs the app it is typed `app_update`
/// for, on the friends branch, as build 4100.
const FETCHES_ANY_APP: &str = "#!/bin/sh\n\
    folder=\n\
    while [ $# -gt 0 ]; do\n\
      case \"$1\" in\n\
        +force_install_dir) folder=\"$2\"; shift ;;\n\
      esac\n\
      shift\n\
    done\n\
    echo \"Logging in user 'build_bot' [U:1:1] to Steam Public...OK\"\n\
    printf 'Steam>'\n\
    read -r typed\n\
    echo \"$typed\"\n\
    app=${typed#app_update }\n\
    app=${app%% *}\n\
    mkdir -p \"$folder/steamapps\"\n\
    printf '\"AppState\"\\n{\\n\"buildid\" \"4100\"\\n\"UserConfig\"\\n{\\n\"BetaKey\" \"friends\"\\n}\\n}\\n' \
      > \"$folder/steamapps/appmanifest_$app.acf\"\n\
    echo \"Success! App '$app' fully installed.\"\n\
    printf 'Steam>'\n\
    read -r quit\n";

/// Fetches the friends branch of each of `apps` with `--keep-password`, so that a password is
/// kept for each.
fn keep_passwords(home: &Path, apps: &[&str]) {
    saved_login(home);
    for app in apps {
        let (code, stdout, stderr) = fetching(
            home,
            &[*app, "--branch", "friends", "--keep-password"],
            &[("STEAMSHIP_BRANCH_PASSWORD", "open-sesame")],
        );
        assert_eq!(code, Some(0_i32), "{app}: {stdout}{stderr}");
    }
}

/// What the store keeps for `home`, as its kinds in order.
fn kept_kinds(home: &Path) -> Vec<&'static str> {
    let mut kinds: Vec<&'static str> = common::store::kept_for(home)
        .unwrap()
        .iter()
        .map(|item| item.kind)
        .collect();
    kinds.sort_unstable();
    kinds
}

/// Whether a fetch of app `app`'s friends branch with no password given finds one kept.
fn finds_a_password(home: &Path, app: &str) -> bool {
    saved_login(home);
    let (code, stdout, stderr) = fetching(home, &[app, "--branch", "friends"], &[]);
    assert!(matches!(code, Some(0_i32 | 2_i32)), "{stdout}{stderr}");
    code == Some(0_i32)
}

#[test]
fn logout_forgets_every_branch_password_kept_for_its_home_and_the_key() {
    let home = common::store::Home::of(faked_with(FETCHES_ANY_APP), logout);
    keep_a_key(home.path());
    keep_passwords(home.path(), &["1000", "1001"]);
    assert_eq!(
        kept_kinds(home.path()),
        ["branch-password", "branch-password", "web-api-key"]
    );
    assert!(finds_a_password(home.path(), "1000") && finds_a_password(home.path(), "1001"));
    let (code, stdout, stderr) = steamship(&["logout"], Some(home.path()), &[]);
    assert_eq!((code, stderr.as_str()), (Some(0_i32), ""), "{stdout}");
    assert!(
        stdout.contains("  api key   \u{2713} forgotten\n  passwords \u{2713} 2 forgotten\n"),
        "{stdout}"
    );
    assert!(kept_kinds(home.path()).is_empty());
    assert!(!finds_a_password(home.path(), "1000"));
    assert!(!finds_a_password(home.path(), "1001"));
    let (again, said, _) = steamship(&["logout"], Some(home.path()), &[]);
    assert_eq!(again, Some(0_i32), "{said}");
    assert!(
        said.contains("  api key   none kept\n  passwords none kept\n"),
        "{said}"
    );
}

#[test]
fn logout_forgets_the_branch_passwords_of_a_home_that_kept_no_key() {
    let home = common::store::Home::of(faked_with(FETCHES_ANY_APP), logout);
    keep_passwords(home.path(), &["1000"]);
    assert_eq!(kept_kinds(home.path()), ["branch-password"]);
    let (code, stdout, stderr) = steamship(&["logout"], Some(home.path()), &[]);
    assert_eq!((code, stderr.as_str()), (Some(0_i32), ""), "{stdout}");
    assert!(
        stdout.contains("  api key   none kept\n  passwords \u{2713} 1 forgotten\n"),
        "{stdout}"
    );
    assert!(kept_kinds(home.path()).is_empty());
}

/// Copies the folder `from` and all it holds to the empty folder `to`.
fn copy_tree(from: &Path, to: &Path) {
    for entry in fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            fs::create_dir_all(&target).unwrap();
            copy_tree(&entry.path(), &target);
        } else {
            let _: u64 = fs::copy(entry.path(), target).unwrap();
        }
    }
}

/// Homes beside another, whose key and passwords are forgotten by `logout` before they go.
struct Beside(Vec<tempfile::TempDir>);

impl Drop for Beside {
    fn drop(&mut self) {
        for home in &self.0 {
            logout(home.path());
        }
    }
}

#[test]
fn logout_leaves_the_key_and_passwords_of_homes_whose_path_starts_the_same() {
    let home = common::store::Home::of(faked_with(FETCHES_ANY_APP), logout);
    let stem = home.path().file_name().unwrap().to_string_lossy().into_owned();
    let parent = home.path().parent().unwrap();
    let beside = Beside(
        ["x", " b"]
            .map(|tail| {
                let other = tempfile::Builder::new()
                    .prefix(&format!("{stem}{tail}"))
                    .rand_bytes(0)
                    .tempdir_in(parent)
                    .unwrap();
                copy_tree(home.path(), other.path());
                other
            })
            .into(),
    );
    for other in &beside.0 {
        keep_a_key(other.path());
        keep_passwords(other.path(), &["1000"]);
    }
    keep_a_key(home.path());
    keep_passwords(home.path(), &["1000", "1001"]);
    let (code, stdout, stderr) = steamship(&["logout"], Some(home.path()), &[]);
    assert_eq!((code, stderr.as_str()), (Some(0_i32), ""), "{stdout}");
    assert!(
        stdout.contains("  passwords \u{2713} 2 forgotten\n"),
        "{stdout}"
    );
    assert!(kept_kinds(home.path()).is_empty());
    for other in &beside.0 {
        assert_eq!(
            kept_kinds(other.path()),
            ["branch-password", "web-api-key"],
            "{}",
            other.path().display()
        );
        assert!(
            finds_a_password(other.path(), "1000"),
            "{}",
            other.path().display()
        );
    }
}
