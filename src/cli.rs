//! The command line: every command, its options, its exit codes and the environment it reads.
//!
//! Kept in the library rather than beside `main` so that the documentation site is built from
//! these same definitions, and what it says cannot drift from what `--help` prints.

use std::path::PathBuf;

use clap::{Parser, Subcommand};

use crate::{ci, init, platform, show, update, webapi};

/// Something failed; what, and why, is printed.
pub const FAILED: u8 = 1;
/// `check` refused the scripts, or the command line was wrong (clap uses 2 for the latter too).
pub const REFUSED: u8 = 2;
/// steamcmd has no login it can use; `login` again.
pub const LOGIN: u8 = 3;
/// steamcmd is missing or has been altered.
pub const STEAMCMD: u8 = 4;

/// Every code steamship exits with, and what it means to whoever reads it.
pub const EXIT_CODES: [(u8, &str); 5] = [
    (0, "Done"),
    (
        FAILED,
        "The upload or login failed; the reason is printed, and for an upload the logs' paths",
    ),
    (
        REFUSED,
        "`check` refused the scripts, or the command line was wrong",
    ),
    (
        LOGIN,
        "Not logged in, or the token expired; run `steamship login`",
    ),
    (STEAMCMD, "steamcmd is missing or has been altered"),
];

/// The build account, when it is not the one `login` remembered.
pub const ACCOUNT: &str = "STEAMSHIP_ACCOUNT";

/// Every environment variable steamship reads, and what it is for. There is no password among
/// them, nor anywhere else.
pub const VARIABLES: [(&str, &str); 5] = [
    (
        platform::HOME,
        "Where steamcmd, its token and build output live; a per-user folder by default",
    ),
    (
        ACCOUNT,
        "The build account, if not the one `login` remembered",
    ),
    (
        ci::VARIABLE,
        "In CI, the login `steamship ci` set as a secret; `upload` and `status` use it",
    ),
    (
        webapi::KEY,
        "The publisher Web API key `builds` and `promote` use, before any kept one",
    ),
    (
        update::OPT_OUT,
        "Set to anything to never ask GitHub for a newer release",
    ),
];

#[derive(Debug, Parser)]
#[command(
    name = "steamship",
    bin_name = "steamship",
    version,
    about = "Uploads game builds to Steam with Valve's steamcmd.",
    styles = show::HELP
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Log in to Steam, once, for uploads.
    ///
    /// Asks for the account's name when none is given or remembered, then the password, and a
    /// Steam Guard code or approval in the Steam Mobile app. What you type is passed directly to
    /// steamcmd, never logged or saved. At a terminal it then offers to keep the publisher Web
    /// API key that `builds` and `promote` use, in the system's credential store.
    Login {
        /// The build account, if not the one the last login remembered.
        #[arg(long, env = ACCOUNT)]
        account: Option<String>,
        /// Only keep the publisher Web API key, checked with Steam first, without logging in.
        #[arg(long)]
        web_api_key: bool,
    },
    /// Show the login and steamcmd, checking the login with Steam.
    ///
    /// Shows the home, the build account and steamcmd, then logs in with the login steamcmd
    /// saved, as an upload does, and says whether Steam takes it. Nothing is asked for.
    Status {
        /// The build account, if not the one the last login remembered.
        #[arg(long, env = ACCOUNT)]
        account: Option<String>,
    },
    /// Check the build scripts, without logging in.
    ///
    /// Reads an app build script and every depot script it names, and refuses anything that
    /// would upload the wrong thing.
    Check {
        /// The app build script, such as `steam/app_build.vdf`.
        script: PathBuf,
    },
    /// Check, build and upload, then print the build ID.
    ///
    /// Runs `check`, then the build, with steamcmd out of sight, and says the build's ID and
    /// the branch it was set live on.
    Upload {
        /// The app build script, such as `steam/app_build.vdf`.
        script: PathBuf,
        /// The version being shipped, which with the commit becomes the build's description.
        #[arg(long)]
        version: String,
        /// Valve's dry run: the whole build is computed and logged, and nothing is uploaded.
        #[arg(long)]
        preview: bool,
        /// The build account, if not the one the last login remembered.
        #[arg(long, env = ACCOUNT)]
        account: Option<String>,
    },
    /// Upload a Workshop item, then print its ID.
    ///
    /// Checks a `workshopitem` script and what it names, then has steamcmd upload the item with
    /// the saved login. A script with no `publishedfileid` makes a new item, whose ID is printed
    /// to add to the script so that later uploads update the same item. In CI that is refused
    /// without `--new`, since every run would make another.
    Workshop {
        /// The item script, such as `workshop/item.vdf`.
        script: PathBuf,
        /// The build account, if not the one the last login remembered.
        #[arg(long, env = ACCOUNT)]
        account: Option<String>,
        /// Make a new item from CI, which a script with no `publishedfileid` otherwise is not.
        #[arg(long)]
        new: bool,
    },
    /// Show an app's branches and last builds.
    ///
    /// Lists each branch with the build live on it, then the last builds uploaded, through
    /// Steam's partner Web API, with the publisher key from `STEAMSHIP_WEB_API_KEY`, or else the
    /// one `login` kept; at a terminal, one is asked for when there is neither.
    Builds {
        /// The app, by its ID or its app build script.
        app: String,
        /// How many of the last builds to list.
        #[arg(long, default_value_t = 10)]
        count: u32,
    },
    /// Show an app's achievements, or check them against a file.
    ///
    /// Lists the achievements Steam holds for the app, in English, through Steam's partner Web
    /// API with the publisher key as `builds` finds it. With `--check`, compares them with a
    /// file by API name, and exits 2 naming every achievement missing on either side and every
    /// display name, description, hidden flag or icon that differs. Nothing on Steam is changed.
    Achievements {
        /// The app, by its ID or its app build script.
        app: String,
        /// A JSON file of the achievements the app should have, each with its `api_name`,
        /// `name`, `description` and `hidden`.
        #[arg(long, value_name = "FILE")]
        check: Option<PathBuf>,
    },
    /// Set an uploaded build live on a branch.
    ///
    /// Sets a build live on a beta branch without uploading it again, through Steam's partner
    /// Web API, with the publisher key as `builds` finds it. The default branch is set live in
    /// Steamworks only.
    Promote {
        /// The app, by its ID or its app build script.
        app: String,
        /// The build, by its build ID.
        #[arg(long)]
        build: u64,
        /// The branch to set it live on.
        #[arg(long)]
        branch: String,
    },
    /// Set up uploads from CI, the login kept as a secret.
    ///
    /// Finds the GitHub repository, its app build script and the saved login, checks the script,
    /// and the login with Steam, then sets the login as a secret with the GitHub command line,
    /// `gh`, and shows the workflow step that uploads. What it cannot find it asks for, and each
    /// option answers ahead. The secret holds the token steamcmd saved, never a password.
    Ci {
        /// The app build script, if not the one found in the repository.
        #[arg(long)]
        script: Option<PathBuf>,
        /// The GitHub repository as owner/name, if not the one `origin` points at.
        #[arg(long)]
        repo: Option<String>,
        /// The build account, if not the one the last login remembered.
        #[arg(long, env = ACCOUNT)]
        account: Option<String>,
        /// The name of the secret the login is kept in.
        #[arg(long, default_value = ci::VARIABLE)]
        secret: String,
        /// Write the login to this file instead, for a CI other than GitHub Actions.
        #[arg(long)]
        output: Option<PathBuf>,
    },
    /// Forget the saved login.
    ///
    /// Removes the login steamcmd saved, the account steamship remembered and the Web API key
    /// kept for `builds` and `promote`; the next upload needs `steamship login` first.
    Logout,
    /// Write starter build scripts for an app.
    ///
    /// Writes an app build script, and a depot build script for each depot, into `steam/`,
    /// commented, in Valve's own format, for `check` and `upload` to read. Without `--depot`,
    /// the one depot is the app's ID plus one, shipping `build/`. Nothing already there is
    /// overwritten.
    Init {
        /// The app's ID, from Steamworks.
        app: u32,
        /// A depot and the folder its files ship from, such as 1001=build/windows; once for each
        /// depot.
        #[arg(long = "depot", value_name = "ID=FOLDER", value_parser = init::depot)]
        depots: Vec<init::Depot>,
        /// The folder the scripts are written into.
        #[arg(long, default_value = "steam")]
        folder: PathBuf,
    },
    /// Install or verify the pinned steamcmd.
    ///
    /// `login` and `upload` do this themselves; this does it ahead of time, as in CI.
    Install,
    #[command(
        about = "Print tab completion for bash, zsh, fish, PowerShell or elvish",
        long_about = concat!(
            "Print tab completion for bash, zsh, fish, PowerShell or elvish, for the shell to ",
            "load.\n",
            "\n  bash:        steamship completions bash > ",
            "~/.local/share/bash-completion/completions/steamship",
            "\n  zsh:         steamship completions zsh > \"${fpath[1]}/_steamship\"",
            "\n  fish:        steamship completions fish > ~/.config/fish/completions/steamship.fish",
            "\n  PowerShell:  steamship completions powershell >> $PROFILE",
        )
    )]
    Completions {
        /// The shell to complete in.
        shell: clap_complete::Shell,
    },
}

#[cfg(test)]
mod tests {
    use clap::CommandFactory as _;

    use super::*;

    #[test]
    fn the_command_line_is_one_clap_accepts() {
        Cli::command().debug_assert();
    }

    #[test]
    fn every_exit_code_is_listed_once_in_order() {
        let codes: Vec<u8> = EXIT_CODES.iter().map(|&(code, _)| code).collect();
        assert_eq!(codes, [0, FAILED, REFUSED, LOGIN, STEAMCMD]);
        assert_eq!(codes, [0, 1, 2, 3, 4], "the codes scripts already test for");
    }

    #[test]
    fn every_variable_is_one_steamship_reads_and_none_is_a_password() {
        let names: Vec<&str> = VARIABLES.iter().map(|&(name, _)| name).collect();
        assert_eq!(
            names,
            [
                "STEAMSHIP_HOME",
                "STEAMSHIP_ACCOUNT",
                "STEAMSHIP_LOGIN",
                "STEAMSHIP_WEB_API_KEY",
                "STEAMSHIP_NO_UPDATE_CHECK"
            ]
        );
        assert!(names.iter().all(|name| !name.contains("PASSWORD")));
    }
}
