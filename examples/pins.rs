//! Rewrites `pins/` from the steamcmd manifests Valve serves now: `cargo run --example pins`.
//!
//! Renovate raises only the version inside a pin, and `.github/workflows/pins.yml` finishes the
//! raise on Renovate's branch the same way. This is that step by hand, for a pin raised any other
//! way; the result is reviewed and committed like any other change.
#![expect(
    clippy::print_stdout,
    clippy::print_stderr,
    reason = "a command line tool"
)]

use std::fs;
use std::path::Path;
use std::process::ExitCode;

use steamship::download;
use steamship::manifest::Manifest;
use steamship::platform::Platform;

fn main() -> ExitCode {
    for platform in [Platform::Windows, Platform::MacOs, Platform::Linux] {
        let name = platform.manifest_name();
        let url = format!("{}steam_cmd_{name}", download::CDN);
        let text = match download::text(&url) {
            Ok(text) => text,
            Err(error) => {
                eprintln!("{error}");
                return ExitCode::FAILURE;
            }
        };
        let manifest = match Manifest::parse(&text) {
            Ok(manifest) if manifest.system == name => manifest,
            Ok(manifest) => {
                eprintln!("{url} is the manifest for {}, not {name}", manifest.system);
                return ExitCode::FAILURE;
            }
            Err(error) => {
                eprintln!("{url}: {error}");
                return ExitCode::FAILURE;
            }
        };
        let pin = Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("pins/steam_cmd_{name}.vdf"));
        if let Err(error) = fs::write(&pin, &text) {
            eprintln!("{}: {error}", pin.display());
            return ExitCode::FAILURE;
        }
        println!(
            "{name}: version {}, {} packages",
            manifest.version,
            manifest.packages.len()
        );
    }
    ExitCode::SUCCESS
}
