//! How steamship's commands look.
//!
//! A title, then one aligned line per step, a tick or a cross at the end, and a spinner while
//! steamcmd is silent. Colour and the spinner appear only on a terminal, and colour not with
//! `NO_COLOR` set; written anywhere else, the same lines are plain.

use std::io::{self, IsTerminal as _, Write};
use std::mem;
use std::panic;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use anstream::AutoStream;
use anstyle::{AnsiColor, Style};
use clap::builder::styling::Styles;

const DIM: Style = Style::new().dimmed();
const BOLD: Style = Style::new().bold();
const GREEN: Style = AnsiColor::Green.on_default();
const RED: Style = AnsiColor::Red.on_default();
const YELLOW: Style = AnsiColor::Yellow.on_default();
const CYAN: Style = AnsiColor::Cyan.on_default();

/// The width labels are padded to, so that what follows them lines up.
const LABEL: usize = 10;

const FRAMES: [char; 10] = [
    '\u{280b}', '\u{2819}', '\u{2839}', '\u{2838}', '\u{283c}', '\u{2834}', '\u{2826}', '\u{2827}',
    '\u{2807}', '\u{280f}',
];

const BLUE: Style = AnsiColor::Blue.on_default();

/// The colours of `--help`: headings bold, commands and options as the hints show a command,
/// and what to fill in dim.
pub const HELP: Styles = Styles::styled()
    .header(BOLD)
    .usage(BOLD)
    .literal(CYAN.bold())
    .placeholder(DIM)
    .valid(GREEN)
    .invalid(YELLOW)
    .error(RED.bold());

/// steamship's ship, a line at a time, as the banner draws it: mast, sail, deck, hull and sea.
/// The documentation site draws the same one.
pub const SHIP: [&str; 5] = [
    "      |\\",
    "      | \\",
    "   ___|__\\___",
    "   \\_________/",
    " ~~~~~~~~~~~~~~~",
];

/// What steamship does, beside the ship.
pub const TAGLINE: &str = "uploads your build to Steam";

/// A ship beside the name, the version and what steamship does.
#[must_use]
pub fn banner() -> String {
    let version = env!("CARGO_PKG_VERSION");
    let [mast, sail, deck, hull, sea] = SHIP;
    format!(
        "{BLUE}{mast}{BLUE:#}\n\
         {BLUE}{sail}{BLUE:#}         {BOLD}steamship{BOLD:#} {DIM}{version}{DIM:#}\n\
         {BLUE}{deck}{BLUE:#}     {DIM}{TAGLINE}{DIM:#}\n\
         {BLUE}{hull}{BLUE:#}\n\
         {BLUE}{DIM}{sea}{DIM:#}{BLUE:#}"
    )
}

/// The banner and a blank line, on a terminal only: a log has no use for it.
pub fn banner_on_terminal() {
    if io::stdout().is_terminal() {
        anstream::println!("{}\n", banner());
    }
}

/// `steamship <command>`, the first thing a command shows.
pub fn title(command: &str) {
    anstream::println!("{BOLD}steamship{BOLD:#} {DIM}{command}{DIM:#}");
}

/// A step: a label, then what it came to.
pub fn field(label: &str, value: &str) {
    // A label as wide as the column, such as an achievement's API name, still gets a space.
    let width = LABEL.max(label.chars().count().saturating_add(1));
    anstream::println!("  {DIM}{label:<width$}{DIM:#}{value}");
}

/// A step that went as it should.
pub fn done(label: &str, value: &str) {
    field(label, &format!("{GREEN}\u{2713}{GREEN:#} {value}"));
}

/// A step that did not go as it should.
pub fn failed(label: &str, value: &str) {
    field(label, &format!("{RED}\u{2717}{RED:#} {value}"));
}

/// The label of a prompt, left for what is typed to follow on the same line.
///
/// # Errors
///
/// When it cannot be shown.
pub fn prompt(label: &str) -> io::Result<()> {
    anstream::print!("  {DIM}{label:<LABEL$}{DIM:#}");
    io::stdout().flush()
}

/// The command's outcome, when it is what was asked for.
pub fn success(headline: &str, detail: &str) {
    anstream::println!("  {GREEN}\u{2713}{GREEN:#} {BOLD}{headline}{BOLD:#}");
    if !detail.is_empty() {
        anstream::println!("    {DIM}{detail}{DIM:#}");
    }
}

/// The command's outcome, when it is not. `hint`, if any, says what to do, with its `command`
/// set apart.
pub fn failure(headline: &str, reason: &str, hint: Option<Hint<'_>>) {
    let reason = if reason.is_empty() {
        String::new()
    } else {
        format!(": {reason}")
    };
    anstream::eprintln!("  {RED}\u{2717}{RED:#} {BOLD}{headline}{BOLD:#}{reason}");
    if let Some(hint) = hint {
        hinted(hint);
    }
}

/// What to do next, under the failures it follows.
pub fn hint(hint: Hint<'_>) {
    hinted(hint);
}

/// Text for the program reading steamship's output rather than a person, as it is: GitHub
/// Actions' workflow commands, which it reads only from the start of a line on standard output.
pub fn plain(text: &str) {
    anstream::print!("{text}");
}

fn hinted(
    Hint {
        before,
        command,
        after,
    }: Hint<'_>,
) {
    anstream::eprintln!("    {DIM}{before}{DIM:#}{CYAN}{command}{CYAN:#}{DIM}{after}{DIM:#}");
}

/// That steamship `latest` is out while this is `this`, and `how` to upgrade, set apart from what
/// the command itself printed.
pub fn upgrade(latest: &str, this: &str, how: Hint<'_>) {
    anstream::eprintln!();
    anstream::eprintln!(
        "  {YELLOW}\u{2191}{YELLOW:#} {BOLD}steamship {latest} is out{BOLD:#}{DIM}, this is {this}{DIM:#}"
    );
    hinted(how);
}

/// What to do after a failure: `before`, then a `command` to run, then `after`.
#[derive(Debug, Clone, Copy)]
pub struct Hint<'text> {
    pub before: &'text str,
    pub command: &'text str,
    pub after: &'text str,
}

/// More about a failure, under it.
pub fn note(line: &str) {
    anstream::eprintln!("    {DIM}{line}{DIM:#}");
}

/// Text to be copied elsewhere as it is, after a blank line and without colour or indent added.
pub fn verbatim(text: &str) {
    anstream::println!();
    anstream::print!("{text}");
}

/// A line of another program's that steamship has no meaning for, passed on, set apart.
pub fn aside(line: &str) {
    anstream::println!("    {DIM}{line}{DIM:#}");
}

/// `count` `noun`s, the count in groups of three digits.
#[must_use]
pub fn counted(count: usize, noun: &str) -> String {
    let digits = count.to_string();
    let mut grouped = String::with_capacity(digits.len().saturating_mul(2));
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && digits.len().saturating_sub(index).is_multiple_of(3) {
            grouped.push(',');
        }
        grouped.push(digit);
    }
    let plural = if count == 1 { "" } else { "s" };
    format!("{grouped} {noun}{plural}")
}

/// A duration as a person reads it: seconds up to two minutes, then whole minutes.
#[must_use]
pub fn took(elapsed: Duration) -> String {
    let seconds = elapsed.as_secs();
    if seconds < 120 {
        format!("{seconds} s")
    } else {
        format!("{} min", seconds.div_euclid(60))
    }
}

/// How long each frame of a spinner is shown.
const FRAME: Duration = Duration::from_millis(90);

/// Draws a spinner on `out` until `running` goes false, then clears its line. Each frame is drawn
/// over the last from the start of the line, and the line is cleared by writing spaces over it:
/// with `NO_COLOR` set every escape sequence is left out, the one that clears a line included.
fn spin<Out>(
    out: &mut Out,
    running: &AtomicBool,
    label: &str,
    text: &str,
    clock: bool,
    started: Instant,
) -> io::Result<()>
where
    Out: Write,
{
    let mut widest = 0;
    for frame in FRAMES.iter().cycle() {
        if !running.load(Ordering::Relaxed) {
            break;
        }
        let clock = if clock {
            format!(" {} s", started.elapsed().as_secs())
        } else {
            String::new()
        };
        let plain = format!("  {label:<LABEL$}{frame} {text}{clock}");
        widest = widest.max(plain.chars().count());
        write!(
            out,
            "\r  {DIM}{label:<LABEL$}{DIM:#}{YELLOW}{frame}{YELLOW:#} {text}{DIM}{clock}{DIM:#}"
        )?;
        out.flush()?;
        thread::sleep(FRAME);
    }
    write!(out, "\r{:widest$}\r", "")?;
    out.flush()
}

/// Standard output, when it is a terminal a spinner can be drawn on.
fn terminal() -> Option<AutoStream<io::Stdout>> {
    io::stdout()
        .is_terminal()
        .then(|| AutoStream::auto(io::stdout()))
}

/// A step under way, with a spinner and how long it has taken, redrawn on a terminal. Anywhere
/// else it is written once, and its end once more.
#[derive(Debug)]
pub struct Spinner {
    label: String,
    started: Instant,
    running: Arc<AtomicBool>,
    /// Output that could not be written ends the drawing early, with nowhere left to show it.
    drawing: Option<JoinHandle<io::Result<()>>>,
}

impl Spinner {
    /// Starts showing `label` and `text`; `clock` adds the time it has taken.
    #[must_use]
    pub fn start(label: &str, text: &str, clock: bool) -> Self {
        Self::begun(label, text, clock, Instant::now(), terminal())
    }

    /// A spinner for a step that began at `started`, drawn on `out`, or written once without.
    fn begun<Out>(label: &str, text: &str, clock: bool, started: Instant, out: Option<Out>) -> Self
    where
        Out: Write + Send + 'static,
    {
        let running = Arc::new(AtomicBool::new(true));
        let drawing = out.map_or_else(
            || {
                field(label, &format!("{text}..."));
                None
            },
            |mut out| {
                let running = Arc::clone(&running);
                let (label, text) = (label.to_owned(), text.to_owned());
                Some(thread::spawn(move || {
                    spin(&mut out, &running, &label, &text, clock, started)
                }))
            },
        );
        Self {
            label: label.to_owned(),
            started,
            running,
            drawing,
        }
    }

    /// How long the step has taken so far.
    #[must_use]
    pub fn elapsed(&self) -> Duration {
        self.started.elapsed()
    }

    /// Stops the spinner, and waits for its line to be cleared so that nothing written after is
    /// cleared with it.
    fn halt(&mut self) {
        self.running.store(false, Ordering::Relaxed);
        if let Some(drawing) = self.drawing.take()
            && let Err(panic) = drawing.join()
        {
            panic::resume_unwind(panic);
        }
    }

    /// Stops the spinner, leaving the step's label for what came of it.
    fn stop(mut self) -> String {
        self.halt();
        mem::take(&mut self.label)
    }

    /// Ends the step as done.
    pub fn done(self, value: &str) {
        let label = self.stop();
        done(&label, value);
    }

    /// Ends the step as failed.
    pub fn failed(self, value: &str) {
        let label = self.stop();
        failed(&label, value);
    }

    /// Stops the spinner while something else is shown, and starts it again after.
    #[must_use]
    pub fn around<Show>(self, show: Show, text: &str, clock: bool) -> Self
    where
        Show: FnOnce(),
    {
        let started = self.started;
        let label = self.stop();
        show();
        Self::begun(&label, text, clock, started, terminal())
    }
}

impl Drop for Spinner {
    fn drop(&mut self) {
        self.halt();
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;

    #[test]
    fn the_banner_names_steamship_its_version_and_what_it_does() {
        let banner = banner();
        assert!(banner.contains("steamship"), "{banner}");
        assert!(banner.contains(env!("CARGO_PKG_VERSION")), "{banner}");
        assert!(banner.contains("uploads your build to Steam"), "{banner}");
        assert_eq!(banner.lines().count(), 5, "{banner}");
    }

    #[test]
    fn a_count_is_grouped_in_threes_and_its_noun_agrees() {
        let cases = [
            (0, "0 files"),
            (1, "1 file"),
            (999, "999 files"),
            (1_000, "1,000 files"),
            (1_284, "1,284 files"),
            (100_000, "100,000 files"),
            (1_234_567, "1,234,567 files"),
        ];
        for (count, expected) in cases {
            assert_eq!(counted(count, "file"), expected);
        }
    }

    #[test]
    fn a_duration_is_in_seconds_up_to_two_minutes_then_in_minutes() {
        let cases = [
            (0, "0 s"),
            (119, "119 s"),
            (120, "2 min"),
            (179, "2 min"),
            (7_200, "120 min"),
        ];
        for (seconds, expected) in cases {
            assert_eq!(took(Duration::from_secs(seconds)), expected);
        }
    }

    /// Output a test can read back while a spinner still holds it.
    #[derive(Clone, Default)]
    struct Shared(Arc<Mutex<Vec<u8>>>);

    impl Shared {
        fn text(&self) -> String {
            String::from_utf8(self.0.lock().unwrap().clone()).unwrap()
        }
    }

    impl Write for Shared {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.0
                .lock()
                .map_err(|poisoned| io::Error::other(poisoned.to_string()))?
                .extend_from_slice(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn a_spinner_redraws_its_step_until_done_then_clears_it() {
        let out = Shared::default();
        let spinner = Spinner::begun(
            "steam",
            "uploading",
            true,
            Instant::now(),
            Some(out.clone()),
        );
        thread::sleep(FRAME.saturating_mul(4));
        spinner.done("uploaded");
        let drawn = out.text();
        let first = format!(
            "\r  {DIM}steam     {DIM:#}{YELLOW}\u{280b}{YELLOW:#} uploading{DIM} 0 s{DIM:#}\r"
        );
        assert!(drawn.starts_with(&first), "{drawn:?}");
        let second = format!("\u{2819}{YELLOW:#} uploading");
        assert!(drawn.contains(&second), "{drawn:?}");
        // As wide as "  steam     ⠋ uploading 0 s", the widest frame.
        assert!(
            drawn.ends_with(&format!("\r{}\r", " ".repeat(27))),
            "{drawn:?}"
        );
        thread::sleep(FRAME.saturating_mul(2));
        assert_eq!(out.text(), drawn, "drawn after it was done");
    }

    #[test]
    fn a_spinner_without_a_clock_draws_no_time_and_a_dropped_one_stops() {
        let out = Shared::default();
        let spinner = Spinner::begun(
            "approve",
            "waiting",
            false,
            Instant::now(),
            Some(out.clone()),
        );
        thread::sleep(FRAME.saturating_mul(2));
        drop(spinner);
        let drawn = out.text();
        assert!(drawn.contains("waiting"), "{drawn:?}");
        assert!(!drawn.contains(" s"), "{drawn:?}");
        assert!(
            drawn.ends_with(&format!("\r{}\r", " ".repeat(21))),
            "{drawn:?}"
        );
    }

    #[test]
    fn a_spinner_times_its_step_from_when_it_began() {
        let began = Instant::now().checked_sub(Duration::from_secs(5)).unwrap();
        let spinner = Spinner::begun("steam", "uploading", true, began, None::<Shared>);
        assert!(spinner.elapsed() >= Duration::from_secs(5));
        let again = spinner.around(|| {}, "uploading", true);
        assert!(again.elapsed() >= Duration::from_secs(5));
    }
}
