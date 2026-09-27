//! What the build refuses to publish: a link or anchor that goes nowhere, code too wide to read
//! without scrolling, and characters that do not belong in a page.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

/// The widest line of code a page shows whole, at the site's one width: a release archive's
/// whole address after `curl -fLO`, with room for a longer version.
const CODE_COLUMNS: usize = 128;

/// Checks every file the build wrote to `out`.
///
/// # Errors
///
/// Every problem found, one line each, or why `out` could not be read.
pub fn site(out: &Path) -> Result<(), String> {
    let mut files = BTreeMap::new();
    let entries = fs::read_dir(out).map_err(|error| format!("{}: {error}", out.display()))?;
    for entry in entries {
        let path = entry
            .map_err(|error| format!("{}: {error}", out.display()))?
            .path();
        let name = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        let text = fs::read_to_string(&path).map_err(|error| format!("{name}: {error}"))?;
        let _earlier = files.insert(name, text);
    }
    let problems = problems(&files);
    if problems.is_empty() {
        Ok(())
    } else {
        Err(problems.join("\n"))
    }
}

/// Everything wrong with `files`, the site's files by name, one line each.
fn problems(files: &BTreeMap<String, String>) -> Vec<String> {
    let mut problems = Vec::new();
    for (name, text) in files {
        if let Some(bad) = text
            .chars()
            .find(|&character| character.is_control() && character != '\n')
        {
            problems.push(format!(
                "{name}: holds the control character U+{:04X}",
                u32::from(bad)
            ));
        }
        let links = if is(name, "html") {
            html_links(text)
        } else if is(name, "md") {
            markdown_links(text)
        } else {
            Vec::new()
        };
        for link in links {
            if let Some(problem) = broken(name, &link, files) {
                problems.push(problem);
            }
        }
        if is(name, "html") {
            for line in code_lines(text) {
                let width = line.chars().count();
                if width > CODE_COLUMNS {
                    problems.push(format!(
                        "{name}: a line of code is {width} columns, over {CODE_COLUMNS}: {line}"
                    ));
                }
                if let Some(drawn) = line.chars().find(
                    |&character| matches!(u32::from(character), 0x2190..=0x21FF | 0x2500..=0x259F),
                ) {
                    problems.push(format!(
                        "{name}: code holds {drawn}, which the code font has no glyph for"
                    ));
                }
            }
        }
    }
    problems
}

/// What is wrong with `link` in the file `from`, when it points at a file or anchor the site
/// does not have. Links to other sites are not followed.
fn broken(from: &str, link: &str, files: &BTreeMap<String, String>) -> Option<String> {
    if link.contains("://") || link.starts_with("//") || link.starts_with("mailto:") {
        return None;
    }
    let (file, fragment) = link.split_once('#').unwrap_or((link, ""));
    let target = if file.is_empty() { from } else { file };
    let Some(text) = files.get(target) else {
        return Some(format!(
            "{from}: links to {link}, which the site does not have"
        ));
    };
    let anchored = if is(target, "md") {
        files.get(&target.replace(".md", ".html")).unwrap_or(text)
    } else {
        text
    };
    (!fragment.is_empty() && !anchored.contains(&format!(" id=\"{fragment}\"")))
        .then(|| format!("{from}: links to {link}, whose anchor the page does not have"))
}

/// Whether the file `name` is of the kind `extension`, whatever its case.
fn is(name: &str, extension: &str) -> bool {
    Path::new(name)
        .extension()
        .is_some_and(|found| found.eq_ignore_ascii_case(extension))
}

/// Every `href` and `src` an HTML page makes.
fn html_links(html: &str) -> Vec<String> {
    let mut links = Vec::new();
    for attribute in ["href=\"", "src=\""] {
        for (at, _) in html.match_indices(attribute) {
            let rest = html
                .get(at.saturating_add(attribute.len())..)
                .unwrap_or_default();
            if let Some((link, _)) = rest.split_once('"') {
                links.push(link.replace("&amp;", "&"));
            }
        }
    }
    links
}

/// Every link a markdown page makes, `[text](target)`, outside its code.
fn markdown_links(markdown: &str) -> Vec<String> {
    let mut links = Vec::new();
    let mut fenced = false;
    for line in markdown.lines() {
        if line.trim_start().starts_with("```") {
            fenced = !fenced;
            continue;
        }
        if fenced {
            continue;
        }
        let prose: String = line.split('`').step_by(2).collect();
        for (at, _) in prose.match_indices("](") {
            let rest = prose.get(at.saturating_add(2)..).unwrap_or_default();
            if let Some((link, _)) = rest.split_once(')') {
                links.push(link.to_owned());
            }
        }
    }
    links
}

/// Every line of code on an HTML page, as it reads: tags taken out and entities put back.
fn code_lines(html: &str) -> Vec<String> {
    let mut lines = Vec::new();
    for block in html.split("<pre><code>").skip(1) {
        let code = block
            .split_once("</code></pre>")
            .map_or(block, |(code, _)| code);
        let mut plain = String::with_capacity(code.len());
        let mut in_tag = false;
        for character in code.chars() {
            match character {
                '<' => in_tag = true,
                '>' if in_tag => in_tag = false,
                _ if !in_tag => plain.push(character),
                _ => {}
            }
        }
        let plain = plain
            .replace("&lt;", "<")
            .replace("&gt;", ">")
            .replace("&quot;", "\"")
            .replace("&amp;", "&");
        lines.extend(plain.lines().map(str::to_owned));
    }
    lines
}

#[cfg(test)]
mod tests {
    use std::{env, process};

    use super::*;

    fn site_of(files: &[(&str, &str)]) -> BTreeMap<String, String> {
        files
            .iter()
            .map(|&(name, text)| (name.to_owned(), text.to_owned()))
            .collect()
    }

    #[test]
    fn a_whole_site_has_no_problems() {
        let files = site_of(&[
            (
                "index.html",
                "<h2 id=\"a\">A</h2><a href=\"b.html#c\">b</a><a href=\"#a\">a</a><img src=\"i.svg\" />",
            ),
            (
                "b.html",
                "<h2 id=\"c\">C</h2><a href=\"https://example.com/x\">x</a>",
            ),
            (
                "b.md",
                "[back](index.md#a) and `[not](a link)`\n```\n[nor](this)\n```\n",
            ),
            ("index.md", ""),
            ("i.svg", "<svg/>"),
        ]);
        assert_eq!(problems(&files), Vec::<String>::new());
    }

    #[test]
    fn a_missing_file_or_anchor_is_refused() {
        let files = site_of(&[
            (
                "index.html",
                "<a href=\"gone.html\">g</a><a href=\"b.html#nowhere\">n</a><a href=\"#self\">s</a>",
            ),
            ("b.html", "<h2 id=\"c\">C</h2>"),
            ("b.md", "[x](gone.md)"),
        ]);
        assert_eq!(
            problems(&files),
            [
                "b.md: links to gone.md, which the site does not have",
                "index.html: links to gone.html, which the site does not have",
                "index.html: links to b.html#nowhere, whose anchor the page does not have",
                "index.html: links to #self, whose anchor the page does not have",
            ]
        );
    }

    #[test]
    fn code_too_wide_or_with_glyphs_the_font_lacks_is_refused() {
        let wide = "x".repeat(CODE_COLUMNS.saturating_add(1));
        let files = site_of(&[
            ("a.html", &format!("<pre><code>{wide}</code></pre>")),
            (
                "b.html",
                &format!(
                    "<pre><code><i class=\"c\">&lt;{}&gt;</i>\n\u{2500}</code></pre>",
                    "y".repeat(CODE_COLUMNS.saturating_sub(2))
                ),
            ),
        ]);
        assert_eq!(
            problems(&files),
            [
                format!(
                    "a.html: a line of code is {} columns, over {CODE_COLUMNS}: {wide}",
                    CODE_COLUMNS.saturating_add(1)
                ),
                "b.html: code holds \u{2500}, which the code font has no glyph for".to_owned(),
            ]
        );
    }

    #[test]
    fn a_control_character_is_refused() {
        let files = site_of(&[("a.md", "fine\nnot\u{7}fine")]);
        assert_eq!(
            problems(&files),
            ["a.md: holds the control character U+0007"]
        );
    }

    #[test]
    fn the_site_as_written_is_checked_from_disk() {
        let out = env::temp_dir().join(format!("steamship-site-{}", process::id()));
        fs::create_dir_all(&out).unwrap_or_default();
        fs::write(out.join("index.html"), "<a href=\"gone.html\">g</a>").unwrap_or_default();
        let refused = site(&out);
        fs::write(out.join("gone.html"), "").unwrap_or_default();
        let whole = site(&out);
        fs::remove_dir_all(&out).unwrap_or_default();
        assert_eq!(
            refused,
            Err("index.html: links to gone.html, which the site does not have".to_owned())
        );
        assert_eq!(whole, Ok(()));
    }
}
