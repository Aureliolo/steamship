//! `steamship install`: the pinned steamcmd, installed or verified.

use std::process::ExitCode;

use steamship::install;
use steamship::show;

use super::ready::ready;

pub(crate) fn run_install() -> ExitCode {
    show::title("install");
    match ready() {
        Ok((_held, home, _)) => {
            show::field("folder", &home.join(install::FOLDER).display().to_string());
            ExitCode::SUCCESS
        }
        Err(code) => code,
    }
}
