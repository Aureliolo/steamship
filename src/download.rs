//! Fetching from Valve's CDN, which answers 503 often enough that one attempt is not a download.

use std::error;
use std::fmt;
use std::fs::{self, File};
use std::io::{self, Read as _, Write as _};
use std::path::Path;
use std::thread;
use std::time::Duration;

use crate::digest::Hashing;

pub const CDN: &str = "https://cdn.steamstatic.com/client/";

/// Attempts in all. The waits between them double from one second, so the last one starts about
/// a minute after the first: long enough for the CDN's short outages, not long enough to hide a
/// real one.
const ATTEMPTS: u32 = 7;

#[derive(Debug)]
pub enum Error {
    /// The server said the file is not there, which retrying will not change.
    Gone {
        url: String,
        status: u16,
    },
    /// Every attempt failed on something that might have passed next time.
    GaveUp {
        url: String,
        last: String,
    },
    /// The bytes arrived but are not the ones pinned.
    Mismatch {
        url: String,
        reason: String,
    },
    Io(io::Error),
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Gone { url, status } => write!(formatter, "{url} answered {status}"),
            Self::GaveUp { url, last } => {
                write!(
                    formatter,
                    "{url} failed {ATTEMPTS} times; the last time: {last}"
                )
            }
            Self::Mismatch { url, reason } => write!(formatter, "{url} {reason}"),
            Self::Io(error) => write!(formatter, "{error}"),
        }
    }
}

impl error::Error for Error {}

impl From<io::Error> for Error {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

/// What one attempt came to.
enum Failure {
    Retry(String),
    Stop(Error),
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
pub fn text(url: &str) -> Result<String, Error> {
    text_waiting(url, thread::sleep)
}

fn text_waiting<Sleep>(url: &str, sleep: Sleep) -> Result<String, Error>
where
    Sleep: Fn(Duration),
{
    let agent = agent();
    with_retries(url, sleep, || {
        let mut response = agent
            .get(url)
            .call()
            .map_err(|error| classify(url, &error))?;
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
pub fn file(url: &str, into: &Path, size: u64, sha256: &[u8; 32]) -> Result<(), Error> {
    file_waiting(url, into, (size, sha256), thread::sleep)
}

fn file_waiting<Sleep>(
    url: &str,
    into: &Path,
    (size, sha256): (u64, &[u8; 32]),
    sleep: Sleep,
) -> Result<(), Error>
where
    Sleep: Fn(Duration),
{
    let agent = agent();
    let part = into.with_extension("part");
    with_retries(url, sleep, || {
        let response = agent
            .get(url)
            .call()
            .map_err(|error| classify(url, &error))?;
        let created = File::create(&part).map_err(|error| Failure::Stop(error.into()))?;
        let mut hashing = Hashing::new(created);
        // One byte more than pinned is read if it is there, which is how a longer file shows.
        let mut body = response
            .into_body()
            .into_reader()
            .take(size.saturating_add(1));
        let received =
            io::copy(&mut body, &mut hashing).map_err(|error| Failure::Retry(error.to_string()))?;
        if received > size {
            return Err(Failure::Stop(Error::Mismatch {
                url: url.to_owned(),
                reason: format!("is longer than the pinned {size} bytes"),
            }));
        }
        if received < size {
            return Err(Failure::Retry(format!(
                "ended after {received} of {size} bytes"
            )));
        }
        let (digest, mut out) = hashing.finish();
        if digest != *sha256 {
            return Err(Failure::Stop(Error::Mismatch {
                url: url.to_owned(),
                reason: "does not have the pinned SHA-256".to_owned(),
            }));
        }
        out.flush()
            .and_then(|()| out.sync_all())
            .map_err(|error| Failure::Stop(error.into()))
    })?;
    fs::rename(&part, into)?;
    Ok(())
}

/// A 5xx, a 429 or a broken connection may pass next time; any other answer will not.
fn classify(url: &str, error: &ureq::Error) -> Failure {
    if let &ureq::Error::StatusCode(status) = error {
        if status >= 500 || status == 429 {
            Failure::Retry(format!("answered {status}"))
        } else {
            Failure::Stop(Error::Gone {
                url: url.to_owned(),
                status,
            })
        }
    } else {
        Failure::Retry(error.to_string())
    }
}

fn with_retries<T, Sleep, Attempt>(
    url: &str,
    sleep: Sleep,
    mut attempt: Attempt,
) -> Result<T, Error>
where
    Sleep: Fn(Duration),
    Attempt: FnMut() -> Result<T, Failure>,
{
    let mut last = String::new();
    for tried in 0..ATTEMPTS {
        if let Some(wait) = tried.checked_sub(1) {
            sleep(Duration::from_secs(
                1_u64.checked_shl(wait).unwrap_or(u64::MAX),
            ));
        }
        match attempt() {
            Ok(value) => return Ok(value),
            Err(Failure::Stop(error)) => return Err(error),
            Err(Failure::Retry(reason)) => last = reason,
        }
    }
    Err(Error::GaveUp {
        url: url.to_owned(),
        last,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::iter;
    use std::net::TcpListener;

    use crate::digest;

    #[test]
    fn retries_what_may_pass_and_waits_longer_each_time() {
        let waits = RefCell::new(Vec::new());
        let mut calls = 0_u32;
        let result = with_retries(
            "u",
            |wait| waits.borrow_mut().push(wait.as_secs()),
            || {
                calls = calls.saturating_add(1);
                if calls < 4 {
                    Err(Failure::Retry("503".into()))
                } else {
                    Ok(calls)
                }
            },
        );
        assert_eq!(result.ok(), Some(4));
        assert_eq!(*waits.borrow(), [1, 2, 4]);
    }

    #[test]
    fn stops_at_once_on_what_will_not_pass() {
        let mut calls = 0_u32;
        let result: Result<(), _> = with_retries(
            "u",
            |_| {},
            || {
                calls = calls.saturating_add(1);
                Err(Failure::Stop(Error::Gone {
                    url: "u".into(),
                    status: 404,
                }))
            },
        );
        assert!(matches!(result, Err(Error::Gone { status: 404, .. })));
        assert_eq!(calls, 1);
    }

    #[test]
    fn gives_up_after_the_last_attempt_with_its_reason() {
        let mut calls = 0_u32;
        let result: Result<(), _> = with_retries(
            "u",
            |_| {},
            || {
                calls = calls.saturating_add(1);
                Err(Failure::Retry(format!("503 on try {calls}")))
            },
        );
        assert_eq!(calls, ATTEMPTS);
        assert!(matches!(result, Err(Error::GaveUp { last, .. }) if last == "503 on try 7"));
    }

    #[test]
    fn a_status_is_retried_only_when_it_may_pass() {
        for (status, retried) in [
            (500, true),
            (503, true),
            (429, true),
            (404, false),
            (403, false),
        ] {
            let failure = classify("u", &ureq::Error::StatusCode(status));
            assert_eq!(matches!(failure, Failure::Retry(_)), retried, "{status}");
        }
        assert!(matches!(
            classify("u", &ureq::Error::HostNotFound),
            Failure::Retry(_)
        ));
    }

    #[test]
    fn says_what_went_wrong() {
        let cases = [
            (
                Error::Gone {
                    url: "u".into(),
                    status: 404,
                },
                "u answered 404",
            ),
            (
                Error::GaveUp {
                    url: "u".into(),
                    last: "503".into(),
                },
                "u failed 7 times; the last time: 503",
            ),
            (
                Error::Mismatch {
                    url: "u".into(),
                    reason: "is short".into(),
                },
                "u is short",
            ),
        ];
        for (error, said) in cases {
            assert_eq!(error.to_string(), said);
        }
    }

    /// A full HTTP response with `body`.
    fn response(status: &str, body: &[u8]) -> Vec<u8> {
        let head = format!(
            "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        );
        [head.as_bytes(), body].concat()
    }

    /// Serves `responses` in order, one per connection, on a local port; answers the address.
    /// Any request past the script is answered with a server error, so code that asks more often
    /// than it should fails at once instead of waiting on a server that has nothing to say.
    fn serve(responses: Vec<Vec<u8>>) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/package", listener.local_addr().unwrap());
        let unscripted = response("599 Not In The Script", b"");
        let replies = responses.into_iter().chain(iter::repeat_n(unscripted, 16));
        drop(thread::spawn(move || {
            for reply in replies {
                let (mut stream, _) = listener.accept().unwrap();
                let mut request = Vec::new();
                let mut byte = [0_u8; 1];
                while !request.ends_with(b"\r\n\r\n") {
                    stream.read_exact(&mut byte).unwrap();
                    request.extend_from_slice(&byte);
                }
                stream.write_all(&reply).unwrap();
            }
        }));
        url
    }

    fn pinned(body: &[u8]) -> (u64, [u8; 32]) {
        let size = u64::try_from(body.len()).unwrap();
        (size, digest::sha256(&mut &*body).unwrap())
    }

    const BODY: &[u8] = b"the pinned package";

    fn fetch(responses: Vec<Vec<u8>>) -> (Result<(), Error>, tempfile::TempDir) {
        let folder = tempfile::tempdir().unwrap();
        let (size, sha256) = pinned(BODY);
        let into = folder.path().join("package.zip");
        let result = file_waiting(&serve(responses), &into, (size, &sha256), |_| {});
        (result, folder)
    }

    #[test]
    fn a_download_that_fails_then_passes_is_kept_once_checked() {
        let (result, folder) = fetch(vec![
            response("503 Service Unavailable", b""),
            response("200 OK", BODY),
        ]);
        assert!(matches!(result, Ok(())), "{result:?}");
        let into = folder.path().join("package.zip");
        assert_eq!(fs::read(&into).unwrap(), BODY);
        assert!(!into.with_extension("part").exists());
    }

    #[test]
    fn a_short_file_is_fetched_again() {
        let (result, _folder) = fetch(vec![
            response("200 OK", BODY.first_chunk::<4>().unwrap()),
            response("200 OK", BODY),
        ]);
        assert!(matches!(result, Ok(())), "{result:?}");
    }

    #[test]
    fn a_file_that_is_not_the_pinned_one_is_refused_at_once() {
        let longer = [BODY, b"!"].concat();
        let (result, folder) = fetch(vec![response("200 OK", &longer)]);
        let into = folder.path().join("package.zip");
        assert!(
            matches!(&result, Err(Error::Mismatch { reason, .. }) if reason.starts_with("is longer")),
            "{result:?}"
        );
        assert!(!into.exists());
        let changed = BODY.to_ascii_uppercase();
        let (refused, _folder) = fetch(vec![response("200 OK", &changed)]);
        assert!(
            matches!(&refused, Err(Error::Mismatch { reason, .. }) if reason.contains("SHA-256")),
            "{refused:?}"
        );
    }

    #[test]
    fn a_missing_file_is_not_retried() {
        let (result, _folder) = fetch(vec![response("404 Not Found", b"")]);
        assert!(
            matches!(result, Err(Error::Gone { status: 404, .. })),
            "{result:?}"
        );
    }

    #[test]
    fn text_is_fetched_with_the_same_retries() {
        let url = serve(vec![
            response("502 Bad Gateway", b""),
            response("200 OK", b"\"linux\" {}"),
        ]);
        let result = text_waiting(&url, |_| {});
        assert_eq!(result.ok().as_deref(), Some("\"linux\" {}"));
    }

    #[test]
    fn the_public_calls_fetch_the_same_way() {
        let url = serve(vec![response("200 OK", b"text")]);
        assert_eq!(text(&url).ok().as_deref(), Some("text"));
        let folder = tempfile::tempdir().unwrap();
        let into = folder.path().join("package.zip");
        let (size, sha256) = pinned(BODY);
        let package = serve(vec![response("200 OK", BODY)]);
        assert!(matches!(file(&package, &into, size, &sha256), Ok(())));
        assert_eq!(fs::read(&into).unwrap(), BODY);
    }

    #[test]
    fn a_failure_to_write_is_said_as_it_is() {
        let error = Error::from(io::Error::new(io::ErrorKind::PermissionDenied, "no"));
        assert_eq!(error.to_string(), "no");
    }
}
