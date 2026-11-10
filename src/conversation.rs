//! What steamcmd says while it logs in, read into what steamship shows or asks. steamcmd's own
//! wording is kept to the lines matched here. Anything else it says is passed on as it is.

use crate::terminal::Event;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Step {
    /// steamcmd waits for the password.
    Password,
    /// steamcmd waits for a Steam Guard code, from the email Steam sent or from the authenticator.
    Code,
    /// steamcmd waits for the login to be approved in the Steam Mobile app.
    Approve,
    /// The login was approved.
    Approved,
    /// steamcmd logs in with the login it saved last time, and asks for nothing.
    Saved,
    /// steamcmd refused the login, for this reason in its own words.
    Refused(String),
    /// A line steamship gives no meaning to, shown as steamcmd wrote it.
    Other(String),
    /// Nothing to show: steamcmd's own chatter, or a line not yet finished.
    Quiet,
}

/// The lines steamcmd writes on every run, which say nothing about this login. One of them names
/// the account, which steamship never prints.
const CHATTER: [&str; 15] = [
    "Redirecting stderr",
    "Logging directory",
    "Steam Console Client",
    "-- type 'quit' to exit --",
    "Loading Steam API",
    "\"@",
    "Unloading Steam API",
    "Cached credentials not found",
    "Proceeding with login",
    "Logging in user",
    "Waiting for client config",
    "Waiting for user info",
    "Waiting for confirmation",
    "Waiting for compat in post-logon",
    "password:",
];

/// What `event`, from steamcmd logging in, means.
#[must_use]
pub fn step(event: &Event) -> Step {
    match event {
        Event::Waiting(pending) => {
            let prompt = pending.trim_end();
            if prompt.ends_with("password:") {
                Step::Password
            } else if prompt.ends_with("code:") {
                Step::Code
            } else {
                Step::Quiet
            }
        }
        Event::Line(line) => line_step(line.trim()),
    }
}

fn line_step(line: &str) -> Step {
    if let Some(reason) = refusal(line) {
        return Step::Refused(reason.to_owned());
    }
    if line.contains("Please confirm the login in the Steam Mobile app") {
        return Step::Approve;
    }
    if line.contains("Waiting for confirmation...OK") {
        return Step::Approved;
    }
    if line.starts_with("Logging in using cached credentials") {
        return Step::Saved;
    }
    let chatter = line.is_empty()
        || line.contains("code:")
        || line.contains("This account is protected by a Steam Guard mobile authenticator")
        || CHATTER.iter().any(|start| line.starts_with(start));
    if chatter {
        Step::Quiet
    } else {
        Step::Other(line.to_owned())
    }
}

/// The reason in a line where steamcmd says the login failed: `FAILED (reason)` or, as it says
/// for a wrong password, `ERROR (reason)`.
#[must_use]
pub fn refusal(line: &str) -> Option<&str> {
    ["FAILED (", "ERROR ("].iter().find_map(|marker| {
        let (_, after) = line.split_once(marker)?;
        after.split_once(')').map(|(reason, _)| reason.trim())
    })
}

/// steamcmd prints this in place of a refusal when it never reached Steam.
pub const NO_CONNECTION: &str = "No Connection";

/// The reason given when a login line says it failed and names none.
const NO_REASON: &str = "Steam refused the saved login";

/// Why steamcmd could not log in with the login it saved, when that is what its `console` says.
/// The reason is taken from the line and not the line itself, which names the account.
///
/// A missing saved login is a refusal here. `step` treats the same words as chatter because a run
/// that asks for a password carries on to the prompt.
#[must_use]
pub fn refused_login(console: &str) -> Option<String> {
    console.lines().find_map(|line| {
        if line.contains("Cached credentials not found") {
            return Some(line.trim().to_owned());
        }
        let logging_in = line.contains("Logging in");
        match refusal(line) {
            Some(NO_CONNECTION) => None,
            Some(reason) if logging_in || reason.starts_with("No cached credentials") => {
                Some(reason.to_owned())
            }
            None if logging_in && line.contains("FAILED") => Some(NO_REASON.to_owned()),
            Some(_) | None => None,
        }
    })
}

/// A refused saved login in words for the person who must act on it, from the `reason`
/// [`refused_login`] gave. Also whether logging in again is the advice: it is not when Steam is
/// limiting attempts.
#[must_use]
pub fn saved_login_words(reason: &str) -> (String, bool) {
    let limited = ["Account Login Denied Throttle", "Rate Limit Exceeded"]
        .iter()
        .any(|said| reason.eq_ignore_ascii_case(said));
    if limited {
        (
            "Steam is limiting login attempts. Wait, then try again".to_owned(),
            false,
        )
    } else if reason.starts_with("Cached credentials not found")
        || reason.starts_with("No cached credentials")
    {
        (
            "steamcmd holds no saved login for the build account".to_owned(),
            true,
        )
    } else if reason == NO_REASON {
        (NO_REASON.to_owned(), true)
    } else {
        (format!("{NO_REASON} ({reason})"), true)
    }
}

/// A refusal in words for the person who typed the password, where steamcmd's own are terse.
#[must_use]
pub fn explain(reason: &str) -> String {
    let known = [
        ("Invalid Password", "Steam refused the password"),
        ("Two-factor code mismatch", "Steam refused the code"),
        ("Invalid Login Auth Code", "Steam refused the code"),
        (
            "Account Login Denied Throttle",
            "Steam is limiting login attempts. Wait, then try again",
        ),
        (
            "Rate Limit Exceeded",
            "Steam is limiting login attempts. Wait, then try again",
        ),
        (
            "Expired Login Auth Code",
            "the code had expired. Try again with a new one",
        ),
    ];
    known
        .iter()
        .find(|(said, _)| reason.eq_ignore_ascii_case(said))
        .map_or_else(
            || format!("steamcmd says: {reason}"),
            |(_, plain)| (*plain).to_owned(),
        )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::terminal::Reader;

    fn steps(transcript: &[u8]) -> Vec<Step> {
        Reader::default()
            .read(transcript)
            .iter()
            .map(step)
            .filter(|step| *step != Step::Quiet)
            .collect()
    }

    /// A real login with the Steam Mobile app, as steamcmd wrote it, the account renamed.
    const APPROVED: &[u8] = b"Redirecting stderr to 'C:\\steamship\\steamcmd\\logs\\stderr.txt'\r\n\
        Logging directory: 'C:\\steamship\\steamcmd/logs'\r\n\
        Steam Console Client (c) Valve Corporation - version 1788292693\r\n\
        -- type 'quit' to exit --\r\n\
        Loading Steam API...OK\r\n\
        \"@ShutdownOnFailedCommand\" = \"1\"\r\n\
        Cached credentials not found.\r\n\
        \r\n\
        password: \r\n\
        \r\n\
        Proceeding with login using username/password.\r\n\
        Logging in user 'someone' [U:1:0] to Steam Public...This account is protected by a Steam Guard mobile authenticator.\r\n\
        Please confirm the login in the Steam Mobile app on your phone.\r\n\
        \r\n\
        Waiting for confirmation...\r\n\
        Waiting for confirmation...OK\r\n\
        Waiting for client config...OK\r\n\
        Waiting for user info...OK\r\n\
        Unloading Steam API...OK\r\n";

    #[test]
    fn a_login_approved_in_the_app_shows_only_the_approval() {
        assert_eq!(steps(APPROVED), [Step::Approve, Step::Approved]);
    }

    #[test]
    fn nothing_shown_names_the_account() {
        let shown = format!("{:?}", steps(APPROVED));
        assert!(!shown.contains("someone"), "{shown}");
    }

    #[test]
    fn a_wrong_password_is_a_refusal_with_steamcmds_reason() {
        let transcript =
            b"Logging in user 'someone' [U:1:0] to Steam Public...ERROR (Invalid Password)\r\n";
        assert_eq!(
            steps(transcript),
            [Step::Refused("Invalid Password".to_owned())]
        );
        let failed =
            b"Logging in user 'someone' to Steam Public...FAILED (Rate Limit Exceeded)\r\n";
        assert_eq!(
            steps(failed),
            [Step::Refused("Rate Limit Exceeded".to_owned())]
        );
    }

    #[test]
    fn a_folder_named_like_a_refusal_is_not_a_refused_login() {
        // steamcmd's own words for the log folder, as in `APPROVED`, with only the folder's name
        // changed to what a user could have chosen.
        for named in ["FAILED (old)", "ERROR (old)"] {
            let line = format!("Logging directory: 'C:\\steamship\\{named}/logs'\r\n");
            assert_eq!(refused_login(&line), None, "{line}");
        }
    }

    #[test]
    fn a_wrong_password_on_a_saved_login_is_a_refusal_through_every_reader() {
        let line =
            "Logging in user 'someone' [U:1:0] to Steam Public...ERROR (Invalid Password)\r\n";
        assert_eq!(
            steps(line.as_bytes()),
            [Step::Refused("Invalid Password".to_owned())]
        );
        assert_eq!(refused_login(line), Some("Invalid Password".to_owned()));
    }

    #[test]
    fn a_refused_saved_login_is_put_in_words_for_the_person_who_must_act() {
        let wait = (
            "Steam is limiting login attempts. Wait, then try again".to_owned(),
            false,
        );
        assert_eq!(saved_login_words("Rate Limit Exceeded"), wait);
        assert_eq!(saved_login_words("account login denied throttle"), wait);
        let none = (
            "steamcmd holds no saved login for the build account".to_owned(),
            true,
        );
        assert_eq!(saved_login_words("Cached credentials not found."), none);
        assert_eq!(
            saved_login_words("No cached credentials and @NoPromptForPassword is set"),
            none
        );
        assert_eq!(
            saved_login_words("Steam refused the saved login"),
            ("Steam refused the saved login".to_owned(), true)
        );
        assert_eq!(
            saved_login_words("Invalid Password"),
            (
                "Steam refused the saved login (Invalid Password)".to_owned(),
                true
            )
        );
    }

    #[test]
    fn a_login_steamcmd_saved_is_recognised() {
        assert_eq!(
            step(&Event::Line(
                "Logging in using cached credentials.".to_owned()
            )),
            Step::Saved
        );
    }

    #[test]
    fn a_prompt_is_answered_while_steamcmd_waits_at_it() {
        assert_eq!(
            step(&Event::Waiting("password: ".to_owned())),
            Step::Password
        );
        assert_eq!(
            step(&Event::Waiting("Two-factor code:".to_owned())),
            Step::Code
        );
        assert_eq!(
            step(&Event::Waiting("Steam Guard code: ".to_owned())),
            Step::Code
        );
        assert_eq!(
            step(&Event::Waiting("Loading Steam API...".to_owned())),
            Step::Quiet
        );
    }

    #[test]
    fn a_line_steamship_has_no_meaning_for_is_shown_as_it_is() {
        let line = "Please check your email for the message from Steam, and enter the Steam Guard";
        assert_eq!(
            step(&Event::Line(line.to_owned())),
            Step::Other(line.to_owned())
        );
    }

    #[test]
    fn a_refusal_is_explained_in_plain_words_or_quoted() {
        assert_eq!(explain("Invalid Password"), "Steam refused the password");
        assert_eq!(explain("invalid password"), "Steam refused the password");
        assert_eq!(explain("Something New"), "steamcmd says: Something New");
    }
}
