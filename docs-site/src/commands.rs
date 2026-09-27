//! The command reference, made from the command line itself.
//!
//! Each command's help, its usage and every argument, then the exit codes and the environment.
//! What `steamship --help` prints and what this page says are the same definitions.

use std::fmt::{self, Write as _};

use clap::{Arg, Command, CommandFactory as _};
use steamship::cli::{Cli, EXIT_CODES, VARIABLES};

use crate::markdown::{escaped, inline};

const LEDE: &str = "Every command steamship takes and every option, generated from its own command \
                    line, so this page and `steamship --help` say the same thing. \
                    `steamship help <command>` prints any of these at a terminal.";

/// The reference as the site's HTML.
#[must_use]
pub fn html() -> String {
    let mut page = format!(
        "<h1 id=\"commands\">Commands</h1>\n<p class=\"lede\">{}</p>\n",
        inline(LEDE)
    );
    for command in commands() {
        let name = command.get_name();
        let (about, more) = described(&command);
        let _written: fmt::Result = write!(
            page,
            "<section class=\"tool\" id=\"{name}\">\n<h2 id=\"steamship-{name}\"><code>steamship \
             {name}</code></h2>\n<p>{}</p>\n",
            inline(&about)
        );
        for paragraph in more {
            let _more: fmt::Result = writeln!(page, "<p>{}</p>", inline(&paragraph));
        }
        let _usage: fmt::Result = writeln!(
            page,
            "<figure class=\"code\"><pre><code>{}</code></pre></figure>",
            escaped(&usage(&command))
        );
        let arguments = arguments(&command);
        if !arguments.is_empty() {
            page.push_str("<dl class=\"args\">\n");
            for argument in arguments {
                let _entry: fmt::Result = writeln!(
                    page,
                    "<dt><code>{}</code>{}</dt><dd>{}</dd>",
                    escaped(&shape(argument)),
                    tags(argument),
                    inline(&help(argument))
                );
            }
            page.push_str("</dl>\n");
        }
        page.push_str("</section>\n");
    }
    page.push_str(
        "<h2 id=\"exit-codes\">Exit codes</h2>\n<div class=\"scroll\"><table><thead><tr><th>Code</th>\
         <th>Meaning</th></tr></thead><tbody>\n",
    );
    for (code, meaning) in EXIT_CODES {
        let _row: fmt::Result = writeln!(
            page,
            "<tr><td><code>{code}</code></td><td>{}</td></tr>",
            inline(meaning)
        );
    }
    page.push_str(
        "</tbody></table></div>\n<h2 id=\"environment\">Environment</h2>\n<div class=\"scroll\">\
         <table><thead><tr><th>Variable</th><th>Meaning</th></tr></thead><tbody>\n",
    );
    for (name, meaning) in VARIABLES {
        let _row: fmt::Result = writeln!(
            page,
            "<tr><td><code>{name}</code></td><td>{}</td></tr>",
            inline(meaning)
        );
    }
    page.push_str(
        "</tbody></table></div>\n<p>There is no password setting anywhere: a password is typed \
         at <code>steamship login</code> and passed straight to steamcmd.</p>\n",
    );
    page
}

/// The same reference as markdown.
#[must_use]
pub fn text() -> String {
    let mut page = format!("# Commands\n\n{LEDE}\n");
    for command in commands() {
        let name = command.get_name();
        let (about, more) = described(&command);
        let _written: fmt::Result = write!(page, "\n## `steamship {name}`\n\n{about}\n\n");
        for paragraph in more {
            let _more: fmt::Result = write!(page, "{paragraph}\n\n");
        }
        let _usage: fmt::Result = writeln!(page, "```text\n{}\n```", usage(&command));
        for argument in arguments(&command) {
            let _entry: fmt::Result = write!(
                page,
                "\n- `{}`{}: {}",
                shape(argument),
                text_tags(argument),
                help(argument)
            );
        }
        if !arguments(&command).is_empty() {
            page.push('\n');
        }
    }
    page.push_str("\n## Exit codes\n\n| Code | Meaning |\n| ---- | ------- |\n");
    for (code, meaning) in EXIT_CODES {
        let _row: fmt::Result = writeln!(page, "| {code} | {meaning} |");
    }
    page.push_str("\n## Environment\n\n| Variable | Meaning |\n| -------- | ------- |\n");
    for (name, meaning) in VARIABLES {
        let _row: fmt::Result = writeln!(page, "| `{name}` | {meaning} |");
    }
    page.push_str(
        "\nThere is no password setting anywhere: a password is typed at `steamship login` and \
         passed straight to steamcmd.\n",
    );
    page
}

/// Every command, in the order `--help` lists them, without clap's own `help`.
fn commands() -> Vec<Command> {
    Cli::command()
        .get_subcommands()
        .filter(|command| command.get_name() != "help")
        .cloned()
        .collect()
}

/// A command's one line, as a sentence, and each paragraph its longer help adds. clap drops the
/// full stop from the one line and keeps it in the longer help, so the two compare without it.
fn described(command: &Command) -> (String, Vec<String>) {
    let about = command
        .get_about()
        .map(ToString::to_string)
        .unwrap_or_default();
    let about = about.trim_end_matches('.');
    let long = command
        .get_long_about()
        .map(ToString::to_string)
        .unwrap_or_default();
    let more = long
        .split("\n\n")
        .map(|paragraph| paragraph.split_whitespace().collect::<Vec<_>>().join(" "))
        .filter(|paragraph| !paragraph.is_empty() && paragraph.trim_end_matches('.') != about)
        .collect();
    (format!("{about}."), more)
}

/// How the command is typed, as its help's usage line puts it.
fn usage(command: &Command) -> String {
    let rendered = command
        .clone()
        .bin_name(format!("steamship {}", command.get_name()))
        .render_usage()
        .to_string();
    rendered
        .strip_prefix("Usage: ")
        .unwrap_or(&rendered)
        .to_owned()
}

/// The arguments a person types, without clap's own `--help`.
fn arguments(command: &Command) -> Vec<&Arg> {
    command
        .get_arguments()
        .filter(|argument| argument.get_id() != "help")
        .collect()
}

/// An argument as it is typed: `<SCRIPT>`, `--version <VERSION>`, or `--preview`.
fn shape(argument: &Arg) -> String {
    let value = argument
        .get_value_names()
        .and_then(<[_]>::first)
        .map_or_else(
            || argument.get_id().as_str().to_uppercase(),
            ToString::to_string,
        );
    match argument.get_long() {
        Some(long) if argument.get_action().takes_values() => format!("--{long} <{value}>"),
        Some(long) => format!("--{long}"),
        None => format!("<{value}>"),
    }
}

/// What an argument's help says, on one line.
fn help(argument: &Arg) -> String {
    let said = argument
        .get_long_help()
        .or_else(|| argument.get_help())
        .map(ToString::to_string)
        .unwrap_or_default();
    said.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .trim_end_matches('.')
        .to_owned()
}

/// Whether an argument must be given, its default, and the variable it can come from instead.
fn notes(argument: &Arg) -> Vec<String> {
    let mut notes = Vec::new();
    if argument.is_required_set() {
        notes.push("required".to_owned());
    }
    if let Some(default) = argument.get_default_values().first() {
        notes.push(format!("default {}", default.to_string_lossy()));
    }
    if let Some(variable) = argument.get_env() {
        notes.push(format!("or {}", variable.to_string_lossy()));
    }
    notes
}

fn tags(argument: &Arg) -> String {
    notes(argument)
        .iter()
        .fold(String::new(), |mut tags, note| {
            let _tag: fmt::Result = write!(tags, " <span class=\"tag\">{}</span>", escaped(note));
            tags
        })
}

fn text_tags(argument: &Arg) -> String {
    let notes = notes(argument);
    if notes.is_empty() {
        String::new()
    } else {
        format!(" ({})", notes.join(", "))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_command_is_on_the_page_with_its_usage_and_options() {
        let page = html();
        for command in commands() {
            let name = command.get_name();
            assert!(
                page.contains(&format!("<section class=\"tool\" id=\"{name}\">")),
                "{name}"
            );
        }
        assert!(!page.contains("id=\"help\""), "clap's own help is left out");
        assert!(
            page.contains(
                "<pre><code>steamship upload [OPTIONS] --version &lt;VERSION&gt; &lt;SCRIPT&gt;</code></pre>"
            ),
            "{page}"
        );
        assert!(
            page.contains(
                "<dt><code>--account &lt;ACCOUNT&gt;</code> <span class=\"tag\">or STEAMSHIP_ACCOUNT</span></dt>"
            ),
            "{page}"
        );
        assert!(
            page.contains(
                "<dt><code>--count &lt;COUNT&gt;</code> <span class=\"tag\">default 10</span></dt>"
            ),
            "{page}"
        );
        assert!(
            page.contains("<dt><code>--version &lt;VERSION&gt;</code> <span class=\"tag\">required</span></dt>"),
            "{page}"
        );
        assert!(page.contains("<dt><code>--preview</code></dt>"), "{page}");
        assert!(
            page.contains(
                "<dt><code>&lt;SCRIPT&gt;</code> <span class=\"tag\">required</span></dt>"
            ),
            "{page}"
        );
        assert!(!page.contains("<dt><code>--help"), "{page}");
    }

    #[test]
    fn exit_codes_and_the_environment_come_from_the_cli() {
        let page = html();
        for (code, _) in EXIT_CODES {
            assert!(
                page.contains(&format!("<tr><td><code>{code}</code></td>")),
                "{code}"
            );
        }
        for (name, _) in VARIABLES {
            assert!(
                page.contains(&format!("<tr><td><code>{name}</code></td>")),
                "{name}"
            );
        }
    }

    #[test]
    fn the_markdown_copy_says_the_same() {
        let text = text();
        assert!(text.starts_with("# Commands\n\n"), "{text}");
        assert!(text.contains("\n## `steamship ci`\n\n"), "{text}");
        assert!(
            text.contains("\n- `--secret <SECRET>` (default STEAMSHIP_LOGIN): The name of the secret the login is kept in"),
            "{text}"
        );
        assert!(
            text.contains("| 3 | Not logged in, or the token expired; run `steamship login` |"),
            "{text}"
        );
        assert!(text.contains("| `STEAMSHIP_HOME` |"), "{text}");
    }

    #[test]
    fn the_longer_help_adds_only_what_the_one_line_does_not_say() {
        let login = commands()
            .into_iter()
            .find(|command| command.get_name() == "login");
        let (about, more) = login.as_ref().map(described).unwrap_or_default();
        assert_eq!(about, "Log in to Steam, once, for uploads.");
        assert_eq!(more.len(), 1, "{more:?}");
        assert!(
            more.iter()
                .all(|paragraph| paragraph.starts_with("Asks for the account's name")),
            "{more:?}"
        );
    }
}
