//! The `*` and `?` wildcards of `LocalPath` and `FileExclusion`.

/// Answers whether `text` matches `pattern`.
///
/// `*` stands for any run of characters, path separators included, and `?` for any one. Case is
/// ignored where the file system usually ignores it, on Windows and macOS.
#[must_use]
pub fn matches(pattern: &str, text: &str) -> bool {
    matches_folding(pattern, text, cfg!(any(windows, target_os = "macos")))
}

/// The same, with the choice about case made by the caller.
///
/// Iterative, with one point to fall back to: every `*` supersedes the one before it, so the work
/// stays within pattern length times text length and nothing recurses on hostile input.
#[must_use]
pub fn matches_folding(pattern: &str, text: &str, fold: bool) -> bool {
    let pattern: Vec<char> = pattern.chars().collect();
    let text: Vec<char> = text.chars().collect();
    let mut wanted: &[char] = &pattern;
    let mut given: &[char] = &text;
    // What follows the latest `*`, and the text that `*` has not yet swallowed.
    let mut fallback: Option<(&[char], &[char])> = None;
    loop {
        match (wanted.split_first(), given.split_first()) {
            (Some((&'*', after_star)), _) => {
                fallback = Some((after_star, given));
                wanted = after_star;
            }
            (Some((&one, wanted_rest)), Some((&other, given_rest)))
                if one == '?' || same(one, other, fold) =>
            {
                wanted = wanted_rest;
                given = given_rest;
            }
            (None, None) => return true,
            _ => {
                // The last `*` swallows one more character and matching resumes after it.
                let Some((after_star, unswallowed)) = fallback else {
                    return false;
                };
                let Some((_, swallowed_one_more)) = unswallowed.split_first() else {
                    return false;
                };
                fallback = Some((after_star, swallowed_one_more));
                wanted = after_star;
                given = swallowed_one_more;
            }
        }
    }
}

fn same(one: char, other: char, fold: bool) -> bool {
    one == other || (fold && one.to_lowercase().eq(other.to_lowercase()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_like_valves_examples() {
        for (pattern, text, expected) in [
            ("*", "game.exe", true),
            ("*", "", true),
            ("*.pdb", "game.pdb", true),
            ("*.pdb", "game.pdb.bak", false),
            ("bin/tools*", "bin/tools/a/b.exe", true),
            ("bin/server.exe", "bin/server.exe", true),
            ("bin/server.exe", "bin/server.exe2", false),
            ("data?.pck", "data1.pck", true),
            ("data?.pck", "data.pck", false),
            ("a*b*c", "aXbYbZc", true),
            ("a*b*c", "aXbYbZ", false),
            ("**", "anything", true),
            ("", "", true),
            ("", "x", false),
        ] {
            assert_eq!(
                matches_folding(pattern, text, false),
                expected,
                "{pattern} on {text}"
            );
        }
    }

    #[test]
    fn ignores_case_only_when_asked() {
        assert!(matches_folding("*.PDB", "game.pdb", true));
        assert!(!matches_folding("*.PDB", "game.pdb", false));
    }

    #[test]
    fn a_literal_star_in_the_text_is_still_one_character() {
        assert!(matches_folding("a?c", "a*c", false));
        assert!(!matches_folding("abc", "a*c", false));
    }
}
