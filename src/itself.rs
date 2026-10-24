//! Whether this steamship is its release's own binary, by the SBOM published with the release.
//!
//! The SBOM lists every file of every archive with its SHA-256, the binary among them, so the
//! release's own word on what was built needs no signature check here: a published release
//! cannot be changed, and it is fetched over HTTPS from GitHub.

use serde_json::Value as Json;

use crate::digest;

const RELEASES: &str = "https://github.com/Aureliolo/steamship/releases/download";

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

/// Where the SBOM of the release of `version` is published, where `lookup` reads the
/// environment.
pub fn sbom<Lookup>(version: &str, lookup: Lookup) -> String
where
    Lookup: Fn(&str) -> Option<String>,
{
    // A port and nothing after it: `http://127.0.0.1:80@example.com` names example.com.
    let loopback = |host: &String| {
        host.strip_prefix("http://127.0.0.1:")
            .is_some_and(|port| !port.is_empty() && port.bytes().all(|byte| byte.is_ascii_digit()))
    };
    let releases = lookup(STAND_IN)
        .filter(|host| cfg!(debug_assertions) && loopback(host))
        .unwrap_or_else(|| RELEASES.to_owned());
    format!("{releases}/v{version}/steamship-{version}.spdx.json")
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
            "https://github.com/Aureliolo/steamship/releases/download/v1.2.3/steamship-1.2.3.spdx.json"
        );
    }

    #[test]
    fn only_a_stand_in_on_this_machine_takes_the_place_of_github() {
        let with =
            |value: &'static str| move |name: &str| (name == STAND_IN).then(|| value.to_owned());
        assert_eq!(
            sbom("1.2.3", with("http://127.0.0.1:8123")),
            "http://127.0.0.1:8123/v1.2.3/steamship-1.2.3.spdx.json"
        );
        for elsewhere in [
            "http://127.0.0.1:80@example.com",
            "http://example.com:8123",
            "http://127.0.0.1:",
        ] {
            assert!(
                sbom("1.2.3", with(elsewhere)).starts_with("https://github.com/"),
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
