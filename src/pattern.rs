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
    let (mut p, mut t) = (0, 0);
    let mut fallback: Option<(usize, usize)> = None;
    while t < text.len() {
        match pattern.get(p) {
            Some('*') => {
                fallback = Some((p, t));
                p += 1;
            }
            Some(&wanted) if wanted == '?' || same(wanted, text[t], fold) => {
                p += 1;
                t += 1;
            }
            _ => match fallback {
                Some((star, from)) => {
                    p = star + 1;
                    t = from + 1;
                    fallback = Some((star, from + 1));
                }
                None => return false,
            },
        }
    }
    pattern[p..].iter().all(|&rest| rest == '*')
}

fn same(a: char, b: char, fold: bool) -> bool {
    a == b || (fold && a.to_lowercase().eq(b.to_lowercase()))
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
            ("gam?.exe", "game.exe", true),
            ("gam?.exe", "gam.exe", false),
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
