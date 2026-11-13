//! What `steamship builds` shows of an app's branches and builds, to a person and as `--json`,
//! and the `error` each `--json` listing prints when Steam does not answer.

use serde_json::{Value, json};

use crate::branch;
use crate::calendar;
use crate::redact::{self, Redactor};
use crate::upload;
use crate::webapi::{self, Branch, Build, Overview};

/// How many branches and builds there are, in words.
#[must_use]
pub fn summary(overview: &Overview) -> String {
    let branches = match overview.branches.len() {
        1 => "1 branch".to_owned(),
        count => format!("{count} branches"),
    };
    let builds = match overview.builds.len() {
        1 => "1 build".to_owned(),
        count => format!("{count} builds"),
    };
    format!("{branches} and {builds}")
}

/// Every line shown: each branch, then each build by its ID, as a label and what follows it.
#[must_use]
pub fn lines(overview: &Overview) -> Vec<(String, String)> {
    let branches = overview
        .branches
        .iter()
        .map(|branch| ("branch".to_owned(), branch_line(branch)));
    let builds = overview.builds.iter().map(|build| {
        (
            build.build_id.to_string(),
            build_line(build, &overview.branches),
        )
    });
    branches.chain(builds).collect()
}

/// The branches and builds of `app` as `builds --json` prints them.
///
/// A build's `version` and `commit` are read back from the description `upload` wrote, and are
/// null for a build described otherwise. `created` is null when Steam gave no time.
#[must_use]
pub fn json(app: u32, overview: &Overview) -> Value {
    let branches: Vec<Value> = overview
        .branches
        .iter()
        .map(|branch| {
            json!({
                "name": branch::shown_name(&branch.name),
                "build_id": branch.build_id,
                "description": branch.description,
                "password": branch.locked,
            })
        })
        .collect();
    let builds: Vec<Value> = overview
        .builds
        .iter()
        .map(|build| {
            let (version, commit) = upload::version_and_commit(&build.description).unzip();
            let live_on: Vec<&str> = overview
                .branches
                .iter()
                .filter(|branch| branch.build_id == build.build_id)
                .map(|branch| branch::shown_name(&branch.name))
                .collect();
            json!({
                "build_id": build.build_id,
                "created": (build.created > 0).then(|| calendar::timestamp(build.created)),
                "description": build.description,
                "version": version,
                "commit": commit,
                "live_on": live_on,
            })
        })
        .collect();
    json!({ "app": app, "branches": branches, "builds": builds })
}

/// `branch` as a line: its name, the build live on it and its description, and whether it takes
/// a password.
fn branch_line(branch: &Branch) -> String {
    let mut parts = vec![format!(
        "{}: BuildID {}",
        branch::shown_name(&branch.name),
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
fn build_line(build: &Build, branches: &[Branch]) -> String {
    let (year, month, day) = calendar::date(build.created);
    let mut parts = vec![format!("{year}-{month:02}-{day:02}")];
    if !build.description.is_empty() {
        parts.push(build.description.clone());
    }
    let names: Vec<&str> = branches
        .iter()
        .filter(|branch| branch.build_id == build.build_id)
        .map(|branch| branch::shown_name(&branch.name))
        .collect();
    if !names.is_empty() {
        parts.push(format!("live on {}", names.join(" and ")));
    }
    parts.join(", ")
}

/// The `error` of a failed run's `--json` line: a `code` word and the `message` a person would
/// read.
#[must_use]
pub fn failure_json(code: &str, message: &str) -> Value {
    json!({ "code": code, "message": message })
}

/// What `--json` prints when Steam did not answer about `app`.
///
/// The code is [`webapi::Error::code`], or `no-build-live` when `why` says that is the reason. The
/// message is what a person would read, with the secrets of `redactor` replaced in the words Steam
/// gave.
#[must_use]
pub fn error_json(
    app: u32,
    error: &webapi::Error,
    why: Option<&str>,
    redactor: Option<&Redactor>,
) -> Value {
    let said = error.said_with(&|steam| redact::answer(redactor, steam));
    let (code, message) = why.map_or_else(
        || (error.code(), said.clone()),
        |why| ("no-build-live", format!("{said}: {why}")),
    );
    json!({ "app": app, "error": failure_json(code, &message) })
}

#[cfg(test)]
mod tests {
    use super::*;

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
            description: "0.1.0 7a6b5c4".to_owned(),
            created: 1_790_553_599,
        };
        assert_eq!(
            build_line(&build, &branches),
            "2026-09-27, 0.1.0 7a6b5c4, live on testing and staging"
        );
        let older = Build {
            build_id: 1100,
            description: String::new(),
            created: 0,
        };
        assert_eq!(build_line(&older, &branches), "1970-01-01");
    }

    #[test]
    fn the_default_branch_is_shown_by_the_name_steamworks_gives_it() {
        // As Steam's Web API answered for Falcon Ridge Overseer.
        let public = Branch {
            description: "Public default branch".to_owned(),
            ..branch("public", 20_000_001, false)
        };
        assert_eq!(
            branch_line(&public),
            "default: BuildID 20000001, Public default branch"
        );
        let build = Build {
            build_id: 20_000_001,
            description: "0.1.0-test 0a1b2c3d4e5f".to_owned(),
            created: 0,
        };
        assert_eq!(
            build_line(&build, &[public]),
            "1970-01-01, 0.1.0-test 0a1b2c3d4e5f, live on default"
        );
        assert_eq!(branch::shown_name("Public"), "default");
        assert_eq!(branch::shown_name("testing"), "testing");
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
        assert_eq!(summary(&overview), "1 branch and 2 builds");
        assert_eq!(
            lines(&overview),
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
        assert_eq!(summary(&none), "0 branches and 1 build");
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
            json(480, &overview).pointer("/builds/0"),
            Some(&json!({
                "build_id": 7_u64, "created": null, "description": "0.1.0", "version": null,
                "commit": null, "live_on": ["default", "testing"],
            }))
        );
    }

    #[test]
    fn steam_not_answering_for_no_build_live_is_printed_with_that_code_and_reason() {
        let held = error_json(
            1_000_030,
            &webapi::Error::Status(500, String::new()),
            Some("none live"),
            Some(&Redactor::default()),
        );
        assert_eq!(
            held,
            json!({"app": 1_000_030_u32, "error": {
                "code": "no-build-live",
                "message": "Steam answered HTTP 500: none live",
            }})
        );
    }

    #[test]
    fn every_error_is_a_code_and_a_message() {
        let errors = [
            webapi::Error::NotAKey,
            webapi::Error::Refused,
            webapi::Error::Status(500, String::new()),
            webapi::Error::Unreadable("the build list"),
            webapi::Error::Failed("no".to_owned()),
        ];
        let mut lines = vec![failure_json("x", "y")];
        for error in &errors {
            for why in [None, Some("none live")] {
                let shown = error_json(480, error, why, Some(&Redactor::default()));
                lines.extend(shown.pointer("/error").cloned());
            }
        }
        for line in lines {
            let mut keys: Vec<&String> = line.as_object().unwrap().keys().collect();
            keys.sort();
            assert_eq!(keys, ["code", "message"], "{line}");
        }
    }
}
