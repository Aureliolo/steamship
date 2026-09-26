//! Valve's steamcmd package manifest: a version, and every package with its size and SHA-256.
//!
//! The manifest for each system is pinned in `pins/`, byte for byte as Valve served it. steamcmd
//! is installed only from the packages a pin names, and a raise of the pin is a new copy of the
//! manifest, reviewed like any other change (`cargo run --example pins`).

use std::error;
use std::fmt;

use crate::digest;
use crate::platform::Platform;
use crate::vdf::{self, Block, Value};

const PINNED_WIN32: &str = include_str!("../pins/steam_cmd_win32.vdf");
const PINNED_OSX: &str = include_str!("../pins/steam_cmd_osx.vdf");
const PINNED_LINUX: &str = include_str!("../pins/steam_cmd_linux.vdf");

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Manifest {
    /// The system the manifest is for, as Valve names it: `win32`, `osx` or `linux`.
    pub system: String,
    pub version: u64,
    pub packages: Vec<Package>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Package {
    pub name: String,
    /// The file name on Valve's CDN, which carries a hash of its own and never changes content.
    pub file: String,
    pub size: u64,
    pub sha256: [u8; 32],
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error(pub String);

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "the steamcmd manifest {}", self.0)
    }
}

impl error::Error for Error {}

fn fail<T, Reason>(reason: Reason) -> Result<T, Error>
where
    Reason: Into<String>,
{
    Err(Error(reason.into()))
}

impl Manifest {
    /// The pinned manifest for `platform`.
    ///
    /// # Errors
    ///
    /// Only if the pinned copy does not parse, which the tests rule out for every platform.
    pub fn pinned(platform: Platform) -> Result<Self, Error> {
        let text = match platform {
            Platform::Windows => PINNED_WIN32,
            Platform::MacOs => PINNED_OSX,
            Platform::Linux => PINNED_LINUX,
        };
        Self::parse(text)
    }

    /// Reads a manifest in Valve's format.
    ///
    /// # Errors
    ///
    /// Anything that is not a manifest, or a package whose file name, size or hash cannot be
    /// used as it stands.
    pub fn parse(text: &str) -> Result<Self, Error> {
        let document =
            vdf::parse(text).map_err(|error| Error(format!("is not KeyValues: {error}")))?;
        let Some(first) = document.pairs.first() else {
            return fail("is empty");
        };
        let Value::Block(block) = &first.value else {
            return fail(format!(
                "starts with \"{}\", which is not a block",
                first.key
            ));
        };
        let version = number(block, "version")?;
        let mut packages = Vec::new();
        for pair in &block.pairs {
            if let Value::Block(package) = &pair.value {
                packages.push(Package::parse(&pair.key, package)?);
            }
        }
        if packages.is_empty() {
            return fail("names no packages");
        }
        Ok(Self {
            system: first.key.clone(),
            version,
            packages,
        })
    }
}

impl Package {
    fn parse(name: &str, block: &Block) -> Result<Self, Error> {
        let Some(file) = block.text("file") else {
            return fail(format!("package \"{name}\" has no file"));
        };
        // The name is appended to the CDN's address and used as a local file name, so it must
        // be a plain name: no separators, no way up.
        let plain = !file.is_empty()
            && !file.starts_with('.')
            && file.chars().all(|character| {
                character.is_ascii_alphanumeric() || matches!(character, '.' | '_' | '-')
            });
        if !plain {
            return fail(format!(
                "package \"{name}\" names the file \"{file}\", which is not a plain file name"
            ));
        }
        let size = number(block, "size")
            .map_err(|error| Error(format!("package \"{name}\": {}", error.0)))?;
        let Some(sha256) = block.text("sha2").and_then(digest::from_hex) else {
            return fail(format!(
                "package \"{name}\" has no SHA-256 of 64 hexadecimal digits"
            ));
        };
        Ok(Self {
            name: name.to_owned(),
            file: file.to_owned(),
            size,
            sha256,
        })
    }
}

fn number(block: &Block, key: &str) -> Result<u64, Error> {
    let Some(text) = block.text(key) else {
        return fail(format!("has no \"{key}\""));
    };
    if !text.bytes().all(|byte| byte.is_ascii_digit()) {
        return fail(format!("has \"{key}\" \"{text}\", which is not a number"));
    }
    text.parse().map_err(|error| {
        Error(format!(
            "has \"{key}\" \"{text}\", which cannot be read as a number: {error}"
        ))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_pin_parses_for_the_system_it_is_for() {
        for platform in [Platform::Windows, Platform::MacOs, Platform::Linux] {
            let manifest = Manifest::pinned(platform).unwrap();
            assert_eq!(manifest.system, platform.manifest_name());
            assert!(manifest.version > 0, "{platform:?}");
            let bootstrapper = format!("steamcmd_{}.zip.", platform.manifest_name());
            assert!(
                manifest
                    .packages
                    .iter()
                    .any(|package| package.file.starts_with(&bootstrapper)),
                "{platform:?} has no {bootstrapper} package"
            );
        }
    }

    const SAMPLE: &str = r#""linux"
{
	"version"		"1788292693"
	"steamcmd_linux"
	{
		"file"		"steamcmd_linux.zip.917c71eb05be1fb555d36ada71f0736c548fa035"
		"size"		"7924254"
		"sha2"		"81070e9bd2425bb9d3a89425923c4a9771b6755174fb072f8502836744c5d7e0"
		"IsBootstrapperPackage"		"1"
	}
}
"kvsign2"
{
	"linux"		"eb45dd38"
}
"#;

    #[test]
    fn reads_the_version_and_each_package() {
        let manifest = Manifest::parse(SAMPLE).unwrap();
        assert_eq!(manifest.system, "linux");
        assert_eq!(manifest.version, 1_788_292_693);
        let [package] = manifest.packages.as_slice() else {
            panic!("expected one package, found {:?}", manifest.packages);
        };
        assert_eq!(package.name, "steamcmd_linux");
        assert_eq!(package.size, 7_924_254);
        assert_eq!(
            digest::hex(&package.sha256),
            "81070e9bd2425bb9d3a89425923c4a9771b6755174fb072f8502836744c5d7e0"
        );
    }

    #[test]
    fn refuses_what_it_could_not_use_safely() {
        for (from, to, reason) in [
            (
                "\"1788292693\"",
                "\"17x\"",
                "has \"version\" \"17x\", which is not a number",
            ),
            (
                "\"1788292693\"",
                "\"99999999999999999999\"",
                "has \"version\" \"99999999999999999999\", which cannot be read as a number: number too large to fit in target type",
            ),
            (
                "\"7924254\"",
                "\"-1\"",
                "package \"steamcmd_linux\": has \"size\" \"-1\", which is not a number",
            ),
            (
                "steamcmd_linux.zip.917c",
                "../evil.zip.917c",
                "package \"steamcmd_linux\" names the file \"../evil.zip.917c71eb05be1fb555d36ada71f0736c548fa035\", which is not a plain file name",
            ),
            (
                "steamcmd_linux.zip.917c",
                ".hidden.zip.917c",
                "package \"steamcmd_linux\" names the file \".hidden.zip.917c71eb05be1fb555d36ada71f0736c548fa035\", which is not a plain file name",
            ),
            (
                "81070e9b",
                "+1070e9b",
                "package \"steamcmd_linux\" has no SHA-256 of 64 hexadecimal digits",
            ),
            (
                "\"sha2\"",
                "\"sha1\"",
                "package \"steamcmd_linux\" has no SHA-256 of 64 hexadecimal digits",
            ),
            (
                "\"file\"",
                "\"name\"",
                "package \"steamcmd_linux\" has no file",
            ),
            ("\"version\"", "\"release\"", "has no \"version\""),
        ] {
            let changed = SAMPLE.replacen(from, to, 1);
            assert_eq!(
                Manifest::parse(&changed).map_err(|error| error.0),
                Err(reason.to_owned()),
                "{from} -> {to}"
            );
        }
    }

    #[test]
    fn says_it_is_the_manifest_that_is_wrong() {
        let error = Manifest::parse("").unwrap_err();
        assert_eq!(error.to_string(), "the steamcmd manifest is empty");
    }

    #[test]
    fn refuses_a_document_that_is_not_a_manifest() {
        for (text, reason) in [
            ("", "is empty"),
            (
                "\"linux\" \"x\"",
                "starts with \"linux\", which is not a block",
            ),
            ("\"linux\" { \"version\" \"1\" }", "names no packages"),
            (
                "\"linux\" {",
                "is not KeyValues: line 1, column 10: a block is missing its closing }",
            ),
        ] {
            assert_eq!(
                Manifest::parse(text).map_err(|error| error.0),
                Err(reason.to_owned())
            );
        }
    }
}
