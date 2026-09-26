//! Fetching from Valve's CDN, which answers 503 often enough that one attempt is not a download.

use std::fmt;
use std::fs;
use std::io::{self, Read, Write};
use std::path::Path;
use std::time::Duration;

use sha2::{Digest, Sha256};

pub const CDN: &str = "https://cdn.steamstatic.com/client/";

/// Attempts in all. The waits between them double from one second, so the last one starts about
/// a minute after the first: long enough for the CDN's short outages, not long enough to hide a
/// real one.
const ATTEMPTS: u32 = 7;

#[derive(Debug)]
pub enum DownloadError {
    /// The server said the file is not there, which retrying will not change.
    Gone { url: String, status: u16 },
    /// Every attempt failed on something that might have passed next time.
    GaveUp { url: String, last: String },
    /// The bytes arrived but are not the ones pinned.
    Mismatch { url: String, reason: String },
    Io(io::Error),
}

impl fmt::Display for DownloadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Gone { url, status } => write!(f, "{url} answered {status}"),
            Self::GaveUp { url, last } => {
                write!(f, "{url} failed {ATTEMPTS} times; the last time: {last}")
            }
            Self::Mismatch { url, reason } => write!(f, "{url} {reason}"),
            Self::Io(error) => write!(f, "{error}"),
        }
    }
}

impl std::error::Error for DownloadError {}

impl From<io::Error> for DownloadError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

/// What one attempt came to.
enum Failure {
    Retry(String),
    Stop(DownloadError),
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_connect(Some(Duration::from_secs(30)))
        .timeout_recv_response(Some(Duration::from_secs(60)))
        .timeout_recv_body(Some(Duration::from_secs(600)))
        .build()
        .into()
}

/// The body of `url` as text, for a manifest.
///
/// # Errors
///
/// When the server refuses, or keeps failing, or the body is not UTF-8.
pub fn text(url: &str) -> Result<String, DownloadError> {
    let agent = agent();
    with_retries(url, std::thread::sleep, || {
        let mut response = agent.get(url).call().map_err(|error| classify(url, error))?;
        response
            .body_mut()
            .read_to_string()
            .map_err(|error| Failure::Retry(error.to_string()))
    })
}

/// Downloads `url` to `into`, refusing it unless it is exactly `size` bytes with SHA-256 `sha256`.
/// The bytes go to a neighbouring `.part` file first, so `into` only ever holds a checked file.
///
/// # Errors
///
/// When the server refuses or keeps failing, the file is not the pinned one, or writing fails.
pub fn file(url: &str, into: &Path, size: u64, sha256: &[u8; 32]) -> Result<(), DownloadError> {
    let agent = agent();
    let part = into.with_extension("part");
    with_retries(url, std::thread::sleep, || {
        let response = agent.get(url).call().map_err(|error| classify(url, error))?;
        let mut body = response.into_body().into_reader();
        let mut out = fs::File::create(&part).map_err(|e| Failure::Stop(e.into()))?;
        let mut hasher = Sha256::new();
        let mut received: u64 = 0;
        let mut buffer = vec![0u8; 64 * 1024];
        loop {
            let count = match body.read(&mut buffer) {
                Ok(0) => break,
                Ok(count) => count,
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(error) => return Err(Failure::Retry(error.to_string())),
            };
            received += count as u64;
            if received > size {
                return Err(mismatch(url, format!("is longer than the pinned {size} bytes")));
            }
            hasher.update(&buffer[..count]);
            out.write_all(&buffer[..count]).map_err(|e| Failure::Stop(e.into()))?;
        }
        if received != size {
            return Err(Failure::Retry(format!("ended after {received} of {size} bytes")));
        }
        if hasher.finalize().as_slice() != sha256 {
            return Err(mismatch(url, "does not have the pinned SHA-256".to_owned()));
        }
        out.sync_all().map_err(|e| Failure::Stop(e.into()))?;
        Ok(())
    })
    .inspect_err(|_| {
        let _ = fs::remove_file(&part);
    })?;
    fs::rename(&part, into)?;
    Ok(())
}

fn mismatch(url: &str, reason: String) -> Failure {
    Failure::Stop(DownloadError::Mismatch { url: url.to_owned(), reason })
}

/// A 5xx, a 429 or a broken connection may pass next time; any other answer will not.
fn classify(url: &str, error: ureq::Error) -> Failure {
    match error {
        ureq::Error::StatusCode(status) if status >= 500 || status == 429 => {
            Failure::Retry(format!("answered {status}"))
        }
        ureq::Error::StatusCode(status) => {
            Failure::Stop(DownloadError::Gone { url: url.to_owned(), status })
        }
        other => Failure::Retry(other.to_string()),
    }
}

fn with_retries<T>(
    url: &str,
    sleep: impl Fn(Duration),
    mut attempt: impl FnMut() -> Result<T, Failure>,
) -> Result<T, DownloadError> {
    let mut last = String::new();
    for tried in 0..ATTEMPTS {
        if tried > 0 {
            sleep(Duration::from_secs(1 << (tried - 1)));
        }
        match attempt() {
            Ok(value) => return Ok(value),
            Err(Failure::Stop(error)) => return Err(error),
            Err(Failure::Retry(reason)) => last = reason,
        }
    }
    Err(DownloadError::GaveUp { url: url.to_owned(), last })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    #[test]
    fn retries_what_may_pass_and_waits_longer_each_time() {
        let waits = RefCell::new(Vec::new());
        let mut calls = 0;
        let result = with_retries(
            "u",
            |wait| waits.borrow_mut().push(wait.as_secs()),
            || {
                calls += 1;
                if calls < 4 { Err(Failure::Retry("503".into())) } else { Ok(calls) }
            },
        );
        assert_eq!(result.ok(), Some(4));
        assert_eq!(*waits.borrow(), [1, 2, 4]);
    }

    #[test]
    fn stops_at_once_on_what_will_not_pass() {
        let mut calls = 0;
        let result: Result<(), _> = with_retries(
            "u",
            |_| {},
            || {
                calls += 1;
                Err(Failure::Stop(DownloadError::Gone { url: "u".into(), status: 404 }))
            },
        );
        assert!(matches!(result, Err(DownloadError::Gone { status: 404, .. })));
        assert_eq!(calls, 1);
    }

    #[test]
    fn gives_up_after_the_last_attempt_with_its_reason() {
        let mut calls = 0;
        let result: Result<(), _> = with_retries(
            "u",
            |_| {},
            || {
                calls += 1;
                Err(Failure::Retry(format!("503 on try {calls}")))
            },
        );
        assert_eq!(calls, ATTEMPTS);
        assert!(
            matches!(result, Err(DownloadError::GaveUp { last, .. }) if last == "503 on try 7")
        );
    }

    #[test]
    fn a_status_is_retried_only_when_it_may_pass() {
        assert!(matches!(classify("u", ureq::Error::StatusCode(503)), Failure::Retry(_)));
        assert!(matches!(classify("u", ureq::Error::StatusCode(429)), Failure::Retry(_)));
        assert!(matches!(
            classify("u", ureq::Error::StatusCode(404)),
            Failure::Stop(DownloadError::Gone { status: 404, .. })
        ));
    }
}
