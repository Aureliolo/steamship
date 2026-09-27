//! Whether a newer steamship is out, and how to get it.
//!
//! GitHub is asked at most once a day. The answer is kept in the home with when it was asked, and
//! every run in between reads that file and asks nothing. Only a person at a terminal is told:
//! a run in CI, with its output captured, or with `STEAMSHIP_NO_UPDATE_CHECK` set asks nothing.

use std::env::consts::EXE_SUFFIX;
use std::ffi::OsString;
use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::{Duration, SystemTime};

use crate::show::Hint;

/// Set to anything, and steamship never asks GitHub for a newer release.
pub const OPT_OUT: &str = "STEAMSHIP_NO_UPDATE_CHECK";

/// The newest release's page, which GitHub answers with a redirect to that release's tag.
pub const LATEST: &str = "https://github.com/Aureliolo/steamship/releases/latest";
const TAG: &str = "https://github.com/Aureliolo/steamship/releases/tag/v";

/// The file in the home that keeps the last answer.
pub const KEPT: &str = "latest-release";

/// How long an answer is kept before GitHub is asked again.
pub const KEEP: Duration = Duration::from_hours(24);

/// The longest asking may take, and so the longest a finished command waits for the answer.
const LIMIT: Duration = Duration::from_secs(2);

/// A release's version.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Version {
    major: u64,
    minor: u64,
    patch: u64,
}

impl Version {
    /// A release version such as `1.4.0`. Anything else is `None`, a pre-release included, so
    /// that nothing but digits and dots from a server or a file is ever printed.
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        let mut parts = text.split('.').map(number);
        match (parts.next(), parts.next(), parts.next(), parts.next()) {
            (Some(Some(major)), Some(Some(minor)), Some(Some(patch)), None) => Some(Self {
                major,
                minor,
                patch,
            }),
            _ => None,
        }
    }

    /// This steamship's version.
    #[must_use]
    pub fn this() -> Option<Self> {
        Self::parse(env!("CARGO_PKG_VERSION"))
    }
}

fn number(part: &str) -> Option<u64> {
    // u64's own parser takes a leading `+`.
    if !part.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    part.parse().ok()
}

impl fmt::Display for Version {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

/// Whether to ask at all: only for a person, which a terminal suggests and CI rules out.
pub fn wanted<Lookup>(lookup: Lookup, terminal: bool) -> bool
where
    Lookup: Fn(&str) -> Option<OsString>,
{
    let set = |name: &str| lookup(name).is_some_and(|value| !value.is_empty());
    terminal && !set("CI") && !set(OPT_OUT)
}

/// The version a redirect to `location` names, when it is to one of steamship's release tags.
#[must_use]
pub fn tagged(location: &str) -> Option<Version> {
    location.strip_prefix(TAG).and_then(Version::parse)
}

/// The newest version, as the redirect from `url` names it, or `None` when it cannot be had
/// within [`LIMIT`]. The redirect is read, never followed.
fn ask_at(url: &str) -> Option<Version> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(LIMIT))
        .max_redirects(0)
        .build()
        .into();
    let response = agent.get(url).call().ok()?;
    tagged(response.headers().get("location")?.to_str().ok()?)
}

/// The answer kept in `home`, and when it was had.
fn kept(home: &Path) -> Option<(SystemTime, Version)> {
    let text = fs::read_to_string(home.join(KEPT)).ok()?;
    let (asked, latest) = text.trim_end().split_once(' ')?;
    let asked = SystemTime::UNIX_EPOCH.checked_add(Duration::from_secs(number(asked)?))?;
    Some((asked, Version::parse(latest)?))
}

fn keep(home: &Path, asked: SystemTime, latest: Version) -> io::Result<()> {
    let seconds = asked
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_err(io::Error::other)?
        .as_secs();
    fs::create_dir_all(home)?;
    fs::write(home.join(KEPT), format!("{seconds} {latest}\n"))
}

/// Whether an answer had at `asked` still holds at `now`. One from the future, after the clock
/// was set back, does not.
fn fresh(asked: SystemTime, now: SystemTime) -> bool {
    now.duration_since(asked).is_ok_and(|age| age < KEEP)
}

/// The newest version: the kept answer while it is fresh, or else GitHub's, asked on a thread of
/// its own while the command runs.
#[derive(Debug)]
pub struct Check {
    known: Option<Version>,
    asking: Option<Receiver<Option<Version>>>,
}

impl Check {
    /// The check for the steamship whose home is `home`, asking GitHub if the kept answer is stale.
    #[must_use]
    pub fn start(home: PathBuf) -> Self {
        Self::asking_with(home, SystemTime::now(), || ask_at(LATEST))
    }

    /// [`Check::start`] at `now`, with GitHub asked by `ask`.
    pub fn asking_with<Ask>(home: PathBuf, now: SystemTime, ask: Ask) -> Self
    where
        Ask: FnOnce() -> Option<Version> + Send + 'static,
    {
        if let Some((asked, latest)) = kept(&home)
            && fresh(asked, now)
        {
            return Self {
                known: Some(latest),
                asking: None,
            };
        }
        let (sender, receiver) = mpsc::channel();
        let _asking = thread::spawn(move || {
            // No answer is kept as nothing newer, so that a machine offline waits for GitHub once
            // a day rather than at the end of every command.
            let latest = ask().or_else(Version::this);
            // Neither failing matters: a home that cannot be written to is asked again next time,
            // and a command that stopped waiting has no use for the answer.
            if let Some(latest) = latest {
                drop(keep(&home, now, latest));
            }
            let _sent = sender.send(latest);
        });
        Self {
            known: None,
            asking: Some(receiver),
        }
    }

    /// A version newer than this one, if there is one, waiting at most [`LIMIT`] for GitHub.
    #[must_use]
    pub fn newer(self) -> Option<Version> {
        self.newer_than(Version::this()?, LIMIT)
    }

    /// A version newer than `this`, waiting at most `wait` for GitHub.
    #[must_use]
    pub fn newer_than(self, this: Version, wait: Duration) -> Option<Version> {
        let latest = match self.asking {
            Some(receiver) => receiver.recv_timeout(wait).ok().flatten(),
            None => self.known,
        };
        latest.filter(|latest| *latest > this)
    }
}

/// How this steamship was installed, which is how it is upgraded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Installed {
    /// By Homebrew, which keeps each version in its Cellar.
    Homebrew,
    /// By Scoop, which keeps each app in its `apps` folder.
    Scoop,
    /// By winget, which keeps a portable program in a folder named for its package.
    Winget,
    /// By cargo-binstall, or by Cargo with cargo-binstall at hand.
    Binstall,
    /// By Cargo, from source.
    Cargo,
    /// From a release's archive, anywhere.
    Archive,
}

impl Installed {
    /// How the steamship at `program`, with any link to it followed, was installed: by where a
    /// package manager puts it, into a folder Cargo keeps its record of installs beside, or
    /// anywhere else.
    pub fn of<Exists>(program: &Path, exists: Exists) -> Self
    where
        Exists: Fn(&Path) -> bool,
    {
        // The folders the program is in, nearest first, as the systems that matter here compare
        // names: without regard to case.
        let folders: Vec<String> = program
            .ancestors()
            .skip(1)
            .filter_map(Path::file_name)
            .map(|name| name.to_string_lossy().to_ascii_lowercase())
            .collect();
        let within = |inner: &dyn Fn(&str) -> bool, outer: &str| {
            folders
                .windows(2)
                .any(|pair| matches!(pair, [name, parent] if inner(name) && parent == outer))
        };
        // Cellar/steamship/<version>/bin, apps/steamship/<version or current>, and
        // Packages/Aureliolo.steamship_<source>/<the archive's folder>.
        if within(&|name| name == "steamship", "cellar") {
            return Self::Homebrew;
        }
        if within(&|name| name == "steamship", "apps") {
            return Self::Scoop;
        }
        if within(&|name| name.starts_with("aureliolo.steamship_"), "packages") {
            return Self::Winget;
        }
        let Some(bin) = program.parent() else {
            return Self::Archive;
        };
        // Cargo and cargo-binstall both record what they install in `.crates.toml`, in the
        // folder above the one they put programs in.
        if !bin
            .parent()
            .is_some_and(|root| exists(&root.join(".crates.toml")))
        {
            return Self::Archive;
        }
        let binstall = format!("cargo-binstall{EXE_SUFFIX}");
        if exists(&bin.join(binstall)) {
            Self::Binstall
        } else {
            Self::Cargo
        }
    }

    /// What to do to upgrade.
    #[must_use]
    pub const fn hint(self) -> Hint<'static> {
        let (before, command) = match self {
            Self::Homebrew => ("upgrade with ", "brew upgrade steamship"),
            Self::Scoop => ("upgrade with ", "scoop update steamship"),
            Self::Winget => ("upgrade with ", "winget upgrade Aureliolo.steamship"),
            Self::Binstall => ("upgrade with ", "cargo binstall steamship"),
            Self::Cargo => ("upgrade with ", "cargo install --locked steamship"),
            Self::Archive => ("download it from ", LATEST),
        };
        Hint {
            before,
            command,
            after: "",
        }
    }
}

#[cfg(test)]
mod tests {
    use std::io::{Read as _, Write as _};
    use std::net::TcpListener;
    use std::time::Instant;

    use super::*;

    fn version(text: &str) -> Version {
        Version::parse(text).unwrap()
    }

    #[test]
    fn reads_release_versions_and_nothing_else() {
        assert_eq!(version("0.12.3").to_string(), "0.12.3");
        assert_eq!(
            version("18446744073709551615.0.0").to_string(),
            "18446744073709551615.0.0"
        );
        for other in [
            "",
            "1",
            "1.2",
            "1.2.3.4",
            "1..3",
            "1.2.",
            "v1.2.3",
            "+1.2.3",
            "1.2.3-rc.1",
            "1.2.3 ",
            "1.2.\u{1b}[2J",
            "18446744073709551616.0.0",
        ] {
            assert_eq!(Version::parse(other), None, "{other:?}");
        }
        assert!(Version::this().is_some());
    }

    #[test]
    fn orders_versions_by_number_not_by_text() {
        assert!(version("0.10.0") > version("0.9.9"));
        assert!(version("1.0.0") > version("0.99.99"));
        assert!(version("0.2.1") > version("0.2.0"));
    }

    #[test]
    fn asks_only_for_a_person_at_a_terminal() {
        let with = |variables: &'static [(&'static str, &'static str)]| {
            move |name: &str| {
                variables
                    .iter()
                    .find(|(key, _)| *key == name)
                    .map(|(_, value)| OsString::from(value))
            }
        };
        assert!(wanted(with(&[]), true));
        assert!(wanted(
            with(&[("CI", ""), ("STEAMSHIP_NO_UPDATE_CHECK", "")]),
            true
        ));
        assert!(!wanted(with(&[]), false));
        assert!(!wanted(with(&[("CI", "true")]), true));
        assert!(!wanted(with(&[("STEAMSHIP_NO_UPDATE_CHECK", "1")]), true));
    }

    #[test]
    fn reads_the_version_only_from_a_redirect_to_a_release_tag() {
        assert_eq!(
            tagged("https://github.com/Aureliolo/steamship/releases/tag/v0.3.0"),
            Some(version("0.3.0"))
        );
        for other in [
            "https://github.com/Aureliolo/steamship/releases",
            "https://github.com/Aureliolo/steamship/releases/tag/v",
            "https://github.com/Aureliolo/steamship/releases/tag/0.3.0",
            "https://github.com/someone/else/releases/tag/v0.3.0",
            "https://github.com/Aureliolo/steamship/releases/tag/v0.3.0-rc.1",
        ] {
            assert_eq!(tagged(other), None, "{other}");
        }
    }

    /// The address of a server on this machine that answers one request with `response`.
    fn answering(response: &'static str) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let _serving = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0_u8; 4096];
            let _: usize = stream.read(&mut request).unwrap();
            stream.write_all(response.as_bytes()).unwrap();
        });
        format!("http://{address}/releases/latest")
    }

    #[test]
    fn asks_for_the_redirect_and_reads_the_tag_it_names_without_following_it() {
        let redirect = "HTTP/1.1 302 Found\r\n\
            Location: https://github.com/Aureliolo/steamship/releases/tag/v1.2.3\r\n\
            Content-Length: 0\r\nConnection: close\r\n\r\n";
        assert_eq!(ask_at(&answering(redirect)), Some(version("1.2.3")));
        let page = "HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok";
        assert_eq!(ask_at(&answering(page)), None);
        let elsewhere = "HTTP/1.1 302 Found\r\nLocation: https://example.com/v1.2.3\r\n\
            Content-Length: 0\r\nConnection: close\r\n\r\n";
        assert_eq!(ask_at(&answering(elsewhere)), None);
    }

    const NOW: Duration = Duration::from_hours(500_000);

    fn at(seconds: Duration) -> SystemTime {
        SystemTime::UNIX_EPOCH.checked_add(seconds).unwrap()
    }

    fn never() -> Option<Version> {
        panic!("GitHub was asked while the kept answer was fresh")
    }

    #[test]
    fn a_fresh_answer_is_used_without_asking() {
        let home = tempfile::tempdir().unwrap();
        let asked = NOW
            .checked_sub(KEEP)
            .unwrap()
            .saturating_add(Duration::from_secs(1));
        fs::write(
            home.path().join(KEPT),
            format!("{} 9.0.0\n", asked.as_secs()),
        )
        .unwrap();
        let check = Check::asking_with(home.path().to_owned(), at(NOW), never);
        assert_eq!(
            check.newer_than(version("1.0.0"), Duration::ZERO),
            Some(version("9.0.0"))
        );
    }

    fn asked_again(kept: Option<String>) -> (Option<Version>, String) {
        let home = tempfile::tempdir().unwrap();
        if let Some(kept) = kept {
            fs::write(home.path().join(KEPT), kept).unwrap();
        }
        let check = Check::asking_with(home.path().to_owned(), at(NOW), || Some(version("2.0.0")));
        let newer = check.newer_than(version("1.0.0"), Duration::from_secs(30));
        (newer, fs::read_to_string(home.path().join(KEPT)).unwrap())
    }

    #[test]
    fn a_stale_future_or_unreadable_answer_is_asked_again_and_kept() {
        let kept = format!("{} 2.0.0\n", NOW.as_secs());
        for before in [
            None,
            Some(format!(
                "{} 9.0.0\n",
                NOW.checked_sub(KEEP).unwrap().as_secs()
            )),
            Some(format!("{} 9.0.0\n", NOW.saturating_add(KEEP).as_secs())),
            Some("tomorrow 9.0.0\n".to_owned()),
            Some(format!("{} 9.0\n", NOW.as_secs())),
            Some(String::new()),
        ] {
            assert_eq!(
                asked_again(before.clone()),
                (Some(version("2.0.0")), kept.clone()),
                "{before:?}"
            );
        }
    }

    #[test]
    fn no_answer_is_kept_as_nothing_newer() {
        let home = tempfile::tempdir().unwrap();
        let check = Check::asking_with(home.path().to_owned(), at(NOW), || None);
        assert_eq!(
            check.newer_than(version("0.0.0"), Duration::from_secs(30)),
            Version::this()
        );
        assert_eq!(
            fs::read_to_string(home.path().join(KEPT)).unwrap(),
            format!("{} {}\n", NOW.as_secs(), env!("CARGO_PKG_VERSION"))
        );
    }

    #[test]
    fn keeps_the_answer_in_a_home_not_made_yet() {
        let folder = tempfile::tempdir().unwrap();
        let home = folder.path().join("steamship");
        let check = Check::asking_with(home.clone(), at(NOW), || Some(version("2.0.0")));
        assert!(
            check
                .newer_than(version("1.0.0"), Duration::from_secs(30))
                .is_some()
        );
        assert!(home.join(KEPT).exists());
    }

    #[test]
    fn tells_only_of_a_newer_version() {
        let home = tempfile::tempdir().unwrap();
        fs::write(home.path().join(KEPT), format!("{} 1.0.0\n", NOW.as_secs())).unwrap();
        for this in ["1.0.0", "1.0.1", "2.0.0"] {
            let check = Check::asking_with(home.path().to_owned(), at(NOW), never);
            assert_eq!(
                check.newer_than(version(this), Duration::ZERO),
                None,
                "{this}"
            );
        }
    }

    #[test]
    fn a_command_does_not_wait_longer_than_it_is_told_and_a_late_answer_is_still_kept() {
        let home = tempfile::tempdir().unwrap();
        let (answer, waiting) = mpsc::channel::<()>();
        let check = Check::asking_with(home.path().to_owned(), at(NOW), move || {
            let _released = waiting.recv();
            Some(version("2.0.0"))
        });
        assert_eq!(
            check.newer_than(version("1.0.0"), Duration::from_millis(10)),
            None
        );
        drop(answer);
        let deadline = Instant::now().checked_add(Duration::from_secs(30)).unwrap();
        while kept(home.path()).is_none() {
            assert!(Instant::now() < deadline, "the late answer was never kept");
            thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(kept(home.path()), Some((at(NOW), version("2.0.0"))));
    }

    #[test]
    fn says_to_upgrade_the_way_steamship_was_installed() {
        let root = Path::new("cargo");
        let program = root.join("bin").join("steamship");
        let binstall = root.join("bin").join(format!("cargo-binstall{EXE_SUFFIX}"));
        let record = root.join(".crates.toml");
        let with = |files: &[&Path]| {
            let files: Vec<PathBuf> = files.iter().map(|file| file.to_path_buf()).collect();
            Installed::of(&program, move |path: &Path| {
                files.iter().any(|file| file == path)
            })
        };
        assert_eq!(with(&[&record, &binstall]), Installed::Binstall);
        assert_eq!(with(&[&record]), Installed::Cargo);
        assert_eq!(with(&[&binstall]), Installed::Archive);
        assert_eq!(with(&[]), Installed::Archive);
        assert_eq!(
            Installed::of(Path::new("steamship"), |_: &Path| true),
            Installed::Archive
        );
        assert_eq!(
            Installed::of(Path::new(""), |_: &Path| true),
            Installed::Archive
        );
        assert_eq!(
            Installed::Binstall.hint().command,
            "cargo binstall steamship"
        );
        assert_eq!(
            Installed::Cargo.hint().command,
            "cargo install --locked steamship"
        );
        assert_eq!(Installed::Archive.hint().command, LATEST);
    }

    #[test]
    fn knows_each_package_managers_folders_and_says_its_upgrade() {
        let nothing = |_: &Path| false;
        for (program, installed, command) in [
            (
                "/opt/homebrew/Cellar/steamship/0.4.0/bin/steamship",
                Installed::Homebrew,
                "brew upgrade steamship",
            ),
            (
                "/home/linuxbrew/.linuxbrew/Cellar/steamship/0.4.0_1/bin/steamship",
                Installed::Homebrew,
                "brew upgrade steamship",
            ),
            (
                r"C:\Users\someone\scoop\apps\steamship\current\steamship.exe",
                Installed::Scoop,
                "scoop update steamship",
            ),
            (
                r"D:\Tools\Scoop\Apps\Steamship\0.4.0\steamship.exe",
                Installed::Scoop,
                "scoop update steamship",
            ),
            (
                r"C:\Users\someone\AppData\Local\Microsoft\WinGet\Packages\Aureliolo.steamship__DefaultSource\steamship-0.4.0-x86_64-pc-windows-msvc\steamship.exe",
                Installed::Winget,
                "winget upgrade Aureliolo.steamship",
            ),
        ] {
            // Written with the separators of the system that runs the test.
            let program: PathBuf = program.split(['/', '\\']).collect();
            assert_eq!(Installed::of(&program, nothing), installed, "{program:?}");
            assert_eq!(installed.hint().command, command);
        }
        for elsewhere in [
            "/opt/Cellar/other/0.4.0/bin/steamship",
            "/home/someone/apps/steamship.d/steamship",
            "/home/someone/Packages/Other.steamship_x/steamship",
            "/home/someone/steamship/Cellar/bin/steamship",
        ] {
            let program: PathBuf = elsewhere.split('/').collect();
            assert_eq!(
                Installed::of(&program, nothing),
                Installed::Archive,
                "{elsewhere}"
            );
        }
    }
}
