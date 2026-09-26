//! How steamship's commands look.
//!
//! A title, then one aligned line per step, a tick or a cross at the end, and a spinner while
//! steamcmd is silent. Colour and the spinner appear only on a terminal, and colour not with
//! `NO_COLOR` set; written anywhere else, the same lines are plain.

use std::io::{self, IsTerminal as _, Write as _};
use std::mem;
use std::panic;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use anstream::AutoStream;
use anstyle::{AnsiColor, Style};

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

/// `steamship <command>`, the first thing a command shows.
pub fn title(command: &str) {
    anstream::println!("{BOLD}steamship{BOLD:#} {DIM}{command}{DIM:#}");
}

/// A step: a label, then what it came to.
pub fn field(label: &str, value: &str) {
    anstream::println!("  {DIM}{label:<LABEL$}{DIM:#}{value}");
}

/// A step that went as it should.
pub fn done(label: &str, value: &str) {
    field(label, &format!("{GREEN}\u{2713}{GREEN:#} {value}"));
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
    if let Some(Hint {
        before,
        command,
        after,
    }) = hint
    {
        anstream::eprintln!("    {DIM}{before}{DIM:#}{CYAN}{command}{CYAN:#}{DIM}{after}{DIM:#}");
    }
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

/// A line of another program's that steamship has no meaning for, passed on, set apart.
pub fn aside(line: &str) {
    anstream::println!("    {DIM}{line}{DIM:#}");
}

/// Draws a spinner until `running` goes false, then clears its line. Output that cannot be
/// written ends it early: there is nowhere left to show it.
fn spin(running: &AtomicBool, label: &str, text: &str, clock: bool, started: Instant) {
    let mut out = AutoStream::auto(io::stdout());
    for frame in FRAMES.iter().cycle() {
        let going = running.load(Ordering::Relaxed);
        let line = if going {
            let clock = if clock {
                format!(" {DIM}{} s{DIM:#}", started.elapsed().as_secs())
            } else {
                String::new()
            };
            format!("\r\x1b[2K  {DIM}{label:<LABEL$}{DIM:#}{YELLOW}{frame}{YELLOW:#} {text}{clock}")
        } else {
            "\r\x1b[2K".to_owned()
        };
        if write!(out, "{line}").and_then(|()| out.flush()).is_err() || !going {
            return;
        }
        thread::sleep(Duration::from_millis(90));
    }
}

/// A step under way, with a spinner and how long it has taken, redrawn on a terminal. Anywhere
/// else it is written once, and its end once more.
#[derive(Debug)]
pub struct Spinner {
    label: String,
    started: Instant,
    running: Arc<AtomicBool>,
    drawing: Option<JoinHandle<()>>,
}

impl Spinner {
    /// Starts showing `label` and `text`; `clock` adds the time it has taken.
    #[must_use]
    pub fn start(label: &str, text: &str, clock: bool) -> Self {
        let started = Instant::now();
        let running = Arc::new(AtomicBool::new(true));
        let drawing = if io::stdout().is_terminal() {
            let running = Arc::clone(&running);
            let (label, text) = (label.to_owned(), text.to_owned());
            Some(thread::spawn(move || {
                spin(&running, &label, &text, clock, started);
            }))
        } else {
            field(label, &format!("{text}..."));
            None
        };
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
            && !thread::panicking()
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
        field(&label, &format!("{RED}\u{2717}{RED:#} {value}"));
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
        let mut again = Self::start(&label, text, clock);
        again.started = started;
        again
    }
}

impl Drop for Spinner {
    fn drop(&mut self) {
        self.halt();
    }
}
