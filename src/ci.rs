//! Uploading from CI, where nobody is there to log in.
//!
//! steamcmd's saved login is a token in one file. `steamship ci` packs that file and the build
//! account's name into one value for a CI secret, and `upload` and `status` unpack it into the
//! home when `STEAMSHIP_LOGIN` holds it. It never holds a password or a Steam Guard secret, but
//! it logs the account in until the token expires, so it is kept like one.

use std::error;
use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use zeroize::Zeroizing;

use crate::account::Account;
use crate::digest;
use crate::platform::Platform;
use crate::steamcmd;
use crate::vdf;

/// The environment variable a CI hands the packed login over in, and the secret's name.
pub const VARIABLE: &str = "STEAMSHIP_LOGIN";

/// Marks a value as a login packed by steamship, and how it was packed.
const TAG: &str = "steamship-login-1:";

/// A saved login and the account it is for.
pub struct Login {
    account: Account,
    config: Zeroizing<Vec<u8>>,
}

impl fmt::Debug for Login {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Neither half is ever shown: the name is half of what logs in, the file the other.
        formatter.debug_struct("Login").finish_non_exhaustive()
    }
}

#[derive(Debug)]
pub enum Error {
    /// The value is not a packed login. None of it is repeated, as it may be one damaged.
    Unreadable,
    /// There is no saved login to pack.
    NotSaved {
        path: PathBuf,
    },
    Io {
        path: PathBuf,
        error: io::Error,
    },
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unreadable => write!(
                formatter,
                "{VARIABLE} does not hold a login packed by `steamship ci`"
            ),
            Self::NotSaved { path } => {
                write!(formatter, "there is no saved login in {}", path.display())
            }
            Self::Io { path, error } => write!(formatter, "{}: {error}", path.display()),
        }
    }
}

impl error::Error for Error {}

impl Login {
    /// The login steamcmd saved in `home` on `platform`, for `account`.
    ///
    /// # Errors
    ///
    /// When there is none, or it cannot be read.
    pub fn saved(home: &Path, platform: Platform, account: Account) -> Result<Self, Error> {
        let path = steamcmd::saved_login(home, platform);
        match fs::read(&path) {
            Ok(config) if !config.is_empty() => Ok(Self {
                account,
                config: Zeroizing::new(config),
            }),
            Ok(_) => Err(Error::NotSaved { path }),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Err(Error::NotSaved { path }),
            Err(error) => Err(Error::Io { path, error }),
        }
    }

    /// The login as one line of text, for a secret.
    #[must_use]
    pub fn packed(&self) -> Zeroizing<String> {
        let digits = Zeroizing::new(digest::hex(&self.config));
        Zeroizing::new(format!("{TAG}{}:{}", self.account.name(), *digits))
    }

    /// The login `text` packs.
    ///
    /// # Errors
    ///
    /// When `text` is not a login packed by [`Login::packed`].
    pub fn unpack(text: &str) -> Result<Self, Error> {
        let rest = text.trim().strip_prefix(TAG).ok_or(Error::Unreadable)?;
        let (name, digits) = rest.split_once(':').ok_or(Error::Unreadable)?;
        // The name is not repeated in the error, as the rest may be a real login.
        let account = Account::parse(name).ok().ok_or(Error::Unreadable)?;
        let config = digest::bytes_from_hex(digits)
            .filter(|config| !config.is_empty())
            .ok_or(Error::Unreadable)?;
        Ok(Self {
            account,
            config: Zeroizing::new(config),
        })
    }

    #[must_use]
    pub const fn account(&self) -> &Account {
        &self.account
    }

    /// Puts the login where steamcmd on `platform` looks for it in `home`, over any there, and
    /// says where that is.
    ///
    /// # Errors
    ///
    /// When the file or its folder cannot be written.
    pub fn restore(&self, home: &Path, platform: Platform) -> Result<PathBuf, Error> {
        let path = steamcmd::saved_login(home, platform);
        let at = |error| Error::Io {
            path: path.clone(),
            error,
        };
        if let Some(folder) = path.parent() {
            fs::create_dir_all(folder).map_err(at)?;
        }
        write_private(&path, &self.config).map_err(at)?;
        Ok(path)
    }
}

/// Writes `contents` to `path` readable by its owner alone where the file system says who may
/// read; on Windows the file takes the permissions of its folder.
///
/// # Errors
///
/// When the file cannot be written.
pub fn write_private(path: &Path, contents: &[u8]) -> io::Result<()> {
    let mut options = fs::OpenOptions::new();
    let options = options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    let options = {
        use std::os::unix::fs::{OpenOptionsExt as _, PermissionsExt as _};

        // A file already there keeps its own mode through `open`, so it is set as well.
        if path.exists() {
            fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
        }
        options.mode(0o600)
    };
    io::Write::write_all(&mut options.open(path)?, contents)
}

/// Whether `name` can name a GitHub Actions secret: letters, digits and underscores, not
/// starting with a digit or `GITHUB_`.
#[must_use]
pub fn secret_name(name: &str) -> bool {
    name.bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
        && name
            .bytes()
            .next()
            .is_some_and(|first| !first.is_ascii_digit())
        && !name.to_ascii_uppercase().starts_with("GITHUB_")
}

/// The `owner/name` of the GitHub repository a Git remote at `url` points at.
#[must_use]
pub fn github_repository(url: &str) -> Option<String> {
    let url = url.trim();
    let path = [
        "https://github.com/",
        "ssh://git@github.com/",
        "git@github.com:",
    ]
    .iter()
    .find_map(|prefix| url.strip_prefix(prefix))?;
    let path = path.strip_suffix('/').unwrap_or(path);
    let path = path.strip_suffix(".git").unwrap_or(path);
    let (owner, name) = path.split_once('/')?;
    let fine = |part: &str| {
        !part.is_empty()
            && part
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"-_.".contains(&byte))
    };
    (fine(owner) && fine(name)).then(|| format!("{owner}/{name}"))
}

/// How many folders deep the search for app build scripts goes.
const DEEPEST: usize = 8;

/// A script larger than this is not a build script, and is not read.
const LARGEST: u64 = 1 << 20;

/// Folders that tools fill, which hold no build script of the project's own.
const SKIPPED: [&str; 2] = ["target", "node_modules"];

/// The app build scripts under `root`, found by what they hold rather than by name.
///
/// That is every `.vdf` file whose outer block is `AppBuild`. Hidden folders and those tools
/// fill are not searched, nor are folders that cannot be read.
///
/// # Errors
///
/// When `root` itself cannot be read.
pub fn app_scripts(root: &Path) -> io::Result<Vec<PathBuf>> {
    let mut found = Vec::new();
    for entry in fs::read_dir(root)? {
        look(&entry?, 0, &mut found);
    }
    found.sort();
    Ok(found)
}

fn look(entry: &fs::DirEntry, depth: usize, found: &mut Vec<PathBuf>) {
    let Ok(kind) = entry.file_type() else {
        return;
    };
    let name = entry.file_name();
    let name = name.to_string_lossy();
    let path = entry.path();
    if kind.is_dir() {
        if depth < DEEPEST && !name.starts_with('.') && !SKIPPED.contains(&name.as_ref()) {
            for inner in fs::read_dir(&path).into_iter().flatten().flatten() {
                look(&inner, depth.saturating_add(1), found);
            }
        }
        return;
    }
    let script = path
        .extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("vdf"))
        && entry
            .metadata()
            .is_ok_and(|metadata| metadata.len() <= LARGEST)
        && is_app_script(&path);
    if script {
        found.push(path);
    }
}

fn is_app_script(path: &Path) -> bool {
    fs::read_to_string(path)
        .ok()
        .and_then(|text| vdf::parse(&text).ok())
        .is_some_and(|document| document.block("AppBuild").is_some())
}

/// The workflow step that uploads `script`, a path from the repository's root.
///
/// The action is pinned to commit `pinned`, which is `release`, and logs in with the secret
/// `secret`. The step goes after the steps that build the content.
#[must_use]
pub fn step(script: &str, pinned: &str, release: &str, secret: &str) -> String {
    format!(
        "      - name: Upload to Steam\n        \
         uses: Aureliolo/steamship@{pinned} # {release}\n        \
         with:\n          \
         script: {script}\n          \
         version: ${{{{ github.ref_name }}}}\n          \
         login: ${{{{ secrets.{secret} }}}}\n"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn login() -> Login {
        Login {
            account: Account::parse("build_bot").unwrap(),
            config: Zeroizing::new(b"\"token\" \"a_saved_login_token\"\n".to_vec()),
        }
    }

    #[test]
    fn a_packed_login_unpacks_to_the_same_account_and_file() {
        let packed = login().packed();
        assert!(
            packed.starts_with("steamship-login-1:build_bot:"),
            "{}",
            *packed
        );
        assert!(
            !packed.contains("a_saved_login_token"),
            "the file is not readable in it"
        );
        let unpacked = Login::unpack(&format!("  {}\n", *packed)).unwrap();
        assert_eq!(unpacked.account().name(), "build_bot");
        assert_eq!(*unpacked.config, *login().config);
    }

    #[test]
    fn only_a_login_packed_by_steamship_unpacks_and_none_of_another_is_repeated() {
        let packed = login().packed();
        let digits = packed.rsplit(':').next().unwrap();
        for wrong in [
            String::new(),
            "steamship-login-2:build_bot:00".to_owned(),
            format!("steamship-login-1:{digits}"),
            format!("steamship-login-1:+quit:{digits}"),
            "steamship-login-1:build_bot:".to_owned(),
            "steamship-login-1:build_bot:0".to_owned(),
            "steamship-login-1:build_bot:zz".to_owned(),
        ] {
            let error = Login::unpack(&wrong).unwrap_err();
            assert!(matches!(error, Error::Unreadable), "{wrong}");
            assert_eq!(
                error.to_string(),
                "STEAMSHIP_LOGIN does not hold a login packed by `steamship ci`"
            );
        }
    }

    #[test]
    fn a_login_is_never_shown_by_debug() {
        let shown = format!("{:?}", login());
        assert!(
            !shown.contains("build_bot") && !shown.contains("token"),
            "{shown}"
        );
    }

    #[test]
    fn a_login_is_restored_where_steamcmd_looks_on_each_system_and_read_back() {
        for platform in [Platform::Windows, Platform::Linux, Platform::MacOs] {
            let home = tempfile::tempdir().unwrap();
            let path = login().restore(home.path(), platform).unwrap();
            assert_eq!(path, steamcmd::saved_login(home.path(), platform));
            assert_eq!(fs::read(&path).unwrap(), *login().config);
            let saved =
                Login::saved(home.path(), platform, Account::parse("build_bot").unwrap()).unwrap();
            assert_eq!(*saved.packed(), *login().packed());
        }
    }

    #[test]
    fn a_restored_login_replaces_the_one_there() {
        let home = tempfile::tempdir().unwrap();
        let path = steamcmd::saved_login(home.path(), Platform::Linux);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(
            &path,
            "an older, longer login than the one restored over it",
        )
        .unwrap();
        drop(login().restore(home.path(), Platform::Linux).unwrap());
        assert_eq!(fs::read(&path).unwrap(), *login().config);
    }

    #[cfg(unix)]
    #[test]
    fn a_restored_login_is_readable_by_its_owner_alone() {
        use std::os::unix::fs::PermissionsExt as _;

        let home = tempfile::tempdir().unwrap();
        let path = steamcmd::saved_login(home.path(), Platform::Linux);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, "").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
        drop(login().restore(home.path(), Platform::Linux).unwrap());
        let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
    }

    #[test]
    fn no_saved_login_or_an_empty_one_is_not_packed() {
        let home = tempfile::tempdir().unwrap();
        let account = || Account::parse("build_bot").unwrap();
        let missing = Login::saved(home.path(), Platform::Linux, account()).unwrap_err();
        let path = steamcmd::saved_login(home.path(), Platform::Linux);
        assert_eq!(
            missing.to_string(),
            format!("there is no saved login in {}", path.display())
        );
        fs::create_dir_all(&path).unwrap();
        let unreadable = Login::saved(home.path(), Platform::Linux, account()).unwrap_err();
        assert!(matches!(unreadable, Error::Io { .. }), "{unreadable}");
        assert!(
            unreadable
                .to_string()
                .starts_with(&format!("{}: ", path.display())),
            "{unreadable}"
        );
        fs::remove_dir(&path).unwrap();
        fs::write(&path, "").unwrap();
        let empty = Login::saved(home.path(), Platform::Linux, account()).unwrap_err();
        assert!(matches!(empty, Error::NotSaved { .. }), "{empty}");
    }

    #[test]
    fn names_a_secret_only_as_github_allows() {
        for good in ["STEAMSHIP_LOGIN", "steam_login_2", "_X"] {
            assert!(secret_name(good), "{good}");
        }
        for bad in [
            "",
            "2FA",
            "GITHUB_TOKEN",
            "github_x",
            "A-B",
            "A B",
            "A\u{e9}",
        ] {
            assert!(!secret_name(bad), "{bad}");
        }
    }

    #[test]
    fn reads_the_repository_from_every_way_github_is_cloned() {
        for url in [
            "https://github.com/Aureliolo/fantasy-guild-manager",
            "https://github.com/Aureliolo/fantasy-guild-manager.git",
            "https://github.com/Aureliolo/fantasy-guild-manager/",
            "git@github.com:Aureliolo/fantasy-guild-manager.git",
            "ssh://git@github.com/Aureliolo/fantasy-guild-manager.git\n",
        ] {
            assert_eq!(
                github_repository(url).as_deref(),
                Some("Aureliolo/fantasy-guild-manager"),
                "{url}"
            );
        }
        for url in [
            "https://gitlab.com/a/b",
            "https://github.com/a",
            "https://github.com/a/b/c",
            "https://github.com//b",
            "https://github.com/a/b c",
            "C:/Users/a/b",
        ] {
            assert_eq!(github_repository(url), None, "{url}");
        }
    }

    #[test]
    fn finds_app_build_scripts_by_what_they_hold() {
        let root = tempfile::tempdir().unwrap();
        let put = |relative: &str, text: &str| {
            let path = root.path().join(relative);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, text).unwrap();
        };
        let app = r#""AppBuild" { "AppID" "1000" }"#;
        put("steam/app_build.vdf", app);
        put("deploy/Other.VDF", r#""appbuild" { "AppID" "1001" }"#);
        put(
            "steam/depot_build.vdf",
            r#""DepotBuild" { "DepotID" "1001" }"#,
        );
        put("steam/broken.vdf", "\"AppBuild\" {");
        put("steam/app_build.txt", app);
        put(".hidden/app_build.vdf", app);
        put("target/app_build.vdf", app);
        put("node_modules/x/app_build.vdf", app);
        put("a/b/c/d/e/f/g/h/i/app_build.vdf", app);
        put("a/b/c/d/e/f/g/h/app_build.vdf", app);
        let found: Vec<PathBuf> = app_scripts(root.path())
            .unwrap()
            .iter()
            .map(|path| path.strip_prefix(root.path()).unwrap().to_path_buf())
            .collect();
        assert_eq!(
            found,
            [
                Path::new("a/b/c/d/e/f/g/h/app_build.vdf").to_path_buf(),
                Path::new("deploy/Other.VDF").to_path_buf(),
                Path::new("steam/app_build.vdf").to_path_buf(),
            ]
            .map(|path| path.components().collect::<PathBuf>())
        );
        drop(app_scripts(&root.path().join("missing")).unwrap_err());
    }

    #[test]
    fn the_step_pins_the_action_by_commit_and_takes_the_login_from_the_secret() {
        let step = step(
            "steam/app_build.vdf",
            "0123abcd",
            "v0.4.0",
            "STEAMSHIP_LOGIN",
        );
        assert_eq!(
            step,
            "      - name: Upload to Steam\n        \
             uses: Aureliolo/steamship@0123abcd # v0.4.0\n        \
             with:\n          \
             script: steam/app_build.vdf\n          \
             version: ${{ github.ref_name }}\n          \
             login: ${{ secrets.STEAMSHIP_LOGIN }}\n"
        );
    }
}
