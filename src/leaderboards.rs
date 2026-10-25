//! An app's leaderboards as a file in its repository says they should be, against Steam's.
//!
//! Steam makes a leaderboard it lacks as the file has it, but keeps an existing one's settings,
//! and its scores with them; so one that differs is reported, never changed or made again.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use serde_json::Value;

use crate::show;
use crate::webapi::Leaderboard;

/// The longest name Steam takes, in bytes.
const LONGEST: usize = 128;

/// What the file holds: the app it is for, if it says, the leaderboards in its order, and the
/// templates of dated leaderboards.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Listed {
    pub app: Option<u64>,
    pub leaderboards: Vec<Leaderboard>,
    pub templates: Vec<Template>,
}

/// How often a games-kit leaderboard with a `period` starts a new board.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Period {
    Day,
    Week,
    Month,
}

impl Period {
    /// What the game puts after the template's name and an `_`: `20261025` for a day, the ISO
    /// week `2026W43`, or `202610` for a month, each by the UTC date.
    fn dates(self, suffix: &str) -> bool {
        let digits = |part: &[u8]| part.iter().all(u8::is_ascii_digit);
        let bytes = suffix.as_bytes();
        match self {
            Self::Day => bytes.len() == 8 && digits(bytes),
            Self::Week => match bytes {
                [year @ .., b'W', week_tens, week_ones] => {
                    year.len() == 4 && digits(year) && digits(&[*week_tens, *week_ones])
                }
                _ => false,
            },
            Self::Month => bytes.len() == 6 && digits(bytes),
        }
    }

    /// The bytes the game adds to the template's name, its `_` included.
    const fn added(self) -> usize {
        match self {
            Self::Day => 9,
            Self::Week => 8,
            Self::Month => 7,
        }
    }
}

/// A games-kit leaderboard record with a `period`: the game makes one board each period, named
/// after the template and the date, with its sort and display. No board has the template's own
/// name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Template {
    pub board: Leaderboard,
    pub period: Period,
}

impl Template {
    /// Whether the game made the board `name` from this template.
    #[must_use]
    pub fn made(&self, name: &str) -> bool {
        name.strip_prefix(self.board.name.as_str())
            .and_then(|rest| rest.strip_prefix('_'))
            .is_some_and(|suffix| self.period.dates(suffix))
    }
}

/// The leaderboards and templates of every file, as [`together`] gathers them.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Wanted {
    pub leaderboards: Vec<Leaderboard>,
    pub templates: Vec<Template>,
}

impl Wanted {
    /// Whether the file holds `name`, as a leaderboard or as one a template makes.
    #[must_use]
    pub fn holds(&self, name: &str) -> bool {
        self.leaderboards
            .iter()
            .any(|leaderboard| leaderboard.name == name)
            || self.templates.iter().any(|template| template.made(name))
    }
}

/// Reads the file's `text`: an object with a `leaderboards` list, or a list of games-kit's
/// records, of which those typed `leaderboard` are read by the name in `steam`.
///
/// Each entry has its `name`, `sort` (`ascending` or `descending`) and `display` (`numeric`,
/// `seconds` or `milliseconds`), `trusted_writes` and `friends_only` when they are set, and a
/// `period` that makes it a [`Template`]. Other
/// keys, among them a record's `upload` and `details`, which the game sends with each score, are
/// left alone.
///
/// # Errors
///
/// When the text is neither, an entry lacks its name, sort or display or has one Steam does not
/// take, a value has the wrong type, or a name is there twice.
pub fn read(text: &str) -> Result<Listed, String> {
    let file: Value = serde_json::from_str(text).map_err(|error| format!("not JSON: {error}"))?;
    let mut entries = Vec::new();
    let app = if let Value::Array(records) = &file {
        for (index, record) in records.iter().enumerate() {
            let label = format!("record {}", index.saturating_add(1));
            let Some(Value::String(kind)) = record.get("type") else {
                return Err(format!("{label} has no \"type\" text"));
            };
            if kind == "leaderboard" {
                entries.push(leaderboard(record, &label, "steam")?);
            }
        }
        None
    } else {
        let listed = file
            .get("leaderboards")
            .and_then(Value::as_array)
            .ok_or("there is no \"leaderboards\" list")?;
        for (index, entry) in listed.iter().enumerate() {
            let label = format!("leaderboard {}", index.saturating_add(1));
            entries.push(leaderboard(entry, &label, "name")?);
        }
        match file.get("app") {
            None => None,
            Some(app) => Some(app.as_u64().ok_or("\"app\" is not an app ID")?),
        }
    };
    let mut seen = BTreeSet::new();
    if let Some((again, _)) = entries.iter().find(|(board, _)| !seen.insert(&board.name)) {
        return Err(format!("{} is in the file twice", again.name));
    }
    let mut leaderboards = Vec::new();
    let mut templates = Vec::new();
    for (board, period) in entries {
        match period {
            None => leaderboards.push(board),
            Some(period) => templates.push(Template { board, period }),
        }
    }
    Ok(Listed {
        app,
        leaderboards,
        templates,
    })
}

/// The leaderboard `entry`, its name under `named_by`, called `label` in what is wrong with it
/// until that name is known; and its period when it is a template.
fn leaderboard(
    entry: &Value,
    label: &str,
    named_by: &str,
) -> Result<(Leaderboard, Option<Period>), String> {
    let name = match entry.get(named_by) {
        Some(Value::String(name)) if !name.is_empty() => name.clone(),
        _ => return Err(format!("{label} has no \"{named_by}\" text")),
    };
    if name.len() > LONGEST {
        return Err(format!("{name}: longer than Steam's {LONGEST} bytes"));
    }
    let one_of = |key: &str, words: &[&str]| -> Result<String, String> {
        let given = entry.get(key).and_then(Value::as_str).unwrap_or_default();
        words
            .iter()
            .find(|word| word.eq_ignore_ascii_case(given))
            .map(|&word| word.to_owned())
            .ok_or_else(|| {
                let lowered: Vec<String> =
                    words.iter().map(|word| word.to_ascii_lowercase()).collect();
                format!("{name}: \"{key}\" is not {}", or_list(&lowered))
            })
    };
    let flag = |key: &str| match entry.get(key) {
        None => Ok(false),
        Some(Value::Bool(on)) => Ok(*on),
        Some(_) => Err(format!("{name}: \"{key}\" is not true or false")),
    };
    let period = if entry.get("period").is_some() {
        match one_of("period", &["none", "day", "week", "month"])?.as_str() {
            "day" => Some(Period::Day),
            "week" => Some(Period::Week),
            "month" => Some(Period::Month),
            _ => None,
        }
    } else {
        None
    };
    let board = Leaderboard {
        sort: one_of("sort", &["Ascending", "Descending"])?,
        display: one_of("display", &["Numeric", "Seconds", "MilliSeconds"])?,
        trusted_writes: flag("trusted_writes")?,
        friends_only: flag("friends_only")?,
        entries: 0,
        name,
    };
    if let Some(period) = period {
        if board.name.len().saturating_add(period.added()) > LONGEST {
            return Err(format!(
                "{}: longer than Steam's {LONGEST} bytes once the game adds the date",
                board.name
            ));
        }
        if board.trusted_writes || board.friends_only {
            return Err(format!(
                "{}: the game makes a board each period, which cannot have trusted writes or \
                 friends only, as Steamworks sets them board by board",
                board.name
            ));
        }
    }
    Ok((board, period))
}

/// The leaderboards of the `files`, each read and named, of each in turn.
///
/// # Errors
///
/// When a leaderboard or template is in two of the files, naming both, or a leaderboard has a
/// name one of the templates makes, so that two records would write to one board.
pub fn together(files: Vec<(String, Listed)>) -> Result<Wanted, String> {
    let mut whole = Wanted::default();
    let mut from: BTreeMap<String, String> = BTreeMap::new();
    for (file, listed) in files {
        let names = listed
            .leaderboards
            .iter()
            .chain(listed.templates.iter().map(|template| &template.board));
        for wanted in names {
            if let Some(first) = from.insert(wanted.name.clone(), file.clone()) {
                return Err(format!("{} is in both {first} and {file}", wanted.name));
            }
        }
        whole.leaderboards.extend(listed.leaderboards);
        whole.templates.extend(listed.templates);
    }
    for leaderboard in &whole.leaderboards {
        if let Some(template) = whole
            .templates
            .iter()
            .find(|template| template.made(&leaderboard.name))
        {
            return Err(format!(
                "{} is a name the game gives a board of {} on some date, so the two would share \
                 one board",
                leaderboard.name, template.board.name
            ));
        }
    }
    Ok(whole)
}

/// `words` as a sentence gives a choice: `a`, `a or b`, `a, b or c`.
fn or_list(words: &[String]) -> String {
    match words.split_last() {
        Some((last, rest)) if !rest.is_empty() => format!("{} or {last}", rest.join(", ")),
        _ => words.join(""),
    }
}

/// A way Steam's leaderboards and the file's differ.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Drift {
    /// In the file, not on Steam.
    NotOnSteam(String),
    /// On Steam, not in the file.
    NotInFile(String),
    /// A setting that is one thing on Steam and another in the file.
    Differs {
        name: String,
        field: &'static str,
        steam: String,
        file: String,
    },
}

impl fmt::Display for Drift {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotOnSteam(name) => {
                write!(
                    formatter,
                    "{name}: in the file, not on Steam; --create makes it"
                )
            }
            Self::NotInFile(name) => write!(
                formatter,
                "{name}: on Steam, not in the file; --delete-missing deletes it while the app is \
                 unreleased"
            ),
            Self::Differs {
                name,
                field,
                steam,
                file,
            } => write!(
                formatter,
                "{name}: {field} is \"{steam}\" on Steam and \"{file}\" in the file"
            ),
        }
    }
}

/// `leaderboard` as `steamship leaderboards` lists it: its sort, its display, its scores, and
/// who may write and read them when that is narrowed.
#[must_use]
pub fn line(leaderboard: &Leaderboard) -> String {
    let mut line = format!(
        "{}, {}, {}",
        leaderboard.sort.to_ascii_lowercase(),
        display(&leaderboard.display),
        show::counted(
            usize::try_from(leaderboard.entries).unwrap_or(usize::MAX),
            "score"
        )
    );
    if leaderboard.trusted_writes {
        line.push_str(", trusted writes only");
    }
    if leaderboard.friends_only {
        line.push_str(", friends only");
    }
    line
}

/// A display type as it is shown: Steam leaves it empty when none was set.
fn display(display: &str) -> String {
    if display.is_empty() {
        "no display type".to_owned()
    } else {
        display.to_ascii_lowercase()
    }
}

/// Every way `held`, Steam's leaderboards, differ from those the file `wants`.
///
/// First each of the file's in its order, then each board a template made, then those only Steam
/// has, in Steam's. A template's own name is no board, so it is never missing: the game makes each
/// period's board on first use.
#[must_use]
pub fn compare(wants: &Wanted, held: &[Leaderboard]) -> Vec<Drift> {
    let mut drift = Vec::new();
    for wanted in &wants.leaderboards {
        let Some(steam) = held
            .iter()
            .find(|leaderboard| leaderboard.name == wanted.name)
        else {
            drift.push(Drift::NotOnSteam(wanted.name.clone()));
            continue;
        };
        settings(steam, wanted, &mut drift);
    }
    for steam in held {
        if let Some(template) = wants
            .templates
            .iter()
            .find(|template| template.made(&steam.name))
        {
            settings(steam, &template.board, &mut drift);
        }
    }
    drift.extend(
        held.iter()
            .filter(|steam| !wants.holds(&steam.name))
            .map(|steam| Drift::NotInFile(steam.name.clone())),
    );
    drift
}

/// Each setting of `steam` that is not as the file's `wanted` has it, under `steam`'s name.
fn settings(steam: &Leaderboard, wanted: &Leaderboard, drift: &mut Vec<Drift>) {
    let yes_no = |on: bool| if on { "yes" } else { "no" }.to_owned();
    for (field, on_steam, in_file) in [
        (
            "the sort",
            steam.sort.to_ascii_lowercase(),
            wanted.sort.to_ascii_lowercase(),
        ),
        (
            "the display",
            display(&steam.display),
            display(&wanted.display),
        ),
        (
            "trusted writes only",
            yes_no(steam.trusted_writes),
            yes_no(wanted.trusted_writes),
        ),
        (
            "friends only",
            yes_no(steam.friends_only),
            yes_no(wanted.friends_only),
        ),
    ] {
        if on_steam != in_file {
            drift.push(Drift::Differs {
                name: steam.name.clone(),
                field,
                steam: on_steam,
                file: in_file,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use std::slice;

    use super::*;

    fn board(name: &str, sort: &str, display: &str) -> Leaderboard {
        Leaderboard {
            name: name.to_owned(),
            sort: sort.to_owned(),
            display: display.to_owned(),
            trusted_writes: false,
            friends_only: false,
            entries: 0,
        }
    }

    #[test]
    fn a_file_is_read_with_steams_words_whatever_their_case() {
        let listed = read(
            r#"{"app": 1000010, "leaderboards": [
                {"name": "fastest_season", "sort": "ascending", "display": "SECONDS",
                 "trusted_writes": true, "icon": "ours"},
                {"name": "gold", "sort": "Descending", "display": "numeric", "friends_only": true}
            ]}"#,
        )
        .unwrap();
        assert_eq!(listed.app, Some(1_000_010));
        let mut fastest = board("fastest_season", "Ascending", "Seconds");
        fastest.trusted_writes = true;
        let mut gold = board("gold", "Descending", "Numeric");
        gold.friends_only = true;
        assert_eq!(listed.leaderboards, [fastest, gold]);
        assert_eq!(
            read(r#"{"leaderboards": []}"#).unwrap(),
            Listed {
                app: None,
                leaderboards: Vec::new(),
                templates: Vec::new(),
            }
        );
    }

    #[test]
    fn a_file_steam_would_not_take_says_why() {
        let long = "x".repeat(129);
        for (text, why) in [
            ("{}".to_owned(), "there is no \"leaderboards\" list"),
            ("{".to_owned(), "not JSON"),
            (
                r#"{"app": "x", "leaderboards": []}"#.to_owned(),
                "\"app\" is not",
            ),
            (
                r#"{"leaderboards": [{"sort": "ascending"}]}"#.to_owned(),
                "leaderboard 1 has no \"name\"",
            ),
            (
                r#"{"leaderboards": [{"name": ""}]}"#.to_owned(),
                "leaderboard 1 has no \"name\"",
            ),
            (
                format!(r#"{{"leaderboards": [{{"name": "{long}"}}]}}"#),
                &format!("{long}: longer than Steam's 128 bytes"),
            ),
            (
                r#"{"leaderboards": [{"name": "a", "display": "numeric"}]}"#.to_owned(),
                "a: \"sort\" is not ascending or descending",
            ),
            (
                r#"{"leaderboards": [{"name": "a", "sort": "up", "display": "numeric"}]}"#
                    .to_owned(),
                "a: \"sort\" is not",
            ),
            (
                r#"{"leaderboards": [{"name": "a", "sort": "ascending", "display": "time"}]}"#
                    .to_owned(),
                "a: \"display\" is not numeric, seconds or milliseconds",
            ),
            (
                r#"{"leaderboards": [{"name": "a", "sort": "ascending", "display": "numeric",
                    "friends_only": "yes"}]}"#
                    .to_owned(),
                "a: \"friends_only\" is not true or false",
            ),
            (
                r#"{"leaderboards": [
                    {"name": "a", "sort": "ascending", "display": "numeric"},
                    {"name": "a", "sort": "ascending", "display": "numeric"}]}"#
                    .to_owned(),
                "a is in the file twice",
            ),
        ] {
            let error = read(&text).unwrap_err();
            assert!(error.starts_with(why), "{text}: {error}");
        }
        let longest = "x".repeat(128);
        assert!(
            read(&format!(
                r#"{{"leaderboards": [{{"name": "{longest}", "sort": "ascending", "display": "numeric"}}]}}"#
            ))
            .is_ok(),
            "Steam's longest is taken"
        );
    }

    #[test]
    fn games_kits_leaderboard_records_are_read_by_their_steam_name_and_others_left_alone() {
        let listed = read(
            r#"[{"type": "stat", "id": "base:floors", "steam": "floors"},
                {"type": "leaderboard", "id": "base:fastest_inn", "steam": "FASTEST_INN",
                 "name": "Fastest Inn", "sort": "ascending", "display": "seconds",
                 "upload": "keep_best", "details": 2},
                {"type": "leaderboard", "steam": "GOLD", "sort": "descending",
                 "display": "numeric", "upload": "force", "trusted_writes": true}]"#,
        )
        .unwrap();
        let mut gold = board("GOLD", "Descending", "Numeric");
        gold.trusted_writes = true;
        assert_eq!(
            listed,
            Listed {
                app: None,
                leaderboards: vec![board("FASTEST_INN", "Ascending", "Seconds"), gold],
                templates: Vec::new(),
            }
        );
        assert_eq!(read("[]").unwrap().leaderboards, []);
        for (text, why) in [
            (r#"[{"steam": "A"}]"#, "record 1 has no \"type\" text"),
            (
                r#"[{"type": "stat", "steam": "s"}, {"type": "leaderboard", "name": "A"}]"#,
                "record 2 has no \"steam\" text",
            ),
            (
                r#"[{"type": "leaderboard", "steam": "A", "sort": "ascending"}]"#,
                "A: \"display\" is not numeric, seconds or milliseconds",
            ),
            (
                r#"[{"type": "leaderboard", "steam": "A", "sort": "ascending", "display": "numeric"},
                    {"type": "leaderboard", "steam": "A", "sort": "ascending", "display": "numeric"}]"#,
                "A is in the file twice",
            ),
        ] {
            assert_eq!(read(text).unwrap_err(), why, "{text}");
        }
    }

    #[test]
    fn several_files_are_read_as_one_and_none_may_repeat_another() {
        let listed = |names: &[&str]| Listed {
            app: None,
            leaderboards: names
                .iter()
                .map(|name| board(name, "Ascending", "Numeric"))
                .collect(),
            templates: Vec::new(),
        };
        assert_eq!(
            together(vec![
                ("base.json".to_owned(), listed(&["a", "b"])),
                ("more.json".to_owned(), listed(&["c"])),
            ])
            .unwrap()
            .leaderboards,
            [
                board("a", "Ascending", "Numeric"),
                board("b", "Ascending", "Numeric"),
                board("c", "Ascending", "Numeric"),
            ]
        );
        assert_eq!(
            together(vec![
                ("base.json".to_owned(), listed(&["a"])),
                ("more.json".to_owned(), listed(&["a"])),
            ])
            .unwrap_err(),
            "a is in both base.json and more.json"
        );
    }

    #[test]
    fn a_record_with_a_period_is_a_template_of_dated_boards() {
        let listed = read(
            r#"[{"type": "leaderboard", "steam": "DAILY", "sort": "descending",
                 "display": "numeric", "period": "day"},
                {"type": "leaderboard", "steam": "WEEKLY", "sort": "ascending",
                 "display": "seconds", "period": "Week"},
                {"type": "leaderboard", "steam": "MONTHLY", "sort": "ascending",
                 "display": "numeric", "period": "month"},
                {"type": "leaderboard", "steam": "BEST", "sort": "descending",
                 "display": "numeric", "period": "none"}]"#,
        )
        .unwrap();
        assert_eq!(
            listed.leaderboards,
            [board("BEST", "Descending", "Numeric")]
        );
        let [daily, weekly, monthly] = listed.templates.as_slice() else {
            panic!("three templates: {:?}", listed.templates);
        };
        assert_eq!(
            (daily.period, weekly.period, monthly.period),
            (Period::Day, Period::Week, Period::Month)
        );
        assert_eq!(daily.board, board("DAILY", "Descending", "Numeric"));
        for (template, name, made) in [
            (daily, "DAILY_20261025", true),
            (daily, "DAILY", false),
            (daily, "DAILY_2026102", false),
            (daily, "DAILY_202610255", false),
            (daily, "DAILY_2026102x", false),
            (daily, "DAILY20261025", false),
            (daily, "DAILY_X_20261025", false),
            (weekly, "WEEKLY_2026W43", true),
            (weekly, "WEEKLY_2026W4", false),
            (weekly, "WEEKLY_2026X43", false),
            (weekly, "WEEKLY_202xW43", false),
            (weekly, "WEEKLY_2026W4x", false),
            (weekly, "WEEKLY_20261025", false),
            (monthly, "MONTHLY_202610", true),
            (monthly, "MONTHLY_20261", false),
            (monthly, "MONTHLY_2026100", false),
            (monthly, "MONTHLY_20261x", false),
        ] {
            assert_eq!(template.made(name), made, "{name}");
        }
    }

    #[test]
    fn a_template_the_game_could_not_make_says_why() {
        let record = |steam: &str, more: &str| {
            format!(
                r#"[{{"type": "leaderboard", "steam": "{steam}", "sort": "ascending",
                     "display": "numeric"{more}}}]"#
            )
        };
        for (text, why) in [
            (
                record("A", r#", "period": "year""#),
                "A: \"period\" is not none, day, week or month",
            ),
            (
                record("A", r#", "period": "day", "trusted_writes": true"#),
                "A: the game makes a board each period, which cannot have trusted writes or \
                 friends only, as Steamworks sets them board by board",
            ),
            (
                record("A", r#", "period": "week", "friends_only": true"#),
                "A: the game makes a board each period, which cannot have trusted writes or \
                 friends only, as Steamworks sets them board by board",
            ),
            (
                record(&"x".repeat(120), r#", "period": "day""#),
                &format!(
                    "{}: longer than Steam's 128 bytes once the game adds the date",
                    "x".repeat(120)
                ),
            ),
        ] {
            assert_eq!(read(&text).unwrap_err(), why, "{text}");
        }
        for (name, period) in [
            ("x".repeat(119), "day"),
            ("x".repeat(120), "week"),
            ("x".repeat(121), "month"),
        ] {
            assert!(
                read(&record(&name, &format!(r#", "period": "{period}""#))).is_ok(),
                "the longest a {period} takes"
            );
        }
        for (name, period) in [("x".repeat(121), "week"), ("x".repeat(122), "month")] {
            assert!(
                read(&record(&name, &format!(r#", "period": "{period}""#))).is_err(),
                "one past the longest a {period} takes"
            );
        }
        let both = together(vec![(
            "boards.json".to_owned(),
            read(
                r#"[{"type": "leaderboard", "steam": "DAILY", "sort": "ascending",
                     "display": "numeric", "period": "day"},
                    {"type": "leaderboard", "steam": "DAILY_20270101", "sort": "ascending",
                     "display": "numeric"}]"#,
            )
            .unwrap(),
        )]);
        assert_eq!(
            both.unwrap_err(),
            "DAILY_20270101 is a name the game gives a board of DAILY on some date, so the two \
             would share one board"
        );
        let template = |file: &str| {
            (
                file.to_owned(),
                read(&record("DAILY", r#", "period": "day""#)).unwrap(),
            )
        };
        assert_eq!(
            together(vec![template("base.json"), template("more.json")]).unwrap_err(),
            "DAILY is in both base.json and more.json"
        );
    }

    #[test]
    fn each_dated_board_is_held_to_its_template_and_none_is_missing() {
        let wants = together(vec![(
            "boards.json".to_owned(),
            read(
                r#"[{"type": "leaderboard", "steam": "DAILY", "sort": "descending",
                     "display": "numeric", "period": "day"}]"#,
            )
            .unwrap(),
        )])
        .unwrap();
        assert!(wants.holds("DAILY_20261025") && !wants.holds("DAILY") && !wants.holds("old"));
        assert_eq!(compare(&wants, &[]), [], "the game makes a day's board");
        let drift = compare(
            &wants,
            &[
                board("DAILY_20261024", "Descending", "Numeric"),
                board("DAILY_20261025", "Ascending", "Numeric"),
                board("DAILY", "Descending", "Numeric"),
            ],
        );
        let said: Vec<String> = drift.iter().map(ToString::to_string).collect();
        assert_eq!(
            said,
            [
                "DAILY_20261025: the sort is \"ascending\" on Steam and \"descending\" in the file",
                "DAILY: on Steam, not in the file; --delete-missing deletes it while the app is \
                 unreleased",
            ]
        );
    }

    #[test]
    fn a_choice_reads_as_a_sentence() {
        let words =
            |list: &[&str]| -> Vec<String> { list.iter().map(|&word| word.to_owned()).collect() };
        assert_eq!(or_list(&words(&[])), "");
        assert_eq!(or_list(&words(&["a"])), "a");
        assert_eq!(or_list(&words(&["a", "b"])), "a or b");
        assert_eq!(or_list(&words(&["a", "b", "c"])), "a, b or c");
    }

    #[test]
    fn a_leaderboard_is_listed_by_its_settings_and_scores() {
        let mut fastest = board("fastest_season", "Ascending", "Seconds");
        fastest.entries = 1;
        assert_eq!(line(&fastest), "ascending, seconds, 1 score");
        fastest.trusted_writes = true;
        fastest.friends_only = true;
        fastest.entries = 12;
        assert_eq!(
            line(&fastest),
            "ascending, seconds, 12 scores, trusted writes only, friends only"
        );
        assert_eq!(
            line(&board("gold", "Descending", "")),
            "descending, no display type, 0 scores"
        );
    }

    #[test]
    fn drift_is_every_setting_that_differs_and_every_leaderboard_on_one_side_only() {
        let fastest = board("fastest_season", "Ascending", "Seconds");
        let mut changed = board("fastest_season", "Descending", "");
        changed.trusted_writes = true;
        changed.friends_only = true;
        changed.entries = 40;
        let gold = board("gold", "Descending", "Numeric");
        let old = board("old", "Descending", "Numeric");
        let wanted = |leaderboards: &[Leaderboard]| Wanted {
            leaderboards: leaderboards.to_vec(),
            templates: Vec::new(),
        };
        assert_eq!(
            compare(
                &wanted(&[fastest.clone(), gold.clone()]),
                &[fastest.clone(), gold.clone()]
            ),
            [],
            "the scores are not the file's to say"
        );
        let mut shown_differently = fastest.clone();
        shown_differently.sort = "ASCENDING".to_owned();
        assert_eq!(
            compare(&wanted(slice::from_ref(&fastest)), &[shown_differently]),
            []
        );
        let drift = compare(&wanted(&[fastest, gold]), &[old, changed]);
        let said: Vec<String> = drift.iter().map(ToString::to_string).collect();
        assert_eq!(
            said,
            [
                "fastest_season: the sort is \"descending\" on Steam and \"ascending\" in the file",
                "fastest_season: the display is \"no display type\" on Steam and \"seconds\" in \
                 the file",
                "fastest_season: trusted writes only is \"yes\" on Steam and \"no\" in the file",
                "fastest_season: friends only is \"yes\" on Steam and \"no\" in the file",
                "gold: in the file, not on Steam; --create makes it",
                "old: on Steam, not in the file; --delete-missing deletes it while the app is \
                 unreleased",
            ]
        );
    }
}
