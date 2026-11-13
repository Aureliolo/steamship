//! Installing steamcmd from packages built here, fetched by copying, so every path the installer
//! can take is exercised without Valve's CDN.
use std::cell::Cell;
use std::fs::{self, File};
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use steamship::install::{self, Error, Held, Outcome, State};
use steamship::manifest::{Manifest, Package};
use steamship::{digest, download};
use zip::ZipWriter;
use zip::write::SimpleFileOptions;

enum Entry<'content> {
    File(&'content [u8]),
    Folder,
    Link(&'content str),
}

struct Setup {
    _temp: tempfile::TempDir,
    home: PathBuf,
    packages: PathBuf,
}

impl Setup {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("home (x86)");
        let packages = temp.path().join("cdn");
        fs::create_dir_all(&packages).unwrap();
        Self {
            _temp: temp,
            home,
            packages,
        }
    }

    /// A zip of `entries` on the stand-in CDN, and the package that pins it.
    fn package(&self, name: &str, entries: &[(&str, Entry<'_>)]) -> Package {
        let path = self.packages.join(format!("{name}.zip"));
        let mut zip = ZipWriter::new(File::create(&path).unwrap());
        let options =
            SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
        for (entry, content) in entries {
            match content {
                Entry::File(bytes) => {
                    zip.start_file(*entry, options).unwrap();
                    zip.write_all(bytes).unwrap();
                }
                Entry::Folder => zip.add_directory(*entry, options).unwrap(),
                Entry::Link(target) => zip.add_symlink(*entry, *target, options).unwrap(),
            }
        }
        drop(zip.finish().unwrap());
        let bytes = fs::read(&path).unwrap();
        Package {
            name: name.to_owned(),
            file: format!("{name}.zip"),
            size: u64::try_from(bytes.len()).unwrap(),
            sha256: digest::sha256(&mut bytes.as_slice()).unwrap(),
        }
    }

    fn manifest(version: u64, packages: Vec<Package>) -> Manifest {
        Manifest {
            system: "test".to_owned(),
            version,
            packages,
            text: format!("\"test\" {{ \"version\" \"{version}\" }}"),
        }
    }

    /// Installs, and counts the packages fetched.
    fn install(&self, manifest: &Manifest, fetched: &Cell<usize>) -> Result<Outcome, Error> {
        install::using(&self.held(), manifest, |package, into| {
            fetched.set(fetched.get().saturating_add(1));
            let _: u64 = fs::copy(self.packages.join(&package.file), into).unwrap();
            Ok(())
        })
    }

    /// The home, held, which nothing else in a test holds.
    fn held(&self) -> Held {
        install::hold(&self.home, || panic!("nothing else holds the home")).unwrap()
    }

    fn root(&self) -> PathBuf {
        self.home.join(install::FOLDER)
    }
}

fn changes(result: Result<Outcome, Error>) -> Vec<String> {
    match result {
        Err(Error::Altered { changes, .. }) => changes,
        other => panic!("expected the install to be reported altered, got {other:?}"),
    }
}

/// A stand-in for steamcmd. Its program is not called `.exe`: on Windows an installed `.exe` must
/// carry Valve's signature, which nothing built here can, and that check has tests of its own.
fn steamcmd(setup: &Setup) -> Manifest {
    let tool = setup.package(
        "steamcmd_test",
        &[
            ("steamcmd.bin", Entry::File(b"MZ steamcmd")),
            ("steamcmd.sh", Entry::File(b"#!/bin/sh\n")),
        ],
    );
    let data = setup.package(
        "steamcmd_public_all",
        // Valve's zips list their folders as entries of their own, as this one does.
        &[
            ("public/", Entry::Folder),
            ("public/strings.txt", Entry::File(b"text")),
        ],
    );
    Setup::manifest(1, vec![tool, data])
}

#[cfg(not(unix))]
#[test]
fn a_link_in_a_package_for_this_system_is_refused() {
    use std::io::ErrorKind;

    let setup = Setup::new();
    let linked = setup.package("steamcmd_linked", &[("a/link", Entry::Link("b"))]);
    let result = setup.install(&Setup::manifest(1, vec![linked]), &Cell::new(0));
    assert!(
        matches!(&result, Err(Error::Io { error, .. }) if error.kind() == ErrorKind::Unsupported),
        "{result:?}"
    );
}

#[test]
fn installs_once_then_verifies_without_fetching_again() {
    let setup = Setup::new();
    let manifest = steamcmd(&setup);
    let fetched = Cell::new(0);
    assert!(matches!(
        setup.install(&manifest, &fetched),
        Ok(Outcome::Installed)
    ));
    assert_eq!(fetched.get(), 2);
    assert_eq!(
        fs::read(setup.root().join("steamcmd.bin")).unwrap(),
        b"MZ steamcmd"
    );
    assert_eq!(
        fs::read(setup.root().join("public/strings.txt")).unwrap(),
        b"text"
    );
    assert!(
        !setup.home.join("downloads").exists(),
        "the verified packages are not kept"
    );
    assert!(matches!(
        setup.install(&manifest, &fetched),
        Ok(Outcome::Verified)
    ));
    assert_eq!(fetched.get(), 2);
    install::verify(&setup.held(), &manifest).unwrap();
}

#[test]
fn the_state_of_an_install_is_read_without_changing_it() {
    let setup = Setup::new();
    let manifest = steamcmd(&setup);
    assert!(matches!(
        install::state(&setup.home, &manifest),
        Ok(State::Missing)
    ));
    assert!(!setup.home.exists(), "nothing is made to read the state");
    let _: Outcome = setup.install(&manifest, &Cell::new(0)).unwrap();
    assert!(matches!(
        install::state(&setup.home, &manifest),
        Ok(State::Pinned)
    ));
    let raised = Setup::manifest(2, manifest.packages.clone());
    assert!(matches!(
        install::state(&setup.home, &raised),
        Ok(State::Earlier)
    ));
    fs::write(setup.root().join("steamcmd.bin"), b"MZ newer").unwrap();
    assert_eq!(
        changes(install::state(&setup.home, &manifest).map(|_| Outcome::Verified)),
        ["steamcmd.bin has changed"]
    );
    assert!(matches!(
        install::state(&setup.home, &raised),
        Ok(State::Earlier)
    ));
}

#[test]
fn every_change_to_the_install_is_named() {
    let setup = Setup::new();
    let manifest = steamcmd(&setup);
    let fetched = Cell::new(0);
    let _: Outcome = setup.install(&manifest, &fetched).unwrap();
    fs::write(setup.root().join("steamcmd.bin"), b"MZ newer").unwrap();
    fs::remove_file(setup.root().join("public/strings.txt")).unwrap();
    fs::write(setup.root().join("update.dll"), b"MZ arrived").unwrap();
    fs::write(setup.root().join("logs.txt"), b"steamcmd writes these").unwrap();
    fs::create_dir_all(setup.root().join("config")).unwrap();
    fs::write(setup.root().join("config/tool.exe"), b"MZ in config").unwrap();
    let mut found = changes(setup.install(&manifest, &fetched));
    found.sort();
    assert_eq!(
        found,
        [
            "public/strings.txt is missing",
            "steamcmd.bin has changed",
            "update.dll is new"
        ]
    );
    assert!(
        install::verify(&setup.held(), &manifest)
            .is_err_and(|error| matches!(error, Error::Altered { .. }))
    );
}

#[test]
fn steamcmd_is_told_its_version_and_not_to_update_itself_and_held_to_both() {
    let setup = Setup::new();
    let manifest = steamcmd(&setup);
    let _: Outcome = setup.install(&manifest, &Cell::new(0)).unwrap();
    let settings = setup.root().join("steam.cfg");
    assert_eq!(
        fs::read_to_string(&settings).unwrap(),
        "BootStrapperInhibitAll=enable\n"
    );
    let copy = setup.root().join("package/steam_cmd_test.manifest");
    assert_eq!(fs::read_to_string(&copy).unwrap(), manifest.text);
    fs::write(&settings, "").unwrap();
    fs::write(&copy, "").unwrap();
    let mut found = changes(install::verify(&setup.held(), &manifest).map(|()| Outcome::Verified));
    found.sort();
    assert_eq!(
        found,
        [
            "package/steam_cmd_test.manifest has changed",
            "steam.cfg has changed"
        ]
    );
}

#[test]
fn a_new_version_replaces_the_install_and_keeps_the_login() {
    let setup = Setup::new();
    let fetched = Cell::new(0);
    let _: Outcome = setup.install(&steamcmd(&setup), &fetched).unwrap();
    fs::create_dir_all(setup.root().join("config")).unwrap();
    fs::write(setup.root().join("config/config.vdf"), b"token").unwrap();
    let newer = setup.package("steamcmd_newer", &[("steamcmd.bin", Entry::File(b"MZ v2"))]);
    let manifest = Setup::manifest(2, vec![newer]);
    assert!(matches!(
        setup.install(&manifest, &fetched),
        Ok(Outcome::Installed)
    ));
    assert_eq!(
        fs::read(setup.root().join("config/config.vdf")).unwrap(),
        b"token"
    );
    assert_eq!(
        fs::read(setup.root().join("steamcmd.bin")).unwrap(),
        b"MZ v2"
    );
    assert!(
        !setup.root().join("public").exists(),
        "nothing of the old version is left"
    );
    install::verify(&setup.held(), &manifest).unwrap();
}

#[test]
fn a_package_that_would_write_outside_is_refused() {
    let setup = Setup::new();
    let evil = setup.package("steamcmd_evil", &[("../outside.txt", Entry::File(b"x"))]);
    let result = setup.install(&Setup::manifest(1, vec![evil]), &Cell::new(0));
    assert!(
        matches!(&result, Err(Error::Package { reason, .. })
            if reason == "holds \"../outside.txt\", which would land outside steamcmd's folder"),
        "{result:?}"
    );
    assert!(!setup.home.join("outside.txt").exists());
    assert!(!setup.root().exists(), "a refused package installs nothing");
}

#[test]
fn a_fetch_that_brings_the_wrong_file_is_refused() {
    let setup = Setup::new();
    let manifest = steamcmd(&setup);
    let result = install::using(&setup.held(), &manifest, |_, into| {
        fs::write(into, b"not the package").unwrap();
        Ok(())
    });
    assert!(
        matches!(&result, Err(Error::Package { reason, .. }) if reason == "is not the pinned file"),
        "{result:?}"
    );
}

#[test]
fn a_package_the_cdn_no_longer_has_is_the_pin_withdrawn_and_any_other_refusal_is_as_it_came() {
    let setup = Setup::new();
    let manifest = steamcmd(&setup);
    for (status, withdrawn) in [(404, true), (410, true), (403, false)] {
        let result = install::using(&setup.held(), &manifest, |package, _| {
            Err(Error::Download(download::Error::Gone {
                url: package.file.clone(),
                status,
            }))
        });
        match result {
            Err(Error::Withdrawn {
                version,
                system,
                file,
            }) if withdrawn => {
                assert_eq!(
                    (version, system.as_str(), file.as_str()),
                    (
                        manifest.version,
                        "test",
                        manifest.packages.first().unwrap().file.as_str()
                    )
                );
            }
            Err(Error::Download(download::Error::Gone { status: kept, .. })) if !withdrawn => {
                assert_eq!(kept, status);
            }
            other => panic!("{status}: {other:?}"),
        }
    }
    let result = install::using(&setup.held(), &manifest, |package, _| {
        Err(Error::Download(download::Error::GaveUp {
            url: package.file.clone(),
            last: "answered 503".to_owned(),
        }))
    });
    assert!(
        matches!(result, Err(Error::Download(download::Error::GaveUp { .. }))),
        "{result:?}"
    );
}

#[test]
fn a_package_already_downloaded_and_pinned_is_not_fetched_again() {
    let setup = Setup::new();
    let manifest = steamcmd(&setup);
    let downloads = setup.home.join("downloads");
    fs::create_dir_all(&downloads).unwrap();
    for package in &manifest.packages {
        let _: u64 = fs::copy(
            setup.packages.join(&package.file),
            downloads.join(&package.file),
        )
        .unwrap();
    }
    let fetched = Cell::new(0);
    assert!(matches!(
        setup.install(&manifest, &fetched),
        Ok(Outcome::Installed)
    ));
    assert_eq!(fetched.get(), 0);
}

#[test]
fn an_unreadable_inventory_means_installing_afresh() {
    let setup = Setup::new();
    let manifest = steamcmd(&setup);
    let fetched = Cell::new(0);
    let _: Outcome = setup.install(&manifest, &fetched).unwrap();
    fs::write(setup.home.join("steamcmd.inventory"), b"not an inventory").unwrap();
    assert!(matches!(
        setup.install(&manifest, &fetched),
        Ok(Outcome::Installed)
    ));
    install::verify(&setup.held(), &manifest).unwrap();
}

#[test]
fn verifying_with_nothing_installed_says_so() {
    let setup = Setup::new();
    let manifest = steamcmd(&setup);
    let error = install::verify(&setup.held(), &manifest).unwrap_err();
    assert!(
        error
            .to_string()
            .ends_with("no install of version 1 is recorded"),
        "{error}"
    );
}

#[test]
fn a_second_steamship_waits_until_the_first_lets_go_of_the_home() {
    let setup = Setup::new();
    let first = setup.held();
    let (told, waiting) = mpsc::channel();
    let home = setup.home.clone();
    let second = thread::spawn(move || {
        install::hold(&home, || told.send(()).unwrap()).map(|held| held.home().to_path_buf())
    });
    waiting.recv_timeout(Duration::from_secs(10)).unwrap();
    thread::sleep(Duration::from_millis(200));
    assert!(
        !second.is_finished(),
        "it waits while the first holds the home"
    );
    drop(first);
    assert_eq!(second.join().unwrap().unwrap(), setup.home);
    drop(install::hold(&setup.home, || panic!("the second let go when it ended")).unwrap());
}

#[cfg(unix)]
#[test]
fn programs_are_made_executable_and_the_rest_is_not() {
    use std::os::unix::fs::PermissionsExt as _;

    let setup = Setup::new();
    let _: Outcome = setup.install(&steamcmd(&setup), &Cell::new(0)).unwrap();
    let mode = |relative: &str| {
        fs::metadata(setup.root().join(relative))
            .unwrap()
            .permissions()
            .mode()
            & 0o777
    };
    assert_eq!(mode("steamcmd.sh"), 0o755);
    assert_eq!(mode("steamcmd.bin"), 0o755);
    assert_eq!(mode("public/strings.txt"), 0o644);
}

#[cfg(unix)]
#[test]
fn links_are_kept_inside_and_checked_like_files() {
    use std::os::unix::fs::symlink;

    let setup = Setup::new();
    let framework = setup.package(
        "steamcmd_breakpad",
        &[
            (
                "Breakpad.framework/Versions/A/Breakpad",
                Entry::File(b"\xca\xfe\xba\xbe"),
            ),
            ("Breakpad.framework/Versions/Current", Entry::Link("A")),
        ],
    );
    let manifest = Setup::manifest(1, vec![framework]);
    let fetched = Cell::new(0);
    let _: Outcome = setup.install(&manifest, &fetched).unwrap();
    let link = setup.root().join("Breakpad.framework/Versions/Current");
    assert_eq!(fs::read_link(&link).unwrap(), Path::new("A"));
    fs::remove_file(&link).unwrap();
    symlink("B", &link).unwrap();
    assert_eq!(
        changes(setup.install(&manifest, &fetched)),
        ["Breakpad.framework/Versions/Current no longer links to A"]
    );
}

#[cfg(unix)]
#[test]
fn a_link_that_would_point_outside_is_refused() {
    let setup = Setup::new();
    let escaping = setup.package(
        "steamcmd_escape",
        &[("a/link", Entry::Link("../../outside"))],
    );
    let result = setup.install(&Setup::manifest(1, vec![escaping]), &Cell::new(0));
    assert!(
        matches!(&result, Err(Error::Package { reason, .. })
            if reason == "links \"a/link\" to \"../../outside\", outside steamcmd's folder"),
        "{result:?}"
    );
}

#[test]
fn an_install_of_another_version_does_not_verify() {
    let setup = Setup::new();
    let installed = steamcmd(&setup);
    let _: Outcome = setup.install(&installed, &Cell::new(0)).unwrap();
    let newer = Setup::manifest(2, installed.packages);
    let error = install::verify(&setup.held(), &newer).unwrap_err();
    assert!(
        error
            .to_string()
            .ends_with("no install of version 2 is recorded"),
        "{error}"
    );
}

#[test]
fn a_recorded_link_that_is_not_there_is_named() {
    let setup = Setup::new();
    let manifest = steamcmd(&setup);
    let fetched = Cell::new(0);
    let _: Outcome = setup.install(&manifest, &fetched).unwrap();
    let inventory = setup.home.join("steamcmd.inventory");
    let recorded = fs::read_to_string(&inventory).unwrap();
    fs::write(
        &inventory,
        format!("{recorded}link Versions/Current -> A\n"),
    )
    .unwrap();
    assert_eq!(
        changes(setup.install(&manifest, &fetched)),
        ["Versions/Current no longer links to A"]
    );
}

/// Keeps a file out of the installer's reach, neither readable nor removable, until dropped.
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

#[test]
fn a_download_that_cannot_be_read_is_an_error_not_a_refetch() {
    let setup = Setup::new();
    let manifest = steamcmd(&setup);
    let downloads = setup.home.join("downloads");
    fs::create_dir_all(&downloads).unwrap();
    let unreadable = downloads.join(&manifest.packages.first().unwrap().file);
    fs::write(&unreadable, b"whatever").unwrap();
    let held = Kept::new(&unreadable);
    let fetched = Cell::new(0);
    let result = setup.install(&manifest, &fetched);
    drop(held);
    assert!(
        matches!(&result, Err(Error::Io { path, .. }) if *path == unreadable),
        "{result:?}"
    );
    assert_eq!(fetched.get(), 0);
}

#[test]
fn leftovers_that_cannot_be_cleared_are_an_error() {
    let setup = Setup::new();
    let staging = setup.home.join("steamcmd.new");
    fs::create_dir_all(staging.join("stuck")).unwrap();
    fs::write(staging.join("stuck/file"), b"x").unwrap();
    let held = Kept::new(&staging.join("stuck/file"));
    let result = setup.install(&steamcmd(&setup), &Cell::new(0));
    drop(held);
    assert!(
        matches!(&result, Err(Error::Io { path, .. }) if *path == staging),
        "{result:?}"
    );
}

/// What a refusal on a folder ends on.
const FOLDER_STEP: &str = "That folder or something inside it is held open or cannot be written. \
                           Fix or remove that path";
/// What a refusal on any other path ends on.
const PATH_STEP: &str = ". Fix or remove that path";

/// Version 1 of the stand-in steamcmd, installed, with a login in it.
fn logged_in(setup: &Setup) -> Manifest {
    let first = steamcmd(setup);
    let _: Outcome = setup.install(&first, &Cell::new(0)).unwrap();
    fs::create_dir_all(setup.root().join("config")).unwrap();
    fs::write(setup.root().join("config/config.vdf"), b"token").unwrap();
    first
}

fn token(setup: &Setup) -> Vec<u8> {
    fs::read(setup.root().join("config/config.vdf")).unwrap()
}

/// A second version of the stand-in steamcmd, in one package of `entries`.
fn second(setup: &Setup, entries: &[(&str, Entry<'_>)]) -> Manifest {
    Setup::manifest(2, vec![setup.package("steamcmd_second", entries)])
}

fn refusal(result: Result<Outcome, Error>) -> (PathBuf, String) {
    match result {
        Err(Error::Io { path, error }) => (path, error.to_string()),
        other => panic!("expected the install to be refused, got {other:?}"),
    }
}

#[test]
fn an_install_stopped_clearing_an_earlier_ones_leftovers_keeps_the_login() {
    let setup = Setup::new();
    let first = logged_in(&setup);
    let leftovers = setup.home.join("steamcmd.old");
    fs::create_dir_all(leftovers.join("stuck")).unwrap();
    fs::write(leftovers.join("stuck/file"), b"x").unwrap();
    let held = Kept::new(&leftovers.join("stuck/file"));
    let manifest = second(&setup, &[("steamcmd.bin", Entry::File(b"MZ v2"))]);
    let mut stops = Vec::new();
    for _ in 0_u8..2_u8 {
        stops.push(refusal(setup.install(&manifest, &Cell::new(0))));
        install::verify(&setup.held(), &first).unwrap();
    }
    drop(held);
    for (path, said) in stops {
        assert_eq!(path, leftovers);
        assert!(said.ends_with(FOLDER_STEP), "{said}");
    }
    assert!(matches!(
        setup.install(&manifest, &Cell::new(0)),
        Ok(Outcome::Installed)
    ));
    assert_eq!(token(&setup), b"token");
}

#[test]
fn an_install_stopped_by_a_held_file_of_the_old_version_keeps_the_login() {
    let setup = Setup::new();
    let first = logged_in(&setup);
    let public = setup.root().join("public");
    let held = Kept::new(&public.join("strings.txt"));
    let manifest = second(&setup, &[("steamcmd.bin", Entry::File(b"MZ v2"))]);
    let mut stops = Vec::new();
    for _ in 0_u8..2_u8 {
        stops.push(refusal(setup.install(&manifest, &Cell::new(0))));
        assert_eq!(token(&setup), b"token");
    }
    assert!(matches!(
        install::state(&setup.home, &first),
        Ok(State::Missing)
    ));
    drop(held);
    for (path, said) in stops {
        assert_eq!(path, public);
        assert!(said.ends_with(FOLDER_STEP), "{said}");
    }
    assert!(matches!(
        setup.install(&manifest, &Cell::new(0)),
        Ok(Outcome::Installed)
    ));
    assert_eq!(token(&setup), b"token");
    assert!(!public.exists());
    install::verify(&setup.held(), &manifest).unwrap();
}

#[cfg(windows)]
#[test]
fn an_install_stopped_by_an_open_file_of_the_new_version_keeps_the_login() {
    use std::cell::RefCell;
    use std::os::windows::fs::OpenOptionsExt as _;

    let setup = Setup::new();
    drop(logged_in(&setup));
    let tool = setup.package("steamcmd_tool", &[("steamcmd.bin", Entry::File(b"MZ v2"))]);
    let data = setup.package("steamcmd_data", &[("data.txt", Entry::File(b"text"))]);
    let manifest = Setup::manifest(2, vec![tool, data]);
    let staged = setup.home.join("steamcmd.new/steamcmd.bin");
    let open = RefCell::new(None);
    let result = install::using(&setup.held(), &manifest, |package, into| {
        let _: u64 = fs::copy(setup.packages.join(&package.file), into).unwrap();
        if package.name == "steamcmd_data" {
            let file = File::options()
                .read(true)
                .share_mode(1)
                .open(&staged)
                .unwrap();
            *open.borrow_mut() = Some(file);
        }
        Ok(())
    });
    let (file, file_said) = refusal(result);
    assert_eq!(file, staged);
    assert!(file_said.ends_with(PATH_STEP), "{file_said}");
    assert!(!file_said.contains("That folder"), "{file_said}");
    let (path, said) = refusal(setup.install(&manifest, &Cell::new(0)));
    assert_eq!(path, setup.home.join("steamcmd.new"));
    assert!(said.ends_with(FOLDER_STEP), "{said}");
    assert_eq!(token(&setup), b"token");
    drop(open.borrow_mut().take());
    assert!(matches!(
        setup.install(&manifest, &Cell::new(0)),
        Ok(Outcome::Installed)
    ));
    assert_eq!(token(&setup), b"token");
}

#[test]
fn a_packages_own_config_file_over_the_login_stops_and_keeps_it() {
    let setup = Setup::new();
    drop(logged_in(&setup));
    let manifest = second(
        &setup,
        &[
            ("steamcmd.bin", Entry::File(b"MZ v2")),
            ("config", Entry::File(b"not a folder")),
        ],
    );
    let (path, said) = refusal(setup.install(&manifest, &Cell::new(0)));
    assert_eq!(path, setup.home.join("steamcmd.new/config"));
    assert!(said.ends_with(PATH_STEP), "{said}");
    assert!(!said.contains("held"), "{said}");
    assert_eq!(token(&setup), b"token");
}

#[test]
fn an_install_ended_with_the_old_version_moved_aside_is_finished_with_the_login_intact() {
    let setup = Setup::new();
    drop(logged_in(&setup));
    let aside = setup.home.join("steamcmd.old");
    fs::rename(setup.root(), &aside).unwrap();
    fs::create_dir_all(setup.root()).unwrap();
    fs::rename(aside.join("config"), setup.root().join("config")).unwrap();
    fs::remove_file(setup.home.join("steamcmd.inventory")).unwrap();
    let manifest = second(&setup, &[("steamcmd.bin", Entry::File(b"MZ v2"))]);
    assert!(matches!(
        setup.install(&manifest, &Cell::new(0)),
        Ok(Outcome::Installed)
    ));
    assert_eq!(token(&setup), b"token");
    assert_eq!(
        fs::read(setup.root().join("steamcmd.bin")).unwrap(),
        b"MZ v2"
    );
    assert!(!aside.exists());
    install::verify(&setup.held(), &manifest).unwrap();
}

#[test]
fn an_install_ended_with_the_new_version_half_moved_in_is_finished_with_the_login_intact() {
    let setup = Setup::new();
    drop(logged_in(&setup));
    let manifest = second(
        &setup,
        &[
            ("steamcmd.bin", Entry::File(b"MZ v2")),
            ("bin/tool.txt", Entry::File(b"tool")),
        ],
    );
    let staging = setup.home.join("steamcmd.new");
    fs::remove_dir_all(setup.root().join("public")).unwrap();
    fs::remove_file(setup.root().join("steamcmd.bin")).unwrap();
    fs::remove_file(setup.root().join("steamcmd.sh")).unwrap();
    fs::create_dir_all(setup.root().join("bin")).unwrap();
    fs::write(setup.root().join("bin/tool.txt"), b"tool").unwrap();
    fs::create_dir_all(&staging).unwrap();
    fs::write(staging.join("steamcmd.bin"), b"MZ v2").unwrap();
    fs::remove_file(setup.home.join("steamcmd.inventory")).unwrap();
    assert!(matches!(
        setup.install(&manifest, &Cell::new(0)),
        Ok(Outcome::Installed)
    ));
    assert_eq!(token(&setup), b"token");
    assert_eq!(
        fs::read(setup.root().join("steamcmd.bin")).unwrap(),
        b"MZ v2"
    );
    assert!(!staging.exists());
    install::verify(&setup.held(), &manifest).unwrap();
}

#[test]
fn a_packages_own_config_folder_never_replaces_the_login() {
    let setup = Setup::new();
    drop(logged_in(&setup));
    let entries = [
        ("steamcmd.bin", Entry::File(b"MZ v2")),
        ("config/config.vdf", Entry::File(b"the package's")),
    ];
    let manifest = second(&setup, &entries);
    assert!(matches!(
        setup.install(&manifest, &Cell::new(0)),
        Ok(Outcome::Installed)
    ));
    assert_eq!(token(&setup), b"token");
    let empty = Setup::new();
    let fresh_manifest = second(&empty, &entries);
    assert!(matches!(
        empty.install(&fresh_manifest, &Cell::new(0)),
        Ok(Outcome::Installed)
    ));
    assert!(!empty.root().join("config").exists());
}

#[test]
fn a_packages_own_config_file_is_installed_and_checked() {
    let setup = Setup::new();
    let manifest = second(
        &setup,
        &[
            ("steamcmd.bin", Entry::File(b"MZ v2")),
            ("config", Entry::File(b"a file")),
        ],
    );
    assert!(matches!(
        setup.install(&manifest, &Cell::new(0)),
        Ok(Outcome::Installed)
    ));
    install::verify(&setup.held(), &manifest).unwrap();
    assert!(matches!(
        setup.install(&manifest, &Cell::new(0)),
        Ok(Outcome::Verified)
    ));
    assert_eq!(fs::read(setup.root().join("config")).unwrap(), b"a file");
}

#[cfg(unix)]
#[test]
fn a_packages_own_config_link_is_installed_and_checked() {
    let setup = Setup::new();
    let manifest = second(
        &setup,
        &[
            ("bin/tool.txt", Entry::File(b"tool")),
            ("config", Entry::Link("bin")),
        ],
    );
    assert!(matches!(
        setup.install(&manifest, &Cell::new(0)),
        Ok(Outcome::Installed)
    ));
    install::verify(&setup.held(), &manifest).unwrap();
    assert!(matches!(
        setup.install(&manifest, &Cell::new(0)),
        Ok(Outcome::Verified)
    ));
    assert!(
        fs::symlink_metadata(setup.root().join("config"))
            .unwrap()
            .is_symlink()
    );
}

#[test]
fn a_refused_package_leaves_the_installed_version_recorded_and_logged_in() {
    let setup = Setup::new();
    let first = logged_in(&setup);
    let good = setup.package("steamcmd_good", &[("steamcmd.bin", Entry::File(b"MZ v2"))]);
    let evil = setup.package("steamcmd_evil", &[("../outside.txt", Entry::File(b"x"))]);
    let result = setup.install(&Setup::manifest(2, vec![good, evil]), &Cell::new(0));
    assert!(matches!(result, Err(Error::Package { .. })), "{result:?}");
    install::verify(&setup.held(), &first).unwrap();
    assert_eq!(token(&setup), b"token");
}

#[test]
fn a_home_with_a_record_and_no_install_installs_again() {
    let setup = Setup::new();
    let first = logged_in(&setup);
    let staging = setup.home.join("steamcmd.new");
    fs::create_dir_all(&staging).unwrap();
    fs::rename(setup.root().join("config"), staging.join("config")).unwrap();
    fs::rename(setup.root(), setup.home.join("steamcmd.old")).unwrap();
    assert!(matches!(
        install::state(&setup.home, &first),
        Ok(State::Missing)
    ));
    assert!(matches!(
        setup.install(&first, &Cell::new(0)),
        Ok(Outcome::Installed)
    ));
    install::verify(&setup.held(), &first).unwrap();
    assert!(!staging.exists());
    assert!(!setup.home.join("steamcmd.old").exists());
}
