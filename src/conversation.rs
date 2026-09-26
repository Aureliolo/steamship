//! What steamcmd says while it logs in, read into what steamship shows or asks. steamcmd's own
//! wording is kept to the lines matched here; anything else it says is passed on as it is.

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
fn refusal(line: &str) -> Option<&str> {
    ["FAILED (", "ERROR ("].iter().find_map(|marker| {
        let (_, after) = line.split_once(marker)?;
        after.split_once(')').map(|(reason, _)| reason.trim())
    })
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
            "Steam is limiting login attempts; wait, then try again",
        ),
        (
            "Rate Limit Exceeded",
            "Steam is limiting login attempts; wait, then try again",
        ),
        (
            "Expired Login Auth Code",
            "the code had expired; try again with a new one",
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
