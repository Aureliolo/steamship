//! Fetching from Valve's CDN, which answers 503 often enough that one attempt is not a download.

use std::error;
use std::fmt;
use std::fs::{self, File};
use std::io::{self, Read as _, Write as _};
use std::path::Path;
use std::thread;
use std::time::Duration;

use crate::digest::Hashing;
use crate::webapi;

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
                    "{url} failed {ATTEMPTS} times (the last time: {last})"
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
    with_retries(url, sleep, || read_text(&agent, url, None))
}

/// One attempt at `url`'s body as text. An `accept` given is sent as its Accept header.
fn read_text(agent: &ureq::Agent, url: &str, accept: Option<&str>) -> Result<String, Failure> {
    let mut request = agent.get(url);
    if let Some(accept) = accept {
        request = request.header("Accept", accept);
    }
    let mut response = request.call().map_err(|error| classify(url, &error))?;
    response
        .body_mut()
        .read_to_string()
        .map_err(|error| Failure::Retry(webapi::unanswered(&error)))
}

/// Where a GitHub release asset is served: its download address, and the API's listing of its
/// release, whose asset of that name must be served from under `assets`.
#[derive(Debug, Clone, Copy)]
pub struct Asset<'url> {
    pub download: &'url str,
    pub release: &'url str,
    pub name: &'url str,
    pub assets: &'url str,
}

/// The body of `asset` as text. Each attempt asks its download address, and when that fails, the
/// API for the same asset: GitHub's download front has answered 504 for one asset while the API
/// served it.
///
/// # Errors
///
/// When both refuse, or keep failing, or the body is not UTF-8.
pub fn asset_text(asset: Asset<'_>) -> Result<String, Error> {
    asset_text_waiting(asset, thread::sleep)
}

fn asset_text_waiting<Sleep>(asset: Asset<'_>, sleep: Sleep) -> Result<String, Error>
where
    Sleep: Fn(Duration),
{
    let agent = agent();
    with_retries(asset.download, sleep, || {
        let direct = match read_text(&agent, asset.download, None) {
            Ok(text) => return Ok(text),
            Err(failure) => failure,
        };
        match (direct, through_api(&agent, asset)) {
            (_, Ok(text)) => Ok(text),
            (Failure::Stop(error), Err(Failure::Stop(_))) => Err(Failure::Stop(error)),
            (direct, Err(api)) => Err(Failure::Retry(format!(
                "{}, and through GitHub's API: {}",
                reason(&direct),
                reason(&api)
            ))),
        }
    })
}

/// What a failed attempt says of itself, as the last reason of a retried one.
fn reason(failure: &Failure) -> String {
    match failure {
        Failure::Retry(reason) => reason.clone(),
        Failure::Stop(Error::Gone { status, .. }) => format!("answered {status}"),
        Failure::Stop(error) => error.to_string(),
    }
}

/// One attempt at `asset` through the API: its release's listing, then the asset it names.
fn through_api(agent: &ureq::Agent, asset: Asset<'_>) -> Result<String, Failure> {
    let listing = read_text(agent, asset.release, Some("application/vnd.github+json"))?;
    let release: serde_json::Value = serde_json::from_str(&listing)
        .map_err(|error| Failure::Retry(format!("a release that is not JSON: {error}")))?;
    let url = release
        .get("assets")
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .find(|listed| listed.get("name").and_then(serde_json::Value::as_str) == Some(asset.name))
        .and_then(|listed| listed.get("url"))
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| {
            Failure::Stop(Error::Mismatch {
                url: asset.release.to_owned(),
                reason: format!("lists no {}", asset.name),
            })
        })?;
    if !url.starts_with(asset.assets) {
        return Err(Failure::Stop(Error::Mismatch {
            url: asset.release.to_owned(),
            reason: format!("serves {} from {url}, outside {}", asset.name, asset.assets),
        }));
    }
    read_text(agent, url, Some("application/octet-stream"))
}

/// Downloads `url` to `into`. A file that is not exactly `size` bytes with SHA-256 `sha256` is
/// refused.
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
        let received = io::copy(&mut body, &mut hashing)
            .map_err(|error| Failure::Retry(copy_failed(error)))?;
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

/// How long apart a file the CDN does not serve yet is asked for.
const APART: Duration = Duration::from_secs(30);

/// What asking until the CDN serves a file came to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Served {
    /// The CDN serves it.
    Yes,
    /// The CDN refused in some other way than "not found", as it does once the request code in
    /// the address has expired. The number is the status it answered.
    Otherwise(u16),
    /// Still not found when patience ran out.
    No,
}

/// Asks for `url` until the CDN serves it or refuses it for some other reason than not having it.
///
/// It is asked at once, then every 30 seconds while `patience` lasts. What may pass next time, a
/// 5xx or a broken connection, is waited through like a 404.
#[must_use]
pub fn served(url: &str, patience: Duration) -> Served {
    served_waiting(url, patience, thread::sleep)
}

fn served_waiting<Sleep>(url: &str, patience: Duration, sleep: Sleep) -> Served
where
    Sleep: Fn(Duration),
{
    let agent = agent();
    let mut left = patience;
    loop {
        match agent.get(url).call().map_err(|error| classify(url, &error)) {
            Ok(_) => return Served::Yes,
            Err(Failure::Stop(Error::Gone { status, .. })) if status != 404 => {
                return Served::Otherwise(status);
            }
            Err(_) => {}
        }
        let Some(rest) = left.checked_sub(APART) else {
            return Served::No;
        };
        sleep(APART);
        left = rest;
    }
}

/// A 5xx, a 429 or a broken connection may pass next time. Any other answer will not.
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
        Failure::Retry(webapi::unanswered(error))
    }
}

/// Why copying a body to disk failed. ureq carries its own errors inside an `io::Error`, and a
/// plain one is the disk's, which a person should not read as a failed connection.
fn copy_failed(error: io::Error) -> String {
    let error = ureq::Error::from(error);
    if let ureq::Error::Io(inner) = &error {
        inner.to_string()
    } else {
        webapi::unanswered(&error)
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
    use std::cell::{Cell, RefCell};
    use std::iter;
    use std::net::TcpListener;
    use std::sync::mpsc;

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

    fn retry_text(failure: Failure) -> String {
        match failure {
            Failure::Retry(text) => text,
            Failure::Stop(error) => panic!("stopped: {error}"),
        }
    }

    #[test]
    fn a_proxy_that_fails_is_named_without_its_address() {
        let failure = classify(
            "u",
            &ureq::Error::ConnectProxyFailed("user:secret@proxy.example".to_owned()),
        );
        assert_eq!(retry_text(failure), "the proxy failed");
    }

    #[test]
    fn a_bad_address_is_named_without_its_text() {
        let failure = classify(
            "u",
            &ureq::Error::BadUri("http://user:secret@proxy.example".to_owned()),
        );
        assert_eq!(retry_text(failure), "the request failed");
    }

    #[test]
    fn giving_up_on_a_proxy_that_fails_says_so_without_its_address() {
        let result: Result<(), _> = with_retries(
            "u",
            |_| {},
            || {
                Err(classify(
                    "u",
                    &ureq::Error::ConnectProxyFailed("user:secret@proxy.example".to_owned()),
                ))
            },
        );
        assert_eq!(
            result.unwrap_err().to_string(),
            "u failed 7 times (the last time: the proxy failed)"
        );
    }

    #[test]
    fn a_copy_that_ureq_failed_is_named_without_what_a_proxy_address_holds() {
        let wrapped =
            ureq::Error::ConnectProxyFailed("user:secret@proxy.example".to_owned()).into_io();
        assert_eq!(copy_failed(wrapped), "the proxy failed");
    }

    #[test]
    fn a_copy_that_the_disk_failed_keeps_its_own_text() {
        let plain = io::Error::new(io::ErrorKind::StorageFull, "no room left");
        assert_eq!(copy_failed(plain), "no room left");
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
                "u failed 7 times (the last time: 503)",
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

    /// Serves `responses` in order, one per connection, on a local port, and returns its address.
    /// Any request past the script is answered with a server error, so code that asks more often
    /// than it should fails at once instead of waiting on a server that has nothing to say.
    fn serve(responses: Vec<Vec<u8>>) -> String {
        format!("{}/package", listen(|_| responses).0)
    }

    /// Serves what `script` gives for the server's own address, as `serve` does, and passes on
    /// each request's head as it is read.
    fn listen<Script>(script: Script) -> (String, mpsc::Receiver<String>)
    where
        Script: FnOnce(&str) -> Vec<Vec<u8>>,
    {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = format!("http://{}", listener.local_addr().unwrap());
        let unscripted = response("599 Not In The Script", b"");
        let replies = script(&address)
            .into_iter()
            .chain(iter::repeat_n(unscripted, 16));
        let (heads, received) = mpsc::channel();
        drop(thread::spawn(move || {
            for reply in replies {
                let (mut stream, _) = listener.accept().unwrap();
                let mut request = Vec::new();
                let mut byte = [0_u8; 1];
                while !request.ends_with(b"\r\n\r\n") {
                    stream.read_exact(&mut byte).unwrap();
                    request.extend_from_slice(&byte);
                }
                drop(heads.send(String::from_utf8_lossy(&request).into_owned()));
                stream.write_all(&reply).unwrap();
            }
        }));
        (address, received)
    }

    const SBOM: &str = "steamship-1.spdx.json";

    /// A release listing that serves `SBOM` from `url`.
    fn release(url: &str) -> Vec<u8> {
        let listing = serde_json::json!({
            "assets": [
                { "name": "steamship-1.zip", "url": "http://elsewhere/zip" },
                { "name": SBOM, "url": url },
            ]
        });
        response("200 OK", listing.to_string().as_bytes())
    }

    /// The answers of both routes, for every attempt.
    fn attempts(both: [Vec<u8>; 2]) -> Vec<Vec<u8>> {
        iter::repeat_n(both, 7).flatten().collect()
    }

    /// Fetches `SBOM` from a server playing both routes, with no waits between attempts.
    fn fetch_asset<Script>(script: Script) -> (Result<String, Error>, Vec<String>)
    where
        Script: FnOnce(&str) -> Vec<Vec<u8>>,
    {
        let (address, heads) = listen(script);
        let download = format!("{address}/download/{SBOM}");
        let release = format!("{address}/api/releases/tags/v1");
        let assets = format!("{address}/api/releases/assets/");
        let asset = Asset {
            download: &download,
            release: &release,
            name: SBOM,
            assets: &assets,
        };
        let result = asset_text_waiting(asset, |_| {});
        (result, heads.try_iter().collect())
    }

    #[test]
    fn an_asset_the_download_address_fails_on_is_read_through_the_api_in_the_same_attempt() {
        let (result, heads) = fetch_asset(|address| {
            vec![
                response("504 Gateway Timeout", b""),
                release(&format!("{address}/api/releases/assets/7")),
                response("200 OK", b"{\"files\": []}"),
            ]
        });
        assert_eq!(result.ok().as_deref(), Some("{\"files\": []}"));
        let [download, listing, asset] = heads.as_slice() else {
            panic!("{heads:?}");
        };
        assert!(
            download.starts_with(&format!("GET /download/{SBOM} ")),
            "{download}"
        );
        assert!(
            listing.starts_with("GET /api/releases/tags/v1 "),
            "{listing}"
        );
        assert!(listing.contains("application/vnd.github+json"), "{listing}");
        assert!(asset.starts_with("GET /api/releases/assets/7 "), "{asset}");
        assert!(asset.contains("application/octet-stream"), "{asset}");
    }

    #[test]
    fn an_asset_the_download_address_serves_is_not_asked_of_the_api() {
        let (result, heads) = fetch_asset(|_| vec![response("200 OK", b"sbom")]);
        assert_eq!(result.ok().as_deref(), Some("sbom"));
        assert_eq!(heads.len(), 1, "{heads:?}");
    }

    #[test]
    fn an_asset_neither_route_has_stops_at_once_with_what_the_download_address_answered() {
        let (result, heads) = fetch_asset(|_| {
            vec![
                response("404 Not Found", b""),
                response("404 Not Found", b""),
            ]
        });
        assert!(
            matches!(&result, Err(Error::Gone { url, status: 404 }) if url.ends_with(SBOM)),
            "{result:?}"
        );
        assert_eq!(heads.len(), 2, "{heads:?}");
    }

    #[test]
    fn an_asset_both_routes_keep_failing_on_gives_up_saying_what_each_answered() {
        let (result, _) = fetch_asset(|_| {
            attempts([
                response("503 Service Unavailable", b""),
                response("404 Not Found", b""),
            ])
        });
        assert!(
            matches!(&result, Err(Error::GaveUp { last, .. })
                if last == "answered 503, and through GitHub's API: answered 404"),
            "{result:?}"
        );
    }

    #[test]
    fn an_asset_the_api_serves_from_elsewhere_is_not_fetched() {
        let (result, heads) = fetch_asset(|address| {
            attempts([
                response("503 Service Unavailable", b""),
                release(&format!("{address}/other/7")),
            ])
        });
        assert!(
            matches!(&result, Err(Error::GaveUp { last, .. })
                if last.starts_with("answered 503, and through GitHub's API: ")
                    && last.contains(&format!("serves {SBOM} from "))
                    && last.ends_with("/api/releases/assets/")),
            "{result:?}"
        );
        assert!(
            heads.iter().all(|head| !head.starts_with("GET /other/")),
            "{heads:?}"
        );
    }

    #[test]
    fn a_release_that_does_not_list_the_asset_is_refused() {
        let (result, _) = fetch_asset(|_| {
            vec![
                response("404 Not Found", b""),
                response("200 OK", b"{\"assets\": []}"),
            ]
        });
        assert!(
            matches!(&result, Err(Error::Gone { status: 404, .. })),
            "{result:?}"
        );
        let (listed, _) = fetch_asset(|_| {
            attempts([
                response("503 Service Unavailable", b""),
                response("200 OK", b"{\"assets\": []}"),
            ])
        });
        assert!(
            matches!(&listed, Err(Error::GaveUp { last, .. })
                if last.ends_with(&format!("lists no {SBOM}"))),
            "{listed:?}"
        );
    }

    #[test]
    fn a_release_listing_that_is_not_json_is_tried_again() {
        let (result, _) = fetch_asset(|address| {
            vec![
                response("503 Service Unavailable", b""),
                response("200 OK", b"<html>"),
                response("503 Service Unavailable", b""),
                release(&format!("{address}/api/releases/assets/7")),
                response("200 OK", b"sbom"),
            ]
        });
        assert_eq!(result.ok().as_deref(), Some("sbom"));
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
    fn a_file_not_served_yet_is_asked_for_every_30_seconds_until_it_is() {
        let waits = RefCell::new(Vec::new());
        let url = serve(vec![
            response("404 Not Found", b""),
            response("503 Service Unavailable", b""),
            response("200 OK", b"manifest"),
        ]);
        let served = served_waiting(&url, Duration::from_mins(30), |wait| {
            waits.borrow_mut().push(wait.as_secs());
        });
        assert_eq!(served, Served::Yes);
        assert_eq!(*waits.borrow(), [30, 30]);
    }

    #[test]
    fn asking_stops_when_patience_runs_out() {
        for (patience, asked) in [(0, 1), (59, 2), (60, 3)] {
            let waits = Cell::new(0_u32);
            let url = serve(vec![response("404 Not Found", b""); 3]);
            let served = served_waiting(&url, Duration::from_secs(patience), |_| {
                waits.set(waits.get().saturating_add(1));
            });
            assert_eq!(served, Served::No, "{patience}");
            assert_eq!(waits.get().saturating_add(1), asked, "{patience}");
        }
    }

    #[test]
    fn a_refusal_other_than_not_found_ends_the_wait_at_once() {
        let url = serve(vec![response("401 Refused", b"")]);
        assert_eq!(
            served_waiting(&url, Duration::from_mins(30), |_| panic!("waited")),
            Served::Otherwise(401)
        );
        let served = serve(vec![response("200 OK", b"")]);
        assert_eq!(super::served(&served, Duration::ZERO), Served::Yes);
    }

    #[test]
    fn a_failure_to_write_is_said_as_it_is() {
        let error = Error::from(io::Error::new(io::ErrorKind::PermissionDenied, "no"));
        assert_eq!(error.to_string(), "no");
    }
}
