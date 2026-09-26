//! Installing steamcmd from the pinned manifest, and checking that an install is still what was
//! pinned.
//!
//! steamcmd updates itself when it runs, so an install is only known to be the pinned one by
//! looking: every file it unpacked is recorded with its SHA-256 in an inventory beside it, and
//! checked against that record after every run. Its `config` folder, where steamcmd keeps the
//! login token, is steamcmd's to write, is never recorded, and is carried across a reinstall.

use std::collections::BTreeMap;
use std::error;
use std::fmt;
use std::fs::{self, File, TryLockError};
use std::io::{self, Read as _, Write as _};
use std::iter;
use std::path::{Component, Path, PathBuf};

use zip::ZipArchive;

use crate::digest;
use crate::download;
use crate::magic;
use crate::manifest::{Manifest, Package};
// What each system does differently lives in a module of its own, which is how every line of it
// is compiled, tested and mutation-tested on the system it is for.
#[cfg(unix)]
use crate::unix as native;
#[cfg(windows)]
use crate::windows as native;

/// steamcmd's folder in the home.
pub const FOLDER: &str = "steamcmd";
const CONFIG: &str = "config";
const INVENTORY: &str = "steamcmd.inventory";
const STAGING: &str = "steamcmd.new";
const PREVIOUS: &str = "steamcmd.old";
const DOWNLOADS: &str = "downloads";
const LOCK: &str = "steamship.lock";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Installed,
    Verified,
}

#[derive(Debug)]
pub enum Error {
    Download(download::Error),
    Io { path: PathBuf, error: io::Error },
    Package { file: String, reason: String },
    Altered { root: PathBuf, changes: Vec<String> },
    Busy { home: PathBuf },
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Download(error) => write!(formatter, "{error}"),
            Self::Io { path, error } => write!(formatter, "{}: {error}", path.display()),
            Self::Package { file, reason } => write!(formatter, "package {file} {reason}"),
            Self::Altered { root, changes } => write!(
                formatter,
                "steamcmd in {} is no longer the pinned one: {}",
                root.display(),
                changes.join("; ")
            ),
            Self::Busy { home } => {
                write!(
                    formatter,
                    "another steamship is using {} right now",
                    home.display()
                )
            }
        }
    }
}

impl error::Error for Error {}

impl From<download::Error> for Error {
    fn from(error: download::Error) -> Self {
        Self::Download(error)
    }
}

fn at(path: &Path) -> impl FnOnce(io::Error) -> Error {
    let path = path.to_path_buf();
    move |error| Error::Io { path, error }
}

/// Installs the steamcmd `manifest` pins into `home`, or checks the one already there.
///
/// # Errors
///
/// When a package cannot be fetched or is not the pinned one, when the install already there has
/// been altered, or when another steamship holds the home.
pub fn install(home: &Path, manifest: &Manifest) -> Result<Outcome, Error> {
    using(home, manifest, |package, into| {
        let url = format!("{}{}", download::CDN, package.file);
        download::file(&url, into, package.size, &package.sha256).map_err(Error::from)
    })
}

/// [`install`], with the fetching of each package left to `fetch`.
///
/// # Errors
///
/// As [`install`], and whatever `fetch` returns.
pub fn using<Fetch>(home: &Path, manifest: &Manifest, fetch: Fetch) -> Result<Outcome, Error>
where
    Fetch: Fn(&Package, &Path) -> Result<(), Error>,
{
    fs::create_dir_all(home).map_err(at(home))?;
    let _held = lock(home)?;
    let root = home.join(FOLDER);
    if let Some(inventory) = Inventory::read(&home.join(INVENTORY))
        && inventory.describes(manifest)
    {
        return inventory.check(&root).map(|()| Outcome::Verified);
    }
    fresh(home, &root, manifest, &fetch)?;
    Ok(Outcome::Installed)
}

/// Checks that the steamcmd in `home` is exactly the one `manifest` pins, as after every run.
///
/// # Errors
///
/// When there is no install of that version, or it has been altered since it was installed.
pub fn verify(home: &Path, manifest: &Manifest) -> Result<(), Error> {
    let root = home.join(FOLDER);
    match Inventory::read(&home.join(INVENTORY)) {
        Some(inventory) if inventory.describes(manifest) => inventory.check(&root),
        _ => Err(Error::Altered {
            root,
            changes: vec![format!(
                "no install of version {} is recorded",
                manifest.version
            )],
        }),
    }
}

/// Held for as long as the returned file is open; the system releases it when the process ends,
/// however it ends, so a crash never leaves the home locked.
fn lock(home: &Path) -> Result<File, Error> {
    let path = home.join(LOCK);
    let file = File::options()
        .create(true)
        .truncate(false)
        .write(true)
        .open(&path)
        .map_err(at(&path))?;
    match file.try_lock() {
        Ok(()) => Ok(file),
        Err(TryLockError::WouldBlock) => Err(Error::Busy {
            home: home.to_path_buf(),
        }),
        Err(TryLockError::Error(error)) => Err(Error::Io { path, error }),
    }
}

fn fresh<Fetch>(home: &Path, root: &Path, manifest: &Manifest, fetch: &Fetch) -> Result<(), Error>
where
    Fetch: Fn(&Package, &Path) -> Result<(), Error>,
{
    let previous = home.join(PREVIOUS);
    // A run that stopped between moving the old install aside and moving the new one in leaves
    // only the old one, with the login in it: put it back before anything else.
    if !root.exists() && previous.exists() {
        fs::rename(&previous, root).map_err(at(root))?;
    }
    let downloads = home.join(DOWNLOADS);
    let staging = home.join(STAGING);
    clear(&staging)?;
    fs::create_dir_all(&downloads).map_err(at(&downloads))?;
    fs::create_dir_all(&staging).map_err(at(&staging))?;
    for package in &manifest.packages {
        let archive = downloads.join(&package.file);
        if !is_pinned(&archive, package)? {
            fetch(package, &archive)?;
            if !is_pinned(&archive, package)? {
                return Err(Error::Package {
                    file: package.file.clone(),
                    reason: "is not the pinned file".to_owned(),
                });
            }
        }
        unpack(&archive, &staging, &package.file)?;
    }
    let inventory = Inventory::take(&staging, manifest)?;
    let config = root.join(CONFIG);
    if config.exists() {
        let carried = staging.join(CONFIG);
        fs::rename(&config, &carried).map_err(at(&carried))?;
    }
    clear(&previous)?;
    if root.exists() {
        fs::rename(root, &previous).map_err(at(&previous))?;
    }
    fs::rename(&staging, root).map_err(at(root))?;
    clear(&previous)?;
    inventory.write(&home.join(INVENTORY))?;
    clear(&downloads)
}

fn clear(path: &Path) -> Result<(), Error> {
    match fs::remove_dir_all(path) {
        Err(error) if error.kind() != io::ErrorKind::NotFound => Err(Error::Io {
            path: path.to_path_buf(),
            error,
        }),
        _ => Ok(()),
    }
}

/// Whether `archive` is there and is exactly the file `package` pins.
fn is_pinned(archive: &Path, package: &Package) -> Result<bool, Error> {
    match File::open(archive) {
        Ok(mut opened) => Ok(digest::sha256(&mut opened).map_err(at(archive))? == package.sha256),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(Error::Io {
            path: archive.to_path_buf(),
            error,
        }),
    }
}

fn sha256(path: &Path) -> Result<[u8; 32], Error> {
    let mut file = File::open(path).map_err(at(path))?;
    digest::sha256(&mut file).map_err(at(path))
}

fn unpack(archive: &Path, into: &Path, name: &str) -> Result<(), Error> {
    let refuse = |reason: String| Error::Package {
        file: name.to_owned(),
        reason,
    };
    let opened = File::open(archive).map_err(at(archive))?;
    let mut zip =
        ZipArchive::new(opened).map_err(|error| refuse(format!("is not a zip: {error}")))?;
    for index in 0..zip.len() {
        let mut entry = zip
            .by_index(index)
            .map_err(|error| refuse(format!("cannot be read: {error}")))?;
        let Some(relative) = entry.enclosed_name() else {
            return Err(refuse(format!(
                "holds \"{}\", which would land outside steamcmd's folder",
                entry.name()
            )));
        };
        let target = into.join(&relative);
        if entry.is_dir() {
            fs::create_dir_all(&target).map_err(at(&target))?;
            continue;
        }
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).map_err(at(parent))?;
        }
        if entry.is_symlink() {
            let mut link = String::new();
            let _: usize = entry
                .read_to_string(&mut link)
                .map_err(|error| refuse(format!("cannot be read: {error}")))?;
            if !stays_inside(&relative, &link) {
                return Err(refuse(format!(
                    "links \"{}\" to \"{link}\", outside steamcmd's folder",
                    relative.display()
                )));
            }
            native::make_link(&link, &target).map_err(at(&target))?;
            continue;
        }
        let mut out = File::create(&target).map_err(at(&target))?;
        // The zip reader checks each entry's CRC as it is read, so a short or damaged entry
        // fails here rather than being counted.
        let _: u64 = io::copy(&mut entry, &mut out)
            .map_err(|error| refuse(format!("cannot be unpacked: {error}")))?;
        out.flush().map_err(at(&target))?;
        native::mark_if_program(&target).map_err(at(&target))?;
    }
    Ok(())
}

/// Whether a link at `entry` pointing to `target` resolves inside the folder `entry` is in.
fn stays_inside(entry: &Path, target: &str) -> bool {
    let mut depth = entry
        .parent()
        .map_or(0, |parent| parent.components().count());
    for component in Path::new(target).components() {
        match component {
            Component::Normal(_) => depth = depth.saturating_add(1),
            Component::CurDir => {}
            Component::ParentDir => match depth.checked_sub(1) {
                Some(up) => depth = up,
                None => return false,
            },
            Component::RootDir | Component::Prefix(_) => return false,
        }
    }
    true
}

/// What was unpacked: every file with its SHA-256 and every link with its target, by path from
/// steamcmd's folder, written with `/` on every system.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Inventory {
    system: String,
    version: u64,
    files: BTreeMap<String, [u8; 32]>,
    links: BTreeMap<String, String>,
}

const HEADER: &str = "steamship inventory 1";

impl Inventory {
    fn describes(&self, manifest: &Manifest) -> bool {
        self.system == manifest.system && self.version == manifest.version
    }

    fn take(root: &Path, manifest: &Manifest) -> Result<Self, Error> {
        let mut inventory = Self {
            system: manifest.system.clone(),
            version: manifest.version,
            files: BTreeMap::new(),
            links: BTreeMap::new(),
        };
        for (relative, kind) in walk(root)? {
            let path = root.join(&relative);
            match kind {
                Kind::File => inventory.files.extend([(relative, sha256(&path)?)]),
                Kind::Link => {
                    let target = fs::read_link(&path).map_err(at(&path))?;
                    inventory
                        .links
                        .extend([(relative, target.to_string_lossy().into_owned())]);
                }
            }
        }
        Ok(inventory)
    }

    fn write(&self, path: &Path) -> Result<(), Error> {
        let header = format!(
            "{HEADER}\nsystem {}\nversion {}\n",
            self.system, self.version
        );
        let files = self
            .files
            .iter()
            .map(|(relative, found)| format!("file {} {relative}\n", digest::hex(found)));
        let links = self
            .links
            .iter()
            .map(|(relative, target)| format!("link {relative} -> {target}\n"));
        let text: String = iter::once(header).chain(files).chain(links).collect();
        let part = path.with_extension("part");
        fs::write(&part, text).map_err(at(&part))?;
        fs::rename(&part, path).map_err(at(path))
    }

    /// The inventory at `path`, or none when there is none or it cannot be read as one, either
    /// of which means installing afresh from the pinned packages.
    fn read(path: &Path) -> Option<Self> {
        let text = fs::read_to_string(path).ok()?;
        let mut lines = text.lines();
        if lines.next()? != HEADER {
            return None;
        }
        let system = lines.next()?.strip_prefix("system ")?.to_owned();
        let version = lines.next()?.strip_prefix("version ")?.parse().ok()?;
        let mut inventory = Self {
            system,
            version,
            files: BTreeMap::new(),
            links: BTreeMap::new(),
        };
        for line in lines {
            if let Some(entry) = line.strip_prefix("file ") {
                let (hex, relative) = entry.split_once(' ')?;
                inventory
                    .files
                    .extend([(relative.to_owned(), digest::from_hex(hex)?)]);
            } else {
                let (relative, target) = line.strip_prefix("link ")?.split_once(" -> ")?;
                inventory
                    .links
                    .extend([(relative.to_owned(), target.to_owned())]);
            }
        }
        Some(inventory)
    }

    /// Every way the install at `root` differs from this record. A file steamcmd writes as it
    /// runs (a log, a cache) is its business; a new program is not, since that is what an update
    /// looks like.
    fn check(&self, root: &Path) -> Result<(), Error> {
        let mut changes = Vec::new();
        for (relative, pinned) in &self.files {
            let path = root.join(relative);
            let now = if path.is_file() {
                Some(sha256(&path)?)
            } else {
                None
            };
            match now {
                None => changes.push(format!("{relative} is missing")),
                Some(found) if found != *pinned => changes.push(format!("{relative} has changed")),
                Some(_) => {}
            }
        }
        for (relative, pinned) in &self.links {
            let path = root.join(relative);
            let now = fs::read_link(&path).map(|target| target.to_string_lossy().into_owned());
            if now.as_deref().ok() != Some(pinned.as_str()) {
                changes.push(format!("{relative} no longer links to {pinned}"));
            }
        }
        for (relative, kind) in walk(root)? {
            let recorded = self.files.contains_key(&relative) || self.links.contains_key(&relative);
            let path = root.join(&relative);
            if !recorded
                && (kind == Kind::Link || magic::looks_like_program(&path).map_err(at(&path))?)
            {
                changes.push(format!("{relative} is new"));
            }
        }
        if changes.is_empty() {
            Ok(())
        } else {
            Err(Error::Altered {
                root: root.to_path_buf(),
                changes,
            })
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    File,
    Link,
}

/// Every file and link under `root`, by path from it with `/` between names, leaving out the
/// top-level `config` folder. Links are listed and never followed.
fn walk(root: &Path) -> Result<Vec<(String, Kind)>, Error> {
    let mut found = Vec::new();
    let mut pending = vec![(root.to_path_buf(), String::new())];
    while let Some((folder, prefix)) = pending.pop() {
        for entry in fs::read_dir(&folder).map_err(at(&folder))? {
            let entry = entry.map_err(at(&folder))?;
            let name = entry.file_name().to_string_lossy().into_owned();
            let relative = if prefix.is_empty() {
                name
            } else {
                format!("{prefix}/{name}")
            };
            let kind = entry.file_type().map_err(at(&entry.path()))?;
            if kind.is_symlink() {
                found.push((relative, Kind::Link));
            } else if kind.is_dir() {
                if relative != CONFIG {
                    pending.push((entry.path(), relative));
                }
            } else {
                found.push((relative, Kind::File));
            }
        }
    }
    Ok(found)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_inventory_describes_only_its_own_system_and_version() {
        let inventory = Inventory {
            system: "win32".into(),
            version: 7,
            files: BTreeMap::new(),
            links: BTreeMap::new(),
        };
        let manifest = |system: &str, version| Manifest {
            system: system.into(),
            version,
            packages: Vec::new(),
        };
        assert!(inventory.describes(&manifest("win32", 7)));
        assert!(!inventory.describes(&manifest("win32", 8)));
        assert!(!inventory.describes(&manifest("linux", 7)));
    }

    #[test]
    fn an_inventory_reads_back_as_it_was_written() {
        let folder = tempfile::tempdir().unwrap();
        let path = folder.path().join("steamcmd.inventory");
        let inventory = Inventory {
            system: "osx".into(),
            version: 9,
            files: BTreeMap::from([("dir/with space.dylib".to_owned(), [7_u8; 32])]),
            links: BTreeMap::from([("Current".to_owned(), "A".to_owned())]),
        };
        inventory.write(&path).unwrap();
        assert_eq!(Inventory::read(&path), Some(inventory));
    }

    #[test]
    fn says_what_went_wrong() {
        let home = Path::new("home");
        let cases = [
            (
                Error::from(download::Error::Gone {
                    url: "u".into(),
                    status: 404,
                }),
                "u answered 404".to_owned(),
            ),
            (
                Error::Io {
                    path: home.to_path_buf(),
                    error: io::Error::new(io::ErrorKind::PermissionDenied, "denied"),
                },
                "home: denied".to_owned(),
            ),
            (
                Error::Package {
                    file: "p.zip".into(),
                    reason: "is not a zip".into(),
                },
                "package p.zip is not a zip".to_owned(),
            ),
            (
                Error::Altered {
                    root: home.join("steamcmd"),
                    changes: vec!["a".into(), "b".into()],
                },
                format!(
                    "steamcmd in {} is no longer the pinned one: a; b",
                    home.join("steamcmd").display()
                ),
            ),
            (
                Error::Busy {
                    home: home.to_path_buf(),
                },
                "another steamship is using home right now".to_owned(),
            ),
        ];
        for (error, said) in cases {
            assert_eq!(error.to_string(), said);
        }
    }

    #[test]
    fn a_link_may_point_anywhere_inside_its_folder_and_nowhere_else() {
        let inside = [
            ("Frameworks/B.framework/Versions/Current", "A"),
            (
                "Frameworks/B.framework/Breakpad",
                "Versions/Current/Breakpad",
            ),
            ("a/b/c", "../../x"),
            ("a/b", "./c/../d"),
        ];
        for (entry, target) in inside {
            assert!(
                stays_inside(Path::new(entry), target),
                "{entry} -> {target}"
            );
        }
        let outside = [
            ("a/b", "../../x"),
            ("top", "../x"),
            ("a/b", "/etc/passwd"),
            ("a", ".."),
        ];
        for (entry, target) in outside {
            assert!(
                !stays_inside(Path::new(entry), target),
                "{entry} -> {target}"
            );
        }
    }
}
