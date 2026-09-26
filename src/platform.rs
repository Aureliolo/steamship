//! Which system this is, as a value rather than a compile-time switch, so that what differs
//! between systems is ordinary code every system's tests can reach.

use std::error;
use std::ffi::OsString;
use std::fmt;
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Platform {
    Windows,
    MacOs,
    Linux,
}

impl Platform {
    pub const THIS: Self = if cfg!(windows) {
        Self::Windows
    } else if cfg!(target_os = "macos") {
        Self::MacOs
    } else {
        Self::Linux
    };

    /// The name Valve's steamcmd manifests use for this system.
    #[must_use]
    pub const fn manifest_name(self) -> &'static str {
        match self {
            Self::Windows => "win32",
            Self::MacOs => "osx",
            Self::Linux => "linux",
        }
    }

    /// Where steamship keeps steamcmd, its login and build output: `STEAMSHIP_HOME` when set,
    /// otherwise the per-user data folder each system has for this.
    ///
    /// # Errors
    ///
    /// When neither `STEAMSHIP_HOME` nor the variable the default is built from is set.
    pub fn home<Lookup>(self, lookup: Lookup) -> Result<PathBuf, HomeError>
    where
        Lookup: Fn(&str) -> Option<OsString>,
    {
        let set = |name: &str| lookup(name).filter(|value| !value.is_empty());
        if let Some(home) = set("STEAMSHIP_HOME") {
            return Ok(PathBuf::from(home));
        }
        let (base, under) = match self {
            Self::Windows => ("LOCALAPPDATA", &[][..]),
            Self::MacOs => ("HOME", &["Library", "Application Support"][..]),
            Self::Linux => match set("XDG_DATA_HOME") {
                Some(data) => return Ok(PathBuf::from(data).join("steamship")),
                None => ("HOME", &[".local", "share"][..]),
            },
        };
        let mut home = PathBuf::from(set(base).ok_or(HomeError { variable: base })?);
        home.extend(under);
        home.push("steamship");
        Ok(home)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HomeError {
    pub variable: &'static str,
}

impl fmt::Display for HomeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "neither STEAMSHIP_HOME nor {} is set, so there is nowhere to keep steamcmd",
            self.variable
        )
    }
}

impl error::Error for HomeError {}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn home(platform: Platform, variables: &[(&str, &str)]) -> Result<PathBuf, HomeError> {
        platform.home(|name| {
            variables
                .iter()
                .find(|(key, _)| *key == name)
                .map(|(_, value)| OsString::from(value))
        })
    }

    #[test]
    fn steamship_home_wins_everywhere() {
        for platform in [Platform::Windows, Platform::MacOs, Platform::Linux] {
            let found = home(platform, &[("STEAMSHIP_HOME", "/chosen"), ("HOME", "/h")]);
            assert_eq!(found, Ok(PathBuf::from("/chosen")), "{platform:?}");
        }
    }

    #[test]
    fn each_system_has_its_own_default() {
        assert_eq!(
            home(
                Platform::Windows,
                &[("LOCALAPPDATA", "C:/Users/a/AppData/Local")]
            ),
            Ok(Path::new("C:/Users/a/AppData/Local").join("steamship"))
        );
        assert_eq!(
            home(Platform::MacOs, &[("HOME", "/Users/a")]),
            Ok(Path::new("/Users/a/Library/Application Support/steamship").to_path_buf())
        );
        assert_eq!(
            home(Platform::Linux, &[("HOME", "/home/a")]),
            Ok(Path::new("/home/a/.local/share/steamship").to_path_buf())
        );
        assert_eq!(
            home(
                Platform::Linux,
                &[("HOME", "/home/a"), ("XDG_DATA_HOME", "/data")]
            ),
            Ok(Path::new("/data/steamship").to_path_buf())
        );
    }

    #[test]
    fn an_empty_variable_counts_as_unset() {
        assert_eq!(
            home(
                Platform::Linux,
                &[
                    ("STEAMSHIP_HOME", ""),
                    ("XDG_DATA_HOME", ""),
                    ("HOME", "/h")
                ]
            ),
            Ok(Path::new("/h/.local/share/steamship").to_path_buf())
        );
    }

    #[test]
    fn says_which_variable_it_needed() {
        let missing = home(Platform::Windows, &[]).map_err(|error| error.to_string());
        assert_eq!(
            missing,
            Err(
                "neither STEAMSHIP_HOME nor LOCALAPPDATA is set, so there is nowhere to keep \
                 steamcmd"
                    .to_owned()
            )
        );
    }

    #[test]
    fn names_each_system_as_valves_manifests_do() {
        let names =
            [Platform::Windows, Platform::MacOs, Platform::Linux].map(Platform::manifest_name);
        assert_eq!(names, ["win32", "osx", "linux"]);
    }
}
