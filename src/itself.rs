//! Whether this steamship is its release's own binary, by the SBOM published with the release.
//!
//! The SBOM lists every file of every archive with its SHA-256, the binary among them, so the
//! release's own word on what was built needs no signature check here: a published release
//! cannot be changed, and it is fetched over HTTPS from GitHub.

use serde_json::Value as Json;

use crate::{digest, download, webapi};

const RELEASES: &str = "https://github.com/Aureliolo/steamship/releases/download";
const REPOSITORY: &str = "/repos/Aureliolo/steamship";
const API: &str = "https://api.github.com/repos/Aureliolo/steamship";

/// The system a release builds a binary for, as its archives are named, when it builds one for
/// the system this steamship was built for.
#[must_use]
pub const fn target() -> Option<&'static str> {
    if cfg!(all(target_os = "windows", target_arch = "x86_64")) {
        Some("x86_64-pc-windows-msvc")
    } else if cfg!(all(target_os = "windows", target_arch = "aarch64")) {
        Some("aarch64-pc-windows-msvc")
    } else if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
        Some("aarch64-apple-darwin")
    } else if cfg!(all(target_os = "macos", target_arch = "x86_64")) {
        Some("x86_64-apple-darwin")
    } else if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
        Some("x86_64-linux-musl")
    } else {
        None
    }
}

/// Where a debug build, as the tests run, may read releases from instead of GitHub: a stand-in
/// on this machine. A release build has no such place.
pub const STAND_IN: &str = "STEAMSHIP_RELEASES_STAND_IN";

/// Where the SBOM of a release is published: its download address, and the API's listing of its
/// release, which must serve it from under `assets`.
#[derive(Debug, PartialEq, Eq)]
pub struct Sbom {
    pub download: String,
    pub release: String,
    pub name: String,
    pub assets: String,
}

impl Sbom {
    #[must_use]
    pub fn asset(&self) -> download::Asset<'_> {
        download::Asset {
            download: &self.download,
            release: &self.release,
            name: &self.name,
            assets: &self.assets,
        }
    }
}

/// Where the SBOM of the release of `version` is published, where `lookup` reads the
/// environment.
pub fn sbom<Lookup>(version: &str, lookup: Lookup) -> Sbom
where
    Lookup: Fn(&str) -> Option<String>,
{
    let stand_in = lookup(STAND_IN).filter(|host| {
        host.strip_prefix("http://")
            .is_some_and(webapi::is_stand_in)
    });
    let (releases, api) = stand_in.map_or_else(
        || (RELEASES.to_owned(), API.to_owned()),
        |host| (host.clone(), format!("{host}{REPOSITORY}")),
    );
    let name = format!("steamship-{version}.spdx.json");
    Sbom {
        download: format!("{releases}/v{version}/{name}"),
        release: format!("{api}/releases/tags/v{version}"),
        assets: format!("{api}/releases/assets/"),
        name,
    }
}

/// The SHA-256 the SBOM `text` gives the binary of the release of `version` built for `target`.
///
/// # Errors
///
/// When `text` is not an SPDX SBOM, or lists no such binary, or its SHA-256 is not one.
pub fn listed(text: &str, version: &str, target: &str) -> Result<[u8; 32], String> {
    let document: Json =
        serde_json::from_str(text).map_err(|error| format!("is not JSON: {error}"))?;
    let files = document
        .get("files")
        .and_then(Json::as_array)
        .ok_or("lists no files")?;
    let folder = format!("steamship-{version}-{target}/");
    let binary = files
        .iter()
        .find(|file| {
            file.get("fileName")
                .and_then(Json::as_str)
                .and_then(|name| name.strip_prefix(&folder))
                .is_some_and(|name| name == "steamship" || name == "steamship.exe")
        })
        .ok_or_else(|| format!("lists no binary in {folder}"))?;
    let given = binary
        .get("checksums")
        .and_then(Json::as_array)
        .into_iter()
        .flatten()
        .find(|checksum| checksum.get("algorithm").and_then(Json::as_str) == Some("SHA256"))
        .and_then(|checksum| checksum.get("checksumValue").and_then(Json::as_str))
        .ok_or_else(|| format!("gives the binary in {folder} no SHA-256"))?;
    digest::from_hex(given)
        .ok_or_else(|| format!("gives the binary in {folder} a SHA-256 that is not one: {given}"))
}

#[cfg(test)]
mod tests {
    use std::env::consts;

    use super::*;

    const SBOM: &str = r#"{"spdxVersion": "SPDX-2.3", "files": [
      {"fileName": "steamship-1.2.3-x86_64-linux-musl/README.md",
       "checksums": [{"algorithm": "SHA256", "checksumValue": "1111111111111111111111111111111111111111111111111111111111111111"}]},
      {"fileName": "steamship-1.2.3-x86_64-linux-musl/steamship",
       "checksums": [{"algorithm": "SHA1", "checksumValue": "2222222222222222222222222222222222222222"},
                     {"algorithm": "SHA256", "checksumValue": "a783c5d805b07e2d0b2f9a619dbdd4ac08c16f570fda847138ca637992dc82e5"}]},
      {"fileName": "steamship-1.2.3-x86_64-pc-windows-msvc/steamship.exe",
       "checksums": [{"algorithm": "SHA256", "checksumValue": "59cf597d554226c53ff7fe87213357a569f84616bdf4d5f74b525355cdd05555"}]},
      {"fileName": "steamship-1.2.3-aarch64-apple-darwin/steamship",
       "checksums": [{"algorithm": "SHA256", "checksumValue": "not hex"}]}
    ]}"#;

    #[test]
    fn the_sbom_is_the_one_published_with_this_versions_release() {
        assert_eq!(
            sbom("1.2.3", |_| None),
            Sbom {
                download: "https://github.com/Aureliolo/steamship/releases/download/v1.2.3/steamship-1.2.3.spdx.json".to_owned(),
                release: "https://api.github.com/repos/Aureliolo/steamship/releases/tags/v1.2.3".to_owned(),
                name: "steamship-1.2.3.spdx.json".to_owned(),
                assets: "https://api.github.com/repos/Aureliolo/steamship/releases/assets/".to_owned(),
            }
        );
    }

    #[test]
    fn the_asset_fetched_is_the_one_named_by_the_sbom() {
        let published = sbom("1.2.3", |_| None);
        let asset = published.asset();
        assert_eq!(
            [asset.download, asset.release, asset.name, asset.assets],
            [
                published.download.as_str(),
                &published.release,
                &published.name,
                &published.assets
            ]
        );
    }

    #[test]
    fn only_a_stand_in_on_this_machine_takes_the_place_of_github() {
        let with =
            |value: &'static str| move |name: &str| (name == STAND_IN).then(|| value.to_owned());
        assert_eq!(
            sbom("1.2.3", with("http://127.0.0.1:8123")),
            Sbom {
                download: "http://127.0.0.1:8123/v1.2.3/steamship-1.2.3.spdx.json".to_owned(),
                release: "http://127.0.0.1:8123/repos/Aureliolo/steamship/releases/tags/v1.2.3"
                    .to_owned(),
                name: "steamship-1.2.3.spdx.json".to_owned(),
                assets: "http://127.0.0.1:8123/repos/Aureliolo/steamship/releases/assets/"
                    .to_owned(),
            }
        );
        for elsewhere in [
            "http://127.0.0.1:80@example.com",
            "http://example.com:8123",
            "http://127.0.0.1:",
        ] {
            assert_eq!(
                sbom("1.2.3", with(elsewhere)),
                sbom("1.2.3", |_| None),
                "{elsewhere}"
            );
        }
    }

    #[test]
    fn the_binary_built_for_each_system_is_found_by_its_archives_folder() {
        assert_eq!(
            listed(SBOM, "1.2.3", "x86_64-linux-musl").map(|found| digest::hex(&found)),
            Ok("a783c5d805b07e2d0b2f9a619dbdd4ac08c16f570fda847138ca637992dc82e5".to_owned())
        );
        assert_eq!(
            listed(SBOM, "1.2.3", "x86_64-pc-windows-msvc").map(|found| digest::hex(&found)),
            Ok("59cf597d554226c53ff7fe87213357a569f84616bdf4d5f74b525355cdd05555".to_owned())
        );
    }

    #[test]
    fn an_sbom_without_the_binary_says_what_it_lacks() {
        assert_eq!(
            listed(SBOM, "1.2.4", "x86_64-linux-musl"),
            Err("lists no binary in steamship-1.2.4-x86_64-linux-musl/".to_owned())
        );
        assert_eq!(
            listed(SBOM, "1.2.3", "aarch64-apple-darwin"),
            Err(
                "gives the binary in steamship-1.2.3-aarch64-apple-darwin/ a SHA-256 that is not \
                 one: not hex"
                    .to_owned()
            )
        );
        assert_eq!(
            listed(
                r#"{"files": [{"fileName": "steamship-1.2.3-x86_64-pc-windows-msvc/steamship.exe"}]}"#,
                "1.2.3",
                "x86_64-pc-windows-msvc"
            ),
            Err(
                "gives the binary in steamship-1.2.3-x86_64-pc-windows-msvc/ no SHA-256".to_owned()
            )
        );
        assert_eq!(
            listed("{}", "1.2.3", "x86_64-linux-musl"),
            Err("lists no files".to_owned())
        );
        assert!(
            listed("not json", "1.2.3", "x86_64-linux-musl")
                .is_err_and(|why| why.starts_with("is not JSON: "))
        );
    }

    #[test]
    fn this_system_is_named_as_its_releases_archive_is() {
        let named = target().unwrap();
        let system = match consts::OS {
            "windows" => "-pc-windows-msvc",
            "macos" => "-apple-darwin",
            _ => "-linux-musl",
        };
        assert_eq!(named, format!("{}{system}", consts::ARCH));
    }
}
