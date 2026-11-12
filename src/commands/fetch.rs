//! `steamship fetch`: a branch's build downloaded into a folder of its own.

use std::env;
use std::fmt::Display;
use std::fs;
use std::io;
use std::path::{self, Path, PathBuf};
use std::process::ExitCode;

use steamship::account::Account;
use steamship::cli::{self, FAILED, REFUSED};
use steamship::install;
use steamship::platform::Platform;
use steamship::redact::Redactor;
use steamship::show::{self, Spinner};
use steamship::typing::{self, Echo};
use steamship::{branch, dump, fetch, keychain, run, steamcmd, webapi};
use zeroize::Zeroizing;

use super::exit::fail;
use super::game::{app_named, app_or_game};
use super::key::{offer_to_keep, web_api, web_api_failed, why_unanswered};
use super::ready::{
    account, home, login_refused, named, packed_login, ready, restore, steamcmd_failed,
};

pub(crate) fn try_fetch(request: &cli::Fetch) -> Result<ExitCode, ExitCode> {
    if request.json {
        show::divert();
    }
    show::title("fetch");
    let app = app_or_game(request.app.clone(), request.demo)?;
    let app_id = app_named(&app)?;
    let home = home()?;
    let branch = branch_to_fetch(&home, app_id, request)?;
    let shown = branch::shown_name(&branch.name);
    let system = request.system.unwrap_or(Platform::THIS);
    let given = request
        .into
        .clone()
        .unwrap_or_else(|| fetch::folder(&home, app_id, &branch.name, system));
    show::field("branch", shown);
    show::field("system", fetch::system_name(system));
    show::field("folder", &given.display().to_string());
    let folder = fetch_folder(&given, app_id, shown)?;
    let password = if branch.locked {
        Some(branch_password(
            &home,
            app_id,
            &branch.name,
            request.password.on_command_line,
        )?)
    } else {
        None
    };
    let held = fetched(&Fetching {
        app: app_id,
        branch: &branch,
        system,
        folder: &folder,
        named: named(request.account.as_deref()),
        password: password.as_ref(),
        on_command_line: request.password.on_command_line,
    })?;
    if let Some(password) = &password {
        keep_branch_password(&home, app_id, &branch.name, password, request.password.keep);
    }
    if held.build_id != branch.build_id || held.branch != shown {
        return Err(fail(
            &format!(
                "steamcmd put build {} of the {} branch in {}, where Steam's Web API showed build \
                 {} live on {shown}. If a build was set live meanwhile, fetch again",
                held.build_id,
                held.branch,
                given.display(),
                branch.build_id
            ),
            FAILED,
        ));
    }
    show::success(
        &format!(
            "app {app_id}: build {} of the {shown} branch is in {}",
            held.build_id,
            given.display()
        ),
        "",
    );
    if request.json {
        show::plain(&format!(
            "{}\n",
            fetch::json(app_id, &held, system, &folder)
        ));
    }
    Ok(ExitCode::SUCCESS)
}

/// The branch `request` asks to fetch of `app`, among the branches Steam's Web API shows.
fn branch_to_fetch(
    home: &Path,
    app: u32,
    request: &cli::Fetch,
) -> Result<webapi::Branch, ExitCode> {
    let (api, from) = web_api(home)?;
    let spinner = Spinner::start("steam", "asking for the branches", false);
    let branches = match api.branches(app) {
        Ok(branches) => branches,
        Err(error) => {
            let why = why_unanswered(&api, app, &error);
            let code = web_api_failed(spinner, &error);
            if let Some(why) = &why {
                show::note(why);
            }
            return Err(code);
        }
    };
    spinner.done(&match branches.len() {
        1 => "1 branch".to_owned(),
        count => format!("{count} branches"),
    });
    offer_to_keep(home, &api, from);
    fetch::chosen(&branches, app, request.branch.as_deref(), request.build)
        .cloned()
        .map_err(|why| fail(&why, REFUSED))
}

/// `folder` as a path from the root, made if it is not there. Refused when it holds `app`'s
/// build of another branch than `branch`, or files and no build of `app`, which steamcmd would
/// mix the build into.
fn fetch_folder(folder: &Path, app: u32, branch: &str) -> Result<PathBuf, ExitCode> {
    let refused = |why: &dyn Display| fail(&format!("{}: {why}", folder.display()), REFUSED);
    let record = fetch::manifest(folder, app);
    match fs::read_to_string(&record) {
        Ok(text) => {
            if let Ok(held) = fetch::installed(&text)
                && held.branch != branch
            {
                return Err(refused(&format!(
                    "holds app {app}'s {} branch. Fetch {branch} into a folder of its own",
                    held.branch
                )));
            }
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            if fs::read_dir(folder).is_ok_and(|mut entries| entries.next().is_some()) {
                return Err(refused(&format!(
                    "holds files and no build of app {app}. Fetch into an empty folder, or one a \
                     fetch of {branch} made"
                )));
            }
        }
        Err(error) => return Err(fail(&format!("{}: {error}", record.display()), FAILED)),
    }
    fs::create_dir_all(folder).map_err(|error| refused(&error))?;
    path::absolute(folder).map_err(|error| refused(&error))
}

/// Where the password of a branch being fetched came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PasswordFrom {
    Variable,
    Kept,
    Typed,
}

/// The password of a branch being fetched.
struct GivenPassword {
    text: Zeroizing<String>,
    from: PasswordFrom,
}

/// The password of `app`'s `branch`: the one in the variable, or else the one kept for it, or
/// else one typed at the prompt. Refused when none can be had, or when one bound for steamcmd's
/// console, rather than its command line, is one the console would split.
fn branch_password(
    home: &Path,
    app: u32,
    branch: &str,
    on_command_line: bool,
) -> Result<GivenPassword, ExitCode> {
    let shown = branch::shown_name(branch);
    let given = if let Some(text) = env::var_os(fetch::PASSWORD).filter(|text| !text.is_empty()) {
        let text = text
            .into_string()
            .map_err(|_not_text| fail(&format!("{} is not text", fetch::PASSWORD), REFUSED))?;
        show::field("password", &format!("from {}", fetch::PASSWORD));
        GivenPassword {
            text: Zeroizing::new(text),
            from: PasswordFrom::Variable,
        }
    } else if let Some(text) = kept_branch_password(home, app, branch) {
        show::field("password", &format!("kept in {}", keychain::STORE));
        GivenPassword {
            text,
            from: PasswordFrom::Kept,
        }
    } else {
        let needed = || {
            fail(
                &format!(
                    "app {app}'s {shown} branch takes a password. Set {} to it, or run fetch \
                     without --json and type it when asked",
                    fetch::PASSWORD
                ),
                REFUSED,
            )
        };
        // With --json, standard output holds the JSON alone, and the prompt would go there.
        if show::diverted() {
            return Err(needed());
        }
        show::aside(&format!(
            "app {app}'s {shown} branch takes a password, the one its Steamworks betas page shows"
        ));
        let text = typing::ask("password", Echo::Dots)
            .ok()
            .filter(|text| !text.is_empty())
            .ok_or_else(needed)?;
        GivenPassword {
            text,
            from: PasswordFrom::Typed,
        }
    };
    match (on_command_line, fetch::typeable(&given.text)) {
        (true, _) => show::aside(
            "the password goes on steamcmd's command line, where any program on this machine can \
             read it while the fetch runs",
        ),
        (false, true) => {}
        (false, false) => {
            return Err(fail(
                &"steamcmd's console splits a password at a space or a quote, so this one can \
                  only be given with --password-on-command-line",
                REFUSED,
            ));
        }
    }
    Ok(given)
}

/// The password kept for `app`'s `branch`, if one is and the store can be read. A store that
/// fails is named, and the password asked for instead.
fn kept_branch_password(home: &Path, app: u32, branch: &str) -> Option<Zeroizing<String>> {
    match keychain::branch_password(home, app, branch) {
        Ok(kept) => kept,
        Err(keychain::Error::Unavailable(_)) => None,
        Err(error) => {
            show::note(&error.to_string());
            None
        }
    }
}

/// Once Steam has taken `password` for `app`'s `branch`, keeps it: one given by the variable or
/// typed when `asked`, and a typed one too when Enter answers the offer.
fn keep_branch_password(
    home: &Path,
    app: u32,
    branch: &str,
    password: &GivenPassword,
    asked: bool,
) {
    let keep = match password.from {
        PasswordFrom::Kept => false,
        PasswordFrom::Variable => asked,
        PasswordFrom::Typed => asked || keeping_offered(),
    };
    if !keep {
        return;
    }
    match keychain::keep_branch_password(home, app, branch, &password.text) {
        Ok(()) => show::done("password", &format!("kept in {}", keychain::STORE)),
        Err(error) => {
            show::failed("password", "not kept");
            show::failure(&error.to_string(), "", None);
        }
    }
}

/// Offers to keep a typed password, and answers whether Enter, or a yes, took the offer.
fn keeping_offered() -> bool {
    show::aside(&format!(
        "Enter keeps the password in {} for next time, and n does not",
        keychain::STORE
    ));
    let keep = typing::ask("keep it", Echo::Typed).is_ok_and(|answer| {
        matches!(
            answer.trim().to_ascii_lowercase().as_str(),
            "" | "y" | "yes"
        )
    });
    if !keep {
        show::field("password", "not kept");
    }
    keep
}

/// What `fetched` downloads, and as whom.
struct Fetching<'command> {
    app: u32,
    branch: &'command webapi::Branch,
    system: Platform,
    folder: &'command Path,
    named: Option<&'command str>,
    password: Option<&'command GivenPassword>,
    on_command_line: bool,
}

/// Has steamcmd download the build, and answers which one the folder then holds.
fn fetched(fetching: &Fetching<'_>) -> Result<fetch::Installed, ExitCode> {
    let packed = packed_login()?;
    let account = match (&packed, fetching.named) {
        (Some(packed), None) => packed.account().clone(),
        _ => account(fetching.named, false)?.0,
    };
    let (held, home, manifest) = ready()?;
    restore(packed.as_ref(), &home)?;
    let before = Redactor::for_home(&home).map_err(|error| fail(&error, FAILED))?;
    let secret = fetching
        .password
        .map_or_else(Zeroizing::default, |password| password.text.clone());
    let steamcmd_logs = steamcmd::state_folder(&home, Platform::THIS).join("logs");
    let marks = dump::Marks::take(&steamcmd_logs);
    let spinner = Spinner::start("steam", "downloading", true);
    let finished = run_fetch(fetching, &account, &home, &secret);
    let took = show::took(spinner.elapsed());
    let finished = match finished {
        Ok(finished) => finished,
        Err(error) => {
            spinner.failed(&format!("could not start after {took}"));
            return Err(fail(&error, FAILED));
        }
    };
    let redactor = before.and(Redactor::for_home(&home).map_err(|error| fail(&error, FAILED))?);
    let console = fetch::hidden(
        &String::from_utf8_lossy(&redactor.redact(&finished.output)),
        &secret,
    );
    let log = marks
        .added(&steamcmd_logs)
        .into_iter()
        .find(|(name, _)| name == "content_log.txt")
        .map(|(_, added)| {
            fetch::hidden(&String::from_utf8_lossy(&redactor.redact(&added)), &secret)
        })
        .unwrap_or_default();
    install::verify(&held, &manifest).map_err(|error| steamcmd_failed(&error))?;
    match fetch::judge(finished.code, &console, &log, fetching.app) {
        fetch::Outcome::Fetched => spinner.done(&format!("downloaded in {took}")),
        fetch::Outcome::NotLoggedIn(reason) => {
            spinner.failed(&format!("refused after {took}"));
            return Err(login_refused(&reason, packed.as_ref()));
        }
        fetch::Outcome::PasswordRefused(reasons) => {
            spinner.failed(&format!("refused after {took}"));
            for reason in &reasons {
                show::failure(reason, "", None);
            }
            forget_refused_password(&home, fetching);
            return Err(ExitCode::from(FAILED));
        }
        fetch::Outcome::Failed(reasons) => {
            spinner.failed(&format!("failed after {took}"));
            for reason in &reasons {
                show::failure(reason, "", None);
            }
            return Err(ExitCode::from(FAILED));
        }
    }
    let record = fetch::manifest(fetching.folder, fetching.app);
    fs::read_to_string(&record)
        .map_err(|error| error.to_string())
        .and_then(|text| fetch::installed(&text))
        .map_err(|why| fail(&format!("{}: {why}", record.display()), FAILED))
}

/// Has steamcmd in `home` fetch as `account`, typing `secret`, the branch's password, into its
/// console, or giving it on its command line when asked to.
fn run_fetch(
    fetching: &Fetching<'_>,
    account: &Account,
    home: &Path,
    secret: &Zeroizing<String>,
) -> io::Result<run::Finished> {
    let branch = branch::key(&fetching.branch.name);
    let system = fetch::system_name(fetching.system);
    if fetching.password.is_some() && !fetching.on_command_line {
        return steamcmd::fetch_console(account, system, fetching.folder).typed(
            home,
            &fetch::typed_command(fetching.app, branch, secret),
            secret,
        );
    }
    steamcmd::fetch(
        account,
        fetching.app,
        branch,
        fetching.password.map(|_| secret.as_str()),
        system,
        fetching.folder,
    )
    .run(home)
}

/// Forgets the branch's password Steam refused, when it was the one kept, so that the next fetch
/// asks for the one the branch takes now.
fn forget_refused_password(home: &Path, fetching: &Fetching<'_>) {
    if !fetching
        .password
        .is_some_and(|password| password.from == PasswordFrom::Kept)
    {
        return;
    }
    match keychain::forget_branch_password(home, fetching.app, &fetching.branch.name) {
        Ok(_) => show::note(&format!(
            "the password kept in {} is forgotten. Fetch again to give the one the branch takes \
             now",
            keychain::STORE
        )),
        Err(error) => show::failure(&error.to_string(), "", None),
    }
}
