//! git as the tests run it: in a folder of their own, cut off from the machine's own config.

use std::path::Path;

use steamship::run;

/// Runs git with `args` in `folder`, as a fixed author.
///
/// # Panics
///
/// When git cannot be started or fails, which fails the test.
pub fn git(folder: &Path, args: &[&str]) {
    let status = run::spawn(
        run::command("git")
            .args([
                "-c",
                "user.name=steamship",
                "-c",
                "user.email=steamship@example.invalid",
            ])
            .args(["-c", "core.autocrlf=false", "-c", "maintenance.auto=false"])
            // The machine's own git config could sign each commit or run its hooks, which a
            // `core.hooksPath` override does not stop when they are set as config.
            .env("GIT_CONFIG_GLOBAL", folder.join("no-git-config"))
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .args(args)
            .current_dir(folder),
    )
    .and_then(|mut git| git.wait())
    .unwrap();
    assert!(status.success(), "git {args:?}");
}
