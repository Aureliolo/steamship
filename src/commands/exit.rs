//! The code a command exits with, and the refusal it prints on the way.

use std::fmt::Display;
use std::process::ExitCode;

use steamship::cli::{REFUSED, UNFINISHED};
use steamship::show;

/// The first code of `codes` that is not success, or success.
pub(crate) fn first_failure(codes: impl IntoIterator<Item = ExitCode>) -> ExitCode {
    codes
        .into_iter()
        .find(|code| *code != ExitCode::SUCCESS)
        .unwrap_or(ExitCode::SUCCESS)
}

/// The code a command exits with, whether it finished or stopped early.
pub(crate) const fn either(result: Result<ExitCode, ExitCode>) -> ExitCode {
    match result {
        Ok(code) | Err(code) => code,
    }
}

pub(crate) fn fail(error: &dyn Display, code: u8) -> ExitCode {
    show::failure(&error.to_string(), "", None);
    ExitCode::from(code)
}

/// The code of a failure once steamcmd has run: `UNFINISHED` when the run left something a second
/// run would duplicate, otherwise `code`.
pub(crate) fn late(code: ExitCode, again: bool) -> ExitCode {
    if again {
        ExitCode::from(UNFINISHED)
    } else {
        code
    }
}

/// Each of `wrong` shown as refused, and the exit code for it.
pub(crate) fn refusals(wrong: &[String]) -> ExitCode {
    for why in wrong {
        show::failure("refused", why, None);
    }
    ExitCode::from(REFUSED)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_first_code_that_is_not_success_is_the_one_a_command_exits_with() {
        assert_eq!(
            first_failure([ExitCode::SUCCESS, ExitCode::from(2), ExitCode::from(1)]),
            ExitCode::from(2)
        );
        assert_eq!(first_failure([]), ExitCode::SUCCESS);
    }

    #[test]
    fn late_gives_5_only_when_the_run_left_something_to_make_again() {
        let codes = [
            late(ExitCode::from(4), false),
            late(ExitCode::from(1), false),
            late(ExitCode::from(4), true),
            late(ExitCode::from(1), true),
        ];
        assert_eq!(
            codes,
            [
                ExitCode::from(4),
                ExitCode::from(1),
                ExitCode::from(5),
                ExitCode::from(5)
            ]
        );
    }
}
