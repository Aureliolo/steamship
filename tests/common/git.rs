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

/// A bare repository called `name` in `folder`, on `main`.
pub fn bare(folder: &Path, name: &str) {
    git(
        folder,
        &["init", "--quiet", "--bare", "--initial-branch=main", name],
    );
}

/// A repository at `root` on `main`, with the bare `origin.git` in `temp` as its `origin`.
pub fn with_origin(temp: &Path, root: &Path) {
    bare(temp, "origin.git");
    git(root, &["init", "--quiet", "--initial-branch=main"]);
    git(
        root,
        &[
            "remote",
            "add",
            "origin",
            &temp.join("origin.git").to_string_lossy(),
        ],
    );
}

/// An empty commit with `message`, on whatever `folder` has checked out.
pub fn commit(folder: &Path, message: &str) {
    git(
        folder,
        &["commit", "--quiet", "--allow-empty", "-m", message],
    );
}

/// The commit `folder` has checked out, in full.
///
/// # Panics
///
/// When git cannot be started or does not print a commit, which fails the test.
#[must_use]
pub fn head(folder: &Path) -> String {
    let output = run::output(
        run::command("git")
            .args(["rev-parse", "HEAD"])
            .current_dir(folder),
    )
    .unwrap();
    String::from_utf8(output.stdout).unwrap().trim().to_owned()
}
