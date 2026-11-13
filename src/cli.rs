//! The command line: every command, its options, its exit codes and the environment it reads.
//!
//! Kept in the library rather than beside `main` so that the documentation site is built from
//! these same definitions, and what it says cannot drift from what `--help` prints.

#![expect(
    unused_results,
    reason = "clap's derive checks that the optional group in `Workshop` has an ID, then drops it"
)]

use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};

use crate::platform::Platform;
use crate::{ci, fetch, init, platform, show, update, webapi, workshop};

/// Something failed. What failed, and why, is printed.
pub const FAILED: u8 = 1;
/// A check refused the input or found a difference, or the command line was wrong (clap uses 2
/// for the latter too).
pub const REFUSED: u8 = 2;
/// steamcmd has no login it can use. Run `login` again.
pub const LOGIN: u8 = 3;
/// steamcmd's install is missing, altered or failed.
pub const STEAMCMD: u8 = 4;
/// Steam holds a build or Workshop item that a second run would duplicate.
pub const UNFINISHED: u8 = 5;
/// Steam is limiting login attempts.
pub const LIMITED: u8 = 6;

/// What a shell sees when a signal reaches steamship, which is no code of the table.
pub const SIGNAL_NOTE: &str = "On Linux and macOS, a SIGINT, SIGHUP or SIGTERM sent to steamship stops steamcmd. steamship prints its logs and ends as that signal would, so a shell sees 128 plus the signal's number: 130, 129 or 143. On Windows, steamcmd ends with steamship and its logs are not printed.";

/// The codes steamship exits with itself, and what each means to whoever reads it.
pub const EXIT_CODES: [(u8, &str); 7] = [
    (0, "Done"),
    (
        FAILED,
        "The command failed and printed why. A second run makes no duplicate",
    ),
    (
        REFUSED,
        "A check refused the input or found a difference, and named each. Or the command line was wrong",
    ),
    (
        LOGIN,
        "Not logged in, or Steam refused the saved login. Run `steamship login`",
    ),
    (
        STEAMCMD,
        "steamcmd's install is missing, altered or failed. A second run makes no duplicate",
    ),
    (
        UNFINISHED,
        "Steam holds a build or Workshop item that a second run would duplicate. The output says what to do",
    ),
    (
        LIMITED,
        "Steam is limiting login attempts. Wait, then run the same command again. Do not log in \
         again",
    ),
];

/// The build account, when it is not the one `login` remembered.
pub const ACCOUNT: &str = "STEAMSHIP_ACCOUNT";

/// Every environment variable steamship's own code reads by name in a release build, and what it
/// is for: its own first, then CI's, then the system's.
///
/// A test holds the list to that. A debug build's stand-in hosts, the tests' fake steamcmd and
/// what only a dependency reads stay out. The Steam account's password is never among them, nor anywhere else. A beta branch's password
/// is, since every player let in shares it.
pub const VARIABLES: [(&str, &str); 14] = [
    (
        platform::HOME,
        "Where steamcmd, its token and build output live, if not the per-user folder `status` shows",
    ),
    (
        ACCOUNT,
        "The build account, if not the one `login` remembered",
    ),
    (
        ci::VARIABLE,
        "In CI, the login `steamship ci` set as a secret, for `upload` and `status`",
    ),
    (
        webapi::KEY,
        "The publisher Web API key, for `builds` and the other Web API commands, before any kept one",
    ),
    (
        fetch::PASSWORD,
        "The password of the beta branch `fetch` downloads, before any kept one",
    ),
    (
        update::OPT_OUT,
        "Set to anything to never ask GitHub for a newer release",
    ),
    (
        "CI",
        "Set by CI systems. A non-empty value silences the update notice. Unless it is `false`, a Workshop item without an ID needs `--new`",
    ),
    (
        "GITHUB_ACTIONS",
        "Set to `true` by GitHub Actions. steamship then masks the CI account's name and prints Steam's logs as collapsed groups",
    ),
    (
        "GITHUB_OUTPUT",
        "In GitHub Actions, the file steamship writes the build ID to, as a step output",
    ),
    (
        "HOME",
        "On macOS, and on Linux when `XDG_DATA_HOME` is not set, the folder the default per-user folder goes in",
    ),
    (
        "XDG_DATA_HOME",
        "On Linux, the folder the default per-user folder goes in, before `HOME`",
    ),
    (
        "LOCALAPPDATA",
        "On Windows, the folder the default per-user folder goes in",
    ),
    (
        "DBUS_SESSION_BUS_ADDRESS",
        "On Linux, the address of the session bus that reaches GNOME Keyring or KWallet, where the kept Web API key and beta branch passwords are",
    ),
    (
        "XDG_RUNTIME_DIR",
        "On Linux, where the session bus socket is looked for when `DBUS_SESSION_BUS_ADDRESS` is not set",
    ),
];

/// Reads an app's ID as `init` takes it.
fn app_id(text: &str) -> Result<u32, String> {
    text.parse::<u32>()
        .ok()
        .filter(|id| *id > 0)
        .ok_or_else(|| {
            format!("'{text}' is not an app ID. Give the number from Steamworks, such as 480")
        })
}

/// Reads how many builds `builds` lists.
fn build_count(text: &str) -> Result<u32, String> {
    text.parse::<u32>().map_err(|_not_a_number| {
        format!("'{text}' is not a number. Give how many builds to list, such as 10")
    })
}

/// Reads a build ID as `fetch` and `promote` take it.
fn build_id(text: &str) -> Result<u64, String> {
    text.parse::<u64>().map_err(|_not_a_number| {
        format!("'{text}' is not a build ID. Give the number `builds` lists, such as 1234567")
    })
}

#[derive(Debug, Parser)]
#[command(
    name = "steamship",
    bin_name = "steamship",
    version,
    about = "Uploads game builds to Steam with Valve's steamcmd.",
    styles = show::HELP
)]
pub struct Cli {
    /// Show how long each step took, at the end of its line.
    #[arg(long, global = true)]
    pub times: bool,
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Log in to Steam, once, for uploads.
    ///
    /// Asks for the account name if none is given or remembered. It then asks for the password
    /// and a Steam Guard code or approval in the Steam Mobile app. steamcmd receives what you
    /// type. steamship never logs or saves it. At a terminal, it then offers to keep the publisher
    /// Web API key in the system's credential store. `builds` and the other Web API commands use
    /// it.
    Login {
        /// The build account, if not the one the last login remembered.
        #[arg(long, env = ACCOUNT)]
        account: Option<String>,
        /// Keep only the publisher Web API key, after Steam accepts it. No login happens.
        #[arg(long)]
        web_api_key_only: bool,
    },
    /// Show the login, steamcmd and whether Steam accepts the login.
    ///
    /// Shows steamship's home, the build account and steamcmd. Then it logs in with the login
    /// steamcmd saved, as an upload does. Nothing is asked for.
    Status {
        /// The build account, if not the one the last login remembered.
        #[arg(long, env = ACCOUNT)]
        account: Option<String>,
    },
    /// Check the build scripts, without logging in.
    ///
    /// Reads an app build script and every depot script it names, and refuses anything that
    /// would upload the wrong thing. Without an app, in a game's folder, it checks the game's
    /// scripts. It then compares the game's achievements, leaderboards, rich presence and
    /// branches on Steam with the game's files, and names every difference.
    ///
    /// Given a build's folder, it checks the files alone, before any script maps them. It names
    /// every DLL the folder's Windows programs and libraries load that neither Windows nor the
    /// folder has.
    ///
    /// In a game's folder, a DLL that a Steamworks Common Redistributable installs is refused
    /// unless the settings snapshot ticks that redistributable. A Common Redistributable is a
    /// runtime Steam installs for the game, such as the Visual C++ runtime. A build without the
    /// Steam Input action manifest the snapshot names is refused too. The snapshot is the demo's
    /// with `--demo`, or for a folder only the demo's depots ship.
    ///
    /// The game's rich presence files are read too, as `rich-presence --dry-run` reads them. All
    /// of this is offline.
    Check {
        /// The app, by its ID or its app build script, such as `steam/app_build.vdf`. A build's
        /// folder, such as `export/windows`, is checked instead of a script. In a game's folder,
        /// the game's own.
        app: Option<String>,
        /// Check the scripts alone, not the content they map, as before the build exists.
        /// `upload` still checks both.
        #[arg(long)]
        scripts_only: bool,
        /// In a game's folder, check the demo, a separate app, instead of the full game. With a
        /// build's folder, judge it against the demo's settings snapshot. A folder only the
        /// demo's depots ship is judged that way without the flag.
        #[arg(long)]
        demo: bool,
    },
    /// Check, build and upload, then print the build ID.
    ///
    /// Runs `check`, then the build, without showing steamcmd's console. It prints the build's ID
    /// and the branch it was set live on.
    ///
    /// Without an app, in a game's folder, it ships the game. That is refused while the game's
    /// tree holds anything uncommitted or origin's default branch is at another commit. The game's
    /// script must set the game's branch live: `testing`, or the branch `steamship.toml` names.
    ///
    /// The game is built with its `build` command, with steamcmd and the build held to 4
    /// processors or the file's `cores`. A file its depots map that the build did not write is
    /// refused.
    ///
    /// The build is checked against the settings snapshot `steam/settings.vdf`, its achievements,
    /// leaderboards and rich presence. Its version is the newest release `CHANGES.md` heads, or
    /// else the one `project.godot` gives.
    ///
    /// A game whose `steamship.toml` sets `green` ships only a commit on that branch of origin,
    /// unless `--allow-ungreen` is given. `--dry-run` skips that check.
    ///
    /// While the app is unreleased, the build goes live on the default branch too. A game whose
    /// `steamship.toml` sets `released = true` never gets a build live on default. Once the build
    /// is live, the branch's description becomes the version, or the file's `description`. An app
    /// with only the default branch gets the build on default alone. steamship then prints the
    /// commands that set it live on the game's branch and describe that branch, to run once the
    /// branch exists.
    Upload(Upload),
    /// Upload a Workshop item, then print its ID.
    ///
    /// Takes the item as a Valve `workshopitem` script, or as fields: the app first, then `--items`
    /// and the other options. From the fields, steamship writes the script itself. It checks the item and what it names, then has steamcmd upload it with the
    /// saved login.
    ///
    /// A script with no `publishedfileid` makes a new item. Its ID is printed for you to add to
    /// the script. Given by fields, an item whose name the `--items` file lacks is made new, and
    /// its ID is written into that file. Either way, later uploads update the same item. In CI, a
    /// new item needs `--new`, since every run would make another.
    ///
    /// With no fields and no first word, in a game's folder it uploads each example mod, or only
    /// the one `--name` names. The game's own app as the first word does the same. An example mod
    /// is a folder in `modding/` holding a `manifest.json`. This is refused unless the tree is
    /// pushed, as for `upload`. Each mod's title, description
    /// and version come from its manifest. Its preview is the `preview.png` in its folder. Each
    /// item's ID is kept in `steam/workshop_items.json`.
    ///
    /// A game's `steamship.toml` can set `held_until_launch` under `[workshop]`. An item for that
    /// game's app then goes up only once Steam shows the game released, or with `--for-launch`.
    Workshop(Workshop),
    /// Show an app's branches and last builds.
    ///
    /// Lists each branch with the build live on it, then the last builds uploaded. It asks Steam's
    /// partner Web API, with the publisher key from `STEAMSHIP_WEB_API_KEY`, or else the one
    /// `login` kept. At a terminal, it asks for a key when there is neither.
    Builds {
        /// The app, by its ID or its app build script. In a game's folder, the game's own.
        app: Option<String>,
        /// In a game's folder, list the demo's branches and builds.
        #[arg(long, conflicts_with = "app")]
        demo: bool,
        /// How many of the last builds to list.
        #[arg(long, default_value_t = 10, value_parser = build_count)]
        count: u32,
        /// Print the branches and builds on standard output as one line of JSON, or what Steam
        /// failed with as an `error` holding a `code`. Everything else goes to standard error.
        #[arg(long)]
        json: bool,
    },
    /// Download a branch's live build into its own folder.
    ///
    /// Asks Steam's partner Web API which build is live on the branch, with the publisher key as
    /// `builds` finds it. steamcmd then downloads that build as the build account, with the saved
    /// login. It downloads for this system, or the one `--system` names.
    ///
    /// Each app, branch and system has its own folder under steamship's home, or the one
    /// `--output` names. Nothing is installed in a Steam library. A fetch into a folder already
    /// holding a build of the branch checks each of the build's files there. It downloads only
    /// those that differ, and leaves files the build does not have in place. A folder holding
    /// another branch is refused. The folder and its build are printed once steamcmd's record of
    /// it shows the build asked for.
    ///
    /// A branch's password comes from `STEAMSHIP_BRANCH_PASSWORD`, or else the system's
    /// credential store, or else the prompt, which can then keep it. steamship types it into
    /// steamcmd's console. `--password-on-command-line` gives it on steamcmd's command line
    /// instead. A kept password that Steam refuses is forgotten.
    Fetch(Fetch),
    /// Show, save or check an app's achievements and stats, or write their import.
    ///
    /// Lists the achievements and stats Steam holds for the app, in English. Each achievement
    /// shows the share of players who have it, and each aggregated stat its total over all
    /// players. This goes through Steam's partner Web API, with the publisher key as `builds`
    /// finds it. `--save` writes them to a file instead, which `--check` then reads.
    ///
    /// With `--check`, it compares them by API name with the files, read together. It exits 2
    /// and names every achievement missing on either side, and every display name, description
    /// or hidden flag that differs. It also names every icon or locked icon Steam lacks. When a
    /// file lists stats, it compares their display names and defaults too, and whether Steam
    /// totals them over all players. With a localisation or gettext catalogues among the files,
    /// it also asks Steam for the achievements in each of their languages. It names every name
    /// or description that differs, or that Steam lacks in a language. Nothing on Steam is
    /// changed.
    ///
    /// With `--write`, it writes the files for the achievement editor's Bulk Import/Export into
    /// the `--output` folder instead, from the same kind of files. Nothing is asked of Steam. The
    /// files hold the definitions, every name and description in each language, the sets' names,
    /// and each icon named for its achievement.
    ///
    /// Either way, the files are refused first, exiting 2, when an achievement unlocks at a value
    /// its stat never takes. They are refused too for an achievement in a set no file lists, or
    /// for more sets or achievements than Steam takes.
    ///
    /// Once the files are imported and published, `--imported` compares each icon Steam serves
    /// with its file, pixel by pixel. When all match, it writes the receipt `steam-icons.json`
    /// into the `--output` folder. `--check` with that `--output` then also names icons changed in the
    /// files since, and icons Steam made anew from an unchanged file. It names icons the receipt
    /// has no import of too. It fetches no images for this.
    ///
    /// In a game's folder, `--check` and `--write` with no files read the game's own. These are
    /// the games-kit records in `mods/base/data` holding achievements, stats or sets, and every
    /// catalogue in `game/locale`. The `--output` folder is `steam/achievements`. `--check` compares
    /// the icons with its receipt once there is one. `--imported` alone writes that receipt.
    #[command(group(clap::ArgGroup::new("file_switches").args(["check", "write"]).multiple(true)))]
    Achievements {
        /// The app, by its ID or its app build script. In a game's folder, the game's own.
        app: Option<String>,
        /// JSON files of the achievements the app should have, each with its `api_name`, `name`,
        /// `description` and `hidden`. Stats to be checked go in them too, each with its
        /// `api_name`, `name` and `default`. A file may instead be a list of games-kit records,
        /// the JSON records a game keeps its content in. Those typed `achievement` and `stat` are
        /// read by the API name in `steam`. Each
        /// achievement and stat is in one file only. A `.csv` file is a localisation in the
        /// columns Steamworks' achievement editor exports. A `.po` file is a gettext catalogue.
        /// Either must translate every achievement. Files follow the app. `--check` or
        /// `--write` says what to do with them.
        #[arg(value_name = "FILES", requires = "file_switches")]
        files: Vec<PathBuf>,
        /// Compare the achievements Steam holds with the files.
        #[arg(long, overrides_with = "check")]
        check: bool,
        /// Write the achievements and stats Steam holds to this JSON file, as `--check` reads
        /// it. Refused, exiting 2, when an achievement has no name in English, which a file must
        /// give. Refused too when the file is there and holds more than `--save` writes, such as
        /// an `icon` or a `sets` list.
        #[arg(
            long,
            value_name = "FILE",
            conflicts_with_all = ["file_switches", "files", "imported", "json"]
        )]
        save: Option<PathBuf>,
        #[command(flatten)]
        writing: Writing,
        /// Print the achievements and stats on standard output as one line of JSON, or what
        /// Steam failed with as an `error` holding a `code`. Everything else goes to standard
        /// error.
        #[arg(long, conflicts_with = "check")]
        json: bool,
    },
    /// Check artwork and icons against Valve's sizes, before they are uploaded.
    ///
    /// Reads every PNG, JPEG, icon and zip in the folder. Each is named for its asset, such as
    /// `header_capsule.png`, `library_logo.png`, `screenshot_01.jpg` or `linux_icons.zip`. A
    /// language's file adds Steam's name for the language, as in `header_capsule_koreana.png`.
    /// A subfolder named for a language, such as `de` or `koreana`, holds that language's files.
    ///
    /// It exits 2 and names every file that is not a size or format Steamworks takes, is named
    /// for no asset, or cannot be read. It also names every file in the Linux icons that is not a
    /// square PNG, and says when there are too few screenshots. Nothing is sent anywhere.
    Assets {
        /// The folder the artwork is kept in, such as `steam/store`.
        folder: PathBuf,
    },
    /// Wrap a Windows executable in Steam DRM, before it is uploaded.
    ///
    /// Valve's servers add the app's Steam DRM to the executable, through steamcmd with the saved
    /// login. The wrapped executable replaces the one given, or goes to `--output`. Nothing is
    /// replaced unless the wrap succeeded. Valve's tool wraps only an executable that already
    /// imports `GetModuleHandleA`, and Godot's do not.
    #[command(name = "drm-wrap", allow_missing_positional = true)]
    DrmWrap {
        /// The app, by its ID or its app build script. In a game's folder, the game's own.
        app: Option<String>,
        /// The Windows executable, such as `build/windows/game.exe`.
        executable: PathBuf,
        /// Write the wrapped executable here instead of replacing the one given.
        #[arg(long, value_name = "FILE")]
        output: Option<PathBuf>,
        /// Wrap in Valve's compatibility mode, for an executable the default wrap breaks.
        #[arg(long)]
        compatibility: bool,
        /// The build account, if not the one the last login remembered.
        #[arg(long, env = ACCOUNT)]
        account: Option<String>,
    },
    /// Show an app's settings in Steamworks, or check them against a snapshot.
    ///
    /// Asks Steam, through steamcmd with the saved login, for the Steamworks settings a build
    /// depends on. These are the app's name, type and systems, its install folder, launch options
    /// and runtime, its depots' systems, and Steam Cloud. `--save` keeps them in a snapshot
    /// file, in Valve's own format. `--check` exits 2 naming every setting that differs from that
    /// file. In a game's folder, with no file given, the file is `settings.vdf` beside the build
    /// scripts. Nothing on Steam is changed.
    Settings(Settings),
    /// Show, save, check, create or delete an app's leaderboards.
    ///
    /// Lists the leaderboards Steam holds for the app, through Steam's partner Web API with the
    /// publisher key as `builds` finds it. `--save` writes them to a file instead, which
    /// `--check` then reads. With `--check`, it compares them by name with the files. It exits 2
    /// and names every leaderboard missing on either side and every setting that differs.
    ///
    /// With `--create`, leaderboards in the files that Steam lacks are made first, with the
    /// files' settings. With `--delete-missing`, leaderboards on Steam that the files no longer
    /// hold are deleted first, with their scores. That happens only while steamcmd's app info
    /// shows the build account the app unreleased. An existing leaderboard is never changed or
    /// made again.
    ///
    /// In a game's folder, `--check` with no files, `--create` or `--delete-missing` alone reads
    /// the games-kit records in `mods/base/data` holding leaderboards.
    Leaderboards(Leaderboards),
    /// Upload an app's rich presence localisation.
    ///
    /// Sends Valve's rich presence files, one per language, through Steam's partner Web API with
    /// the publisher key as `builds` finds it. Each language's tokens on Steam are replaced by the
    /// file's. A token taken out of the file is gone from Steam after the upload.
    ///
    /// Files are checked first, and nothing is sent while one is wrong. A file is refused for a
    /// token that does not start with `#`, or for a token listed twice. It is refused for a text
    /// that refers to `{#Prefix_%field%}` when no token starts with that prefix. It is refused
    /// for a text holding a slot Steam cannot fill: anything but `%field%` and
    /// `{#Token_%field%}`.
    ///
    /// With neither the app nor files, in a game's folder, it sends every
    /// `rich_presence_<language>.vdf` beside the game's build scripts. That is refused unless the
    /// tree is pushed, as for `upload`.
    #[command(name = "rich-presence")]
    RichPresence {
        /// The app, by its ID or its app build script. In a game's folder, the game's own.
        app: Option<String>,
        /// In a game's folder, send the full game's files to its demo.
        #[arg(long, conflicts_with = "app")]
        demo: bool,
        /// The rich presence files after the app, such as `steam/rich_presence_english.vdf`. In a
        /// game's folder, every one beside its build scripts.
        files: Vec<PathBuf>,
        /// Check the files and show what would be sent, without sending anything.
        #[arg(long)]
        dry_run: bool,
    },
    /// Write a game's store, Workshop and rich presence text in every language it speaks.
    ///
    /// In a game's folder, translates the English kept beside its build scripts with every
    /// gettext catalogue in its locale folder. It writes five kinds of file beside the scripts.
    ///
    /// `store/<language>.json` comes from `store_english.json` once that names the page's
    /// `itemid`. The store page's Localization tab uploads these, every language at once.
    ///
    /// `basic_info.json` holds each language's recommended system requirements, by their names on
    /// the Basic Info form. The Localization tab drops them.
    ///
    /// `ai_disclosure.json` is the content survey's statement of how AI was used, by language,
    /// from the `disclosure` in `store_english.json`.
    ///
    /// `workshop/<language>.json` is the Workshop page's title and description, from
    /// `workshop_english.json`.
    ///
    /// `rich_presence_<language>.vdf` comes from `presence_english.json`, or else from a
    /// `rich_presence_english.vdf` kept by hand. `rich-presence` sends it.
    ///
    /// A catalogue translates the English under the context `store page`, `steam workshop` or
    /// `rich presence`. Anything it leaves untranslated or marks fuzzy is refused, and then nothing
    /// is written. The same goes for a short description past the 300 characters the store page
    /// shows, or a Workshop description past the 200 it takes. A file of these kinds that would no
    /// longer be written is removed.
    Text(Text),
    /// Set an uploaded build live on a branch.
    ///
    /// Nothing is uploaded again. It goes through Steam's partner Web API, with the publisher key
    /// as `builds` finds it. The default branch is set live only while Steam shows the build
    /// account the app unreleased. Once the app is released, set its default branch live in
    /// Steamworks instead.
    Promote(Promote),
    /// Set a branch back to its previous build.
    ///
    /// Players on the branch then update to that build. It sets live the build steamship recorded
    /// on the branch before the current one, through Steam's partner Web API, with the publisher
    /// key as `builds` finds it. Uploads from a game record the build they set live on each
    /// branch, in steamship's home. So this works only on the machine that uploaded. A branch with no earlier recorded build is refused. The
    /// default branch is set live only while Steam shows the build account the app unreleased.
    Rollback(Rollback),
    /// Set the description players see for a beta branch.
    ///
    /// Steam shows this text where players choose a beta branch, in the game's Betas properties.
    /// It is set through Steam's partner Web API, with the publisher key as `builds` finds it.
    /// The default branch's description is set in Steamworks only.
    #[command(allow_missing_positional = true)]
    Branch {
        /// The app, by its ID or its app build script. In a game's folder, the game's own.
        app: Option<String>,
        /// The beta branch.
        branch: String,
        /// Its new description, such as the version live on it.
        #[arg(long)]
        description: String,
    },
    /// Set up uploads from CI, with the login as a secret.
    ///
    /// Finds the GitHub repository, its app build script and the saved login. It checks the
    /// script, and checks the login with Steam. It then sets the login as a secret with the GitHub
    /// command line, `gh`, and shows the workflow step that uploads. It asks for anything it
    /// cannot find and an option did not give. The secret holds the token steamcmd saved, never a
    /// password.
    Ci {
        /// The app, by its ID or its app build script, if not the one found in the repository.
        app: Option<String>,
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
    /// Removes the login steamcmd saved, the account steamship remembered and the kept Web API
    /// key. The next upload needs `steamship login` first.
    Logout,
    /// Write starter build scripts for an app.
    ///
    /// Writes a commented app build script, and a depot build script for each depot, into
    /// `--output`. They are in Valve's own format, for `check` and `upload` to read. Without
    /// `--depot`, there is one depot. Its ID is the app's ID plus one. It ships `build/`. Nothing already
    /// there is overwritten.
    Init {
        /// The app's ID, from Steamworks.
        #[arg(value_parser = app_id)]
        app: u32,
        /// A depot and the folder its files ship from, such as 1001=build/windows. The folder is
        /// relative to where `init` runs, not to `--output`. Give it once for each depot.
        #[arg(long = "depot", value_name = "ID=FOLDER", value_parser = init::depot)]
        depots: Vec<init::Depot>,
        /// The folder the scripts are written into.
        #[arg(long, default_value = "steam")]
        output: PathBuf,
    },
    /// Install or verify the pinned steamcmd.
    ///
    /// `login` and `upload` install it themselves. Run this to do it ahead of time, as in CI.
    Install,
    /// List Steam's languages with their locale codes.
    ///
    /// Lists each locale steamship reads a catalogue for with Steam's name for its language, such
    /// as `pt-BR` and `brazilian`. Steam's name is the API language code that names its files. A locale is matched regardless of case, with `_` read as `-`.
    Languages {
        /// Print the list as one JSON object, from each locale to Steam's name for its language.
        #[arg(long)]
        json: bool,
    },
    /// Check this steamship binary.
    #[command(name = "self", subcommand)]
    Itself(Itself),
    #[command(
        about = "Print tab completion for bash, zsh, fish, PowerShell or elvish",
        long_about = concat!(
            "Print tab completion for bash, zsh, fish, PowerShell or elvish.\n",
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

/// The settings `settings` shows, saves or checks, and against which file.
#[derive(Debug, Clone, Args)]
#[command(group(clap::ArgGroup::new("file_switches").args(["check", "save"])))]
pub struct Settings {
    /// The app, by its ID or its app build script. In a game's folder, the game's own.
    pub app: Option<String>,
    /// The snapshot file, after the app. In a game's folder, `settings.vdf` beside its build
    /// scripts.
    #[arg(value_name = "FILE", requires = "file_switches")]
    pub file: Option<PathBuf>,
    /// In a game's folder, uses the demo and its own snapshot beside the build scripts. Takes no
    /// app or file. To name a file, give the demo's ID as the app instead.
    #[arg(long, conflicts_with = "app")]
    pub demo: bool,
    /// Write the settings to the snapshot file, replacing it, to check against later. Refused,
    /// exiting 2, for an install folder of anything but letters and digits.
    #[arg(long, conflicts_with = "check")]
    pub save: bool,
    /// Compare the settings with the snapshot file, as `--save` wrote it. An install folder of
    /// anything but letters and digits is a difference.
    #[arg(long)]
    pub check: bool,
    /// The build account, if not the one the last login remembered.
    #[arg(long, env = ACCOUNT)]
    pub account: Option<String>,
}

/// The leaderboards `leaderboards` lists, checks, makes or deletes.
#[derive(Debug, Clone, Args)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "each is a switch of its own on the command line, which clap reads into a field"
)]
#[command(group(
    clap::ArgGroup::new("file_switches")
        .args(["check", "create", "delete_missing"])
        .multiple(true)
))]
pub struct Leaderboards {
    /// The app, by its ID or its app build script. In a game's folder, the game's own.
    pub app: Option<String>,
    /// JSON files of the leaderboards the app should have, each with its `name`, `sort`,
    /// `display`, `trusted_writes`, `friends_only` and `period`. A file may instead be a list of
    /// games-kit records, the JSON records a game keeps its content in. Those typed `leaderboard`
    /// are read by the name in `steam`. Each leaderboard is in one file only. Files follow the
    /// app. `--check`, `--create` or `--delete-missing` says what to do with them.
    #[arg(value_name = "FILES", requires = "file_switches")]
    pub files: Vec<PathBuf>,
    /// Compare the leaderboards Steam holds with the files.
    #[arg(long, overrides_with = "check")]
    pub check: bool,
    /// Write the leaderboards Steam holds to this JSON file, as `--check` reads it. Refused,
    /// exiting 2, when a leaderboard has no display type, which a file cannot give. Refused too
    /// when the file is there and holds more than `--save` writes, such as a `period`.
    #[arg(long, value_name = "FILE", conflicts_with_all = ["file_switches", "files"])]
    pub save: Option<PathBuf>,
    /// Make the leaderboards in the files that Steam lacks, before checking.
    #[arg(long)]
    pub create: bool,
    /// Delete the leaderboards on Steam that the files do not hold, before checking. Refused
    /// once the app is released, as their players' scores would go with them.
    #[arg(long)]
    pub delete_missing: bool,
    /// Print the leaderboards on standard output as one line of JSON, or what Steam failed with
    /// as an `error` holding a `code`. Everything else goes to standard error.
    #[arg(long, conflicts_with_all = ["check", "save", "create", "delete_missing"])]
    pub json: bool,
    /// The build account that asks Steam whether the app is released, if not the one the last
    /// login remembered.
    #[arg(long, env = ACCOUNT)]
    pub account: Option<String>,
}

/// What `text` writes, checks or prints.
#[derive(Debug, Clone, Copy, Args)]
pub struct Text {
    /// Write nothing, and exit 2 naming each file that differs from what would be written, is
    /// missing, or is left over.
    #[arg(long, conflicts_with = "steam")]
    pub unchanged: bool,
    /// Print every English text Steam shows as a gettext template, for a game's own template to
    /// take in. Nothing is written beside the scripts. Each text is under the context the
    /// catalogues translate it by, with a translator note and `max-length:N` where Steam's room
    /// for it is short. Texts that records or other files supply are left out. The rest are under
    /// `steam achievements`.
    #[arg(long, conflicts_with_all = ["unchanged", "steam"])]
    pub template: bool,
    /// Write nothing. Exit 2 and name each minimum or recommended system requirement that the
    /// game's public store page does not show as given. The page is compared in each language
    /// with `store_english.json` and its translations. Steam shows requirements only once the
    /// page is public, Coming Soon included. Memory, storage, the DirectX version and VR support
    /// are not compared.
    #[arg(long)]
    pub steam: bool,
}

/// What `self` does to steamship itself.
#[derive(Debug, Clone, Copy, Subcommand)]
pub enum Itself {
    /// Check that this steamship is its release's own binary, byte for byte.
    ///
    /// Reads the SPDX SBOM published with this version's GitHub release. It lists the SHA-256 of
    /// each system's binary. This binary's is compared with the one built for this system. A
    /// binary that differs was built elsewhere or altered since. Install the release's own binary, from its archive or with
    /// `cargo binstall`. A build from source, such as `cargo install`, differs from every
    /// release's.
    Check,
}

/// What `achievements` writes for Steamworks' achievement editor, from what, and where.
#[derive(Debug, Args)]
pub struct Writing {
    /// Write the achievement editor's import files from the achievement files, read as `--check` reads them. In a
    /// game's folder, with no files, write from the game's own into `steam/achievements`.
    ///
    /// The uploading page of the docs lists the fields an achievement and a set may give. It also
    /// says how a catalogue's texts are found: <https://aureliolo.github.io/steamship/uploading.html>.
    #[arg(long, overrides_with = "write", conflicts_with_all = ["check", "json"])]
    pub write: bool,
    /// The folder `--write` writes into, made if missing. Files of the kinds it writes for the app
    /// that it would no longer write are removed. Other files, `steam-icons.json` among them, are
    /// left alone. With `--check`, the folder whose `steam-icons.json` the icons are checked
    /// against.
    #[arg(long, value_name = "FOLDER", requires = "file_switches")]
    pub output: Option<PathBuf>,
    /// Write nothing, and exit 2 naming each file in the folder that differs from what `--write`
    /// would write there, is missing, or is left over. `steam-icons.json` is none of these.
    #[arg(long, requires = "write")]
    pub unchanged: bool,
    /// Write only `steam-icons.json` into the folder, from the icons Steam holds once the
    /// folder's files are imported and published. In a game's folder it needs no `--write`.
    ///
    /// It is refused, exiting 2, when the folder is not as `--write` would write it. It is
    /// refused when Steam lacks an achievement or icon the files have, or has an icon for an
    /// achievement the files give none. It is refused when an icon Steam serves is not its file.
    /// That is when more than one pixel in a thousand is more than 64 of 255 off in a channel.
    #[arg(long, conflicts_with_all = ["unchanged", "check", "json"])]
    pub imported: bool,
}

/// What `upload` uploads, and how.
#[derive(Debug, Args)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "each is a switch of its own on the command line, which clap reads into a field"
)]
pub struct Upload {
    /// The app, by its ID or its app build script, such as `steam/app_build.vdf`. In a game's
    /// folder, the game's own.
    pub app: Option<String>,
    /// The version being shipped. With the commit, it becomes the build's description. In a
    /// game's folder, the newest release `CHANGES.md` heads, or else `project.godot`'s version. A
    /// version of all zeros, such as 0.0.0, is refused.
    #[arg(long)]
    pub version: Option<String>,
    /// Compute and log the whole build without uploading anything, as Valve's dry run.
    #[arg(long)]
    pub dry_run: bool,
    /// Also set the build live on the default branch, while Steam shows the app unreleased. This
    /// goes through Steam's partner Web API, with the publisher key as `builds` finds it. Once
    /// the app is released, the build goes up without this. Set its default branch live in
    /// Steamworks then. In a game's folder, an upload does this without the flag.
    #[arg(long, conflicts_with = "dry_run")]
    pub default: bool,
    /// In a game's folder, ship the demo, a separate app, instead of the full game. It is built
    /// with the demo's `build` command. Its depots are kept apart from the full game's and ship
    /// only the Godot exports made with the demo's feature. It is checked against the demo's own
    /// settings snapshot, which must show it typed Demo with the full game as its base game.
    #[arg(long, conflicts_with = "app")]
    pub demo: bool,
    /// The build account, if not the one the last login remembered.
    #[arg(long, env = ACCOUNT)]
    pub account: Option<String>,
    /// Once steamcmd has run, print what came of it on standard output as one line of JSON:
    /// `outcome`, `app`, `build_id`, `branch`, `live`, `default` and `description`. Everything
    /// else goes to standard error.
    #[arg(long)]
    pub json: bool,
    /// Upload a commit that is not on the game's `green` branch, such as a hotfix.
    #[arg(long)]
    pub allow_ungreen: bool,
}

/// The build `fetch` downloads, and where to.
#[derive(Debug, Args)]
pub struct Fetch {
    /// The app, by its ID or its app build script. In a game's folder, the game's own.
    pub app: Option<String>,
    /// In a game's folder, its demo.
    #[arg(long, conflicts_with = "app")]
    pub demo: bool,
    /// The branch whose live build is fetched. The default branch, unless `--build` names a build
    /// live on another.
    #[arg(long)]
    pub branch: Option<String>,
    /// The build, by its build ID. Refused unless it is live on `--branch`, or without
    /// `--branch`, on some branch.
    #[arg(long, value_parser = build_id)]
    pub build: Option<u64>,
    /// The system whose depots are fetched: windows, linux or macos. This system's by default.
    #[arg(long, value_parser = fetch::system)]
    pub system: Option<Platform>,
    /// The folder to fetch the build into. It holds this one branch and system alone.
    #[arg(long, value_name = "FOLDER")]
    pub output: Option<PathBuf>,
    /// The build account, if not the one the last login remembered.
    #[arg(long, env = ACCOUNT)]
    pub account: Option<String>,
    #[command(flatten)]
    pub password: BranchPassword,
    /// Once the build is fetched, print `app`, `branch`, `build_id`, `system` and `folder` on
    /// standard output as one line of JSON. Everything else goes to standard error.
    #[arg(long)]
    pub json: bool,
}

/// How `fetch` handles the password of a branch that takes one.
#[derive(Debug, Clone, Copy, Args)]
pub struct BranchPassword {
    /// Give steamcmd the branch's password on its command line, not typed into its console. Any
    /// program on this machine can read it there while the fetch runs.
    #[arg(long = "password-on-command-line")]
    pub on_command_line: bool,
    /// Keep the branch's password, from `STEAMSHIP_BRANCH_PASSWORD` or typed, in the system's
    /// credential store without asking, once Steam has accepted it.
    #[arg(long = "keep-password")]
    pub keep: bool,
}

/// The build `promote` sets live, and where.
#[derive(Debug, Args)]
pub struct Promote {
    /// The app, by its ID or its app build script. In a game's folder, the game's own.
    pub app: Option<String>,
    /// In a game's folder, its demo.
    #[arg(long, conflicts_with = "app")]
    pub demo: bool,
    /// The build, by its build ID.
    #[arg(long, value_parser = build_id)]
    pub build: u64,
    /// The branch to set it live on.
    #[arg(long)]
    pub branch: String,
    /// The build account that asks Steam whether the app is released, for the default branch, if
    /// not the one the last login remembered.
    #[arg(long, env = ACCOUNT)]
    pub account: Option<String>,
}

/// The branch `rollback` sets back, and for which app.
#[derive(Debug, Args)]
#[command(allow_missing_positional = true)]
pub struct Rollback {
    /// The app, by its ID or its app build script. In a game's folder, the game's own.
    pub app: Option<String>,
    /// The branch to set back.
    pub branch: String,
    /// The build account that asks Steam whether the app is released, for the default branch, if
    /// not the one the last login remembered.
    #[arg(long, env = ACCOUNT)]
    pub account: Option<String>,
}

/// What `workshop` uploads, and how.
#[derive(Debug, Args)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "each is a switch of its own on the command line, which clap reads into a field"
)]
pub struct Workshop {
    /// The app, by its ID or its app build script, when the item is given by fields. Otherwise an
    /// item script such as `workshop/item.vdf`, or the game's own app.
    pub app: Option<String>,
    #[command(flatten)]
    pub item: Option<WorkshopItem>,
    /// The item's name in the `--items` file, such as `example`. In a game's folder, the one
    /// example mod to upload.
    #[arg(long)]
    pub name: Option<String>,
    /// Who may see the item: `public`, `friends-only`, `private` or `unlisted`. Needed to make a
    /// new item. Left out, an existing item keeps its visibility. In a game's folder, it applies
    /// only to the items being made.
    #[arg(long, value_parser = workshop::visibility)]
    pub visibility: Option<workshop::Visibility>,
    /// The build account, if not the one the last login remembered.
    #[arg(long, env = ACCOUNT)]
    pub account: Option<String>,
    /// Make a new item from CI, which is otherwise refused there.
    #[arg(long)]
    pub new: bool,
    /// Upload an item while Steam still shows the game unreleased, though `steamship.toml` holds
    /// its Workshop items until launch.
    #[arg(long)]
    pub for_launch: bool,
    /// Check each item and upload nothing.
    #[arg(long)]
    pub dry_run: bool,
    /// Print what came of each item on standard output as one line of JSON: `outcome`, `app`,
    /// `item`, `name` and `log`. Everything else goes to standard error.
    #[arg(long)]
    pub json: bool,
}

/// The fields a Workshop item is given by, after its app, from which steamship writes its
/// `workshopitem` script.
///
/// One clap group, so a command line holds none of them or `--items`, `--name`, `--content` and
/// `--title` at the least.
#[derive(Debug, Args)]
pub struct WorkshopItem {
    /// The file that keeps the ID of each item made, by the item's name, such as
    /// `steam/workshop_items.json`. Keep it with the game. A run on any machine then updates the
    /// item an earlier run made. steamship writes it and makes it if missing.
    #[arg(
        long,
        value_name = "FILE",
        required = false,
        requires_all = ["name", "content", "title"]
    )]
    pub items: PathBuf,
    /// The folder uploaded as the item's content.
    #[arg(long, value_name = "FOLDER", required = false, requires = "items")]
    pub content: PathBuf,
    /// The item's preview image, 1 MB at most.
    #[arg(long, value_name = "IMAGE", requires = "items")]
    pub preview_image: Option<PathBuf>,
    /// The item's title, 128 bytes at most.
    #[arg(long, required = false, requires = "items")]
    pub title: String,
    /// The item's description, 8000 bytes at most.
    #[arg(long, value_name = "TEXT", requires = "items")]
    pub description: Option<String>,
    /// What changed in this upload, 8000 bytes at most, shown in the item's change notes.
    #[arg(long, value_name = "TEXT", requires = "items")]
    pub change_note: Option<String>,
}

#[warn(
    unused_results,
    reason = "the file expects them of clap's derive, which writes none here"
)]
#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};
    use std::fs;
    use std::path::Path;

    use clap::CommandFactory as _;
    use clap::error::{Error, ErrorKind};

    use super::*;

    #[test]
    fn the_command_line_is_one_clap_accepts() {
        Cli::command().debug_assert();
    }

    /// Every long flag and alias of `command` and of each subcommand under it.
    fn long_flags(command: &clap::Command, found: &mut Vec<String>) {
        for argument in command.get_arguments() {
            found.extend(argument.get_long().map(str::to_owned));
            found.extend(
                argument
                    .get_all_aliases()
                    .into_iter()
                    .flatten()
                    .map(str::to_owned),
            );
        }
        for subcommand in command.get_subcommands() {
            long_flags(subcommand, found);
        }
    }

    #[test]
    fn no_command_takes_a_1_0_flag_name() {
        let mut found = Vec::new();
        long_flags(&Cli::command(), &mut found);
        for old in ["preview", "out", "into", "folder", "web-api-key"] {
            assert!(!found.iter().any(|flag| flag == old), "--{old} is taken");
        }
    }

    #[test]
    fn no_command_names_the_app_by_a_flag() {
        let mut found = Vec::new();
        long_flags(&Cli::command(), &mut found);
        for flag in ["app", "script"] {
            assert!(!found.iter().any(|each| each == flag), "--{flag} is taken");
        }
    }

    #[test]
    fn a_dry_run_is_one_switch_on_every_command_that_sends() {
        let upload = Cli::try_parse_from(["steamship", "upload", "--dry-run"]).unwrap();
        assert!(matches!(
            upload.command,
            Command::Upload(Upload { dry_run: true, .. })
        ));
        let presence =
            Cli::try_parse_from(["steamship", "rich-presence", "480", "f.vdf", "--dry-run"])
                .unwrap();
        assert!(matches!(
            presence.command,
            Command::RichPresence { dry_run: true, .. }
        ));
    }

    #[test]
    fn every_folder_or_file_a_command_writes_to_is_output() {
        for line in [
            &["achievements", "480", "--write", "a.json", "--output", "o"][..],
            &["fetch", "480", "--output", "o"],
            &["init", "480", "--output", "o"],
            &["ci", "--output", "f"],
            &["drm-wrap", "480", "g.exe", "--output", "f"],
        ] {
            let mut args = vec!["steamship"];
            args.extend(line);
            let parsed = Cli::try_parse_from(&args);
            assert!(parsed.is_ok(), "{line:?}: {:?}", parsed.err());
        }
    }

    fn parsed(line: &[&str]) -> Command {
        let mut args = vec!["steamship"];
        args.extend(line);
        Cli::try_parse_from(&args)
            .unwrap_or_else(|error| panic!("{line:?}: {error}"))
            .command
    }

    fn refused(line: &[&str]) -> Error {
        let mut args = vec!["steamship"];
        args.extend(line);
        Cli::try_parse_from(&args).expect_err(&format!("{line:?} parsed"))
    }

    #[test]
    fn the_app_after_a_switch_is_still_the_app() {
        for line in [
            &["achievements", "--check", "480"][..],
            &["achievements", "--write", "480", "--output", "o"],
        ] {
            let Command::Achievements { app, files, .. } = parsed(line) else {
                panic!("{line:?}");
            };
            assert_eq!((app.as_deref(), files.len()), (Some("480"), 0), "{line:?}");
        }
        let Command::Leaderboards(boards) = parsed(&["leaderboards", "--check", "480"]) else {
            panic!("leaderboards");
        };
        assert_eq!(
            (boards.app.as_deref(), boards.files.len()),
            (Some("480"), 0)
        );
        for line in [
            &["settings", "--check", "480"][..],
            &["settings", "--save", "480"],
        ] {
            let Command::Settings(settings) = parsed(line) else {
                panic!("{line:?}");
            };
            assert_eq!(
                (settings.app.as_deref(), settings.file),
                (Some("480"), None),
                "{line:?}"
            );
        }
    }

    #[test]
    fn files_follow_the_app_on_either_side_of_the_switch() {
        for line in [
            &["achievements", "480", "a.json", "de.po", "--check"][..],
            &["achievements", "480", "--check", "a.json", "de.po"],
        ] {
            let Command::Achievements {
                app, files, check, ..
            } = parsed(line)
            else {
                panic!("{line:?}");
            };
            assert_eq!(app.as_deref(), Some("480"), "{line:?}");
            assert_eq!(
                files,
                [PathBuf::from("a.json"), PathBuf::from("de.po")],
                "{line:?}"
            );
            assert!(check, "{line:?}");
        }
    }

    #[test]
    fn files_need_the_switch_that_takes_them() {
        for line in [
            &["achievements", "480", "a.json"][..],
            &["leaderboards", "480", "a.json"],
            &["settings", "480", "kept.vdf"],
            &["achievements", "--output", "o"],
        ] {
            assert_eq!(
                refused(line).kind(),
                ErrorKind::MissingRequiredArgument,
                "{line:?}"
            );
        }
        let Command::Leaderboards(request) = parsed(&["leaderboards", "480", "a.json", "--create"])
        else {
            panic!("leaderboards");
        };
        assert_eq!(request.files, [PathBuf::from("a.json")]);
        assert!(request.create);
    }

    #[test]
    fn save_refuses_a_file_switch_and_files() {
        for line in [
            &["achievements", "480", "--check", "--save", "a.json"][..],
            &["achievements", "480", "--write", "--save", "a.json"],
            &["achievements", "480", "a.json", "--save", "b.json"],
            &["leaderboards", "480", "--check", "--save", "b.json"],
        ] {
            assert_eq!(
                refused(line).kind(),
                ErrorKind::ArgumentConflict,
                "{line:?}"
            );
        }
    }

    #[test]
    fn settings_demo_takes_no_file() {
        let Command::Settings(settings) = parsed(&["settings", "--demo", "--check"]) else {
            panic!("settings");
        };
        assert!(
            settings.demo && settings.check && settings.app.is_none() && settings.file.is_none()
        );
        let error = refused(&["settings", "--demo", "kept.vdf", "--check"]);
        assert_eq!(error.kind(), ErrorKind::ArgumentConflict);
        let said = error.to_string();
        assert!(
            said.contains("'--demo'") && said.contains("[APP]"),
            "{said}"
        );
    }

    #[test]
    fn a_lone_word_is_the_branch_or_the_executable_and_never_the_app() {
        let Command::Rollback(alone) = parsed(&["rollback", "testing"]) else {
            panic!("rollback");
        };
        assert_eq!((alone.app, alone.branch.as_str()), (None, "testing"));
        let Command::Rollback(both) = parsed(&["rollback", "480", "testing"]) else {
            panic!("rollback");
        };
        assert_eq!(
            (both.app.as_deref(), both.branch.as_str()),
            (Some("480"), "testing")
        );
        let Command::Branch { app, branch, .. } = parsed(&["branch", "beta", "--description", "d"])
        else {
            panic!("branch");
        };
        assert_eq!((app, branch.as_str()), (None, "beta"));
        let Command::DrmWrap {
            app: wrapped_app,
            executable,
            ..
        } = parsed(&["drm-wrap", "game.exe"])
        else {
            panic!("drm-wrap");
        };
        assert_eq!((wrapped_app, executable), (None, PathBuf::from("game.exe")));
        assert_eq!(
            refused(&["rollback"]).kind(),
            ErrorKind::MissingRequiredArgument
        );
    }

    #[test]
    fn ci_takes_its_script_as_the_first_word() {
        let Command::Ci { app, output, .. } =
            parsed(&["ci", "steam/app_build.vdf", "--output", "f"])
        else {
            panic!("ci");
        };
        assert_eq!(
            (app.as_deref(), output),
            (Some("steam/app_build.vdf"), Some(PathBuf::from("f")))
        );
        assert_eq!(
            refused(&["ci", "--script", "steam/app_build.vdf"]).kind(),
            ErrorKind::UnknownArgument
        );
    }

    #[test]
    fn every_exit_code_is_listed_once_in_order() {
        let codes: Vec<u8> = EXIT_CODES.iter().map(|&(code, _)| code).collect();
        assert_eq!(
            codes,
            [0, FAILED, REFUSED, LOGIN, STEAMCMD, UNFINISHED, LIMITED]
        );
        assert_eq!(codes, [0, 1, 2, 3, 4, 5, 6], "each code keeps its number");
    }

    #[test]
    fn exits_1_and_4_say_steam_holds_nothing_to_make_again_and_5_that_it_would() {
        let row = |code: u8| {
            EXIT_CODES
                .iter()
                .find(|&&(number, _)| number == code)
                .unwrap()
                .1
        };
        let nothing = "A second run makes no duplicate";
        assert!(row(FAILED).contains(nothing));
        assert!(row(STEAMCMD).contains(nothing));
        assert_eq!(
            row(FAILED),
            "The command failed and printed why. A second run makes no duplicate"
        );
        assert_eq!(
            row(UNFINISHED),
            "Steam holds a build or Workshop item that a second run would duplicate. The output says what to do"
        );
        for (_, words) in EXIT_CODES {
            for old in [
                "Nothing new is on Steam",
                "Nothing was made",
                "not live or not kept",
                "logs' paths",
            ] {
                assert!(!words.contains(old), "{words:?} still holds {old:?}");
            }
        }
    }

    /// Not names of variables, though they are in capitals in a file that reads some.
    const NOT_VARIABLES: [&str; 2] = [
        // The checksum algorithm steamship looks for in an SBOM.
        "SHA256", // What an answer that is not JSON is called in an error.
        "JSON",
    ];

    /// Names read only in a debug build or a test build, with a text that guards each one in its
    /// file, so a release build cannot reach them.
    const NOT_IN_RELEASE: [(&str, &str); 4] = [
        // A stand-in host is taken only where `webapi::is_stand_in` allows it, and the test below
        // holds that rule to a debug build.
        ("STEAMSHIP_WEB_API_STAND_IN", "is_stand_in"),
        ("STEAMSHIP_RELEASES_STAND_IN", "is_stand_in"),
        // The fake steamcmd is compiled only when the tests turn its feature on.
        ("STEAMSHIP_FAKE_STEAMCMD", "feature = \"fake-steamcmd\""),
        // Microsoft's console is taken from this folder only when the tests turn the feature on.
        ("STEAMSHIP_TEST_CONPTY", "feature = \"fake-steamcmd\""),
    ];

    /// Each source file's name without its extension and its code up to the test module, without
    /// comment lines. The binary's modules under `src/commands/` read variables too.
    fn sources() -> Vec<(String, String)> {
        let mut folders = vec![Path::new(env!("CARGO_MANIFEST_DIR")).join("src")];
        let mut paths = Vec::new();
        while let Some(folder) = folders.pop() {
            for entry in fs::read_dir(folder).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    folders.push(path);
                } else if path.extension().is_some_and(|extension| extension == "rs") {
                    paths.push(path);
                } else {
                    // Anything else in `src/` is no source file.
                }
            }
        }
        let mut found = Vec::new();
        for path in paths {
            let text = fs::read_to_string(&path).unwrap();
            let mut code = String::new();
            let mut lines = text.lines();
            while let Some(line) = lines.next() {
                if line.trim() == "#[cfg(test)]" {
                    // Code after a test-only item that is not the test module would go unread.
                    let next = lines.next().unwrap_or_default();
                    assert!(
                        next.trim_start().starts_with("mod tests"),
                        "{path:?}: {next}"
                    );
                    break;
                }
                if !line.trim_start().starts_with("//") {
                    code.push_str(line);
                    code.push('\n');
                }
            }
            found.push((
                path.file_stem().unwrap().to_string_lossy().into_owned(),
                code,
            ));
        }
        found
    }

    fn is_name_char(character: char) -> bool {
        character.is_ascii_uppercase() || character.is_ascii_digit() || character == '_'
    }

    /// The string literals on `line` that are one name in capitals.
    fn capitals(line: &str) -> Vec<&str> {
        let mut found = Vec::new();
        for (at, _) in line.match_indices('"') {
            let rest = line.get(at.saturating_add(1)..).unwrap_or_default();
            let name = rest
                .split(|character| !is_name_char(character))
                .next()
                .unwrap_or_default();
            let closed = rest
                .get(name.len()..)
                .is_some_and(|after| after.starts_with('"'));
            // `env!` is read when steamship is built and names no variable of the one that runs.
            let at_build = line
                .get(..at)
                .is_some_and(|before| before.ends_with("env!("));
            if closed
                && !at_build
                && name.len() > 1
                && !name.starts_with(|character: char| character.is_ascii_digit())
            {
                found.push(name);
            }
        }
        found
    }

    /// The text of a `const NAME: &str = "text";` per file and name.
    fn constants(sources: &[(String, String)]) -> BTreeMap<(String, String), String> {
        let mut found = BTreeMap::new();
        for (file, code) in sources {
            for line in code.lines() {
                let line = line.trim_start().trim_start_matches("pub ");
                let Some((name, rest)) = line
                    .strip_prefix("const ")
                    .and_then(|line| line.split_once(": &str = \""))
                else {
                    continue;
                };
                if let Some((text, _)) = rest.split_once('"') {
                    drop(found.insert((file.clone(), name.to_owned()), text.to_owned()));
                }
            }
        }
        found
    }

    /// The name an argument gives to a read: a literal, or a constant by its path or, in the
    /// file that defines it, by its name. `None` for the closure parameter that forwards a name
    /// some other code chose.
    fn named(
        argument: &str,
        file: &str,
        constants: &BTreeMap<(String, String), String>,
    ) -> Option<String> {
        if argument == "name" {
            return None;
        }
        if let Some(text) = argument
            .strip_prefix('"')
            .and_then(|rest| rest.strip_suffix('"'))
        {
            return Some(text.to_owned());
        }
        let mut path = argument.rsplit("::");
        let constant = path.next().unwrap_or_default();
        let module = path.next().unwrap_or(file);
        let text = constants.get(&(module.to_owned(), constant.to_owned()));
        Some(
            text.unwrap_or_else(|| {
                panic!("{file}: a read of `{argument}` names no literal or constant")
            })
            .clone(),
        )
    }

    /// Every variable named in the code of `file`.
    fn reads(
        file: &str,
        code: &str,
        constants: &BTreeMap<(String, String), String>,
    ) -> BTreeSet<String> {
        let mut found = BTreeSet::new();
        assert!(
            !code.contains("getenv"),
            "{file}: the C library's environment is not scanned"
        );
        let takes_lookup = code.contains("Fn(&str) -> Option<");
        for line in code.lines() {
            if takes_lookup {
                // Names reach an injected lookup through helpers and tuples, so every literal in
                // capitals counts.
                found.extend(
                    capitals(line)
                        .into_iter()
                        .filter(|name| !NOT_VARIABLES.contains(name))
                        .map(str::to_owned),
                );
            }
            if let Some((_, rest)) = line.split_once("env = ") {
                let constant = rest
                    .split(|character| !is_name_char(character))
                    .next()
                    .unwrap_or_default();
                found.extend(named(constant, file, constants));
            }
            for (at, _) in line.match_indices("var") {
                let word: String = line
                    .get(at..)
                    .unwrap_or_default()
                    .chars()
                    .take_while(|&character| character.is_ascii_alphanumeric() || character == '_')
                    .collect();
                let before = line.get(..at).unwrap_or_default();
                if !["var", "var_os", "vars", "vars_os"].contains(&word.as_str())
                    || before.ends_with(|character: char| {
                        character.is_ascii_alphanumeric() || character == '_'
                    })
                {
                    continue;
                }
                assert!(before.ends_with("env::"), "{file}: {line}");
                let after = line
                    .get(at.saturating_add(word.len())..)
                    .unwrap_or_default();
                if word.starts_with("vars") {
                    // The whole environment is copied into a child's, and no name is read.
                    assert_eq!(file, "windows", "{file}: {line}");
                    continue;
                }
                let Some((argument, _)) = after
                    .strip_prefix('(')
                    .and_then(|call| call.split_once(')'))
                else {
                    panic!("{file}: {line}");
                };
                found.extend(named(argument, file, constants));
            }
        }
        found
    }

    #[test]
    fn every_variable_steamship_reads_by_name_is_listed_and_only_a_branch_has_a_password() {
        let sources = sources();
        let constants = constants(&sources);
        let mut read = BTreeSet::new();
        for (file, code) in &sources {
            read.extend(reads(file, code, &constants));
        }
        for (name, guard) in NOT_IN_RELEASE {
            let files: Vec<&String> = sources
                .iter()
                .filter(|(_, code)| code.contains(&format!("\"{name}\"")))
                .map(|(_, code)| code)
                .collect();
            assert!(!files.is_empty(), "{name} is read nowhere");
            assert!(
                files.iter().all(|code| code.contains(guard)),
                "{name} is not guarded by {guard}"
            );
            assert!(read.remove(name), "{name} is not read");
        }
        let (_, webapi) = sources.iter().find(|(file, _)| file == "webapi").unwrap();
        let (_, rule) = webapi.split_once("fn is_stand_in").unwrap();
        let (rule, _) = rule.split_once("\n}").unwrap();
        assert!(
            rule.contains("cfg!(debug_assertions)"),
            "is_stand_in allows a stand-in host outside a debug build"
        );
        let listed: BTreeSet<String> = VARIABLES.iter().map(|&(name, _)| name.to_owned()).collect();
        assert_eq!(
            read.difference(&listed).collect::<Vec<_>>(),
            Vec::<&String>::new(),
            "read in release code and missing from VARIABLES"
        );
        assert_eq!(
            listed.difference(&read).collect::<Vec<_>>(),
            Vec::<&String>::new(),
            "listed in VARIABLES and read nowhere"
        );
        let passwords: Vec<&str> = VARIABLES
            .iter()
            .map(|&(name, _)| name)
            .filter(|name| name.contains("PASSWORD"))
            .collect();
        assert_eq!(passwords, ["STEAMSHIP_BRANCH_PASSWORD"]);
    }
}
