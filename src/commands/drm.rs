//! `steamship drm-wrap`: a Windows program wrapped by Steam's DRM.

use std::fmt::Display;
use std::fs;
use std::path::{self, Path};
use std::process::ExitCode;

use steamship::cli::{FAILED, REFUSED};
use steamship::install;
use steamship::redact::Redactor;
use steamship::show::{self, Spinner};
use steamship::{drm, magic, steamcmd};

use super::exit::fail;
use super::game::app_named;
use super::ready::{account, login_refused, packed_login, ready, restore, steamcmd_failed};

pub(crate) fn try_drm_wrap(
    app: &str,
    executable: &Path,
    output: Option<&Path>,
    mode: drm::Mode,
    named: Option<&str>,
) -> Result<ExitCode, ExitCode> {
    show::title("drm-wrap");
    let app_id = app_named(app)?;
    let refused = |why: &dyn Display| fail(&format!("{}: {why}", executable.display()), REFUSED);
    match magic::looks_like_windows_program(executable) {
        Ok(true) => {}
        Ok(false) => return Err(refused(&"is not a Windows executable")),
        Err(error) => return Err(refused(&error)),
    }
    show::field("program", &executable.display().to_string());
    let given = output.unwrap_or(executable);
    if let Some(output) = output {
        show::field("output", &output.display().to_string());
    }
    let input = path::absolute(executable).map_err(|error| refused(&error))?;
    let destination = path::absolute(given)
        .map_err(|error| fail(&format!("{}: {error}", given.display()), REFUSED))?;
    let (Some(folder), Some(name)) = (destination.parent(), destination.file_name()) else {
        return Err(fail(
            &format!("{}: is not a file's path", destination.display()),
            REFUSED,
        ));
    };
    // Beside where it goes, so that it is moved into place whole, and never copied half-written.
    let wrapped = folder.join(format!(".{}.steamship-drm", name.to_string_lossy()));
    let packed = packed_login()?;
    let account = match (&packed, named) {
        (Some(packed), None) => packed.account().clone(),
        _ => account(named, false)?.0,
    };
    let (held, home, manifest) = ready()?;
    restore(packed.as_ref(), &home)?;
    let before = Redactor::for_home(&home).map_err(|error| fail(&error, FAILED))?;
    drop(fs::remove_file(&wrapped));
    let spinner = Spinner::start("steam", "wrapping", true);
    let finished = steamcmd::drm_wrap(&account, app_id, &input, &wrapped, mode).run(&home);
    let took = show::took(spinner.elapsed());
    let finished = match finished {
        Ok(finished) => finished,
        Err(error) => {
            spinner.failed(&format!("could not start after {took}"));
            return Err(fail(&error, FAILED));
        }
    };
    let redactor = before.and(Redactor::for_home(&home).map_err(|error| fail(&error, FAILED))?);
    let console = redactor.redact(&finished.output);
    install::verify(&held, &manifest).map_err(|error| steamcmd_failed(&error))?;
    let written = fs::metadata(&wrapped).is_ok_and(|found| found.len() > 0);
    match drm::judge(finished.code, &String::from_utf8_lossy(&console), written) {
        drm::Outcome::Wrapped => {
            let placed = fs::metadata(&input)
                .and_then(|original| fs::set_permissions(&wrapped, original.permissions()))
                .and_then(|()| fs::rename(&wrapped, &destination));
            if let Err(error) = placed {
                spinner.failed(&format!("wrapped in {took}, not placed"));
                drop(fs::remove_file(&wrapped));
                return Err(fail(&format!("{}: {error}", destination.display()), FAILED));
            }
            spinner.done(&format!("wrapped in {took}"));
            show::success(
                &format!("app {app_id}: {} is wrapped in Steam DRM", given.display()),
                "",
            );
            Ok(ExitCode::SUCCESS)
        }
        drm::Outcome::NotLoggedIn(reason) => {
            drop(fs::remove_file(&wrapped));
            spinner.failed(&format!("refused after {took}"));
            Err(login_refused(&reason, packed.as_ref()))
        }
        drm::Outcome::Failed(reasons) => {
            drop(fs::remove_file(&wrapped));
            spinner.failed(&format!("failed after {took}"));
            for reason in &reasons {
                show::failure(reason, "", None);
            }
            Err(ExitCode::from(FAILED))
        }
    }
}
