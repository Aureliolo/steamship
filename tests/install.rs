//! Installing steamcmd from packages built here, fetched by copying, so every path the installer
//! can take is exercised without Valve's CDN.
#![expect(
    clippy::unwrap_used,
    clippy::panic,
    reason = "a test reports failure by panicking, its helpers included"
)]

use std::cell::Cell;
use std::fs::{self, File};
use std::io::Write as _;
use std::path::{Path, PathBuf};

use steamship::digest;
use steamship::install::{self, Error, Outcome};
use steamship::manifest::{Manifest, Package};
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
        }
    }

    /// Installs, counting the packages fetched.
    fn install(&self, manifest: &Manifest, fetched: &Cell<usize>) -> Result<Outcome, Error> {
        install::using(&self.home, manifest, |package, into| {
            fetched.set(fetched.get().saturating_add(1));
            let _: u64 = fs::copy(self.packages.join(&package.file), into).unwrap();
            Ok(())
        })
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

fn steamcmd(setup: &Setup) -> Manifest {
    let tool = setup.package(
        "steamcmd_test",
        &[
            ("steamcmd.exe", Entry::File(b"MZ steamcmd")),
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
        fs::read(setup.root().join("steamcmd.exe")).unwrap(),
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
    install::verify(&setup.home, &manifest).unwrap();
}

#[test]
fn every_change_to_the_install_is_named() {
    let setup = Setup::new();
    let manifest = steamcmd(&setup);
    let fetched = Cell::new(0);
    let _: Outcome = setup.install(&manifest, &fetched).unwrap();
    fs::write(setup.root().join("steamcmd.exe"), b"MZ newer").unwrap();
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
            "steamcmd.exe has changed",
            "update.dll is new"
        ]
    );
    assert!(
        install::verify(&setup.home, &manifest)
            .is_err_and(|error| matches!(error, Error::Altered { .. }))
    );
}

#[test]
fn a_new_version_replaces_the_install_and_keeps_the_login() {
    let setup = Setup::new();
    let fetched = Cell::new(0);
    let _: Outcome = setup.install(&steamcmd(&setup), &fetched).unwrap();
    fs::create_dir_all(setup.root().join("config")).unwrap();
    fs::write(setup.root().join("config/config.vdf"), b"token").unwrap();
    let newer = setup.package("steamcmd_newer", &[("steamcmd.exe", Entry::File(b"MZ v2"))]);
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
        fs::read(setup.root().join("steamcmd.exe")).unwrap(),
        b"MZ v2"
    );
    assert!(
        !setup.root().join("public").exists(),
        "nothing of the old version is left"
    );
    install::verify(&setup.home, &manifest).unwrap();
}

#[test]
fn a_swap_cut_short_is_finished_with_the_login_intact() {
    let setup = Setup::new();
    let fetched = Cell::new(0);
    let _: Outcome = setup.install(&steamcmd(&setup), &fetched).unwrap();
    fs::create_dir_all(setup.root().join("config")).unwrap();
    fs::write(setup.root().join("config/config.vdf"), b"token").unwrap();
    // As if the process ended between moving the old install aside and moving the new one in.
    fs::rename(setup.root(), setup.home.join("steamcmd.old")).unwrap();
    let newer = setup.package("steamcmd_newer", &[("steamcmd.exe", Entry::File(b"MZ v2"))]);
    let _: Outcome = setup
        .install(&Setup::manifest(2, vec![newer]), &fetched)
        .unwrap();
    assert_eq!(
        fs::read(setup.root().join("config/config.vdf")).unwrap(),
        b"token"
    );
    assert!(!setup.home.join("steamcmd.old").exists());
}

#[test]
fn a_package_that_would_write_outside_is_refused() {
    let setup = Setup::new();
    let evil = setup.package("steamcmd_evil", &[("../outside.txt", Entry::File(b"x"))]);
    let result = setup.install(&Setup::manifest(1, vec![evil]), &Cell::new(0));
    assert!(
        matches!(&result, Err(Error::Package { reason, .. }) if reason.contains("outside steamcmd's folder")),
        "{result:?}"
    );
    assert!(!setup.home.join("outside.txt").exists());
    assert!(!setup.root().exists(), "a refused package installs nothing");
}

#[test]
fn a_fetch_that_brings_the_wrong_file_is_refused() {
    let setup = Setup::new();
    let manifest = steamcmd(&setup);
    let result = install::using(&setup.home, &manifest, |_, into| {
        fs::write(into, b"not the package").unwrap();
        Ok(())
    });
    assert!(
        matches!(&result, Err(Error::Package { reason, .. }) if reason == "is not the pinned file"),
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
    install::verify(&setup.home, &manifest).unwrap();
}

#[test]
fn verifying_with_nothing_installed_says_so() {
    let setup = Setup::new();
    let manifest = steamcmd(&setup);
    let error = install::verify(&setup.home, &manifest).unwrap_err();
    assert!(
        error
            .to_string()
            .ends_with("no install of version 1 is recorded"),
        "{error}"
    );
}

#[test]
fn a_second_run_at_the_same_time_is_turned_away() {
    let setup = Setup::new();
    fs::create_dir_all(&setup.home).unwrap();
    let held = File::create(setup.home.join("steamship.lock")).unwrap();
    held.try_lock().unwrap();
    let result = setup.install(&steamcmd(&setup), &Cell::new(0));
    assert!(matches!(result, Err(Error::Busy { .. })), "{result:?}");
    held.unlock().unwrap();
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
    assert_eq!(mode("steamcmd.exe"), 0o755);
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
    let error = install::verify(&setup.home, &newer).unwrap_err();
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
struct Held {
    #[cfg(unix)]
    path: PathBuf,
    #[cfg(windows)]
    _open: File,
}

impl Held {
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
impl Drop for Held {
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
    let held = Held::new(&unreadable);
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
    let held = Held::new(&staging.join("stuck/file"));
    let result = setup.install(&steamcmd(&setup), &Cell::new(0));
    drop(held);
    assert!(
        matches!(&result, Err(Error::Io { path, .. }) if *path == staging),
        "{result:?}"
    );
}
