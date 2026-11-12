//! `steamship self check`.

use std::fs::File;
use std::path::Path;

use steamship::itself;

use crate::{steamship, web_api, web_api_answering};

#[test]
fn steamship_run_with_nothing_says_what_it_does_and_exits_0_but_a_command_missing_its_own_exits_2()
{
    let (code, stdout, stderr) = steamship(&[], Some(Path::new(".")), &[]);
    assert_eq!(code, Some(0_i32), "{stdout}{stderr}");
    assert!(
        stdout.contains("Usage: steamship [OPTIONS] <COMMAND>\n")
            && stdout
                .contains("  upload         Check, build and upload, then print the build ID\n"),
        "{stdout}"
    );
    assert!(stderr.is_empty(), "{stderr}");

    let (unfinished, said, why) = steamship(&["self"], Some(Path::new(".")), &[]);
    assert_eq!(unfinished, Some(2_i32), "{said}{why}");
    assert!(
        why.contains("Usage: steamship self [OPTIONS] <COMMAND>"),
        "{why}"
    );
}

/// The SBOM a release of this version would publish, with `sha256` as this system's binary's.
fn sbom_listing(sha256: &str) -> String {
    let version = env!("CARGO_PKG_VERSION");
    let target = itself::target().unwrap();
    format!(
        r#"{{"spdxVersion": "SPDX-2.3", "files": [{{"fileName": "steamship-{version}-{target}/steamship{}", "checksums": [{{"algorithm": "SHA256", "checksumValue": "{sha256}"}}]}}]}}"#,
        if cfg!(windows) { ".exe" } else { "" }
    )
}

/// `self check`, which reads releases from `host`.
fn self_check(host: &str) -> (Option<i32>, String, String) {
    let home = tempfile::tempdir().unwrap();
    steamship(
        &["self", "check"],
        Some(home.path()),
        &[(itself::STAND_IN, host), ("STEAMSHIP_NO_UPDATE_CHECK", "1")],
    )
}

#[test]
fn a_binary_its_release_lists_passes_its_self_check() {
    use steamship::digest;
    let own = digest::hex(
        &digest::sha256(&mut File::open(env!("CARGO_BIN_EXE_steamship")).unwrap()).unwrap(),
    );
    let (host, requests) = web_api(vec![("200 OK", sbom_listing(&own))]);

    let (code, said, told) = self_check(&host);
    assert_eq!(code, Some(0_i32), "{said}{told}");
    let version = env!("CARGO_PKG_VERSION");
    assert!(
        said.contains(&format!("is steamship {version}'s release binary for ")),
        "{said}"
    );
    assert!(said.contains(&own), "{said}");
    assert!(
        requests
            .recv()
            .unwrap()
            .starts_with(&format!("GET /v{version}/steamship-{version}.spdx.json ")),
        "the SBOM of this version's release is read"
    );
}

#[test]
fn a_binary_that_differs_from_its_release_is_refused() {
    let (host, _requests) = web_api(vec![("200 OK", sbom_listing(&"0".repeat(64)))]);

    let (code, said, told) = self_check(&host);
    assert_eq!(code, Some(2_i32), "{said}{told}");
    assert!(
        told.contains(&format!(
            "binary of steamship {}'s release. It was built from source (as by cargo install), \
             built elsewhere, or altered since. Install the release's own binary from \
             https://github.com/Aureliolo/steamship/releases/latest, or with cargo binstall \
             steamship",
            env!("CARGO_PKG_VERSION")
        )),
        "{told}"
    );
    assert!(said.contains(&"0".repeat(64)), "{said}");
}

#[test]
fn a_release_the_download_address_fails_on_is_read_through_githubs_api() {
    use steamship::digest;
    let own = digest::hex(
        &digest::sha256(&mut File::open(env!("CARGO_BIN_EXE_steamship")).unwrap()).unwrap(),
    );
    let version = env!("CARGO_PKG_VERSION");
    let name = format!("steamship-{version}.spdx.json");
    let (host, requests) = web_api_answering(|host| {
        let release = serde_json::json!({ "assets": [{
            "name": name,
            "url": format!("{host}/repos/Aureliolo/steamship/releases/assets/9"),
        }]});
        vec![
            ("504 Gateway Timeout", String::new()),
            ("200 OK", release.to_string()),
            ("200 OK", sbom_listing(&own)),
        ]
    });

    let (code, said, told) = self_check(&host);
    assert_eq!(code, Some(0_i32), "{said}{told}");
    assert!(said.contains(&own), "{said}");
    let asked: Vec<String> = requests
        .try_iter()
        .map(|request| request.lines().next().unwrap_or_default().to_owned())
        .collect();
    assert_eq!(
        asked,
        [
            format!("GET /v{version}/{name} HTTP/1.1"),
            format!("GET /repos/Aureliolo/steamship/releases/tags/v{version} HTTP/1.1"),
            "GET /repos/Aureliolo/steamship/releases/assets/9 HTTP/1.1".to_owned(),
        ]
    );
}

#[test]
fn a_release_without_an_sbom_fails_the_self_check_saying_so() {
    let (host, _requests) = web_api(vec![("404 Not Found", "{}"), ("404 Not Found", "{}")]);
    let (missing, said, told) = self_check(&host);
    assert_eq!(missing, Some(1_i32), "{said}{told}");
    assert!(said.contains("404"), "{said}{told}");

    let (unreadable, read, refusal) = self_check(&web_api(vec![("200 OK", "{}")]).0);
    assert_eq!(unreadable, Some(1_i32), "{read}{refusal}");
    assert!(refusal.contains(".spdx.json lists no files"), "{refusal}");
}
