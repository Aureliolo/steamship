//! Steam's partner Web API: an app's branches and the builds live on them, its recent builds, and
//! setting a build live on a branch after it was uploaded, which steamcmd cannot do.
//!
//! The publisher key comes from `STEAMSHIP_WEB_API_KEY` alone and travels in the `x-webapi-key`
//! header, never in an address, so that nothing which names an address can name the key. Valve
//! documents what each method takes but not what it answers, so answers are read leniently:
//! a list or a map of entries, and numbers written as numbers or as text.

use std::cmp::Reverse;
use std::error;
use std::fmt;
use std::time::Duration;

use serde_json::Value;
use ureq::Body;
use ureq::http::Response;
use zeroize::Zeroizing;

/// The environment variable the publisher key is read from.
pub const KEY: &str = "STEAMSHIP_WEB_API_KEY";

const HOST: &str = "https://partner.steam-api.com";

/// Branches Valve keeps for the default one, which only the Steamworks site sets a build live on.
pub const DEFAULT: [&str; 2] = ["default", "public"];

/// A publisher Web API key.
pub struct Key(Zeroizing<String>);

impl fmt::Debug for Key {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.debug_struct("Key").finish_non_exhaustive()
    }
}

impl Key {
    /// The key `text` holds: 32 hexadecimal digits, as Steamworks shows one.
    ///
    /// # Errors
    ///
    /// When `text` is not one; nothing of it is repeated.
    pub fn parse(text: &str) -> Result<Self, Error> {
        let text = text.trim();
        if text.len() == 32 && text.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            Ok(Self(Zeroizing::new(text.to_owned())))
        } else {
            Err(Error::NotAKey)
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// The key is not one; nothing of it is repeated.
    NotAKey,
    /// Steam refused the key for this app.
    Refused,
    /// Steam answered with this HTTP status.
    Status(u16),
    /// Steam could not be reached.
    Unreachable,
    /// Steam answered with something that is not what the method answers.
    Unreadable(&'static str),
    /// Steam answered that the call failed, saying this.
    Failed(String),
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotAKey => write!(
                formatter,
                "{KEY} does not hold a publisher Web API key, which is 32 hexadecimal digits"
            ),
            Self::Refused => formatter.write_str(
                "Steam refused the Web API key; it must be the publisher key of a group that \
                 holds the app",
            ),
            Self::Status(status) => write!(formatter, "Steam answered HTTP {status}"),
            Self::Unreachable => formatter.write_str("partner.steam-api.com could not be reached"),
            Self::Unreadable(what) => write!(formatter, "Steam's answer holds no {what}"),
            Self::Failed(message) => write!(formatter, "Steam says: {message}"),
        }
    }
}

impl error::Error for Error {}

/// A branch of an app, and the build live on it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Branch {
    pub name: String,
    pub build_id: u64,
    pub description: String,
    pub password: bool,
}

/// A build uploaded to an app.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Build {
    pub build_id: u64,
    pub description: String,
    /// When it was uploaded, in seconds since 1970.
    pub created: u64,
}

/// `value` as a number, written as one or as text.
fn number(value: Option<&Value>) -> Option<u64> {
    match value? {
        Value::Number(number) => number.as_u64(),
        Value::String(text) => text.trim().parse().ok(),
        Value::Null | Value::Bool(_) | Value::Array(_) | Value::Object(_) => None,
    }
}

/// The first of `keys` present in `entry`, for fields whose capitals Valve does not document.
fn field<'entry>(entry: &'entry Value, keys: &[&str]) -> Option<&'entry Value> {
    let object = entry.as_object()?;
    keys.iter().find_map(|key| {
        object
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case(key))
            .map(|(_, value)| value)
    })
}

fn text(entry: &Value, keys: &[&str]) -> String {
    field(entry, keys)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned()
}

/// The entries of the list or map `name` in the answer's `response`, each with its key in a map.
fn entries<'answer>(
    answer: &'answer Value,
    name: &'static str,
) -> Result<Vec<(Option<&'answer str>, &'answer Value)>, Error> {
    let response = field(answer, &["response"]).ok_or(Error::Unreadable("response"))?;
    succeeded(response)?;
    match field(response, &[name]) {
        Some(Value::Object(map)) => Ok(map
            .iter()
            .map(|(key, entry)| (Some(key.as_str()), entry))
            .collect()),
        Some(Value::Array(list)) => Ok(list.iter().map(|entry| (None, entry)).collect()),
        None => Ok(Vec::new()),
        Some(_) => Err(Error::Unreadable(name)),
    }
}

/// Refuses a response whose `result` says the call failed; Steam's 1 is success.
fn succeeded(response: &Value) -> Result<(), Error> {
    match number(field(response, &["result"])) {
        None | Some(1) => Ok(()),
        Some(result) => Err(Error::Failed(
            field(response, &["message", "error"])
                .and_then(Value::as_str)
                .map_or_else(|| format!("result {result}"), str::to_owned),
        )),
    }
}

/// The branches `GetAppBetas` answered, by name.
///
/// # Errors
///
/// When the answer is not one, or says the call failed.
pub fn branches_from(answer: &Value) -> Result<Vec<Branch>, Error> {
    let mut branches: Vec<Branch> = entries(answer, "betas")?
        .into_iter()
        .filter_map(|(key, entry)| {
            let name = key.map_or_else(|| text(entry, &["name", "betakey"]), str::to_owned);
            (!name.is_empty()).then(|| Branch {
                name,
                build_id: number(field(entry, &["buildid"])).unwrap_or(0),
                description: text(entry, &["description"]),
                password: field(entry, &["reqpassword"]).is_some_and(|value| {
                    value
                        .as_bool()
                        .unwrap_or_else(|| number(Some(value)) == Some(1))
                }),
            })
        })
        .collect();
    branches.sort_by(|one, other| one.name.cmp(&other.name));
    Ok(branches)
}

/// The builds `GetAppBuilds` answered, newest first.
///
/// # Errors
///
/// When the answer is not one, or says the call failed.
pub fn builds_from(answer: &Value) -> Result<Vec<Build>, Error> {
    let mut builds: Vec<Build> = entries(answer, "builds")?
        .into_iter()
        .filter_map(|(key, entry)| {
            let build_id = match number(field(entry, &["buildid"])) {
                Some(build_id) => build_id,
                None => key?.parse().ok()?,
            };
            Some(Build {
                build_id,
                description: text(entry, &["description"]),
                created: number(field(entry, &["creationtime", "timecreated"])).unwrap_or(0),
            })
        })
        .collect();
    builds.sort_by_key(|build| Reverse(build.build_id));
    Ok(builds)
}

/// Whether `SetAppBuildLive` answered that it set the build live.
///
/// # Errors
///
/// When the answer is not one, or says the call failed.
pub fn set_live_from(answer: &Value) -> Result<(), Error> {
    let response = field(answer, &["response"]).ok_or(Error::Unreadable("response"))?;
    succeeded(response)
}

/// Whether `branch` names the default branch.
#[must_use]
pub fn is_default(branch: &str) -> bool {
    DEFAULT
        .iter()
        .any(|default| branch.eq_ignore_ascii_case(default))
}

/// `branch` as a line: its name, the build live on it and its description, and whether it takes
/// a password.
#[must_use]
pub fn branch_line(branch: &Branch) -> String {
    let mut parts = vec![format!("{}: BuildID {}", branch.name, branch.build_id)];
    if !branch.description.is_empty() {
        parts.push(branch.description.clone());
    }
    if branch.password {
        parts.push("password".to_owned());
    }
    parts.join(", ")
}

/// `build` as a line: the day it was uploaded, its description, and the `branches` it is live on.
#[must_use]
pub fn build_line(build: &Build, branches: &[Branch]) -> String {
    let (year, month, day) = date(build.created);
    let mut parts = vec![format!("{year}-{month:02}-{day:02}")];
    if !build.description.is_empty() {
        parts.push(build.description.clone());
    }
    let names: Vec<&str> = branches
        .iter()
        .filter(|branch| branch.build_id == build.build_id)
        .map(|branch| branch.name.as_str())
        .collect();
    if !names.is_empty() {
        parts.push(format!("live on {}", names.join(" and ")));
    }
    parts.join(", ")
}

/// An app's branches and last builds, as `steamship builds` shows them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Overview {
    pub branches: Vec<Branch>,
    pub builds: Vec<Build>,
}

impl Overview {
    /// How many branches and builds there are, in words.
    #[must_use]
    pub fn summary(&self) -> String {
        let branches = match self.branches.len() {
            1 => "1 branch".to_owned(),
            count => format!("{count} branches"),
        };
        let builds = match self.builds.len() {
            1 => "1 build".to_owned(),
            count => format!("{count} builds"),
        };
        format!("{branches} and {builds}")
    }

    /// Every line shown: each branch, then each build by its ID, as a label and what follows it.
    #[must_use]
    pub fn lines(&self) -> Vec<(String, String)> {
        let branches = self
            .branches
            .iter()
            .map(|branch| ("branch".to_owned(), branch_line(branch)));
        let builds = self.builds.iter().map(|build| {
            (
                build.build_id.to_string(),
                build_line(build, &self.branches),
            )
        });
        branches.chain(builds).collect()
    }
}

/// The partner Web API, called with one key.
#[derive(Debug)]
pub struct Api {
    agent: ureq::Agent,
    key: Key,
    host: String,
}

impl Api {
    #[must_use]
    pub fn new(key: Key) -> Self {
        Self::at(key, HOST)
    }

    fn at(key: Key, host: &str) -> Self {
        let agent = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(30)))
            .http_status_as_error(false)
            .build()
            .into();
        Self {
            agent,
            key,
            host: host.to_owned(),
        }
    }

    /// The branches of `app`, by name.
    ///
    /// # Errors
    ///
    /// When Steam cannot be reached, refuses the key, or answers with something else.
    pub fn branches(&self, app: u32) -> Result<Vec<Branch>, Error> {
        let answer = self.get("GetAppBetas/v1", &[("appid", app.to_string())])?;
        branches_from(&answer)
    }

    /// The last `count` builds of `app`, newest first.
    ///
    /// # Errors
    ///
    /// As [`Api::branches`].
    pub fn builds(&self, app: u32, count: u32) -> Result<Vec<Build>, Error> {
        let answer = self.get(
            "GetAppBuilds/v1",
            &[("appid", app.to_string()), ("count", count.to_string())],
        )?;
        builds_from(&answer)
    }

    /// The branches of `app` and its last `count` builds.
    ///
    /// # Errors
    ///
    /// As [`Api::branches`].
    pub fn overview(&self, app: u32, count: u32) -> Result<Overview, Error> {
        Ok(Overview {
            branches: self.branches(app)?,
            builds: self.builds(app, count)?,
        })
    }

    /// Sets `build` of `app` live on `branch`.
    ///
    /// # Errors
    ///
    /// As [`Api::branches`], and when Steam refuses the change.
    pub fn set_live(&self, app: u32, build: u64, branch: &str) -> Result<(), Error> {
        let response = self
            .agent
            .post(format!("{}/ISteamApps/SetAppBuildLive/v2/", self.host))
            .header("x-webapi-key", self.key.0.as_str())
            .send_form([
                ("appid", app.to_string()),
                ("buildid", build.to_string()),
                ("betakey", branch.to_owned()),
            ]);
        set_live_from(&read(response)?)
    }

    fn get(&self, method: &str, query: &[(&str, String)]) -> Result<Value, Error> {
        let mut request = self
            .agent
            .get(format!("{}/ISteamApps/{method}/", self.host))
            .header("x-webapi-key", self.key.0.as_str());
        for (name, value) in query {
            request = request.query(*name, value);
        }
        read(request.call())
    }
}

/// The JSON Steam answered, where the status says it answered at all. The transport's own error
/// is never shown, as it can name the address.
fn read(response: Result<Response<Body>, ureq::Error>) -> Result<Value, Error> {
    let mut response = response.ok().ok_or(Error::Unreachable)?;
    match response.status().as_u16() {
        200..=299 => {}
        401 | 403 => return Err(Error::Refused),
        status => return Err(Error::Status(status)),
    }
    let body = response
        .body_mut()
        .read_to_string()
        .ok()
        .ok_or(Error::Unreachable)?;
    serde_json::from_str(&body)
        .ok()
        .ok_or(Error::Unreadable("JSON"))
}

/// The day `seconds` since 1970 fall on, in UTC, as a year, month and day.
#[must_use]
pub fn date(seconds: u64) -> (u64, u64, u64) {
    // Howard Hinnant's civil-from-days: eras of 400 years, each starting on 1 March, so that the
    // leap day falls at the end of a year.
    let days = seconds.div_euclid(86_400).saturating_add(719_468);
    let era = days.div_euclid(146_097);
    let of_era = days.saturating_sub(era.saturating_mul(146_097));
    let year_of_era = of_era
        .saturating_sub(of_era.div_euclid(1_460))
        .saturating_add(of_era.div_euclid(36_524))
        .saturating_sub(of_era.div_euclid(146_096))
        .div_euclid(365);
    let day_of_year = of_era.saturating_sub(
        year_of_era
            .saturating_mul(365)
            .saturating_add(year_of_era.div_euclid(4))
            .saturating_sub(year_of_era.div_euclid(100)),
    );
    let shifted_month = day_of_year
        .saturating_mul(5)
        .saturating_add(2)
        .div_euclid(153);
    let day = day_of_year
        .saturating_sub(
            shifted_month
                .saturating_mul(153)
                .saturating_add(2)
                .div_euclid(5),
        )
        .saturating_add(1);
    let month = if shifted_month < 10 {
        shifted_month.saturating_add(3)
    } else {
        shifted_month.saturating_sub(9)
    };
    let year = year_of_era
        .saturating_add(era.saturating_mul(400))
        .saturating_add(u64::from(month <= 2));
    (year, month, day)
}

#[cfg(test)]
mod tests {
    use std::io::{Read as _, Write as _};
    use std::net::TcpListener;
    use std::sync::mpsc;
    use std::thread;

    use super::*;

    /// `text`, an answer as Steam sends one, read.
    fn answer(text: &str) -> Value {
        serde_json::from_str(text).unwrap()
    }

    #[test]
    fn a_key_is_32_hexadecimal_digits_and_never_shown() {
        let key = Key::parse(" 0123456789ABCDEF0123456789abcdef\n").unwrap();
        assert!(!format!("{key:?}").contains("0123"));
        for wrong in [
            "",
            "0123",
            "0123456789ABCDEF0123456789abcdeg",
            &"0".repeat(33),
        ] {
            let error = Key::parse(wrong).unwrap_err();
            assert_eq!(error, Error::NotAKey, "{wrong}");
            assert!(!error.to_string().contains("0123"));
        }
    }

    #[test]
    fn branches_are_read_from_a_map_or_a_list_in_any_capitals() {
        let map = answer(
            r#"{"response": {"result": 1, "betas": {
                "testing": {"BuildID": 1234, "Description": "0.1.0 9b3d543", "ReqPassword": true},
                "default": {"BuildID": "1200", "Description": "0.0.9", "ReqPassword": false}
            }}}"#,
        );
        let expected = vec![
            Branch {
                name: "default".to_owned(),
                build_id: 1200,
                description: "0.0.9".to_owned(),
                password: false,
            },
            Branch {
                name: "testing".to_owned(),
                build_id: 1234,
                description: "0.1.0 9b3d543".to_owned(),
                password: true,
            },
        ];
        assert_eq!(branches_from(&map).unwrap(), expected);
        let list = answer(
            r#"{"response": {"betas": [
                {"name": "testing", "buildid": 1234, "description": "0.1.0 9b3d543", "reqpassword": 1},
                {"Name": "default", "BuildId": 1200, "Description": "0.0.9"},
                {"description": "a branch with no name is left out"}
            ]}}"#,
        );
        assert_eq!(branches_from(&list).unwrap(), expected);
        assert_eq!(
            branches_from(&answer(r#"{"response": {"result": 1}}"#)).unwrap(),
            []
        );
    }

    #[test]
    fn builds_are_read_newest_first_with_their_id_from_the_entry_or_its_key() {
        let builds = answer(
            r#"{"response": {"builds": {
                "1200": {"Description": "0.0.9", "CreationTime": 1790000000},
                "1234": {"BuildID": 1234, "Description": "0.1.0", "CreationTime": "1790500000"},
                "not a number": {"Description": "left out"}
            }}}"#,
        );
        assert_eq!(
            builds_from(&builds).unwrap(),
            [
                Build {
                    build_id: 1234,
                    description: "0.1.0".to_owned(),
                    created: 1_790_500_000,
                },
                Build {
                    build_id: 1200,
                    description: "0.0.9".to_owned(),
                    created: 1_790_000_000,
                },
            ]
        );
    }

    #[test]
    fn an_answer_that_is_not_one_or_says_it_failed_is_an_error() {
        assert_eq!(
            branches_from(&answer(r#"{"nothing": 1}"#)),
            Err(Error::Unreadable("response"))
        );
        assert_eq!(
            builds_from(&answer(r#"{"response": {"builds": 7}}"#)),
            Err(Error::Unreadable("builds"))
        );
        assert_eq!(
            set_live_from(&answer(
                r#"{"response": {"result": 2, "message": "Invalid build"}}"#
            )),
            Err(Error::Failed("Invalid build".to_owned()))
        );
        assert_eq!(
            set_live_from(&answer(r#"{"response": {"result": 8}}"#)),
            Err(Error::Failed("result 8".to_owned()))
        );
        assert_eq!(
            set_live_from(&answer(r#"{"response": {"result": 1}}"#)),
            Ok(())
        );
        assert_eq!(set_live_from(&answer(r#"{"response": {}}"#)), Ok(()));
    }

    #[test]
    fn errors_say_what_happened_and_never_an_address() {
        for (error, said) in [
            (
                Error::Refused,
                "Steam refused the Web API key; it must be the publisher key of a group that \
                 holds the app",
            ),
            (Error::Status(500), "Steam answered HTTP 500"),
            (
                Error::Unreachable,
                "partner.steam-api.com could not be reached",
            ),
            (Error::Unreadable("betas"), "Steam's answer holds no betas"),
            (Error::Failed("no".to_owned()), "Steam says: no"),
        ] {
            assert_eq!(error.to_string(), said);
        }
    }

    #[test]
    fn the_default_branch_is_known_by_both_its_names() {
        for default in ["default", "public", "Public"] {
            assert!(is_default(default), "{default}");
        }
        assert!(!is_default("testing"));
    }

    fn branch(name: &str, build_id: u64, password: bool) -> Branch {
        Branch {
            name: name.to_owned(),
            build_id,
            description: format!("{build_id} description"),
            password,
        }
    }

    #[test]
    fn a_branch_and_a_build_read_as_one_line_each() {
        let branches = [
            branch("default", 1200, false),
            branch("testing", 1234, true),
            branch("staging", 1234, false),
        ];
        assert_eq!(
            branch_line(&branches[1]),
            "testing: BuildID 1234, 1234 description, password"
        );
        let bare = Branch {
            description: String::new(),
            ..branch("default", 1200, false)
        };
        assert_eq!(branch_line(&bare), "default: BuildID 1200");
        let build = Build {
            build_id: 1234,
            description: "0.1.0 9b3d543".to_owned(),
            created: 1_790_553_599,
        };
        assert_eq!(
            build_line(&build, &branches),
            "2026-09-27, 0.1.0 9b3d543, live on testing and staging"
        );
        let older = Build {
            build_id: 1100,
            description: String::new(),
            created: 0,
        };
        assert_eq!(build_line(&older, &branches), "1970-01-01");
    }

    /// A server on this machine that answers the next request with `status` and `body`, and
    /// hands over the request it read.
    fn answering(status: &'static str, body: &'static str) -> (String, mpsc::Receiver<String>) {
        serving(vec![(status, body)])
    }

    /// A server on this machine that answers one request with each status and body in
    /// `answers`, in turn, and hands over each request it read.
    fn serving(answers: Vec<(&'static str, &'static str)>) -> (String, mpsc::Receiver<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let (sender, requests) = mpsc::channel();
        let _serving = thread::spawn(move || {
            for (status, body) in answers {
                serve(&listener, status, body, &sender);
            }
        });
        (format!("http://{address}"), requests)
    }

    fn serve(listener: &TcpListener, status: &str, body: &str, sender: &mpsc::Sender<String>) {
        {
            let (mut stream, _) = listener.accept().unwrap();
            // The head first, then as much body as it says there is, which can come later.
            let mut received = Vec::new();
            let mut chunk = [0_u8; 8192];
            let request = loop {
                let read = stream.read(&mut chunk).unwrap();
                received.extend_from_slice(chunk.get(..read).unwrap());
                let text = String::from_utf8_lossy(&received).into_owned();
                if let Some((head, sent)) = text.split_once("\r\n\r\n") {
                    let length = head
                        .lines()
                        .find_map(|line| {
                            let (name, value) = line.split_once(':')?;
                            name.eq_ignore_ascii_case("content-length")
                                .then(|| value.trim().parse::<usize>().ok())?
                        })
                        .unwrap_or(0);
                    if sent.len() >= length || read == 0 {
                        break text;
                    }
                }
            };
            let response = format!(
                "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\
                 Connection: close\r\n\r\n{body}",
                body.len()
            );
            stream.write_all(response.as_bytes()).unwrap();
            drop(sender.send(request));
        }
    }

    const KEY_TEXT: &str = "0123456789ABCDEF0123456789ABCDEF";

    fn api(host: &str) -> Api {
        Api::at(Key::parse(KEY_TEXT).unwrap(), host)
    }

    #[test]
    fn branches_are_asked_for_with_the_key_in_a_header_and_never_in_the_address() {
        let (host, requests) = answering(
            "200 OK",
            r#"{"response": {"result": 1, "betas": {"testing": {"BuildID": 1234}}}}"#,
        );
        let branches = api(&host).branches(5_335_950).unwrap();
        assert_eq!(branches.first().map(|branch| branch.build_id), Some(1234));
        let request = requests.recv().unwrap();
        let first = request.lines().next().unwrap();
        assert_eq!(
            first,
            "GET /ISteamApps/GetAppBetas/v1/?appid=5335950 HTTP/1.1"
        );
        assert!(
            request
                .to_ascii_lowercase()
                .contains(&format!("x-webapi-key: {}", KEY_TEXT.to_ascii_lowercase())),
            "{request}"
        );
    }

    #[test]
    fn an_overview_says_how_many_and_shows_each_branch_then_each_build() {
        let branches = vec![branch("testing", 1234, true)];
        let builds = vec![
            Build {
                build_id: 1234,
                description: "0.1.0".to_owned(),
                created: 1_790_553_599,
            },
            Build {
                build_id: 1200,
                description: String::new(),
                created: 0,
            },
        ];
        let overview = Overview { branches, builds };
        assert_eq!(overview.summary(), "1 branch and 2 builds");
        assert_eq!(
            overview.lines(),
            [
                (
                    "branch".to_owned(),
                    "testing: BuildID 1234, 1234 description, password".to_owned()
                ),
                (
                    "1234".to_owned(),
                    "2026-09-27, 0.1.0, live on testing".to_owned()
                ),
                ("1200".to_owned(), "1970-01-01".to_owned()),
            ]
        );
        let none = Overview {
            branches: Vec::new(),
            builds: vec![Build {
                build_id: 1,
                description: String::new(),
                created: 0,
            }],
        };
        assert_eq!(none.summary(), "0 branches and 1 build");
    }

    #[test]
    fn an_overview_asks_for_the_branches_then_the_builds() {
        let (host, requests) = serving(vec![
            (
                "200 OK",
                r#"{"response": {"betas": {"testing": {"BuildID": 7}}}}"#,
            ),
            ("200 OK", r#"{"response": {"builds": {"7": {}}}}"#),
        ]);
        let overview = api(&host).overview(1, 5).unwrap();
        assert_eq!(overview.summary(), "1 branch and 1 build");
        assert!(requests.recv().unwrap().contains("GetAppBetas"));
        assert!(requests.recv().unwrap().contains("GetAppBuilds"));
        let (stopped, _requests) = answering(
            "200 OK",
            r#"{"response": {"betas": {"testing": {"BuildID": 7}}}}"#,
        );
        assert_eq!(
            api(&stopped).overview(1, 5),
            Err(Error::Unreachable),
            "the second call finds no server"
        );
    }

    #[test]
    fn builds_are_asked_for_by_count() {
        let (host, requests) = answering(
            "200 OK",
            r#"{"response": {"builds": {"1234": {"Description": "0.1.0"}}}}"#,
        );
        let builds = api(&host).builds(5_335_950, 25).unwrap();
        assert_eq!(builds.first().map(|build| build.build_id), Some(1234));
        let request = requests.recv().unwrap();
        assert!(
            request.starts_with("GET /ISteamApps/GetAppBuilds/v1/?appid=5335950&count=25 "),
            "{request}"
        );
    }

    #[test]
    fn a_build_is_set_live_with_a_form_that_holds_no_key() {
        let (host, requests) = answering("200 OK", r#"{"response": {"result": 1}}"#);
        api(&host).set_live(5_335_950, 1234, "testing").unwrap();
        let request = requests.recv().unwrap();
        assert!(
            request.starts_with("POST /ISteamApps/SetAppBuildLive/v2/ "),
            "{request}"
        );
        assert!(
            request.ends_with("appid=5335950&buildid=1234&betakey=testing"),
            "{request}"
        );
        assert_eq!(
            request.matches(KEY_TEXT).count(),
            1,
            "the header alone: {request}"
        );
    }

    #[test]
    fn a_refused_key_a_failed_status_and_an_answer_that_is_not_json_are_errors() {
        for (status, body, expected) in [
            ("403 Forbidden", "", Error::Refused),
            ("401 Unauthorized", "", Error::Refused),
            ("500 Internal Server Error", "", Error::Status(500)),
            ("200 OK", "<html>", Error::Unreadable("JSON")),
        ] {
            let (host, _requests) = answering(status, body);
            assert_eq!(api(&host).branches(1), Err(expected), "{status}");
        }
        let closed = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = closed.local_addr().unwrap();
        drop(closed);
        assert_eq!(
            api(&format!("http://{address}")).branches(1),
            Err(Error::Unreachable)
        );
    }

    #[test]
    fn a_day_is_read_off_seconds_since_1970() {
        assert_eq!(date(0), (1970, 1, 1));
        assert_eq!(date(951_782_400), (2000, 2, 29));
        assert_eq!(date(1_790_553_599), (2026, 9, 27));
        assert_eq!(date(4_107_542_400), (2100, 3, 1));
    }
}
