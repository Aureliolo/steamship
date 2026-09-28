//! What only Linux does: the Secret Service, which keeps the Web API key there.
//!
//! GNOME Keyring and `KWallet` provide it on the session bus. Kept in a module of its own so that
//! everything here is compiled, tested and mutation-tested on the system where it runs.

use std::env;
use std::ffi::OsString;
use std::slice;
use std::time::Duration;

use zeroize::Zeroizing;

use crate::dbus::session::Connection;
use crate::dbus::{self, Value};
use crate::keychain::{Error, LABEL};

const SERVICE: &str = "org.freedesktop.secrets";
const ROOT: &str = "/org/freedesktop/secrets";
const SECRETS: &str = "org.freedesktop.Secret.Service";
const COLLECTION: &str = "org.freedesktop.Secret.Collection";
const ITEM: &str = "org.freedesktop.Secret.Item";
const PROMPT: &str = "org.freedesktop.Secret.Prompt";
const BUS: &str = "org.freedesktop.DBus";
const BUS_PATH: &str = "/org/freedesktop/DBus";

/// The path that stands for none, as when no prompt is needed.
const NONE: &str = "/";

/// Long enough for someone to type the password that unlocks their keyring.
const UNLOCKING: Duration = Duration::from_secs(300);

/// Errors that mean nothing provides the Secret Service on this bus.
const ABSENT: [&str; 3] = [
    "org.freedesktop.DBus.Error.ServiceUnknown",
    "org.freedesktop.DBus.Error.NameHasNoOwner",
    "org.freedesktop.DBus.Error.Spawn.",
];

fn failed(error: &dbus::Error) -> Error {
    match error {
        dbus::Error::NoBus => Error::Unavailable(error.to_string()),
        dbus::Error::Failed { name, .. }
            if ABSENT.iter().any(|absent| name.starts_with(absent)) =>
        {
            Error::Unavailable("nothing provides the Secret Service".to_owned())
        }
        dbus::Error::Io(_)
        | dbus::Error::Refused
        | dbus::Error::Malformed(_)
        | dbus::Error::Failed { .. }
        | dbus::Error::Dismissed => Error::Failed(error.to_string()),
    }
}

/// The attributes the key for the steamship home `account` is found by.
fn attributes(account: &str) -> Value {
    Value::texts(&[
        ("application", "steamship"),
        ("kind", "web-api-key"),
        ("home", account),
    ])
}

/// The Secret Service, with a session open to carry secrets in. The session is "plain": the
/// secret crosses only the local socket to the bus, which the kernel keeps to this user.
struct Service {
    bus: Connection,
    session: String,
}

impl Service {
    fn open<Lookup>(lookup: Lookup) -> Result<Self, Error>
    where
        Lookup: Fn(&str) -> Option<OsString>,
    {
        let mut bus = Connection::session(lookup).map_err(|error| failed(&error))?;
        // GNOME Keyring records a caller only after a first message from it, and a session asked
        // for before then ends the daemon, leaving every keyring locked (GNOME/gnome-keyring#190).
        // Asking where the default keyring is, which needs no record, and waiting for the answer
        // leaves the record made: in bursts of fresh callers the daemon died in 13 rounds of 100
        // asked for a session first, and in none asked this first.
        drop(
            bus.call(
                SERVICE,
                ROOT,
                SECRETS,
                "ReadAlias",
                &[Value::Str("default".to_owned())],
            )
            .map_err(|error| failed(&error))?,
        );
        let answer = bus
            .call(
                SERVICE,
                ROOT,
                SECRETS,
                "OpenSession",
                &[
                    Value::Str("plain".to_owned()),
                    Value::Variant(Box::new(Value::Str(String::new()))),
                ],
            )
            .map_err(|error| failed(&error))?;
        let session = answer
            .get(1)
            .and_then(Value::path)
            .ok_or_else(|| Error::Failed("an answer with no session".to_owned()))?
            .to_owned();
        Ok(Self { bus, session })
    }

    fn call(
        &mut self,
        path: &str,
        interface: &str,
        member: &str,
        body: &[Value],
    ) -> Result<Vec<Value>, Error> {
        self.bus
            .call(SERVICE, path, interface, member, body)
            .map_err(|error| failed(&error))
    }

    /// The items kept for `account`: those unlocked, then those locked.
    fn search(&mut self, account: &str) -> Result<(Vec<String>, Vec<String>), Error> {
        let answer = self.call(ROOT, SECRETS, "SearchItems", &[attributes(account)])?;
        let list = |at: usize| {
            answer
                .get(at)
                .and_then(Value::path_list)
                .ok_or_else(|| Error::Failed("an answer with no items".to_owned()))
        };
        Ok((list(0)?, list(1)?))
    }

    fn unlock(&mut self, objects: &[String]) -> Result<(), Error> {
        let answer = self.call(ROOT, SECRETS, "Unlock", &[Value::paths(objects)])?;
        self.prompt(answer.get(1).and_then(Value::path).unwrap_or(NONE))
    }

    /// Has the Secret Service ask its question, as to unlock a keyring, and waits for the answer;
    /// the question is its own, shown by the desktop.
    fn prompt(&mut self, prompt: &str) -> Result<(), Error> {
        if prompt == NONE {
            return Ok(());
        }
        let rule = format!("type='signal',interface='{PROMPT}',member='Completed',path='{prompt}'");
        let _added = self
            .bus
            .call(BUS, BUS_PATH, BUS, "AddMatch", &[Value::Str(rule)])
            .map_err(|error| failed(&error))?;
        let _prompted = self.call(prompt, PROMPT, "Prompt", &[Value::Str(String::new())])?;
        let completed = self
            .bus
            .signal(prompt, PROMPT, "Completed", UNLOCKING)
            .map_err(|error| failed(&error))?;
        if completed.first() == Some(&Value::Bool(true)) {
            return Err(failed(&dbus::Error::Dismissed));
        }
        Ok(())
    }

    fn has(&mut self, account: &str) -> Result<bool, Error> {
        let (unlocked, locked) = self.search(account)?;
        Ok(!unlocked.is_empty() || !locked.is_empty())
    }

    fn kept(&mut self, account: &str) -> Result<Option<Zeroizing<Vec<u8>>>, Error> {
        let (unlocked, locked) = self.search(account)?;
        let item = if let Some(item) = unlocked.first() {
            item.clone()
        } else if let Some(item) = locked.first() {
            self.unlock(slice::from_ref(item))?;
            item.clone()
        } else {
            return Ok(None);
        };
        let session = Value::Path(self.session.clone());
        let answer = self.call(&item, ITEM, "GetSecret", &[session])?;
        match answer.into_iter().next() {
            Some(Value::Struct(fields)) => match fields.into_iter().nth(2) {
                Some(Value::Bytes(secret)) => Ok(Some(secret)),
                _ => Err(Error::Failed("a secret of the wrong shape".to_owned())),
            },
            _ => Err(Error::Failed("an answer with no secret".to_owned())),
        }
    }

    fn keep(&mut self, account: &str, secret: &[u8]) -> Result<(), Error> {
        let answer = self.call(
            ROOT,
            SECRETS,
            "ReadAlias",
            &[Value::Str("default".to_owned())],
        )?;
        let collection = answer
            .first()
            .and_then(Value::path)
            .filter(|collection| *collection != NONE)
            .ok_or_else(|| Error::Failed("there is no default keyring".to_owned()))?
            .to_owned();
        self.unlock(slice::from_ref(&collection))?;
        let property = |name: &str, value: Value| {
            Value::Entry(
                Box::new(Value::Str(format!("org.freedesktop.Secret.Item.{name}"))),
                Box::new(Value::Variant(Box::new(value))),
            )
        };
        let properties = Value::Array(
            "{sv}".to_owned(),
            vec![
                property("Label", Value::Str(LABEL.to_owned())),
                property("Attributes", attributes(account)),
            ],
        );
        let secret = Value::Struct(vec![
            Value::Path(self.session.clone()),
            Value::Bytes(Zeroizing::new(Vec::new())),
            Value::Bytes(Zeroizing::new(secret.to_vec())),
            Value::Str("text/plain".to_owned()),
        ]);
        let created = self.call(
            &collection,
            COLLECTION,
            "CreateItem",
            &[properties, secret, Value::Bool(true)],
        )?;
        self.prompt(created.get(1).and_then(Value::path).unwrap_or(NONE))
    }

    fn forget(&mut self, account: &str) -> Result<bool, Error> {
        let (unlocked, locked) = self.search(account)?;
        let items: Vec<String> = unlocked.into_iter().chain(locked).collect();
        for item in &items {
            let answer = self.call(item, ITEM, "Delete", &[])?;
            self.prompt(answer.first().and_then(Value::path).unwrap_or(NONE))?;
        }
        Ok(!items.is_empty())
    }
}

fn service() -> Result<Service, Error> {
    Service::open(|name| env::var_os(name))
}

/// Whether the Secret Service keeps a secret for the steamship home `account`, found out without
/// unlocking anything.
///
/// # Errors
///
/// When the Secret Service cannot be reached or searched.
pub fn has_secret(account: &str) -> Result<bool, Error> {
    service()?.has(account)
}

/// The secret kept for `account`, if there is one, unlocking it first if need be.
///
/// # Errors
///
/// When the Secret Service cannot be reached or read, or unlocking it was refused.
pub fn kept_secret(account: &str) -> Result<Option<Zeroizing<Vec<u8>>>, Error> {
    service()?.kept(account)
}

/// Keeps `secret` for `account` in the default keyring, in place of any kept before.
///
/// # Errors
///
/// When the Secret Service cannot be reached, has no default keyring, or refuses.
pub fn keep_secret(account: &str, secret: &[u8]) -> Result<(), Error> {
    service()?.keep(account, secret)
}

/// Removes every secret kept for `account`, and says whether there was one.
///
/// # Errors
///
/// When the Secret Service cannot be reached, or refuses.
pub fn forget_secret(account: &str) -> Result<bool, Error> {
    service()?.forget(account)
}

#[cfg(test)]
mod tests {
    use std::io::{Read as _, Write as _};
    use std::iter;
    use std::os::linux::net::SocketAddrExt as _;
    use std::os::unix::net::{SocketAddr, UnixListener, UnixStream};
    use std::process;
    use std::sync::Mutex;
    use std::thread::{self, JoinHandle};

    use super::*;
    use crate::dbus::session::LOGIN_LINE;
    use crate::dbus::{Kind, Message};

    const KEY: &[u8] = b"0123456789abcdef0123456789abcdef";

    /// What the stand-in does when called.
    enum Answer {
        Return(Vec<Value>),
        Error(&'static str),
        /// A return, then the prompt at the path completing with these.
        ThenCompleted(Vec<Value>, &'static str, Vec<Value>),
        /// The prompt at the path completing with these, before the return.
        CompletedFirst(Vec<Value>, &'static str, Vec<Value>),
        /// As many signals as the count that nothing waits for, then the return.
        AfterNoise(usize, Vec<Value>),
        /// A return, then the connection closed.
        ThenHangUp(Vec<Value>),
        /// A return that answers another call.
        ToAnother(Vec<Value>),
        /// A call to steamship, then the return.
        AfterCall(Vec<Value>),
    }

    /// How long the stand-in waits for steamship to say something before hanging up, as a bus
    /// drops a peer that has stalled; steamship waiting for what never comes then fails at once.
    const STALLED: Duration = Duration::from_secs(10);

    fn read_line(stream: &mut UnixStream) -> Vec<u8> {
        let mut line = Vec::new();
        while !line.ends_with(b"\r\n") {
            let mut byte = [0_u8; 1];
            stream.read_exact(&mut byte).unwrap();
            line.extend_from_slice(&byte);
        }
        line
    }

    fn send(stream: &mut UnixStream, serial: &mut u32, message: Message) {
        *serial = serial.saturating_add(1);
        let message = Message {
            serial: *serial,
            ..message
        };
        stream
            .write_all(&dbus::encode(&message, None).unwrap())
            .unwrap();
    }

    fn reply(to: &Message, kind: Kind, error: Option<&str>, body: Vec<Value>) -> Message {
        Message {
            kind,
            serial: 0,
            reply_to: Some(to.serial),
            path: None,
            interface: None,
            member: None,
            error: error.map(str::to_owned),
            body,
        }
    }

    fn completed(prompt: &str, body: Vec<Value>) -> Message {
        Message {
            kind: Kind::Signal,
            serial: 0,
            reply_to: None,
            path: Some(prompt.to_owned()),
            interface: Some(PROMPT.to_owned()),
            member: Some("Completed".to_owned()),
            error: None,
            body,
        }
    }

    /// A stand-in session bus with a Secret Service on it, on a socket in a folder of its own,
    /// greeting a login with `greeting` and answering each call through `answer`. Gives back the
    /// address, and the calls it was sent once the connection has ended.
    fn standing_in<Answering>(
        greeting: &'static [u8],
        answer: Answering,
    ) -> (tempfile::TempDir, String, JoinHandle<Vec<Message>>)
    where
        Answering: Fn(&Message) -> Answer + Send + 'static,
    {
        let folder = tempfile::tempdir().unwrap();
        let socket = folder.path().join("bus");
        let serving = serve(UnixListener::bind(&socket).unwrap(), greeting, answer);
        (folder, format!("unix:path={}", socket.display()), serving)
    }

    /// What the stand-in sends, in order, to answer `call` as `answered` says.
    fn replies(call: &Message, answered: Answer) -> Vec<Message> {
        let returned = |body| reply(call, Kind::Return, None, body);
        let elsewhere = || completed("/elsewhere", Vec::new());
        match answered {
            Answer::Return(body) | Answer::ThenHangUp(body) => vec![returned(body)],
            Answer::Error(name) => {
                let why = vec![Value::Str("the stand-in says no".to_owned())];
                vec![reply(call, Kind::Error, Some(name), why)]
            }
            // Another prompt completes while this one is waited for.
            Answer::ThenCompleted(body, prompt, signal) => {
                vec![returned(body), elsewhere(), completed(prompt, signal)]
            }
            Answer::CompletedFirst(body, prompt, signal) => {
                vec![completed(prompt, signal), returned(body)]
            }
            Answer::AfterNoise(noise, body) => iter::repeat_with(elsewhere)
                .take(noise)
                .chain(iter::once(returned(body)))
                .collect(),
            Answer::ToAnother(body) => vec![Message {
                reply_to: Some(call.serial.saturating_add(1000)),
                ..returned(body)
            }],
            Answer::AfterCall(body) => {
                let ping = Message {
                    kind: Kind::Call,
                    serial: 0,
                    reply_to: None,
                    path: Some(NONE.to_owned()),
                    interface: Some("org.freedesktop.DBus.Peer".to_owned()),
                    member: Some("Ping".to_owned()),
                    error: None,
                    body: Vec::new(),
                };
                vec![ping, returned(body)]
            }
        }
    }

    /// The stand-in bus of [`standing_in`], answering on `listener`.
    fn serve<Answering>(
        listener: UnixListener,
        greeting: &'static [u8],
        answer: Answering,
    ) -> JoinHandle<Vec<Message>>
    where
        Answering: Fn(&Message) -> Answer + Send + 'static,
    {
        thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream.set_read_timeout(Some(STALLED)).unwrap();
            let login = read_line(&mut stream);
            assert!(login.starts_with(b"\0AUTH EXTERNAL "), "{login:?}");
            stream.write_all(greeting).unwrap();
            let mut calls = Vec::new();
            // steamship hangs up on a refusal, and on an answer longer than it reads.
            if !greeting.starts_with(b"OK ") || greeting.len() > LOGIN_LINE {
                return calls;
            }
            assert_eq!(read_line(&mut stream), b"BEGIN\r\n");
            let mut serial = 0_u32;
            loop {
                let mut head = [0_u8; 16];
                if stream.read_exact(&mut head).is_err() {
                    return calls;
                }
                let mut bytes = vec![0_u8; dbus::length(&head).unwrap()];
                let (start, rest) = bytes.split_at_mut(head.len());
                start.copy_from_slice(&head);
                stream.read_exact(rest).unwrap();
                let call = dbus::message(&bytes).unwrap();
                let answered = if call.member.as_deref() == Some("Hello") {
                    Answer::Return(vec![Value::Str(":1.7".to_owned())])
                } else {
                    answer(&call)
                };
                let hang_up = matches!(answered, Answer::ThenHangUp(_));
                for message in replies(&call, answered) {
                    send(&mut stream, &mut serial, message);
                }
                calls.push(call);
                if hang_up {
                    return calls;
                }
            }
        })
    }

    fn opened(address: &str) -> Result<Service, Error> {
        let address = OsString::from(address);
        Service::open(move |name| (name == "DBUS_SESSION_BUS_ADDRESS").then(|| address.clone()))
    }

    fn members(calls: &[Message]) -> Vec<&str> {
        calls
            .iter()
            .filter_map(|call| call.member.as_deref())
            .collect()
    }

    fn session() -> Answer {
        Answer::Return(opened_session())
    }

    fn opened_session() -> Vec<Value> {
        vec![
            Value::Variant(Box::new(Value::Str(String::new()))),
            Value::Path("/org/freedesktop/secrets/session/1".to_owned()),
        ]
    }

    fn found(unlocked: &[&str], locked: &[&str]) -> Answer {
        found_after(1, unlocked, locked)
    }

    fn found_after(noise: usize, unlocked: &[&str], locked: &[&str]) -> Answer {
        let paths = |paths: &[&str]| {
            Value::paths(
                &paths
                    .iter()
                    .map(|path| (*path).to_owned())
                    .collect::<Vec<_>>(),
            )
        };
        Answer::AfterNoise(noise, vec![paths(unlocked), paths(locked)])
    }

    fn secret(key: &[u8]) -> Answer {
        Answer::Return(vec![Value::Struct(vec![
            Value::Path("/org/freedesktop/secrets/session/1".to_owned()),
            Value::Bytes(Zeroizing::new(Vec::new())),
            Value::Bytes(Zeroizing::new(key.to_vec())),
            Value::Str("text/plain".to_owned()),
        ])])
    }

    #[test]
    fn a_locked_key_is_unlocked_through_its_prompt_then_read() {
        let (_folder, address, serving) = standing_in(b"OK 0123\r\n", |call| {
            match call.member.as_deref().unwrap_or_default() {
                "OpenSession" => session(),
                "SearchItems" => found(&[], &["/item/1"]),
                "Unlock" => {
                    Answer::Return(vec![Value::paths(&[]), Value::Path("/prompt/1".to_owned())])
                }
                "Prompt" => Answer::ThenCompleted(
                    Vec::new(),
                    "/prompt/1",
                    vec![
                        Value::Bool(false),
                        Value::Variant(Box::new(Value::paths(&["/item/1".to_owned()]))),
                    ],
                ),
                "GetSecret" => secret(KEY),
                _ => Answer::Return(Vec::new()),
            }
        });
        let mut service = opened(&address).unwrap();
        assert_eq!(service.kept("/home").unwrap().unwrap().as_slice(), KEY);
        drop(service);
        let calls = serving.join().unwrap();
        assert_eq!(
            members(&calls),
            [
                "Hello",
                "ReadAlias",
                "OpenSession",
                "SearchItems",
                "Unlock",
                "AddMatch",
                "Prompt",
                "GetSecret"
            ]
        );
        let call = |at: usize| calls.get(at).expect("the call");
        assert_eq!(call(1).body, [Value::Str("default".to_owned())]);
        assert_eq!(call(3).body, [attributes("/home")]);
        let rule = call(5).body.first().and_then(Value::text).unwrap();
        assert!(rule.contains("path='/prompt/1'"), "{rule}");
        assert_eq!(call(7).path.as_deref(), Some("/item/1"));
    }

    /// Even after more signals than the connection holds, 64, came while nothing waited.
    #[test]
    fn a_prompt_that_completes_before_its_call_returns_is_still_seen() {
        let (_folder, address, serving) = standing_in(b"OK 0123\r\n", |call| {
            match call.member.as_deref().unwrap_or_default() {
                "OpenSession" => session(),
                "SearchItems" => found_after(70, &[], &["/item/1"]),
                "Unlock" => {
                    Answer::Return(vec![Value::paths(&[]), Value::Path("/prompt/2".to_owned())])
                }
                "Prompt" => Answer::CompletedFirst(
                    Vec::new(),
                    "/prompt/2",
                    vec![
                        Value::Bool(false),
                        Value::Variant(Box::new(Value::paths(&[]))),
                    ],
                ),
                "GetSecret" => secret(KEY),
                _ => Answer::Return(Vec::new()),
            }
        });
        let mut service = opened(&address).unwrap();
        assert!(service.kept("/home").unwrap().is_some());
        drop(service);
        drop(serving.join().unwrap());
    }

    #[test]
    fn a_dismissed_prompt_keeps_nothing_and_says_so() {
        let (_folder, address, serving) = standing_in(b"OK 0123\r\n", |call| {
            match call.member.as_deref().unwrap_or_default() {
                "OpenSession" => session(),
                "ReadAlias" => Answer::Return(vec![Value::Path("/collection/login".to_owned())]),
                // Another prompt completing first, which must not be taken for this one.
                "Unlock" => Answer::AfterNoise(
                    1,
                    vec![Value::paths(&[]), Value::Path("/prompt/3".to_owned())],
                ),
                "Prompt" => Answer::ThenCompleted(
                    Vec::new(),
                    "/prompt/3",
                    vec![
                        Value::Bool(true),
                        Value::Variant(Box::new(Value::Str(String::new()))),
                    ],
                ),
                _ => Answer::Return(Vec::new()),
            }
        });
        let mut service = opened(&address).unwrap();
        assert_eq!(
            service.keep("/home", KEY),
            Err(Error::Failed(
                "the prompt to unlock it was dismissed".to_owned()
            ))
        );
        drop(service);
        let calls = serving.join().unwrap();
        assert_eq!(
            members(&calls),
            [
                "Hello",
                "ReadAlias",
                "OpenSession",
                "ReadAlias",
                "Unlock",
                "AddMatch",
                "Prompt"
            ]
        );
    }

    #[test]
    fn a_key_is_kept_in_the_default_keyring_by_its_home_with_no_prompt_when_unlocked() {
        let (_folder, address, serving) = standing_in(b"OK 0123\r\n", |call| {
            match call.member.as_deref().unwrap_or_default() {
                "OpenSession" => session(),
                "ReadAlias" => Answer::Return(vec![Value::Path("/collection/login".to_owned())]),
                "Unlock" => Answer::Return(vec![
                    Value::paths(&["/collection/login".to_owned()]),
                    Value::Path(NONE.to_owned()),
                ]),
                _ => Answer::Return(vec![
                    Value::Path("/item/9".to_owned()),
                    Value::Path(NONE.to_owned()),
                ]),
            }
        });
        let mut service = opened(&address).unwrap();
        service.keep("/home", KEY).unwrap();
        drop(service);
        let calls = serving.join().unwrap();
        assert_eq!(
            members(&calls),
            [
                "Hello",
                "ReadAlias",
                "OpenSession",
                "ReadAlias",
                "Unlock",
                "CreateItem"
            ]
        );
        let created = calls.last().unwrap();
        assert_eq!(created.path.as_deref(), Some("/collection/login"));
        let property = |name: &str, value: Value| {
            Value::Entry(
                Box::new(Value::Str(format!("org.freedesktop.Secret.Item.{name}"))),
                Box::new(Value::Variant(Box::new(value))),
            )
        };
        assert_eq!(
            created.body,
            [
                Value::Array(
                    "{sv}".to_owned(),
                    vec![
                        property("Label", Value::Str(LABEL.to_owned())),
                        property("Attributes", attributes("/home")),
                    ]
                ),
                Value::Struct(vec![
                    Value::Path("/org/freedesktop/secrets/session/1".to_owned()),
                    Value::Bytes(Zeroizing::new(Vec::new())),
                    Value::Bytes(Zeroizing::new(KEY.to_vec())),
                    Value::Str("text/plain".to_owned()),
                ]),
                Value::Bool(true),
            ],
            "the label, the home it is kept for, the key, and replacing any kept before"
        );
    }

    #[test]
    fn with_no_default_keyring_nothing_is_kept() {
        let (_folder, address, serving) = standing_in(b"OK 0123\r\n", |call| {
            match call.member.as_deref().unwrap_or_default() {
                "OpenSession" => session(),
                _ => Answer::Return(vec![Value::Path(NONE.to_owned())]),
            }
        });
        let mut service = opened(&address).unwrap();
        assert_eq!(
            service.keep("/home", KEY),
            Err(Error::Failed("there is no default keyring".to_owned()))
        );
        drop(service);
        drop(serving.join().unwrap());
    }

    #[test]
    fn forgetting_deletes_every_item_kept_for_the_home_locked_or_not() {
        let (_folder, address, serving) = standing_in(b"OK 0123\r\n", |call| {
            match call.member.as_deref().unwrap_or_default() {
                "OpenSession" => session(),
                "SearchItems" => found(&["/item/1"], &["/item/2"]),
                _ => Answer::Return(vec![Value::Path(NONE.to_owned())]),
            }
        });
        let mut service = opened(&address).unwrap();
        assert!(service.has("/home").unwrap());
        assert!(service.forget("/home").unwrap());
        drop(service);
        let calls = serving.join().unwrap();
        let deleted: Vec<&str> = calls
            .iter()
            .filter(|call| call.member.as_deref() == Some("Delete"))
            .filter_map(|call| call.path.as_deref())
            .collect();
        assert_eq!(deleted, ["/item/1", "/item/2"]);
    }

    #[test]
    fn nothing_kept_is_nothing_found_and_nothing_forgotten() {
        let (_folder, address, serving) = standing_in(b"OK 0123\r\n", |call| {
            match call.member.as_deref().unwrap_or_default() {
                "OpenSession" => session(),
                _ => found(&[], &[]),
            }
        });
        let mut service = opened(&address).unwrap();
        assert!(!service.has("/home").unwrap());
        assert!(service.kept("/home").unwrap().is_none());
        assert!(!service.forget("/home").unwrap());
        drop(service);
        drop(serving.join().unwrap());
    }

    #[test]
    fn answers_of_the_wrong_shape_are_failures_not_guesses() {
        for (member, answer, why) in [
            (
                "OpenSession",
                Answer::Return(Vec::new()),
                "an answer with no session",
            ),
            (
                "SearchItems",
                Answer::Return(vec![Value::Byte(1)]),
                "an answer with no items",
            ),
            (
                "GetSecret",
                Answer::Return(vec![Value::Byte(1)]),
                "an answer with no secret",
            ),
            (
                "GetSecret",
                Answer::Return(vec![Value::Struct(vec![Value::Byte(1)])]),
                "a secret of the wrong shape",
            ),
        ] {
            let answer = Mutex::new(Some(answer));
            let (_folder, address, serving) = standing_in(b"OK 0123\r\n", move |call| {
                let called = call.member.as_deref().unwrap_or_default();
                if called == member {
                    return answer.lock().unwrap().take().unwrap();
                }
                match called {
                    "OpenSession" => session(),
                    _ => found(&["/item/1"], &[]),
                }
            });
            let result = opened(&address).and_then(|mut service| service.kept("/home"));
            assert_eq!(result, Err(Error::Failed(why.to_owned())), "{member}");
            drop(serving.join().unwrap());
        }
    }

    #[test]
    fn an_absent_service_is_no_store_and_another_error_a_failure() {
        let (_folder, address, serving) = standing_in(b"OK 0123\r\n", |_| {
            Answer::Error("org.freedesktop.DBus.Error.ServiceUnknown")
        });
        assert!(matches!(
            opened(&address),
            Err(Error::Unavailable(why)) if why == "nothing provides the Secret Service"
        ));
        drop(serving.join().unwrap());
        let (_other, elsewhere, answering) = standing_in(b"OK 0123\r\n", |call| {
            match call.member.as_deref().unwrap_or_default() {
                "OpenSession" => session(),
                _ => Answer::Error("org.freedesktop.Secret.Error.IsLocked"),
            }
        });
        assert_eq!(
            opened(&elsewhere).and_then(|mut service| service.has("/home")),
            Err(Error::Failed(
                "org.freedesktop.Secret.Error.IsLocked: the stand-in says no".to_owned()
            ))
        );
        drop(answering.join().unwrap());
    }

    #[test]
    fn a_bus_that_refuses_the_login_or_hangs_up_mid_prompt_is_a_failure() {
        let (_folder, address, serving) =
            standing_in(b"REJECTED EXTERNAL\r\n", |_| Answer::Return(Vec::new()));
        assert_eq!(
            opened(&address).err(),
            Some(Error::Failed(
                "the session bus refused this user".to_owned()
            ))
        );
        drop(serving.join().unwrap());
        let (_other, elsewhere, answering) = standing_in(b"OK 0123\r\n", |call| {
            match call.member.as_deref().unwrap_or_default() {
                "OpenSession" => session(),
                "SearchItems" => found(&[], &["/item/1"]),
                "Unlock" => {
                    Answer::Return(vec![Value::paths(&[]), Value::Path("/prompt/4".to_owned())])
                }
                "Prompt" => Answer::ThenHangUp(Vec::new()),
                _ => Answer::Return(Vec::new()),
            }
        });
        let result = opened(&elsewhere).and_then(|mut service| service.kept("/home"));
        assert!(
            matches!(&result, Err(Error::Failed(why)) if why.starts_with("the session bus: ")),
            "{result:?}"
        );
        drop(answering.join().unwrap());
    }

    #[test]
    fn a_call_to_steamship_is_passed_over_and_an_answer_to_another_call_refused() {
        let (_folder, address, serving) = standing_in(b"OK 0123\r\n", |call| {
            match call.member.as_deref().unwrap_or_default() {
                "OpenSession" => Answer::AfterCall(opened_session()),
                "ReadAlias" => Answer::Return(vec![Value::Path(NONE.to_owned())]),
                _ => Answer::ToAnother(Vec::new()),
            }
        });
        let mut service = opened(&address).unwrap();
        assert_eq!(
            service.has("/home"),
            Err(Error::Failed(
                "a message from the session bus: an answer to another call".to_owned()
            ))
        );
        drop(service);
        drop(serving.join().unwrap());
    }

    #[test]
    fn with_no_session_bus_there_is_no_store() {
        let none = Some(Error::Unavailable("no D-Bus session bus".to_owned()));
        assert_eq!(opened("unix:path=/nonexistent/steamship/bus").err(), none);
        assert_eq!(opened("unix:abstract=/steamship/nothing/here").err(), none);
        assert_eq!(Service::open(|_| None).err(), none);
    }

    #[test]
    fn with_no_bus_address_the_bus_in_the_runtime_folder_is_used() {
        let (folder, _address, serving) = standing_in(b"OK 0123\r\n", |_| session());
        let runtime = folder.path().as_os_str().to_owned();
        let service = Service::open(|name| (name == "XDG_RUNTIME_DIR").then(|| runtime.clone()));
        assert_eq!(
            service.map(|service| service.session),
            Ok("/org/freedesktop/secrets/session/1".to_owned())
        );
        drop(serving.join().unwrap());
    }

    #[test]
    fn a_bus_in_the_abstract_namespace_is_reached_by_its_name() {
        let name = format!("steamship-test-{}-abstract", process::id());
        let listener =
            UnixListener::bind_addr(&SocketAddr::from_abstract_name(&name).unwrap()).unwrap();
        let serving = serve(listener, b"OK 0123\r\n", |_| session());
        let service = opened(&format!("unix:abstract={name}"));
        assert_eq!(
            service.map(|service| service.session),
            Ok("/org/freedesktop/secrets/session/1".to_owned())
        );
        drop(serving.join().unwrap());
    }

    #[test]
    fn a_login_answer_longer_than_any_bus_gives_is_refused() {
        let answer = |length: usize| -> &'static [u8] {
            let filler = "a".repeat(length.saturating_sub(5));
            Box::leak(format!("OK {filler}\r\n").into_bytes().into_boxed_slice())
        };
        let (_folder, address, serving) = standing_in(answer(LOGIN_LINE), |_| session());
        assert!(opened(&address).is_ok(), "as long as steamship reads");
        drop(serving.join().unwrap());
        let (_other, elsewhere, answering) =
            standing_in(answer(LOGIN_LINE.saturating_add(1)), |_| session());
        assert_eq!(
            opened(&elsewhere).err(),
            Some(Error::Failed(
                "the session bus refused this user".to_owned()
            ))
        );
        drop(answering.join().unwrap());
    }
}
