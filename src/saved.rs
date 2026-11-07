//! Whether a file `--save` is to write over holds only what `--save` writes, so that nothing kept
//! in it by hand is lost.

use serde_json::Value;

/// A list `--save` writes: its key in the file, what one entry in it is called, and the keys an
/// entry has.
#[derive(Debug, Clone, Copy)]
pub struct List {
    pub key: &'static str,
    pub entry: &'static str,
    pub fields: &'static [&'static str],
}

/// Whether `text`, a file `--save` is to write over, holds only an `app` and the `lists`, each
/// entry with only its list's fields.
///
/// # Errors
///
/// When it holds anything else, the first such thing as a clause, such as
/// `holds "icon" in achievement 3, which --save would lose`.
pub fn replaceable(text: &str, lists: &[List]) -> Result<(), String> {
    let lost = |what: String| Err(format!("holds {what}, which --save would lose"));
    let file: Value = serde_json::from_str(text)
        .map_err(|error| format!("is not JSON ({error}), so no file --save wrote"))?;
    let top = match file {
        Value::Object(top) => top,
        Value::Array(_) => return lost("a list of games-kit records".to_owned()),
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {
            return Err("is not a file --save wrote".to_owned());
        }
    };
    for (key, value) in &top {
        if key == "app" {
            continue;
        }
        let Some(written) = lists.iter().find(|each| each.key == key) else {
            return lost(format!("\"{key}\""));
        };
        let Value::Array(entries) = value else {
            return lost(format!("\"{key}\" as something other than a list"));
        };
        for (index, entry) in entries.iter().enumerate() {
            let place = index.saturating_add(1);
            let Value::Object(fields) = entry else {
                return lost(format!(
                    "{} {place} as something other than an object",
                    written.entry
                ));
            };
            if let Some(extra) = fields
                .keys()
                .find(|field| !written.fields.contains(&field.as_str()))
            {
                return lost(format!("\"{extra}\" in {} {place}", written.entry));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const BOARDS: [List; 1] = [List {
        key: "leaderboards",
        entry: "leaderboard",
        fields: &["name", "sort"],
    }];

    #[test]
    fn a_file_holding_only_what_save_writes_may_be_written_over() {
        for text in [
            r#"{"app": 480, "leaderboards": [{"name": "gold", "sort": "ascending"}]}"#,
            r#"{"leaderboards": []}"#,
            "{}",
        ] {
            assert_eq!(replaceable(text, &BOARDS), Ok(()), "{text}");
        }
    }

    #[test]
    fn a_file_holding_more_than_save_writes_names_the_first_thing_it_would_lose() {
        for (text, why) in [
            (
                r#"{"leaderboards": [{"name": "a"}, {"name": "b", "period": "day"}]}"#,
                "holds \"period\" in leaderboard 2, which --save would lose",
            ),
            (
                r#"{"leaderboards": [], "notes": "ours"}"#,
                "holds \"notes\", which --save would lose",
            ),
            (
                r#"{"leaderboards": {"name": "a"}}"#,
                "holds \"leaderboards\" as something other than a list, which --save would lose",
            ),
            (
                r#"{"leaderboards": ["gold"]}"#,
                "holds leaderboard 1 as something other than an object, which --save would lose",
            ),
            (
                r#"[{"type": "leaderboard", "steam": "gold"}]"#,
                "holds a list of games-kit records, which --save would lose",
            ),
            ("12", "is not a file --save wrote"),
        ] {
            assert_eq!(replaceable(text, &BOARDS), Err(why.to_owned()), "{text}");
        }
        let empty = replaceable("", &BOARDS).unwrap_err();
        assert!(
            empty.starts_with("is not JSON (") && empty.ends_with("), so no file --save wrote"),
            "{empty}"
        );
    }
}
