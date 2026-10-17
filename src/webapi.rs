//! Steam's partner Web API: an app's branches and the builds live on them, its recent builds, and
//! setting a build live on a branch after it was uploaded, which steamcmd cannot do.
//!
//! The publisher key travels in the `x-webapi-key` header, never in an address, so that nothing
//! which names an address can name the key. Valve
//! documents what each method takes but not what it answers, so answers are read leniently:
//! a list or a map of entries, and numbers written as numbers or as text.

use std::cmp::Reverse;
use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::error;
use std::fmt;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};
use ureq::Body;
use ureq::http::Response;
use zeroize::Zeroizing;

/// The environment variable the publisher key is read from.
pub const KEY: &str = "STEAMSHIP_WEB_API_KEY";

const HOST: &str = "https://partner.steam-api.com";

/// The names the default branch goes by: `default` on the Steamworks site, `public` in the Web
/// API.
pub const DEFAULT: [&str; 2] = ["default", "public"];

/// The default branch's name in the Web API, which `SetAppBuildLive` takes for it.
pub const DEFAULT_KEY: &str = "public";

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

    /// The key's text, for the credential store that keeps it.
    #[must_use]
    pub fn text(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// The key is not one; nothing of it is repeated.
    NotAKey,
    /// Steam refused the key for this app.
    Refused,
    /// Steam answered with this HTTP status, and the reason it gave, if any.
    Status(u16, String),
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
            Self::NotAKey => {
                formatter.write_str("not a publisher Web API key, which is 32 hexadecimal digits")
            }
            Self::Refused => formatter.write_str(
                "Steam refused the Web API key; it must be the publisher key of a group that \
                 holds the app",
            ),
            Self::Status(status, said) if said.is_empty() => {
                write!(formatter, "Steam answered HTTP {status}")
            }
            Self::Status(status, said) => write!(formatter, "Steam answered HTTP {status}: {said}"),
            Self::Unreachable => formatter.write_str("partner.steam-api.com could not be reached"),
            Self::Unreadable(what) => write!(formatter, "Steam's answer holds no {what}"),
            Self::Failed(message) => write!(formatter, "Steam says: {message}"),
        }
    }
}

impl error::Error for Error {}

impl Error {
    /// What went wrong, as a word a program can match on, which `--json` gives with the message.
    #[must_use]
    pub const fn code(&self) -> &'static str {
        match self {
            Self::NotAKey => "not-a-key",
            Self::Refused => "key-refused",
            Self::Status(..) => "http-error",
            Self::Unreachable => "unreachable",
            Self::Unreadable(_) => "unreadable",
            Self::Failed(_) => "steam-failed",
        }
    }
}

/// What `--json` prints when Steam did not answer about `app`: the code from [`Error::code`],
/// or `no-build-live` when `why` says that is the reason, and the message a person would read.
#[must_use]
pub fn error_json(app: u32, error: &Error, why: Option<&str>) -> Value {
    let (code, message) = why.map_or_else(
        || (error.code(), error.to_string()),
        |why| ("no-build-live", format!("{error}; {why}")),
    );
    json!({ "app": app, "error": { "code": code, "message": message } })
}

/// A branch of an app, and the build live on it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Branch {
    pub name: String,
    pub build_id: u64,
    pub description: String,
    /// Whether players need the branch's password to opt into it.
    pub locked: bool,
}

/// A build uploaded to an app.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Build {
    pub build_id: u64,
    pub description: String,
    /// When it was uploaded, in seconds since 1970.
    pub created: u64,
}

/// An achievement as Steam holds it for an app.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Achievement {
    /// The name the game unlocks it by.
    pub api_name: String,
    /// The name players see.
    pub name: String,
    pub description: String,
    /// Whether players see it only once unlocked.
    pub hidden: bool,
    /// The address of its icon, and of the one shown while it is locked; empty when none.
    pub icon: String,
    pub icon_locked: String,
}

/// A stat as Steam holds it for an app.
#[derive(Debug, Clone, PartialEq)]
pub struct Stat {
    /// The name the game sets it by.
    pub api_name: String,
    /// The name players see; empty when none was given.
    pub name: String,
    /// The value it starts at.
    pub default: f64,
}

/// An app's achievements and stats, as `GetSchemaForGame` answers them.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Schema {
    pub achievements: Vec<Achievement>,
    pub stats: Vec<Stat>,
}

/// The achievements and stats `GetSchemaForGame` answered, each in Steam's order. An app with
/// none is answered with an empty `game`, which is none of either rather than an answer that
/// cannot be read.
///
/// # Errors
///
/// When an achievement or a stat has no API name.
pub fn schema_from(answer: &Value) -> Result<Schema, Error> {
    let listed = |which: &str| {
        answer
            .pointer(&format!("/game/availableGameStats/{which}"))
            .and_then(Value::as_array)
            .map_or(&[][..], Vec::as_slice)
    };
    let api_name = |entry: &Value, what: &'static str| {
        let api_name = text(entry, &["name"]);
        if api_name.is_empty() {
            Err(Error::Unreadable(what))
        } else {
            Ok(api_name)
        }
    };
    let achievements = listed("achievements")
        .iter()
        .map(|entry| {
            Ok(Achievement {
                api_name: api_name(entry, "achievement name")?,
                name: text(entry, &["displayName"]),
                description: text(entry, &["description"]),
                hidden: number(field(entry, &["hidden"])).is_some_and(|hidden| hidden != 0),
                icon: text(entry, &["icon"]),
                icon_locked: text(entry, &["icongray"]),
            })
        })
        .collect::<Result<_, Error>>()?;
    let stats = listed("stats")
        .iter()
        .map(|entry| {
            Ok(Stat {
                api_name: api_name(entry, "stat name")?,
                name: text(entry, &["displayName"]),
                default: field(entry, &["defaultvalue"])
                    .and_then(|value| {
                        value
                            .as_f64()
                            .or_else(|| value.as_str()?.trim().parse().ok())
                    })
                    .unwrap_or(0.0_f64),
            })
        })
        .collect::<Result<_, Error>>()?;
    Ok(Schema {
        achievements,
        stats,
    })
}

/// How far players have got with an app's achievements and stats, where Steam gives it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Global {
    /// The share of players who have each achievement, in percent, by API name.
    pub shares: BTreeMap<String, f64>,
    /// Each stat Steam adds up over all players, by API name, with the total as Steam writes it;
    /// none for one no player has set.
    pub totals: BTreeMap<String, Option<String>>,
}

/// What `GetGlobalStatsForGame` says of the stats it was asked to add up over all players.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Totalled {
    /// Each it adds up, by API name, with the total as Steam writes it; none for one no player
    /// has set.
    pub totals: BTreeMap<String, Option<String>>,
    /// Each it named as one it does not add up.
    pub unaggregated: BTreeSet<String>,
    /// What Steam answered in place of the rest, where it stopped short of them.
    pub withheld: Option<String>,
}

impl Totalled {
    /// Whether Steam adds up the stat `api_name` over all players, where it said.
    #[must_use]
    pub fn aggregated(&self, api_name: &str) -> Option<bool> {
        if self.totals.contains_key(api_name) {
            Some(true)
        } else if self.unaggregated.contains(api_name) {
            Some(false)
        } else {
            None
        }
    }
}

/// The share of players who have each achievement, as `GetGlobalAchievementPercentagesForApp`
/// answers it.
///
/// # Errors
///
/// When an achievement in it has no API name or no share.
pub fn shares_from(answer: &Value) -> Result<BTreeMap<String, f64>, Error> {
    answer
        .pointer("/achievementpercentages/achievements")
        .and_then(Value::as_array)
        .map_or(&[][..], Vec::as_slice)
        .iter()
        .map(|entry| {
            let api_name = text(entry, &["name"]);
            let share = field(entry, &["percent"]).and_then(|share| {
                share
                    .as_f64()
                    .or_else(|| share.as_str()?.trim().parse().ok())
            });
            match share {
                Some(share) if !api_name.is_empty() => Ok((api_name, share)),
                _ => Err(Error::Unreadable("achievement share")),
            }
        })
        .collect()
}

/// What `GetGlobalStatsForGame` answered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Totals {
    /// Each stat asked for, with its total over all players; none for one no player has set.
    Given(BTreeMap<String, Option<String>>),
    /// A stat asked for that Steam does not add up over all players, which it names.
    NotAggregated(String),
    /// Steam answered that it gives none, saying this.
    Withheld(String),
}

/// The totals `GetGlobalStatsForGame` answered, or why it gave none.
///
/// # Errors
///
/// When the answer is not one.
pub fn totals_from(answer: &Value) -> Result<Totals, Error> {
    let response = field(answer, &["response"]).ok_or(Error::Unreadable("global stats"))?;
    if number(field(response, &["result"])) == Some(1) {
        let given = field(response, &["globalstats"])
            .and_then(Value::as_object)
            .ok_or(Error::Unreadable("global stats"))?;
        return Ok(Totals::Given(
            given
                .iter()
                .map(|(api_name, stat)| {
                    let total = match field(stat, &["total"]) {
                        Some(Value::String(total)) => Some(total.clone()),
                        Some(Value::Number(total)) => Some(total.to_string()),
                        _ => None,
                    };
                    (api_name.clone(), total)
                })
                .collect(),
        ));
    }
    let said = text(response, &["error"]);
    // Steam names the stat it stopped at: "Stat 'NumWins' is not an aggregated stat".
    if let Some(named) = said
        .strip_prefix("Stat '")
        .and_then(|rest| rest.strip_suffix("' is not an aggregated stat"))
    {
        return Ok(Totals::NotAggregated(named.to_owned()));
    }
    Ok(Totals::Withheld(said))
}

/// A leaderboard as Steam holds it, or as a file wants it made.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Leaderboard {
    /// The name the game finds it by.
    pub name: String,
    /// `Ascending` or `Descending`, Steam's words for whether the lowest score is first or the
    /// highest.
    pub sort: String,
    /// `Numeric`, `Seconds` or `MilliSeconds`, how a score is shown; empty when none was set.
    pub display: String,
    /// Whether only the Web API, and never the game, may set scores.
    pub trusted_writes: bool,
    /// Whether players are shown only their friends' scores.
    pub friends_only: bool,
    /// How many scores it holds.
    pub entries: u64,
}

/// The leaderboards `GetLeaderboardsForGame` answered, in Steam's order.
///
/// # Errors
///
/// When the answer is not one, says the call failed, or names no leaderboard.
pub fn leaderboards_from(answer: &Value) -> Result<Vec<Leaderboard>, Error> {
    entries(answer, "leaderboards")?
        .into_iter()
        .map(|(_, entry)| leaderboard(entry))
        .collect()
}

/// The leaderboard `FindOrCreateLeaderboard` answered, which is Steam's whether it was just made
/// or was already there.
///
/// # Errors
///
/// When the answer is not one, says the call failed, or holds no leaderboard.
pub fn created_from(answer: &Value) -> Result<Leaderboard, Error> {
    let result = field(answer, &["result"]).ok_or(Error::Unreadable("result"))?;
    succeeded(result)?;
    let entry = field(result, &["leaderboard"]).ok_or(Error::Unreadable("leaderboard"))?;
    // Steam answers a leaderboard it neither found nor made with the ID 0.
    if number(field(entry, &["leaderBoardID", "id"])).unwrap_or(0) == 0 {
        return Err(Error::Failed("Steam made no leaderboard".to_owned()));
    }
    leaderboard(entry)
}

/// A leaderboard in either of Steam's shapes: `GetLeaderboardsForGame`'s, or the prefixed one of
/// `FindOrCreateLeaderboard`.
fn leaderboard(entry: &Value) -> Result<Leaderboard, Error> {
    let name = text(entry, &["name", "leaderboardName"]);
    if name.is_empty() {
        return Err(Error::Unreadable("leaderboard name"));
    }
    let flag = |key: &str| {
        field(entry, &[key]).is_some_and(|value| {
            value
                .as_bool()
                .unwrap_or_else(|| number(Some(value)) == Some(1))
        })
    };
    Ok(Leaderboard {
        name,
        sort: text(entry, &["sortmethod", "leaderBoardSortMethod"]),
        display: text(entry, &["displaytype", "leaderBoardDisplayType"]),
        trusted_writes: flag("onlytrustedwrites"),
        friends_only: flag("onlyfriendsreads"),
        entries: number(field(entry, &["entries", "leaderBoardEntries"])).unwrap_or(0),
    })
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
                locked: field(entry, &["reqpassword"]).is_some_and(|value| {
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

/// Whether a method that changes something, such as `SetAppBuildLive`, answered that it did.
///
/// # Errors
///
/// When the answer is not one, or says the call failed.
pub fn changed_from(answer: &Value) -> Result<(), Error> {
    let response = field(answer, &["response"]).ok_or(Error::Unreadable("response"))?;
    succeeded(response)
}

/// An app a key reaches.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct App {
    pub app_id: u64,
    pub name: String,
}

/// The apps `GetPartnerAppListForWebAPIKey` answered the key reaches, by ID.
///
/// # Errors
///
/// When the answer is not one.
pub fn apps_from(answer: &Value) -> Result<Vec<App>, Error> {
    let apps = field(answer, &["applist"])
        .and_then(|list| field(list, &["apps"]))
        .ok_or(Error::Unreadable("applist"))?;
    let listed = if apps.is_array() {
        Some(apps)
    } else {
        field(apps, &["app"])
    };
    let entries: Vec<&Value> = match listed {
        Some(Value::Array(list)) => list.iter().collect(),
        Some(Value::Object(map)) => map.values().collect(),
        None => Vec::new(),
        Some(_) => return Err(Error::Unreadable("apps")),
    };
    let mut found: Vec<App> = entries
        .into_iter()
        .filter_map(|entry| {
            Some(App {
                app_id: number(field(entry, &["appid"]))?,
                name: text(entry, &["app_name", "name"]),
            })
        })
        .collect();
    found.sort_by_key(|app| app.app_id);
    Ok(found)
}

/// Why Steam gave `error` about `app`, when it is the one Steam gives while an app has no build
/// live on its default branch.
///
/// That is an HTTP 500 about an app the key holds, among `apps`: Steam answered so for Ostinato's
/// both before its first upload and after it, while that build was live nowhere, when it refused
/// an app the key did not hold instead, and answered for Fantasy Guild Manager's, which had one
/// live on default.
#[must_use]
pub fn no_build_yet(error: &Error, app: u32, apps: &[App]) -> Option<String> {
    (matches!(error, Error::Status(500, _))
        && apps.iter().any(|held| held.app_id == u64::from(app)))
    .then(|| {
        format!(
            "this key holds app {app}, and Steam answers so while no build is live on its \
                 default branch: set one live there in Steamworks under SteamPipe, Builds, and \
                 its branches and builds can be asked for"
        )
    })
}

/// Whether `branch` names the default branch.
#[must_use]
pub fn is_default(branch: &str) -> bool {
    DEFAULT
        .iter()
        .any(|default| branch.eq_ignore_ascii_case(default))
}

/// The name Steamworks shows for `branch`: the Web API calls the default branch `public`, which
/// the site, and every message of steamship's, calls `default`.
#[must_use]
pub fn shown_name(branch: &str) -> &str {
    if is_default(branch) {
        "default"
    } else {
        branch
    }
}

/// `branch` as a line: its name, the build live on it and its description, and whether it takes
/// a password.
#[must_use]
pub fn branch_line(branch: &Branch) -> String {
    let mut parts = vec![format!(
        "{}: BuildID {}",
        shown_name(&branch.name),
        branch.build_id
    )];
    if !branch.description.is_empty() {
        parts.push(branch.description.clone());
    }
    if branch.locked {
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
        .map(|branch| shown_name(&branch.name))
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

    /// The branches and builds of `app` as `builds --json` prints them. A build's `version` and
    /// `commit` are read back from the description `upload` wrote, and are null for a build
    /// described otherwise; `created` is null when Steam gave no time.
    #[must_use]
    pub fn json(&self, app: u32) -> Value {
        let branches: Vec<Value> = self
            .branches
            .iter()
            .map(|branch| {
                json!({
                    "name": shown_name(&branch.name),
                    "build_id": branch.build_id,
                    "description": branch.description,
                    "password": branch.locked,
                })
            })
            .collect();
        let builds: Vec<Value> = self
            .builds
            .iter()
            .map(|build| {
                let (version, commit) = version_and_commit(&build.description).unzip();
                let live_on: Vec<&str> = self
                    .branches
                    .iter()
                    .filter(|branch| branch.build_id == build.build_id)
                    .map(|branch| shown_name(&branch.name))
                    .collect();
                json!({
                    "build_id": build.build_id,
                    "created": (build.created > 0).then(|| timestamp(build.created)),
                    "description": build.description,
                    "version": version,
                    "commit": commit,
                    "live_on": live_on,
                })
            })
            .collect();
        json!({ "app": app, "branches": branches, "builds": builds })
    }
}

impl Schema {
    /// The achievements and stats of `app` as `achievements --json` prints them, each with how
    /// far players have got with it from `global`; an icon Steam gave none of, and a share or
    /// total it gave none of, is null.
    #[must_use]
    pub fn json(&self, app: u32, global: &Global) -> Value {
        let icon = |address: &str| (!address.is_empty()).then(|| address.to_owned());
        let achievements: Vec<Value> = self
            .achievements
            .iter()
            .map(|achievement| {
                json!({
                    "api_name": achievement.api_name,
                    "name": achievement.name,
                    "description": achievement.description,
                    "hidden": achievement.hidden,
                    "icon": icon(&achievement.icon),
                    "icon_locked": icon(&achievement.icon_locked),
                    "percent": global.shares.get(&achievement.api_name),
                })
            })
            .collect();
        let stats: Vec<Value> = self
            .stats
            .iter()
            .map(|stat| {
                let total = global
                    .totals
                    .get(&stat.api_name)
                    .and_then(Option::as_deref)
                    .and_then(|total| serde_json::from_str::<serde_json::Number>(total).ok());
                json!({
                    "api_name": stat.api_name,
                    "name": stat.name,
                    "default": stat.default,
                    "total": total,
                })
            })
            .collect();
        json!({ "app": app, "achievements": achievements, "stats": stats })
    }
}

/// The version and commit in a description `upload` wrote, which is the version, a space and
/// the commit Git abbreviated: 7 to 40 lowercase hexadecimal digits.
#[must_use]
pub fn version_and_commit(description: &str) -> Option<(&str, &str)> {
    let (version, commit) = description.rsplit_once(' ')?;
    let hexadecimal = (7..=40).contains(&commit.len())
        && commit
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte));
    (hexadecimal && !version.trim().is_empty()).then_some((version, commit))
}

/// `seconds` since 1970 as an ISO 8601 time in UTC, such as `2026-10-13T14:02:11Z`.
#[must_use]
pub fn timestamp(seconds: u64) -> String {
    let (year, month, day) = date(seconds);
    let of_day = seconds.rem_euclid(86_400);
    format!(
        "{year}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        of_day.div_euclid(3_600),
        of_day.rem_euclid(3_600).div_euclid(60),
        of_day.rem_euclid(60)
    )
}

/// Where a debug build, as the tests run, may be sent instead of Steam.
///
/// It names a stand-in on this machine, so that what follows a key Steam takes can be tested
/// without one. A release build has no such place and can reach Steam alone.
pub const STAND_IN: &str = "STEAMSHIP_WEB_API_STAND_IN";

/// The host the Web API is called at, where `lookup` reads the environment.
fn host<Lookup>(lookup: Lookup) -> String
where
    Lookup: Fn(&str) -> Option<String>,
{
    // A port and nothing after it: `http://127.0.0.1:80@example.com` names example.com.
    let loopback = |host: &String| {
        host.strip_prefix("http://127.0.0.1:")
            .is_some_and(|port| !port.is_empty() && port.bytes().all(|byte| byte.is_ascii_digit()))
    };
    if cfg!(debug_assertions)
        && let Some(stand_in) = lookup(STAND_IN).filter(loopback)
    {
        return stand_in;
    }
    HOST.to_owned()
}

/// Where Steam serves the images it makes of each app's achievement icons, by name, whether the
/// app is released or not. The `icon` addresses the Web API gives serve nothing until it is.
const IMAGES: &str = "https://shared.akamai.steamstatic.com/community_assets/images/apps";

/// The size an image of an icon has to stay under to be read. Steam serves them at 256 pixels
/// in a few kilobytes.
const IMAGE_LIMIT: u64 = 4 * 1024 * 1024;

/// The partner Web API, called with one key.
#[derive(Debug)]
pub struct Api {
    agent: ureq::Agent,
    key: Key,
    host: String,
    images: String,
}

impl Api {
    #[must_use]
    pub fn new(key: Key) -> Self {
        Self::at(key, &host(|name| env::var(name).ok()))
    }

    fn at(key: Key, host: &str) -> Self {
        let agent = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(30)))
            .http_status_as_error(false)
            .build()
            .into();
        let images = if host == HOST {
            IMAGES.to_owned()
        } else {
            format!("{host}/community_assets/images/apps")
        };
        Self {
            agent,
            key,
            host: host.to_owned(),
            images,
        }
    }

    /// The image Steam serves of one of `app`'s achievement icons, by the `name` it serves it
    /// under, as [`crate::editor::steam_image`] gives it. The image is public, so the key is not
    /// sent with the request.
    ///
    /// # Errors
    ///
    /// When `name` is not one Steam gives an image, or the image cannot be fetched, saying why.
    pub fn icon_image(&self, app: u32, name: &str) -> Result<Vec<u8>, String> {
        let named = name.strip_suffix(".jpg").is_some_and(|hash| {
            !hash.is_empty() && hash.bytes().all(|byte| byte.is_ascii_hexdigit())
        });
        if !named {
            return Err(format!("\"{name}\" is not the name of an image Steam made"));
        }
        let mut response = self
            .agent
            .get(format!("{}/{app}/{name}", self.images))
            .call()
            .ok()
            .ok_or("Steam's image host could not be reached")?;
        let status = response.status().as_u16();
        if !(200..=299).contains(&status) {
            return Err(format!("Steam's image host answered HTTP {status}"));
        }
        response
            .body_mut()
            .with_config()
            .limit(IMAGE_LIMIT)
            .read_to_vec()
            .map_err(|error| format!("Steam's image could not be read whole: {error}"))
    }

    /// The key the calls are made with.
    #[must_use]
    pub const fn key(&self) -> &Key {
        &self.key
    }

    /// The apps the key reaches, which says whether Steam takes it at all.
    ///
    /// # Errors
    ///
    /// When Steam cannot be reached, refuses the key, or answers with something else.
    pub fn apps(&self) -> Result<Vec<App>, Error> {
        apps_from(&self.get("GetPartnerAppListForWebAPIKey/v2", &[])?)
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

    /// The achievements and stats Steam holds for `app`, in `language` as Steam names it, such as
    /// `english` or `schinese`.
    ///
    /// # Errors
    ///
    /// As [`Api::branches`].
    pub fn schema(&self, app: u32, language: &str) -> Result<Schema, Error> {
        let answer = self.get_from(
            "ISteamUserStats",
            "GetSchemaForGame/v2",
            &[("appid", app.to_string()), ("l", language.to_owned())],
        )?;
        schema_from(&answer)
    }

    /// How far players have got with `app`'s achievements and with those of its `stats` Steam
    /// adds up over all players; nothing of either where Steam gives none, as for an app no
    /// player has yet.
    ///
    /// # Errors
    ///
    /// As [`Api::branches`], but for Steam refusing the shares, which is their being withheld.
    pub fn global(&self, app: u32, stats: &[String]) -> Result<Global, Error> {
        let shares = match self.get_from(
            "ISteamUserStats",
            "GetGlobalAchievementPercentagesForApp/v2",
            &[("gameid", app.to_string())],
        ) {
            Ok(answer) => shares_from(&answer)?,
            Err(Error::Refused) => BTreeMap::new(),
            Err(error) => return Err(error),
        };
        let totals = self.totalled(app, stats)?.totals;
        Ok(Global { shares, totals })
    }

    /// What Steam says of `app`'s `stats` it is asked to add up over all players: the total of
    /// each it adds up, and which it does not.
    ///
    /// Steam refuses a request for totals at the first stat it does not add up, naming it, so it
    /// is asked again without each it names.
    ///
    /// # Errors
    ///
    /// As [`Api::branches`].
    pub fn totalled(&self, app: u32, stats: &[String]) -> Result<Totalled, Error> {
        let mut totalled = Totalled::default();
        let mut asked: Vec<&String> = stats.iter().collect();
        while !asked.is_empty() {
            let keys: Vec<String> = (0..asked.len())
                .map(|index| format!("name[{index}]"))
                .collect();
            let mut query = vec![
                ("appid", app.to_string()),
                ("count", asked.len().to_string()),
            ];
            query.extend(
                keys.iter()
                    .map(String::as_str)
                    .zip(asked.iter().map(|stat| (*stat).clone())),
            );
            let answer = self.get_from("ISteamUserStats", "GetGlobalStatsForGame/v1", &query)?;
            match totals_from(&answer)? {
                Totals::NotAggregated(named) if asked.iter().any(|stat| **stat == named) => {
                    asked.retain(|stat| **stat != named);
                    let _: bool = totalled.unaggregated.insert(named);
                }
                Totals::Given(totals) => {
                    totalled.totals = totals;
                    break;
                }
                // Asked again, Steam would only name it again.
                Totals::NotAggregated(named) => {
                    totalled.withheld = Some(format!("Stat '{named}' is not an aggregated stat"));
                    break;
                }
                Totals::Withheld(said) => {
                    totalled.withheld = Some(said);
                    break;
                }
            }
        }
        Ok(totalled)
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
        // Steam answers two builds more than it is asked for.
        let mut builds = builds_from(&answer)?;
        builds.truncate(usize::try_from(count).unwrap_or(usize::MAX));
        Ok(builds)
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

    /// Replaces the rich presence of the languages in `input`, the `input_json` of
    /// `IProductInfoService/SetRichPresenceLocalization`.
    ///
    /// # Errors
    ///
    /// As [`Api::branches`], and when Steam's result says the call failed.
    pub fn set_rich_presence(&self, input: &Value) -> Result<(), Error> {
        self.post(
            "IProductInfoService/SetRichPresenceLocalization/v1",
            &[("input_json", input.to_string())],
        )
        .map(drop)
    }

    /// The leaderboards Steam holds for `app`, in Steam's order.
    ///
    /// # Errors
    ///
    /// As [`Api::branches`].
    pub fn leaderboards(&self, app: u32) -> Result<Vec<Leaderboard>, Error> {
        leaderboards_from(&self.get_from(
            "ISteamLeaderboards",
            "GetLeaderboardsForGame/v2",
            &[("appid", app.to_string())],
        )?)
    }

    /// Makes `wanted` for `app`, and answers the leaderboard as Steam then holds it. One that is
    /// already there is answered as it is: Steam never changes its settings.
    ///
    /// # Errors
    ///
    /// As [`Api::branches`], and when Steam's result says the call failed.
    pub fn create_leaderboard(&self, app: u32, wanted: &Leaderboard) -> Result<Leaderboard, Error> {
        let yes_no = |on: bool| if on { "true" } else { "false" }.to_owned();
        created_from(&self.post(
            "ISteamLeaderboards/FindOrCreateLeaderboard/v2",
            &[
                ("appid", app.to_string()),
                ("name", wanted.name.clone()),
                ("sortmethod", wanted.sort.clone()),
                ("displaytype", wanted.display.clone()),
                ("createifnotfound", yes_no(true)),
                ("onlytrustedwrites", yes_no(wanted.trusted_writes)),
                ("onlyfriendsreads", yes_no(wanted.friends_only)),
            ],
        )?)
    }

    fn post(&self, method: &str, form: &[(&str, String)]) -> Result<Value, Error> {
        let response = self
            .agent
            .post(format!("{}/{method}/", self.host))
            .header("x-webapi-key", self.key.0.as_str())
            .send_form(form.iter().map(|(name, value)| (*name, value.as_str())));
        // A service method can answer 200 and say it failed only in its result header.
        if let Ok(answer) = &response {
            let header = |name: &str| {
                answer
                    .headers()
                    .get(name)
                    .and_then(|value| value.to_str().ok())
                    .map(ToOwned::to_owned)
            };
            if let Some(result) = header("x-eresult").filter(|result| result != "1") {
                return Err(Error::Failed(header("x-error_message").map_or_else(
                    || format!("EResult {result}"),
                    |message| format!("EResult {result}, {message}"),
                )));
            }
        }
        read(response)
    }

    /// Sets the description players see for `app`'s beta `branch`.
    ///
    /// # Errors
    ///
    /// As [`Api::branches`], and when Steam refuses the change, as it does for the default
    /// branch.
    pub fn describe_branch(&self, app: u32, branch: &str, description: &str) -> Result<(), Error> {
        changed_from(&self.post(
            "ISteamApps/UpdateAppBranchDescription/v1",
            &[
                ("appid", app.to_string()),
                ("betakey", branch.to_owned()),
                ("description", description.to_owned()),
            ],
        )?)
    }

    /// Sets `build` of `app` live on `branch`.
    ///
    /// # Errors
    ///
    /// As [`Api::branches`], and when Steam refuses the change.
    pub fn set_live(&self, app: u32, build: u64, branch: &str) -> Result<(), Error> {
        changed_from(&self.post(
            "ISteamApps/SetAppBuildLive/v2",
            &[
                ("appid", app.to_string()),
                ("buildid", build.to_string()),
                ("betakey", branch.to_owned()),
            ],
        )?)
    }

    fn get(&self, method: &str, query: &[(&str, String)]) -> Result<Value, Error> {
        self.get_from("ISteamApps", method, query)
    }

    fn get_from(
        &self,
        interface: &str,
        method: &str,
        query: &[(&str, String)],
    ) -> Result<Value, Error> {
        let mut request = self
            .agent
            .get(format!("{}/{interface}/{method}/", self.host))
            .header("x-webapi-key", self.key.0.as_str());
        for (name, value) in query {
            request = request.query(*name, value);
        }
        // Steam's cache in front of some methods keeps an answer for up to an hour, heeds no
        // request to skip it, and its nodes hold different ones, so a leaderboard deleted minutes
        // earlier can still be listed. A query it has not seen is answered from Steam itself.
        let now = SystemTime::now().duration_since(UNIX_EPOCH);
        request = request.query(
            "steamship",
            now.map_or(0, |since| since.as_nanos()).to_string(),
        );
        read(request.call())
    }
}

/// The JSON Steam answered, where the status says it answered at all. The transport's own error
/// is never shown, as it can name the address.
fn read(response: Result<Response<Body>, ureq::Error>) -> Result<Value, Error> {
    let mut response = response.ok().ok_or(Error::Unreachable)?;
    let status = response.status().as_u16();
    if matches!(status, 401 | 403) {
        return Err(Error::Refused);
    }
    let body = response.body_mut().read_to_string().ok();
    let answer = body.and_then(|body| serde_json::from_str::<Value>(&body).ok());
    if !(200..=299).contains(&status) {
        return Err(Error::Status(
            status,
            answer.as_ref().map(said).unwrap_or_default(),
        ));
    }
    answer.ok_or(Error::Unreadable("JSON"))
}

/// The reason Steam gave in an answer, such as "Unable to find specificed betakey x" with a 404;
/// empty when it gave none.
fn said(answer: &Value) -> String {
    field(answer, &["response", "result"])
        .map(|inner| text(inner, &["message"]))
        .unwrap_or_default()
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
        assert_eq!(format!("{key:?}"), "Key { .. }");
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
                locked: false,
            },
            Branch {
                name: "testing".to_owned(),
                build_id: 1234,
                description: "0.1.0 9b3d543".to_owned(),
                locked: true,
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
        let empty =
            answer(r#"{"response": {"betas": {"new": {"BuildID": null, "Description": 7}}}}"#);
        assert_eq!(
            branches_from(&empty).unwrap(),
            [Branch {
                name: "new".to_owned(),
                build_id: 0,
                description: String::new(),
                locked: false,
            }]
        );
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
            changed_from(&answer(
                r#"{"response": {"result": 2, "message": "Invalid build"}}"#
            )),
            Err(Error::Failed("Invalid build".to_owned()))
        );
        assert_eq!(
            changed_from(&answer(r#"{"response": {"result": 8}}"#)),
            Err(Error::Failed("result 8".to_owned()))
        );
        assert_eq!(
            changed_from(&answer(r#"{"response": {"result": 1}}"#)),
            Ok(())
        );
        assert_eq!(changed_from(&answer(r#"{"response": {}}"#)), Ok(()));
    }

    #[test]
    fn errors_say_what_happened_and_never_an_address() {
        for (error, said) in [
            (
                Error::Refused,
                "Steam refused the Web API key; it must be the publisher key of a group that \
                 holds the app",
            ),
            (Error::Status(500, String::new()), "Steam answered HTTP 500"),
            (
                Error::Status(404, "Unable to find specificed betakey x".to_owned()),
                "Steam answered HTTP 404: Unable to find specificed betakey x",
            ),
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

    fn branch(name: &str, build_id: u64, locked: bool) -> Branch {
        Branch {
            name: name.to_owned(),
            build_id,
            description: format!("{build_id} description"),
            locked,
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

    #[test]
    fn a_schema_is_read_as_steam_answers_it_and_none_is_none() {
        // As Steam answered for Spacewar, trimmed to two achievements; its stats shaped as they.
        let schema = schema_from(&answer(
            r#"{"game": {"gameName": "Spacewar", "availableGameStats": {"achievements": [
                {"name": "ACH_WIN_ONE_GAME", "defaultvalue": 0, "displayName": "Winner",
                 "hidden": 0, "description": "Win one game.", "icon": "https://cdn/winner.jpg",
                 "icongray": "https://cdn/winner_bw.jpg"},
                {"name": "ACH_TRAVEL_FAR_SINGLE", "displayName": "Orbiter", "hidden": "1"}
            ], "stats": [
                {"name": "NumGames", "defaultvalue": 0, "displayName": "Games played"},
                {"name": "AverageSpeed", "defaultvalue": 1.5},
                {"name": "Given", "defaultvalue": "7", "displayName": ""}
            ]}}}"#,
        ))
        .unwrap();
        let stat = |api_name: &str, name: &str, default: f64| Stat {
            api_name: api_name.to_owned(),
            name: name.to_owned(),
            default,
        };
        assert_eq!(
            schema.stats,
            [
                stat("NumGames", "Games played", 0.0_f64),
                stat("AverageSpeed", "", 1.5_f64),
                stat("Given", "", 7.0_f64),
            ]
        );
        assert_eq!(
            schema.achievements,
            [
                Achievement {
                    api_name: "ACH_WIN_ONE_GAME".to_owned(),
                    name: "Winner".to_owned(),
                    description: "Win one game.".to_owned(),
                    hidden: false,
                    icon: "https://cdn/winner.jpg".to_owned(),
                    icon_locked: "https://cdn/winner_bw.jpg".to_owned(),
                },
                Achievement {
                    api_name: "ACH_TRAVEL_FAR_SINGLE".to_owned(),
                    name: "Orbiter".to_owned(),
                    description: String::new(),
                    hidden: true,
                    icon: String::new(),
                    icon_locked: String::new(),
                },
            ]
        );
        // As Steam answered for Fantasy Guild Manager before its achievements were entered.
        assert_eq!(
            schema_from(&answer(r#"{"game": {}}"#)).unwrap(),
            Schema::default()
        );
        assert_eq!(
            schema_from(&answer(
                r#"{"game": {"availableGameStats": {"achievements": [{"displayName": "x"}]}}}"#
            )),
            Err(Error::Unreadable("achievement name"))
        );
        assert_eq!(
            schema_from(&answer(
                r#"{"game": {"availableGameStats": {"stats": [{"displayName": "x"}]}}}"#
            )),
            Err(Error::Unreadable("stat name"))
        );
        assert_eq!(
            schema_from(&answer(
                r#"{"game": {"availableGameStats": {"stats": [{"name": "x"}]}}}"#
            ))
            .unwrap()
            .stats,
            [Stat {
                api_name: "x".to_owned(),
                name: String::new(),
                default: 0.0
            }],
            "no default given is Steam's 0"
        );
    }

    #[test]
    fn leaderboards_are_read_in_both_of_steams_shapes() {
        // As Steam answered for Fantasy Guild Manager, with one leaderboard made for a trial.
        let listed = leaderboards_from(&answer(
            r#"{"response": {"result": 1, "leaderboards": [
                {"id": 21153543, "name": "steamship_probe", "entries": 3,
                 "sortmethod": "Ascending", "displaytype": "", "onlytrustedwrites": true,
                 "onlyfriendsreads": true, "onlyusersinsameparty": false,
                 "limitrangearounduser": 0, "limitglobaltopentries": 0},
                {"id": 21153555, "name": "gold", "sortmethod": "Descending",
                 "displaytype": "Numeric", "onlytrustedwrites": 0, "onlyfriendsreads": "1"}
            ]}}"#,
        ))
        .unwrap();
        let probe = Leaderboard {
            name: "steamship_probe".to_owned(),
            sort: "Ascending".to_owned(),
            display: String::new(),
            trusted_writes: true,
            friends_only: true,
            entries: 3,
        };
        let gold = Leaderboard {
            name: "gold".to_owned(),
            sort: "Descending".to_owned(),
            display: "Numeric".to_owned(),
            trusted_writes: false,
            friends_only: true,
            entries: 0,
        };
        assert_eq!(listed, [probe.clone(), gold]);
        assert_eq!(
            leaderboards_from(&answer(
                r#"{"response": {"result": 1, "leaderboards": []}}"#
            ))
            .unwrap(),
            []
        );
        assert!(matches!(
            leaderboards_from(&answer(r#"{"response": {"leaderboards": [{"id": 1}]}}"#)),
            Err(Error::Unreadable("leaderboard name"))
        ));
        assert_eq!(
            leaderboards_from(&answer(r#"{"response": {"result": 8}}"#)),
            Err(Error::Failed("result 8".to_owned()))
        );

        let made = r#"{"result": {"result": 1, "leaderboard": {"leaderboardName": "steamship_probe",
            "leaderBoardID": 21153543, "leaderBoardEntries": 3, "leaderBoardSortMethod": "Ascending",
            "leaderBoardDisplayType": "", "onlytrustedwrites": true, "onlyfriendsreads": true}}}"#;
        assert_eq!(created_from(&answer(made)).unwrap(), probe);
        assert_eq!(
            created_from(&answer(&made.replace("21153543", "0"))),
            Err(Error::Failed("Steam made no leaderboard".to_owned())),
            "the ID Steam gives one it neither found nor made"
        );
        assert_eq!(
            created_from(&answer(r#"{"result": {"result": 1}}"#)),
            Err(Error::Unreadable("leaderboard"))
        );
        assert_eq!(
            created_from(&answer(r#"{"response": {}}"#)),
            Err(Error::Unreadable("result"))
        );
        assert_eq!(
            created_from(&answer(r#"{"result": {"result": 2, "message": "no"}}"#)),
            Err(Error::Failed("no".to_owned()))
        );
    }

    #[test]
    fn a_500_about_an_app_the_key_holds_is_an_app_with_no_build_yet() {
        let apps = [App {
            app_id: 5_335_970,
            name: "Ostinato".to_owned(),
        }];
        let why = no_build_yet(&Error::Status(500, String::new()), 5_335_970, &apps).unwrap();
        assert!(
            why.starts_with("this key holds app 5335970, and Steam"),
            "{why}"
        );
        assert_eq!(
            no_build_yet(&Error::Status(500, String::new()), 480, &apps),
            None
        );
        for other in [
            Error::Status(503, String::new()),
            Error::Refused,
            Error::Unreachable,
        ] {
            assert_eq!(no_build_yet(&other, 5_335_970, &apps), None, "{other:?}");
        }
    }

    #[test]
    fn the_default_branch_is_shown_by_the_name_steamworks_gives_it() {
        // As Steam's Web API answered for Fantasy Guild Manager.
        let public = Branch {
            description: "Public default branch".to_owned(),
            ..branch("public", 25_585_928, false)
        };
        assert_eq!(
            branch_line(&public),
            "default: BuildID 25585928, Public default branch"
        );
        let build = Build {
            build_id: 25_585_928,
            description: "0.1.0-test b4ca836e0512".to_owned(),
            created: 0,
        };
        assert_eq!(
            build_line(&build, &[public]),
            "1970-01-01, 0.1.0-test b4ca836e0512, live on default"
        );
        assert_eq!(shown_name("Public"), "default");
        assert_eq!(shown_name("testing"), "testing");
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

    /// The next request the server read. The server thread keeps the sender while it waits to
    /// accept, so a call that never reaches it would otherwise leave the test waiting forever.
    fn received(requests: &mpsc::Receiver<String>) -> String {
        requests
            .recv_timeout(Duration::from_secs(10))
            .expect("the server was sent a request")
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
    fn icon_images_come_from_steams_image_host_and_only_by_an_image_name() {
        assert_eq!(api(HOST).images, IMAGES);
        assert_eq!(
            api("http://127.0.0.1:1").images,
            "http://127.0.0.1:1/community_assets/images/apps"
        );
        let limit = usize::try_from(IMAGE_LIMIT).unwrap();
        let (host, _requests) = serving(vec![
            ("200 OK", "0".repeat(limit.saturating_sub(1)).leak()),
            ("200 OK", "0".repeat(limit).leak()),
        ]);
        assert_eq!(
            api(&host).icon_image(1, "ab.jpg").map(|image| image.len()),
            Ok(4 * 1024 * 1024 - 1)
        );
        let over = api(&host).icon_image(1, "ab.jpg").unwrap_err();
        assert!(
            over.starts_with("Steam's image could not be read whole: "),
            "{over}"
        );
        for name in ["", ".jpg", "../keys.jpg", "ab.png", "ab.jpg?x", "ab"] {
            assert_eq!(
                api("http://127.0.0.1:1").icon_image(1, name),
                Err(format!("\"{name}\" is not the name of an image Steam made")),
                "{name}"
            );
        }
    }

    #[test]
    fn the_api_is_steams_partner_host_unless_a_test_names_another() {
        assert_eq!(HOST, "https://partner.steam-api.com");
        assert_eq!(host(|_| None), HOST);
        let named =
            |value: &'static str| host(move |name| (name == STAND_IN).then(|| value.to_owned()));
        assert_eq!(named("http://127.0.0.1:4321"), "http://127.0.0.1:4321");
        for elsewhere in [
            "https://partner.steam-api.com.example",
            "http://127.0.0.2:80",
            "http://localhost:80",
            "https://127.0.0.1:443",
            "http://127.0.0.1:80@partner.steam-api.com.example",
            "http://127.0.0.1:",
            "http://127.0.0.1:80/",
        ] {
            assert_eq!(named(elsewhere), HOST, "{elsewhere}");
        }
    }

    #[test]
    fn branches_are_asked_for_with_the_key_in_a_header_and_never_in_the_address() {
        let (host, requests) = answering(
            "200 OK",
            r#"{"response": {"result": 1, "betas": {"testing": {"BuildID": 1234}}}}"#,
        );
        let branches = api(&host).branches(5_335_950).unwrap();
        assert_eq!(branches.first().map(|branch| branch.build_id), Some(1234));
        let request = received(&requests);
        let first = request.lines().next().unwrap();
        let fresh = first
            .strip_prefix("GET /ISteamApps/GetAppBetas/v1/?appid=5335950&steamship=")
            .and_then(|rest| rest.strip_suffix(" HTTP/1.1"));
        assert!(
            fresh.is_some_and(|nanos| nanos.parse::<u128>().is_ok_and(|nanos| nanos > 0)),
            "asked past Steam's cache, by the time: {first}"
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
        assert!(received(&requests).contains("GetAppBetas"));
        assert!(received(&requests).contains("GetAppBuilds"));
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
        let request = received(&requests);
        assert!(
            request
                .starts_with("GET /ISteamApps/GetAppBuilds/v1/?appid=5335950&count=25&steamship="),
            "{request}"
        );
    }

    #[test]
    fn only_the_newest_builds_asked_for_are_kept_of_the_two_more_steam_answers() {
        let (host, _requests) = answering(
            "200 OK",
            r#"{"response": {"builds": {"1232": {}, "1234": {}, "1233": {}}}}"#,
        );
        let builds = api(&host).builds(5_335_950, 1).unwrap();
        assert_eq!(
            builds
                .iter()
                .map(|build| build.build_id)
                .collect::<Vec<_>>(),
            [1234]
        );
    }

    #[test]
    fn a_build_is_set_live_with_a_form_that_holds_no_key() {
        let (host, requests) = answering("200 OK", r#"{"response": {"result": 1}}"#);
        api(&host).set_live(5_335_950, 1234, "testing").unwrap();
        let request = received(&requests);
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
    fn a_key_is_checked_by_the_apps_it_reaches_asked_for_with_the_key_in_a_header() {
        let (host, requests) = answering(
            "200 OK",
            r#"{"applist": {"apps": {"app": [
                {"appid": 5335970, "app_type": "game", "app_name": "Ostinato"},
                {"appid": 5335950, "app_type": "game", "app_name": "Fantasy Guild Manager"}
            ]}}}"#,
        );
        let checked = api(&host);
        assert_eq!(checked.key().text(), KEY_TEXT);
        let apps = checked.apps().unwrap();
        assert_eq!(
            apps,
            [
                App {
                    app_id: 5_335_950,
                    name: "Fantasy Guild Manager".to_owned()
                },
                App {
                    app_id: 5_335_970,
                    name: "Ostinato".to_owned()
                },
            ]
        );
        let request = received(&requests);
        assert!(
            request.starts_with("GET /ISteamApps/GetPartnerAppListForWebAPIKey/v2/?steamship="),
            "{request}"
        );
        assert_eq!(
            request.matches(KEY_TEXT).count(),
            1,
            "the header alone: {request}"
        );
    }

    #[test]
    fn apps_are_read_from_a_list_or_a_map_and_none_is_none() {
        let app = |id| App {
            app_id: id,
            name: String::new(),
        };
        assert_eq!(
            apps_from(&answer(r#"{"applist": {"apps": [{"appid": "7"}]}}"#)),
            Ok(vec![app(7)])
        );
        assert_eq!(
            apps_from(&answer(
                r#"{"applist": {"apps": {"app": {"a": {"appid": 9}, "b": {}}}}}"#
            )),
            Ok(vec![app(9)])
        );
        assert_eq!(
            apps_from(&answer(r#"{"applist": {"apps": {}}}"#)),
            Ok(Vec::new())
        );
        assert_eq!(
            apps_from(&answer(r#"{"applist": {"apps": {"app": 3}}}"#)),
            Err(Error::Unreadable("apps"))
        );
        assert_eq!(
            apps_from(&answer(r#"{"response": {}}"#)),
            Err(Error::Unreadable("applist"))
        );
    }

    #[test]
    fn a_refused_key_a_failed_status_and_an_answer_that_is_not_json_are_errors() {
        for (status, body, expected) in [
            ("403 Forbidden", "", Error::Refused),
            // Only the code is read; the phrase after it is the server's to choose.
            ("401 Refused", "", Error::Refused),
            (
                "500 Internal Server Error",
                "",
                Error::Status(500, String::new()),
            ),
            // As Steam answered UpdateAppBranchDescription for a branch the app lacks.
            (
                "404 Not Found",
                r#"{"response":{"result":2,"message":"Unable to find specificed betakey x"}}"#,
                Error::Status(404, "Unable to find specificed betakey x".to_owned()),
            ),
            (
                "400 Bad Request",
                r#"{"result":{"result":8,"message":"Invalid parameter"}}"#,
                Error::Status(400, "Invalid parameter".to_owned()),
            ),
            (
                "503 Service Unavailable",
                "<html>busy</html>",
                Error::Status(503, String::new()),
            ),
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

    #[test]
    fn a_time_is_written_in_iso_8601_to_the_second_in_utc() {
        assert_eq!(timestamp(0), "1970-01-01T00:00:00Z");
        assert_eq!(timestamp(1_790_553_599), "2026-09-27T23:59:59Z");
        assert_eq!(timestamp(1_791_910_891), "2026-10-13T17:01:31Z");
    }

    #[test]
    fn only_a_description_upload_wrote_gives_a_version_and_commit() {
        for (description, expected) in [
            ("0.1.0 960e3285f582", Some(("0.1.0", "960e3285f582"))),
            ("1.4.0 beta 2 abc1234", Some(("1.4.0 beta 2", "abc1234"))),
            (
                "v2 0123456789abcdef0123456789abcdef01234567",
                Some(("v2", "0123456789abcdef0123456789abcdef01234567")),
            ),
            ("0.1.0", None),
            ("0.1.0 abc123", None),
            ("0.1.0 0123456789abcdef0123456789abcdef012345678", None),
            ("0.1.0 960E3285F582", None),
            ("0.1.0 960e3285g582", None),
            (" 960e3285f582", None),
            ("Public default branch", None),
        ] {
            assert_eq!(version_and_commit(description), expected, "{description:?}");
        }
    }

    #[test]
    fn an_icon_share_or_total_steam_gave_none_of_is_null() {
        let schema = Schema {
            achievements: vec![Achievement {
                api_name: "first_job".to_owned(),
                name: "Open for business".to_owned(),
                description: String::new(),
                hidden: false,
                icon: "https://cdn/w.jpg".to_owned(),
                icon_locked: String::new(),
            }],
            stats: ["NumGames", "FeetTraveled", "NumWins"]
                .map(|api_name| Stat {
                    api_name: api_name.to_owned(),
                    name: String::new(),
                    default: 0.0_f64,
                })
                .to_vec(),
        };
        assert_eq!(
            schema.json(5_335_950, &Global::default()),
            json!({"app": 5_335_950_u32, "achievements": [{
                "api_name": "first_job", "name": "Open for business", "description": "",
                "hidden": false, "icon": "https://cdn/w.jpg", "icon_locked": null, "percent": null,
            }], "stats": [
                {"api_name": "NumGames", "name": "", "default": 0.0_f64, "total": null},
                {"api_name": "FeetTraveled", "name": "", "default": 0.0_f64, "total": null},
                {"api_name": "NumWins", "name": "", "default": 0.0_f64, "total": null},
            ]})
        );
        let global = Global {
            shares: BTreeMap::from([("first_job".to_owned(), 49.6_f64)]),
            totals: BTreeMap::from([
                ("NumGames".to_owned(), Some("211140488449".to_owned())),
                ("FeetTraveled".to_owned(), None),
            ]),
        };
        let printed = schema.json(5_335_950, &global);
        assert_eq!(
            printed.pointer("/achievements/0/percent"),
            Some(&json!(49.6_f64))
        );
        assert_eq!(
            printed.pointer("/stats/0/total"),
            Some(&json!(211_140_488_449_u64))
        );
        assert_eq!(printed.pointer("/stats/1/total"), Some(&Value::Null));
        assert_eq!(printed.pointer("/stats/2/total"), Some(&Value::Null));
    }

    #[test]
    fn the_share_of_players_with_each_achievement_is_read_as_steam_gives_it() {
        // As Steam answered for Team Fortress 2, cut to two.
        let answer = json!({"achievementpercentages": {"achievements": [
            {"name": "TF_SCOUT_LONG_DISTANCE_RUNNER", "percent": "49.6"},
            {"name": "TF_HEAVY_DAMAGE_TAKEN", "percent": 40.0_f64}
        ]}});
        assert_eq!(
            shares_from(&answer),
            Ok(BTreeMap::from([
                ("TF_HEAVY_DAMAGE_TAKEN".to_owned(), 40.0_f64),
                ("TF_SCOUT_LONG_DISTANCE_RUNNER".to_owned(), 49.6_f64),
            ]))
        );
        assert_eq!(shares_from(&json!({})), Ok(BTreeMap::new()));
        assert_eq!(
            shares_from(&json!({"achievementpercentages": {"achievements": [{"name": "A"}]}})),
            Err(Error::Unreadable("achievement share"))
        );
        assert_eq!(
            shares_from(&json!({"achievementpercentages": {"achievements": [{"percent": "1.0"}]}})),
            Err(Error::Unreadable("achievement share"))
        );
    }

    #[test]
    fn global_totals_are_read_with_the_stat_steam_does_not_add_up_named() {
        // Each as Steam answered for Spacewar.
        assert_eq!(
            totals_from(&json!({"response": {"globalstats": {
                "NumGames": {"total": "211140488449"}, "FeetTraveled": {"total": null}
            }, "result": 1_i32}})),
            Ok(Totals::Given(BTreeMap::from([
                ("FeetTraveled".to_owned(), None),
                ("NumGames".to_owned(), Some("211140488449".to_owned())),
            ])))
        );
        assert_eq!(
            totals_from(
                &json!({"response": {"globalstats": {"N": {"total": 7_i32}}, "result": 1_i32}})
            ),
            Ok(Totals::Given(BTreeMap::from([(
                "N".to_owned(),
                Some("7".to_owned())
            )])))
        );
        assert_eq!(
            totals_from(&json!({"response": {"result": 8_i32,
                "error": "Stat 'AverageSpeed' is not an aggregated stat"}})),
            Ok(Totals::NotAggregated("AverageSpeed".to_owned()))
        );
        // As Steam answered for Tall Tales Inn before its release.
        assert_eq!(
            totals_from(
                &json!({"response": {"result": 24_i32, "error": "Insufficient Privilege"}})
            ),
            Ok(Totals::Withheld("Insufficient Privilege".to_owned()))
        );
        assert_eq!(
            totals_from(&json!({"response": {"result": 1_i32}})),
            Err(Error::Unreadable("global stats"))
        );
        assert_eq!(
            totals_from(&json!({})),
            Err(Error::Unreadable("global stats"))
        );
    }

    #[test]
    fn a_build_with_no_time_is_created_null_and_one_live_on_both_names_both() {
        let overview = Overview {
            branches: vec![
                Branch {
                    name: "public".to_owned(),
                    build_id: 7,
                    description: String::new(),
                    locked: false,
                },
                Branch {
                    name: "testing".to_owned(),
                    build_id: 7,
                    description: String::new(),
                    locked: false,
                },
            ],
            builds: vec![Build {
                build_id: 7,
                description: "0.1.0".to_owned(),
                created: 0,
            }],
        };
        assert_eq!(
            overview.json(480).pointer("/builds/0"),
            Some(&json!({
                "build_id": 7_u64, "created": null, "description": "0.1.0", "version": null,
                "commit": null, "live_on": ["default", "testing"],
            }))
        );
    }

    #[test]
    fn every_failure_has_a_code_of_its_own() {
        let codes: Vec<&str> = [
            Error::NotAKey,
            Error::Refused,
            Error::Status(500, String::new()),
            Error::Unreachable,
            Error::Unreadable("betas"),
            Error::Failed("no".to_owned()),
        ]
        .iter()
        .map(Error::code)
        .collect();
        assert_eq!(
            codes,
            [
                "not-a-key",
                "key-refused",
                "http-error",
                "unreachable",
                "unreadable",
                "steam-failed"
            ]
        );
        let held = error_json(
            5_335_970,
            &Error::Status(500, String::new()),
            Some("none live"),
        );
        assert_eq!(
            held,
            json!({"app": 5_335_970_u32, "error": {
                "code": "no-build-live",
                "message": "Steam answered HTTP 500; none live",
            }})
        );
    }
}
