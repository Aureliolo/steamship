//! Just enough of D-Bus, the Linux desktop's message bus, to reach the Secret Service that keeps
//! the Web API key there: the wire format, and on Linux a connection to the session bus.
//!
//! steamship writes little-endian messages and reads either byte order, since the bus passes each
//! message on in its sender's. What is read is bounded, a message by [`LARGEST`] and nesting by
//! [`DEEPEST`], so that nothing a peer sends can exhaust memory or the stack.

use std::error;
use std::fmt;
use std::io;
use std::path::PathBuf;

use zeroize::Zeroizing;

/// The longest message steamship reads. The Secret Service answers in a few hundred bytes.
pub const LARGEST: usize = 1 << 20;

/// The deepest nesting read, as D-Bus itself allows no more than 32 arrays and 32 structures.
const DEEPEST: usize = 64;

/// A value in a message, by its D-Bus type.
#[derive(Clone, PartialEq, Eq)]
pub enum Value {
    Byte(u8),
    Bool(bool),
    /// Any other fixed-width number, by its type code, as the bits it was sent as.
    Number(u8, u64),
    Str(String),
    Path(String),
    Signature(String),
    /// An array of bytes, which is how a secret travels, so wiped once dropped.
    Bytes(Zeroizing<Vec<u8>>),
    /// An array, with the signature of its elements, which an empty one cannot show.
    Array(String, Vec<Self>),
    Struct(Vec<Self>),
    /// A key and its value, as the elements of a dictionary are.
    Entry(Box<Self>, Box<Self>),
    Variant(Box<Self>),
}

impl fmt::Debug for Value {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Byte(byte) => write!(formatter, "Byte({byte})"),
            Self::Bool(bool) => write!(formatter, "Bool({bool})"),
            Self::Number(code, number) => {
                write!(formatter, "Number({}, {number})", char::from(*code))
            }
            Self::Str(text) => write!(formatter, "Str({text:?})"),
            Self::Path(path) => write!(formatter, "Path({path:?})"),
            Self::Signature(signature) => write!(formatter, "Signature({signature:?})"),
            Self::Bytes(bytes) => write!(formatter, "Bytes({} bytes)", bytes.len()),
            Self::Array(element, items) => formatter
                .debug_tuple("Array")
                .field(element)
                .field(items)
                .finish(),
            Self::Struct(fields) => formatter.debug_tuple("Struct").field(fields).finish(),
            Self::Entry(key, value) => formatter
                .debug_tuple("Entry")
                .field(key)
                .field(value)
                .finish(),
            Self::Variant(value) => formatter.debug_tuple("Variant").field(value).finish(),
        }
    }
}

impl Value {
    /// A `u`, the unsigned 32-bit number.
    #[must_use]
    pub fn u32(number: u32) -> Self {
        Self::Number(b'u', u64::from(number))
    }

    /// A dictionary of text to text, `a{ss}`, as the Secret Service's attributes are.
    #[must_use]
    pub fn texts(pairs: &[(&str, &str)]) -> Self {
        Self::Array(
            "{ss}".to_owned(),
            pairs
                .iter()
                .map(|&(key, value)| {
                    Self::Entry(
                        Box::new(Self::Str(key.to_owned())),
                        Box::new(Self::Str(value.to_owned())),
                    )
                })
                .collect(),
        )
    }

    /// An array of object paths, `ao`.
    #[must_use]
    pub fn paths(paths: &[String]) -> Self {
        Self::Array(
            "o".to_owned(),
            paths.iter().cloned().map(Self::Path).collect(),
        )
    }

    /// The value's D-Bus signature.
    #[must_use]
    pub fn signature(&self) -> String {
        let mut signature = String::new();
        self.sign(&mut signature);
        signature
    }

    fn sign(&self, signature: &mut String) {
        match self {
            Self::Byte(_) => signature.push('y'),
            Self::Bool(_) => signature.push('b'),
            Self::Number(code, _) => signature.push(char::from(*code)),
            Self::Str(_) => signature.push('s'),
            Self::Path(_) => signature.push('o'),
            Self::Signature(_) => signature.push('g'),
            Self::Bytes(_) => signature.push_str("ay"),
            Self::Array(element, _) => {
                signature.push('a');
                signature.push_str(element);
            }
            Self::Struct(fields) => {
                signature.push('(');
                for field in fields {
                    field.sign(signature);
                }
                signature.push(')');
            }
            Self::Entry(key, value) => {
                signature.push('{');
                key.sign(signature);
                value.sign(signature);
                signature.push('}');
            }
            Self::Variant(_) => signature.push('v'),
        }
    }

    /// The text of a `Str`, if this is one.
    #[must_use]
    pub fn text(&self) -> Option<&str> {
        if let Self::Str(text) = self {
            Some(text)
        } else {
            None
        }
    }

    /// The path of a `Path`, if this is one.
    #[must_use]
    pub fn path(&self) -> Option<&str> {
        if let Self::Path(path) = self {
            Some(path)
        } else {
            None
        }
    }

    /// The paths of an `ao`, if this is one.
    #[must_use]
    pub fn path_list(&self) -> Option<Vec<String>> {
        if let Self::Array(_, items) = self {
            items
                .iter()
                .map(|item| item.path().map(str::to_owned))
                .collect()
        } else {
            None
        }
    }
}

#[derive(Debug)]
pub enum Error {
    /// No session bus: the environment names none, or none answers where it is named.
    NoBus,
    Io(io::Error),
    /// The bus did not accept steamship as the user it runs as.
    Refused,
    /// A message is not one D-Bus allows, for the reason named.
    Malformed(&'static str),
    /// The bus or a service answered with this error.
    Failed {
        name: String,
        message: String,
    },
    /// The person dismissed the prompt that asked to unlock the store.
    Dismissed,
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoBus => formatter.write_str("no D-Bus session bus"),
            Self::Io(error) => write!(formatter, "the session bus: {error}"),
            Self::Refused => formatter.write_str("the session bus refused this user"),
            Self::Malformed(why) => write!(formatter, "a message from the session bus: {why}"),
            Self::Failed { name, message } if message.is_empty() => formatter.write_str(name),
            Self::Failed { name, message } => write!(formatter, "{name}: {message}"),
            Self::Dismissed => formatter.write_str("the prompt to unlock it was dismissed"),
        }
    }
}

impl error::Error for Error {}

impl From<io::Error> for Error {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

/// The boundary an array's first element of the type starting with `code` begins on. The
/// elements follow the array's 4-byte length, so only the types on 8 can need padding there.
const fn element_alignment(code: u8) -> usize {
    match code {
        b'x' | b't' | b'd' | b'(' | b'{' => 8,
        _ => 4,
    }
}

/// How many bytes a fixed-width number of type `code` takes, if it is one.
const fn width(code: u8) -> Option<usize> {
    match code {
        b'n' | b'q' => Some(2),
        b'i' | b'u' | b'h' => Some(4),
        b'x' | b't' | b'd' => Some(8),
        _ => None,
    }
}

/// Where the one complete type starting at `at` in `signature` ends.
///
/// # Errors
///
/// When there is no complete type there, or it nests deeper than D-Bus allows.
fn complete(signature: &[u8], at: usize, depth: usize) -> Result<usize, Error> {
    let bad = Error::Malformed("a signature");
    if depth > DEEPEST {
        return Err(Error::Malformed("nesting"));
    }
    let code = *signature.get(at).ok_or(bad)?;
    let next = at.checked_add(1).ok_or(Error::Malformed("a signature"))?;
    match code {
        b'y' | b'b' | b's' | b'o' | b'g' | b'v' => Ok(next),
        _ if width(code).is_some() => Ok(next),
        b'a' if signature.get(next) == Some(&b'{') => {
            let key = *signature
                .get(next.saturating_add(1))
                .ok_or(Error::Malformed("a signature"))?;
            if key == b'v' || complete(&[key], 0, depth)? != 1 {
                return Err(Error::Malformed("a dictionary's key"));
            }
            let end = complete(signature, next.saturating_add(2), depth.saturating_add(1))?;
            if signature.get(end) == Some(&b'}') {
                Ok(end.saturating_add(1))
            } else {
                Err(Error::Malformed("a dictionary entry"))
            }
        }
        b'a' => complete(signature, next, depth.saturating_add(1)),
        b'(' => {
            if signature.get(next) == Some(&b')') {
                return Err(Error::Malformed("an empty structure"));
            }
            let mut end = next;
            while signature.get(end) != Some(&b')') {
                end = complete(signature, end, depth.saturating_add(1))?;
            }
            Ok(end.saturating_add(1))
        }
        _ => Err(Error::Malformed("a signature")),
    }
}

/// Whether `path` is an object path as D-Bus allows one: `/`, or `/` before each of one or more
/// non-empty elements of ASCII letters, digits and `_`. A bus drops a connection that sends any
/// other, so none is sent or taken.
fn is_object_path(path: &str) -> bool {
    path == "/"
        || path.strip_prefix('/').is_some_and(|elements| {
            elements.split('/').all(|element| {
                !element.is_empty()
                    && element
                        .bytes()
                        .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
            })
        })
}

#[derive(Default)]
struct Writer {
    bytes: Zeroizing<Vec<u8>>,
}

impl Writer {
    fn pad(&mut self, to: usize) {
        let padded = self.bytes.len().next_multiple_of(to);
        self.bytes.resize(padded, 0);
    }

    fn u32(&mut self, number: u32) {
        self.pad(4);
        self.bytes.extend_from_slice(&number.to_le_bytes());
    }

    fn length(length: usize) -> Result<u32, Error> {
        u32::try_from(length)
            .ok()
            .ok_or(Error::Malformed("too long"))
    }

    fn text(&mut self, text: &str) -> Result<(), Error> {
        if text.contains('\0') {
            return Err(Error::Malformed("text holding a NUL"));
        }
        self.u32(Self::length(text.len())?);
        self.bytes.extend_from_slice(text.as_bytes());
        self.bytes.push(0);
        Ok(())
    }

    fn signature(&mut self, signature: &str) -> Result<(), Error> {
        let length = u8::try_from(signature.len())
            .ok()
            .ok_or(Error::Malformed("a signature"))?;
        self.bytes.push(length);
        self.bytes.extend_from_slice(signature.as_bytes());
        self.bytes.push(0);
        Ok(())
    }

    fn value(&mut self, value: &Value) -> Result<(), Error> {
        match value {
            Value::Byte(byte) => self.bytes.push(*byte),
            Value::Bool(bool) => self.u32(u32::from(*bool)),
            Value::Number(code, number) => {
                let width = width(*code).ok_or(Error::Malformed("a number's type"))?;
                self.pad(width);
                let bytes = number.to_le_bytes();
                self.bytes
                    .extend_from_slice(bytes.get(..width).unwrap_or_default());
            }
            Value::Str(text) => self.text(text)?,
            Value::Path(path) if is_object_path(path) => self.text(path)?,
            Value::Path(_) => return Err(Error::Malformed("an object path")),
            Value::Signature(signature) => self.signature(signature)?,
            Value::Bytes(bytes) => {
                self.u32(Self::length(bytes.len())?);
                self.bytes.extend_from_slice(bytes);
            }
            Value::Array(element, items) => {
                self.u32(0);
                let at = self.bytes.len().saturating_sub(4);
                self.pad(element_alignment(
                    element.bytes().next().unwrap_or_default(),
                ));
                let start = self.bytes.len();
                for item in items {
                    self.value(item)?;
                }
                let length = Self::length(self.bytes.len().saturating_sub(start))?;
                if let Some(slot) = self.bytes.get_mut(at..at.saturating_add(4)) {
                    slot.copy_from_slice(&length.to_le_bytes());
                }
            }
            Value::Struct(fields) => {
                self.pad(8);
                for field in fields {
                    self.value(field)?;
                }
            }
            Value::Entry(key, value) => {
                self.pad(8);
                self.value(key)?;
                self.value(value)?;
            }
            Value::Variant(value) => {
                self.signature(&value.signature())?;
                self.value(value)?;
            }
        }
        Ok(())
    }
}

/// A method call, ready to send: `member` of `interface` on the object at `path` of
/// `destination`, with `body` as its arguments. Wiped once dropped, as it may carry a secret.
///
/// # Errors
///
/// When a value cannot be sent as D-Bus allows, such as text holding a NUL.
pub fn call(
    serial: u32,
    destination: &str,
    path: &str,
    interface: &str,
    member: &str,
    body: &[Value],
) -> Result<Zeroizing<Vec<u8>>, Error> {
    let message = Message {
        kind: Kind::Call,
        serial,
        reply_to: None,
        path: Some(path.to_owned()),
        interface: Some(interface.to_owned()),
        member: Some(member.to_owned()),
        error: None,
        body: body.to_vec(),
    };
    encode(&message, Some(destination))
}

/// `message`, ready to send, to `destination` when it names one: what a service sends as well
/// as what steamship does, for the stand-in services its tests talk to.
///
/// # Errors
///
/// When a value cannot be sent as D-Bus allows, such as text holding a NUL.
pub fn encode(message: &Message, destination: Option<&str>) -> Result<Zeroizing<Vec<u8>>, Error> {
    let mut content = Writer::default();
    for value in &message.body {
        content.value(value)?;
    }
    let signature: String = message.body.iter().map(Value::signature).collect();
    let field =
        |code, value| Value::Struct(vec![Value::Byte(code), Value::Variant(Box::new(value))]);
    let text = |code, text: Option<&String>| text.map(|text| field(code, Value::Str(text.clone())));
    let fields: Vec<Value> = [
        message
            .path
            .as_ref()
            .map(|path| field(1, Value::Path(path.clone()))),
        text(2, message.interface.as_ref()),
        text(3, message.member.as_ref()),
        text(4, message.error.as_ref()),
        message.reply_to.map(|serial| field(5, Value::u32(serial))),
        destination.map(|destination| field(6, Value::Str(destination.to_owned()))),
        (!signature.is_empty()).then(|| field(8, Value::Signature(signature))),
    ]
    .into_iter()
    .flatten()
    .collect();
    let kind = match message.kind {
        Kind::Call => 1,
        Kind::Return => 2,
        Kind::Error => 3,
        Kind::Signal => 4,
    };
    let mut bytes = Writer::default();
    // Little-endian, the message's type, no flags, version 1.
    bytes.bytes.extend_from_slice(&[b'l', kind, 0, 1]);
    bytes.u32(Writer::length(content.bytes.len())?);
    bytes.u32(message.serial);
    bytes.value(&Value::Array("(yv)".to_owned(), fields))?;
    bytes.pad(8);
    bytes.bytes.extend_from_slice(&content.bytes);
    Ok(bytes.bytes)
}

struct Reader<'bytes> {
    bytes: &'bytes [u8],
    at: usize,
    big: bool,
}

impl<'bytes> Reader<'bytes> {
    fn take(&mut self, count: usize) -> Result<&'bytes [u8], Error> {
        let end = self
            .at
            .checked_add(count)
            .ok_or(Error::Malformed("cut short"))?;
        let taken = self
            .bytes
            .get(self.at..end)
            .ok_or(Error::Malformed("cut short"))?;
        self.at = end;
        Ok(taken)
    }

    fn byte(&mut self) -> Result<u8, Error> {
        self.take(1)?
            .first()
            .copied()
            .ok_or(Error::Malformed("cut short"))
    }

    fn pad(&mut self, to: usize) -> Result<(), Error> {
        let padding = self.at.next_multiple_of(to).saturating_sub(self.at);
        if self.take(padding)?.iter().any(|byte| *byte != 0) {
            return Err(Error::Malformed("padding that is not zero"));
        }
        Ok(())
    }

    fn fixed(&mut self, width: usize) -> Result<u64, Error> {
        self.pad(width)?;
        let bytes = self.take(width)?;
        let mut number = [0_u8; 8];
        if self.big {
            let start = 8_usize.saturating_sub(width);
            if let Some(low) = number.get_mut(start..) {
                low.copy_from_slice(bytes);
            }
            Ok(u64::from_be_bytes(number))
        } else {
            if let Some(low) = number.get_mut(..width) {
                low.copy_from_slice(bytes);
            }
            Ok(u64::from_le_bytes(number))
        }
    }

    fn u32(&mut self) -> Result<u32, Error> {
        u32::try_from(self.fixed(4)?)
            .ok()
            .ok_or(Error::Malformed("a number"))
    }

    fn length(&mut self) -> Result<usize, Error> {
        usize::try_from(self.u32()?)
            .ok()
            .ok_or(Error::Malformed("a length"))
    }

    fn text(&mut self, length: usize) -> Result<String, Error> {
        let bytes = self.take(length)?;
        if self.byte()? != 0 || bytes.contains(&0) {
            return Err(Error::Malformed("text not ended by one NUL"));
        }
        String::from_utf8(bytes.to_vec())
            .ok()
            .ok_or(Error::Malformed("text that is not UTF-8"))
    }

    /// The values of the complete types `signature` lists, one after another.
    fn values(&mut self, signature: &[u8]) -> Result<Vec<Value>, Error> {
        let mut values = Vec::new();
        let mut at = 0_usize;
        while at < signature.len() {
            let end = complete(signature, at, 0)?;
            values.push(self.value(signature.get(at..end).unwrap_or_default(), 0)?);
            at = end;
        }
        Ok(values)
    }

    /// The value of the one complete type `signature` is.
    fn value(&mut self, signature: &[u8], depth: usize) -> Result<Value, Error> {
        if depth > DEEPEST {
            return Err(Error::Malformed("nesting"));
        }
        let code = *signature.first().ok_or(Error::Malformed("a signature"))?;
        let inner = signature.get(1..).unwrap_or_default();
        let deeper = depth.saturating_add(1);
        Ok(match code {
            b'y' => Value::Byte(self.byte()?),
            b'b' => match self.u32()? {
                0 => Value::Bool(false),
                1 => Value::Bool(true),
                _ => return Err(Error::Malformed("a boolean that is neither 0 nor 1")),
            },
            b's' | b'o' => {
                let length = self.length()?;
                let text = self.text(length)?;
                if code == b's' {
                    Value::Str(text)
                } else if is_object_path(&text) {
                    Value::Path(text)
                } else {
                    return Err(Error::Malformed("an object path"));
                }
            }
            b'g' => {
                let length = usize::from(self.byte()?);
                Value::Signature(self.text(length)?)
            }
            b'v' => Value::Variant(Box::new(self.variant(deeper)?)),
            b'a' => self.array(inner, deeper)?,
            b'(' => {
                self.pad(8)?;
                let fields = inner
                    .get(..inner.len().saturating_sub(1))
                    .unwrap_or_default();
                let mut values = Vec::new();
                let mut at = 0_usize;
                while at < fields.len() {
                    let end = complete(fields, at, deeper)?;
                    values.push(self.value(fields.get(at..end).unwrap_or_default(), deeper)?);
                    at = end;
                }
                Value::Struct(values)
            }
            b'{' => {
                self.pad(8)?;
                let key = self.value(inner.get(..1).unwrap_or_default(), deeper)?;
                let rest = inner
                    .get(1..inner.len().saturating_sub(1))
                    .unwrap_or_default();
                let value = self.value(rest, deeper)?;
                Value::Entry(Box::new(key), Box::new(value))
            }
            _ => match width(code) {
                Some(width) => Value::Number(code, self.fixed(width)?),
                None => return Err(Error::Malformed("a type")),
            },
        })
    }

    /// The value a variant holds, after the signature it names.
    fn variant(&mut self, depth: usize) -> Result<Value, Error> {
        let length = usize::from(self.byte()?);
        let named = self.text(length)?;
        if complete(named.as_bytes(), 0, depth)? != named.len() {
            return Err(Error::Malformed("a variant of more than one type"));
        }
        self.value(named.as_bytes(), depth)
    }

    fn array(&mut self, element: &[u8], depth: usize) -> Result<Value, Error> {
        let length = self.length()?;
        // D-Bus's own limit on an array, far above anything the Secret Service sends.
        if length > 1_usize << 26_u32 {
            return Err(Error::Malformed("an array longer than D-Bus allows"));
        }
        self.pad(element_alignment(
            *element.first().ok_or(Error::Malformed("a signature"))?,
        ))?;
        if element == b"y" {
            return Ok(Value::Bytes(Zeroizing::new(self.take(length)?.to_vec())));
        }
        let end = self
            .at
            .checked_add(length)
            .filter(|end| *end <= self.bytes.len())
            .ok_or(Error::Malformed("cut short"))?;
        let mut items = Vec::new();
        while self.at < end {
            items.push(self.value(element, depth)?);
        }
        if self.at != end {
            return Err(Error::Malformed(
                "an array whose length is not its elements'",
            ));
        }
        Ok(Value::Array(
            String::from_utf8_lossy(element).into_owned(),
            items,
        ))
    }
}

/// What a message is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Call,
    Return,
    Error,
    Signal,
}

/// A message read from the bus.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Message {
    pub kind: Kind,
    pub serial: u32,
    /// The serial of the call this answers.
    pub reply_to: Option<u32>,
    pub path: Option<String>,
    pub interface: Option<String>,
    pub member: Option<String>,
    /// The name of the error, when this is one.
    pub error: Option<String>,
    pub body: Vec<Value>,
}

/// How long the whole message starting with the 16 bytes `head` is.
///
/// # Errors
///
/// When `head` is not the start of a message, or the message is longer than [`LARGEST`].
pub fn length(head: &[u8]) -> Result<usize, Error> {
    let mut reader = Reader {
        bytes: head,
        at: 0,
        big: order(head.first().copied())?,
    };
    reader.at = 4;
    let body = reader.length()?;
    reader.at = 12;
    let fields = reader.length()?;
    16_usize
        .checked_add(fields)
        .map(|header| header.next_multiple_of(8))
        .and_then(|header| header.checked_add(body))
        .filter(|total| *total <= LARGEST)
        .ok_or(Error::Malformed("longer than steamship reads"))
}

const fn order(byte: Option<u8>) -> Result<bool, Error> {
    match byte {
        Some(b'l') => Ok(false),
        Some(b'B') => Ok(true),
        _ => Err(Error::Malformed("a byte order")),
    }
}

/// The message `bytes` hold, all of them.
///
/// # Errors
///
/// When they are not exactly one message as D-Bus allows.
pub fn message(bytes: &[u8]) -> Result<Message, Error> {
    let mut reader = Reader {
        bytes,
        at: 1,
        big: order(bytes.first().copied())?,
    };
    let kind = match reader.byte()? {
        1 => Kind::Call,
        2 => Kind::Return,
        3 => Kind::Error,
        4 => Kind::Signal,
        _ => return Err(Error::Malformed("a message type")),
    };
    let _flags = reader.byte()?;
    if reader.byte()? != 1 {
        return Err(Error::Malformed("a protocol version"));
    }
    let body_length = reader.length()?;
    let serial = reader.u32()?;
    if serial == 0 {
        return Err(Error::Malformed("a serial of 0"));
    }
    let mut message = Message {
        kind,
        serial,
        reply_to: None,
        path: None,
        interface: None,
        member: None,
        error: None,
        body: Vec::new(),
    };
    let mut signature = String::new();
    // The header fields, `a(yv)`: each a code and a variant, read as they come.
    let fields_length = reader.length()?;
    reader.pad(8)?;
    let fields_end = reader
        .at
        .checked_add(fields_length)
        .ok_or(Error::Malformed("cut short"))?;
    while reader.at < fields_end {
        reader.pad(8)?;
        let code = reader.byte()?;
        match (code, reader.variant(0)?) {
            (1, Value::Path(path)) => message.path = Some(path),
            (2, Value::Str(interface)) => message.interface = Some(interface),
            (3, Value::Str(member)) => message.member = Some(member),
            (4, Value::Str(error)) => message.error = Some(error),
            (5, Value::Number(b'u', answered)) => message.reply_to = u32::try_from(answered).ok(),
            (8, Value::Signature(named)) => signature = named,
            // The destination, the sender and a count of file descriptors: nothing steamship uses.
            (6 | 7, Value::Str(_)) | (9, Value::Number(b'u', _)) => {}
            (1..=9, _) => return Err(Error::Malformed("a header field of the wrong type")),
            _ => {}
        }
    }
    if reader.at != fields_end {
        return Err(Error::Malformed("header fields of another length"));
    }
    reader.pad(8)?;
    let answers = matches!(kind, Kind::Return | Kind::Error);
    if answers != message.reply_to.is_some() || (kind == Kind::Error) != message.error.is_some() {
        return Err(Error::Malformed("header fields its type needs"));
    }
    if reader.at.checked_add(body_length) != Some(bytes.len()) {
        return Err(Error::Malformed("a body of another length"));
    }
    message.body = reader.values(signature.as_bytes())?;
    if reader.at != bytes.len() {
        return Err(Error::Malformed("a body of another length"));
    }
    Ok(message)
}

/// Where a bus may be listening.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Place {
    /// A socket file.
    Path(PathBuf),
    /// A socket in Linux's abstract namespace, which has a name but no file.
    Abstract(Vec<u8>),
}

/// The places a bus address such as `unix:path=/run/user/1000/bus` names, in the order to try
/// them; those steamship cannot reach, such as TCP, are left out.
#[must_use]
pub fn addresses(address: &str) -> Vec<Place> {
    address
        .split(';')
        .filter_map(|entry| {
            let options = entry.strip_prefix("unix:")?;
            options.split(',').find_map(|option| {
                let (key, value) = option.split_once('=')?;
                let value = unescaped(value)?;
                match key {
                    "path" => Some(Place::Path(PathBuf::from(String::from_utf8(value).ok()?))),
                    "abstract" => Some(Place::Abstract(value)),
                    _ => None,
                }
            })
        })
        .collect()
}

/// `value` with each `%` and two hexadecimal digits turned back into the byte they stand for.
fn unescaped(value: &str) -> Option<Vec<u8>> {
    let mut bytes = Vec::with_capacity(value.len());
    let mut rest = value.bytes();
    while let Some(byte) = rest.next() {
        if byte == b'%' {
            let high = char::from(rest.next()?).to_digit(16)?;
            let low = char::from(rest.next()?).to_digit(16)?;
            bytes.push(u8::try_from(high.checked_mul(16)?.checked_add(low)?).ok()?);
        } else {
            bytes.push(byte);
        }
    }
    Some(bytes)
}

/// The session bus itself, which only Linux has.
#[cfg(target_os = "linux")]
pub mod session {
    use std::ffi::OsString;
    use std::io::{Read as _, Write as _};
    use std::os::linux::net::SocketAddrExt as _;
    use std::os::unix::net::{SocketAddr, UnixStream};
    use std::path::PathBuf;
    use std::time::Duration;

    use zeroize::Zeroizing;

    use super::{Error, Kind, Message, Place, Value, addresses, call, length, message};
    use crate::{digest, unix};

    /// How long an answer may take; the bus and the Secret Service answer at once.
    const ANSWER: Duration = Duration::from_secs(30);

    /// Signals held while a call is answered, in case one is waited for next.
    const HELD: usize = 64;

    /// The longest answer to the login read, its line end included; a bus's is under 50 bytes.
    pub const LOGIN_LINE: usize = 512;

    /// A connection to the session bus.
    #[derive(Debug)]
    pub struct Connection {
        stream: UnixStream,
        serial: u32,
        held: Vec<Message>,
    }

    impl Connection {
        /// The session bus named in the environment `lookup` reads, logged in to as this user.
        ///
        /// # Errors
        ///
        /// [`Error::NoBus`] when there is none to reach; otherwise when it cannot be talked to.
        pub fn session<Lookup>(lookup: Lookup) -> Result<Self, Error>
        where
            Lookup: Fn(&str) -> Option<OsString>,
        {
            let mut places = lookup("DBUS_SESSION_BUS_ADDRESS")
                .and_then(|address| address.into_string().ok())
                .map(|address| addresses(&address))
                .unwrap_or_default();
            if places.is_empty()
                && let Some(runtime) = lookup("XDG_RUNTIME_DIR")
            {
                places.push(Place::Path(PathBuf::from(runtime).join("bus")));
            }
            let stream = places
                .iter()
                .find_map(|place| match place {
                    Place::Path(path) => UnixStream::connect(path).ok(),
                    Place::Abstract(name) => SocketAddr::from_abstract_name(name)
                        .and_then(|address| UnixStream::connect_addr(&address))
                        .ok(),
                })
                .ok_or(Error::NoBus)?;
            stream.set_read_timeout(Some(ANSWER))?;
            stream.set_write_timeout(Some(ANSWER))?;
            let mut connection = Self {
                stream,
                serial: 0,
                held: Vec::new(),
            };
            connection.authenticate()?;
            let _name = connection.call(
                "org.freedesktop.DBus",
                "/org/freedesktop/DBus",
                "org.freedesktop.DBus",
                "Hello",
                &[],
            )?;
            Ok(connection)
        }

        /// Logs in as the user steamship runs as, which the bus checks against the socket's peer.
        fn authenticate(&mut self) -> Result<(), Error> {
            let user = digest::hex(unix::user_id().to_string().as_bytes());
            self.stream
                .write_all(format!("\0AUTH EXTERNAL {user}\r\n").as_bytes())?;
            let mut line = Vec::new();
            while !line.ends_with(b"\r\n") {
                if line.len() == LOGIN_LINE {
                    return Err(Error::Refused);
                }
                let mut byte = [0_u8; 1];
                self.stream.read_exact(&mut byte)?;
                line.extend_from_slice(&byte);
            }
            if !line.starts_with(b"OK ") {
                return Err(Error::Refused);
            }
            self.stream.write_all(b"BEGIN\r\n")?;
            Ok(())
        }

        /// Calls `member` of `interface` on the object at `path` of `destination`, and gives
        /// back what it answered.
        ///
        /// # Errors
        ///
        /// When the call cannot be made, or answers with an error.
        pub fn call(
            &mut self,
            destination: &str,
            path: &str,
            interface: &str,
            member: &str,
            body: &[Value],
        ) -> Result<Vec<Value>, Error> {
            self.serial = self
                .serial
                .checked_add(1)
                .ok_or(Error::Malformed("no serial left"))?;
            let serial = self.serial;
            let sent = call(serial, destination, path, interface, member, body)?;
            self.stream.write_all(&sent)?;
            loop {
                let answer = self.read()?;
                match answer.kind {
                    // steamship makes one call at a time, so any other answer is the bus gone
                    // wrong, and waiting on would only wait for nothing.
                    Kind::Return | Kind::Error if answer.reply_to != Some(serial) => {
                        return Err(Error::Malformed("an answer to another call"));
                    }
                    Kind::Return => return Ok(answer.body),
                    Kind::Error => {
                        return Err(Error::Failed {
                            name: answer.error.unwrap_or_default(),
                            message: answer
                                .body
                                .first()
                                .and_then(Value::text)
                                .unwrap_or_default()
                                .to_owned(),
                        });
                    }
                    Kind::Signal => {
                        if self.held.len() >= HELD {
                            drop(self.held.remove(0));
                        }
                        self.held.push(answer);
                    }
                    // steamship offers nothing to call, and a caller given no answer gives up.
                    Kind::Call => {}
                }
            }
        }

        /// Waits up to `limit` for the signal `member` of `interface` from the object at
        /// `path`, which a match rule has asked the bus for, and gives back what it carries.
        ///
        /// # Errors
        ///
        /// When it does not come in time, or the bus cannot be read.
        pub fn signal(
            &mut self,
            path: &str,
            interface: &str,
            member: &str,
            limit: Duration,
        ) -> Result<Vec<Value>, Error> {
            let wanted = |message: &Message| {
                message.kind == Kind::Signal
                    && message.path.as_deref() == Some(path)
                    && message.interface.as_deref() == Some(interface)
                    && message.member.as_deref() == Some(member)
            };
            if let Some(at) = self.held.iter().position(wanted) {
                return Ok(self.held.remove(at).body);
            }
            self.stream.set_read_timeout(Some(limit))?;
            let found = loop {
                match self.read() {
                    Ok(message) if wanted(&message) => break Ok(message.body),
                    Ok(_) => {}
                    Err(error) => break Err(error),
                }
            };
            self.stream.set_read_timeout(Some(ANSWER))?;
            found
        }

        fn read(&mut self) -> Result<Message, Error> {
            let mut head = [0_u8; 16];
            self.stream.read_exact(&mut head)?;
            let mut bytes = Zeroizing::new(vec![0_u8; length(&head)?]);
            let (start, rest) = bytes.split_at_mut(head.len());
            start.copy_from_slice(&head);
            self.stream.read_exact(rest)?;
            message(&bytes)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read_back(bytes: &[u8]) -> Message {
        message(bytes).unwrap()
    }

    #[test]
    fn a_call_is_written_as_the_specification_lays_it_out() {
        let bytes = call(
            7,
            "org.freedesktop.DBus",
            "/org/freedesktop/DBus",
            "org.freedesktop.DBus",
            "Hello",
            &[],
        )
        .unwrap();
        assert_eq!(bytes.get(..4), Some(&[b'l', 1, 0, 1][..]));
        assert_eq!(bytes.get(4..8), Some(&0_u32.to_le_bytes()[..]), "no body");
        assert_eq!(bytes.get(8..12), Some(&7_u32.to_le_bytes()[..]));
        assert!(bytes.len().is_multiple_of(8), "the header is padded to 8");
        assert_eq!(length(&bytes).unwrap(), bytes.len());
        let read = read_back(&bytes);
        assert_eq!(read.kind, Kind::Call);
        assert_eq!(read.serial, 7);
        assert_eq!(read.path.as_deref(), Some("/org/freedesktop/DBus"));
        assert_eq!(read.member.as_deref(), Some("Hello"));
        assert_eq!(read.body, Vec::<Value>::new());
    }

    /// Worked out by hand from the specification, so that a mistake the writer and the reader
    /// share, which a round trip cannot see, shows here.
    #[test]
    fn a_call_is_exactly_the_bytes_the_specification_gives() {
        let bytes = call(
            1,
            "d",
            "/p",
            "i.f",
            "M",
            &[Value::Str("ab".to_owned()), Value::u32(7)],
        )
        .unwrap();
        let expected: &[u8] = &[
            b'l', 1, 0, 1, 12, 0, 0, 0, 1, 0, 0, 0, 72, 0, 0, 0, // head, fields 72 long
            1, 1, b'o', 0, 2, 0, 0, 0, b'/', b'p', 0, 0, 0, 0, 0, 0, // path, padded to 8
            2, 1, b's', 0, 3, 0, 0, 0, b'i', b'.', b'f', 0, 0, 0, 0, 0, // interface
            3, 1, b's', 0, 1, 0, 0, 0, b'M', 0, 0, 0, 0, 0, 0, 0, // member
            6, 1, b's', 0, 1, 0, 0, 0, b'd', 0, 0, 0, 0, 0, 0, 0, // destination
            8, 1, b'g', 0, 2, b's', b'u', 0, // signature, ending on 8
            2, 0, 0, 0, b'a', b'b', 0, 0, 7, 0, 0, 0, // "ab", then 7 on 4
        ];
        assert_eq!(bytes.as_slice(), expected);
    }

    #[test]
    fn each_type_is_aligned_as_the_specification_gives() {
        // The body is the message's last bytes, as many as its header says.
        let body = |values: &[Value]| {
            let bytes = call(1, "d", "/p", "i.f", "M", values).unwrap();
            let length: [u8; 4] = bytes.get(4..8).unwrap().try_into().unwrap();
            let length = usize::try_from(u32::from_le_bytes(length)).unwrap();
            let start = bytes.len().checked_sub(length).unwrap();
            bytes.get(start..).unwrap().to_vec()
        };
        assert_eq!(
            body(&[Value::texts(&[("k", "v")])]),
            [
                14, 0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0, b'k', 0, 0, 0, 1, 0, 0, 0, b'v', 0
            ],
            "an array's length, then its entries from the next 8"
        );
        assert_eq!(
            body(&[
                Value::Byte(1),
                Value::Number(b'n', 0x0203),
                Value::Number(b'x', 5)
            ]),
            [1, 0, 3, 2, 0, 0, 0, 0, 5, 0, 0, 0, 0, 0, 0, 0],
            "numbers on their own width"
        );
        assert_eq!(
            body(&[Value::Bool(true), Value::Variant(Box::new(Value::Byte(9)))]),
            [1, 0, 0, 0, 1, b'y', 0, 9],
            "a boolean as 4 bytes, a variant as its signature then its value"
        );
        assert_eq!(
            body(&[Value::paths(&[])]),
            [0, 0, 0, 0],
            "an empty array of paths is only its length"
        );
    }

    #[test]
    fn every_type_the_secret_service_uses_reads_back_as_written() {
        let secret = Value::Struct(vec![
            Value::Path("/org/freedesktop/secrets/session/1".to_owned()),
            Value::Bytes(Zeroizing::new(Vec::new())),
            Value::Bytes(Zeroizing::new(b"0123456789abcdef".to_vec())),
            Value::Str("text/plain".to_owned()),
        ]);
        let properties = Value::Array(
            "{sv}".to_owned(),
            vec![
                Value::Entry(
                    Box::new(Value::Str("org.freedesktop.Secret.Item.Label".to_owned())),
                    Box::new(Value::Variant(Box::new(Value::Str("label".to_owned())))),
                ),
                Value::Entry(
                    Box::new(Value::Str(
                        "org.freedesktop.Secret.Item.Attributes".to_owned(),
                    )),
                    Box::new(Value::Variant(Box::new(Value::texts(&[("a", "b")])))),
                ),
            ],
        );
        let body = vec![
            properties,
            secret,
            Value::Bool(true),
            Value::Byte(9),
            Value::u32(70_000),
            Value::Number(b'x', u64::MAX),
            Value::Number(b'n', 0xbeef),
            Value::Signature("a{sv}".to_owned()),
            Value::paths(&[]),
            Value::paths(&["/a".to_owned(), "/b".to_owned()]),
        ];
        let bytes = call(1, "d", "/p", "i.f", "M", &body).unwrap();
        let read = read_back(&bytes);
        assert_eq!(read.body, body);
    }

    fn padded(bytes: &mut Vec<u8>) {
        while !bytes.len().is_multiple_of(8) {
            bytes.push(0);
        }
    }

    /// A message as a big-endian sender would write it: a signal carrying `(bv)`, the body a
    /// `true`, then a variant holding the path "/".
    fn big_endian_signal() -> Vec<u8> {
        let mut bytes = vec![b'B', 4, 0, 1];
        let body = [0, 0, 0, 1, 1, b'o', 0, 0, 0, 0, 0, 1, b'/', 0];
        bytes.extend_from_slice(&u32::try_from(body.len()).unwrap().to_be_bytes());
        bytes.extend_from_slice(&3_u32.to_be_bytes());
        let mut fields = Vec::new();
        for (code, signature, text) in [
            (1_u8, b'o', "/p"),
            (2, b's', "org.freedesktop.Secret.Prompt"),
            (3, b's', "Completed"),
        ] {
            padded(&mut fields);
            fields.extend_from_slice(&[code, 1, signature, 0]);
            fields.extend_from_slice(&u32::try_from(text.len()).unwrap().to_be_bytes());
            fields.extend_from_slice(text.as_bytes());
            fields.push(0);
        }
        padded(&mut fields);
        fields.extend_from_slice(&[8, 1, b'g', 0, 2, b'b', b'v', 0]);
        bytes.extend_from_slice(&u32::try_from(fields.len()).unwrap().to_be_bytes());
        bytes.extend_from_slice(&fields);
        padded(&mut bytes);
        bytes.extend_from_slice(&body);
        bytes
    }

    #[test]
    fn a_big_endian_message_is_read_in_its_own_order() {
        let bytes = big_endian_signal();
        assert_eq!(length(&bytes).unwrap(), bytes.len());
        let read = read_back(&bytes);
        assert_eq!(read.kind, Kind::Signal);
        assert_eq!(read.member.as_deref(), Some("Completed"));
        assert_eq!(
            read.body,
            [
                Value::Bool(true),
                Value::Variant(Box::new(Value::Path("/".to_owned())))
            ]
        );
    }

    #[test]
    fn an_error_answer_names_the_call_it_answers_and_the_error() {
        let mut bytes = vec![b'l', 3, 0, 1];
        let body = [5, 0, 0, 0, b'n', b'o', b'p', b'e', b'!', 0];
        bytes.extend_from_slice(&u32::try_from(body.len()).unwrap().to_le_bytes());
        bytes.extend_from_slice(&9_u32.to_le_bytes());
        let name = "org.freedesktop.DBus.Error.ServiceUnknown";
        let mut fields = vec![4, 1, b's', 0];
        fields.extend_from_slice(&u32::try_from(name.len()).unwrap().to_le_bytes());
        fields.extend_from_slice(name.as_bytes());
        fields.push(0);
        padded(&mut fields);
        fields.extend_from_slice(&[5, 1, b'u', 0, 2, 0, 0, 0]);
        fields.extend_from_slice(&[8, 1, b'g', 0, 1, b's', 0]);
        bytes.extend_from_slice(&u32::try_from(fields.len()).unwrap().to_le_bytes());
        bytes.extend_from_slice(&fields);
        padded(&mut bytes);
        bytes.extend_from_slice(&body);
        let read = read_back(&bytes);
        assert_eq!(read.kind, Kind::Error);
        assert_eq!(read.reply_to, Some(2));
        assert_eq!(read.error.as_deref(), Some(name));
        assert_eq!(read.body, [Value::Str("nope!".to_owned())]);
    }

    #[test]
    fn a_message_that_breaks_the_rules_is_refused_not_guessed_at() {
        let good = call(1, "d", "/p", "i.f", "M", &[Value::Str("x".to_owned())]).unwrap();
        // The body is the text's length, "x" and its NUL, after one byte of the header's padding.
        let padding = good.len().checked_sub(7).unwrap();
        let changed = |at: usize, byte: u8| {
            let mut bytes = good.to_vec();
            *bytes.get_mut(at).expect("a byte of the message") = byte;
            bytes
        };
        let mut short = good.to_vec();
        let _: Option<u8> = short.pop();
        let mut long = good.to_vec();
        long.push(0);
        let cases = [
            (changed(0, b'x'), "a byte order"),
            (changed(1, 9), "a message type"),
            (changed(3, 2), "a protocol version"),
            (changed(8, 0), "a serial of 0"),
            (short, "a body of another length"),
            (long, "a body of another length"),
            (changed(padding, 1), "padding that is not zero"),
            (changed(1, 2), "header fields its type needs"),
        ];
        for (bytes, why) in cases {
            assert!(
                matches!(message(&bytes), Err(Error::Malformed(said)) if said == why),
                "{why}"
            );
        }
    }

    #[test]
    fn signatures_are_checked_before_anything_is_read_by_them() {
        for (signature, end) in [
            ("s", 1),
            ("ay", 2),
            ("a{sv}", 5),
            ("(oayays)", 8),
            ("a(yv)", 5),
            ("aao", 3),
        ] {
            assert_eq!(
                complete(signature.as_bytes(), 0, 0).unwrap(),
                end,
                "{signature}"
            );
        }
        for signature in [
            "", "a", "()", "(s", "a{vs}", "a{(s)s}", "{ss}x", "z", "a{s}", "a{sss}",
        ] {
            let whole = complete(signature.as_bytes(), 0, 0)
                .is_ok_and(|end| end == signature.len() && !signature.starts_with('{'));
            assert!(!whole, "{signature}");
        }
        let deepest = format!("{}y", "a".repeat(DEEPEST));
        assert_eq!(
            complete(deepest.as_bytes(), 0, 0).ok(),
            Some(deepest.len()),
            "nesting as deep as steamship reads"
        );
        let deeper = format!("a{deepest}");
        assert!(matches!(
            complete(deeper.as_bytes(), 0, 0),
            Err(Error::Malformed("nesting"))
        ));
    }

    #[test]
    fn a_length_past_the_largest_is_refused_before_anything_is_allocated() {
        let mut head = vec![b'l', 2, 0, 1];
        head.extend_from_slice(&u32::MAX.to_le_bytes());
        head.extend_from_slice(&1_u32.to_le_bytes());
        head.extend_from_slice(&0_u32.to_le_bytes());
        assert!(matches!(
            length(&head),
            Err(Error::Malformed("longer than steamship reads"))
        ));
        assert!(
            matches!(length(b"l\x02\x00\x01"), Err(Error::Malformed("cut short"))),
            "a head cut short"
        );
    }

    #[test]
    fn what_d_bus_cannot_carry_is_not_sent() {
        let sent = |value: Value| call(1, "d", "/p", "i.f", "M", &[value]);
        assert!(matches!(
            sent(Value::Str("a\0b".to_owned())),
            Err(Error::Malformed("text holding a NUL"))
        ));
        assert!(matches!(
            sent(Value::Number(b'z', 0)),
            Err(Error::Malformed("a number's type"))
        ));
        assert!(matches!(
            sent(Value::Signature("y".repeat(256))),
            Err(Error::Malformed("a signature"))
        ));
        for path in ["", "xyzzy", "//", "/a/", "/a//b", "/a-b", "/\u{e9}"] {
            assert!(
                matches!(
                    sent(Value::Path(path.to_owned())),
                    Err(Error::Malformed("an object path"))
                ),
                "{path:?}"
            );
        }
        for path in ["/", "/a", "/org/freedesktop/secrets/collection/login_2"] {
            assert!(sent(Value::Path(path.to_owned())).is_ok(), "{path:?}");
        }
    }

    /// A message of type `kind` with the header fields `fields` and the body `body`, from serial 1.
    fn built(kind: u8, fields: &[u8], body: &[u8]) -> Vec<u8> {
        let mut bytes = vec![b'l', kind, 0, 1];
        bytes.extend_from_slice(&u32::try_from(body.len()).unwrap().to_le_bytes());
        bytes.extend_from_slice(&1_u32.to_le_bytes());
        bytes.extend_from_slice(&u32::try_from(fields.len()).unwrap().to_le_bytes());
        bytes.extend_from_slice(fields);
        padded(&mut bytes);
        bytes.extend_from_slice(body);
        bytes
    }

    #[test]
    fn header_fields_are_read_by_their_code_and_type() {
        let unknown = read_back(&built(1, &[10, 1, b'y', 0, 7], &[]));
        assert_eq!(
            unknown.path, None,
            "a field of a code D-Bus may add later is passed over"
        );

        let cases = [
            (
                built(1, &[1, 1, b's', 0, 1, 0, 0, 0, b'x', 0], &[]),
                "a header field of the wrong type",
            ),
            (
                {
                    let mut bytes = built(1, &[3, 1, b's', 0, 1, 0, 0, 0, b'M', 0], &[]);
                    *bytes.get_mut(12).expect("the fields' length") = 4;
                    bytes
                },
                "header fields of another length",
            ),
            (
                built(1, &[8, 1, b'g', 0, 1, b'y', 0], &[1, 2]),
                "a body of another length",
            ),
            (
                {
                    let mut body = [1, b'v', 0].repeat(DEEPEST.saturating_add(2));
                    body.extend_from_slice(&[1, b'y', 0, 9]);
                    built(1, &[8, 1, b'g', 0, 1, b'v', 0], &body)
                },
                "nesting",
            ),
        ];
        for (bytes, why) in cases {
            assert!(
                matches!(message(&bytes), Err(Error::Malformed(said)) if said == why),
                "{why}"
            );
        }
    }

    #[test]
    fn a_value_that_breaks_the_rules_is_refused_not_guessed_at() {
        let read = |signature: &[u8], bytes: &[u8]| {
            Reader {
                bytes,
                at: 0,
                big: false,
            }
            .value(signature, 0)
        };
        let longest = u32::try_from(1_usize << 26_u32).unwrap();
        let too_long = longest.saturating_add(1);
        let cases: [(&[u8], Vec<u8>, &str); 12] = [
            (b"b", vec![2, 0, 0, 0], "a boolean that is neither 0 nor 1"),
            (b"s", vec![1, 0, 0, 0, b'x', 1], "text not ended by one NUL"),
            (b"s", vec![1, 0, 0, 0, 0, 0], "text not ended by one NUL"),
            (b"s", vec![1, 0, 0, 0, 0xff, 0], "text that is not UTF-8"),
            (
                b"v",
                vec![2, b's', b's', 0],
                "a variant of more than one type",
            ),
            (b"z", vec![0], "a type"),
            (b"o", vec![1, 0, 0, 0, b'x', 0], "an object path"),
            (b"", vec![0], "a signature"),
            (
                b"ay",
                too_long.to_le_bytes().to_vec(),
                "an array longer than D-Bus allows",
            ),
            (b"ay", longest.to_le_bytes().to_vec(), "cut short"),
            (
                b"au",
                vec![5, 0, 0, 0, 1, 0, 0, 0, 2, 0, 0, 0],
                "an array whose length is not its elements'",
            ),
            (b"au", vec![8, 0, 0, 0, 1, 0, 0, 0], "cut short"),
        ];
        for (signature, bytes, why) in cases {
            let refused = read(signature, &bytes);
            assert!(
                matches!(refused, Err(Error::Malformed(said)) if said == why),
                "{why}"
            );
        }
        let mut empty_element = Reader {
            bytes: &[0, 0, 0, 0],
            at: 0,
            big: false,
        };
        assert!(matches!(
            empty_element.array(b"", 0),
            Err(Error::Malformed("a signature"))
        ));
        let mut deepest = Reader {
            bytes: &[7],
            at: 0,
            big: false,
        };
        assert!(matches!(deepest.value(b"y", DEEPEST), Ok(Value::Byte(7))));
        let mut deep = Reader {
            bytes: &[0],
            at: 0,
            big: false,
        };
        assert!(matches!(
            deep.value(b"y", DEEPEST.saturating_add(1)),
            Err(Error::Malformed("nesting"))
        ));
    }

    #[test]
    fn a_secret_is_never_shown_by_debug() {
        let shown = format!("{:?}", Value::Bytes(Zeroizing::new(b"hunter2".to_vec())));
        assert_eq!(shown, "Bytes(7 bytes)");
    }

    #[test]
    fn debug_and_signature_show_each_value_by_its_type() {
        let entry = Value::Entry(
            Box::new(Value::Str("k".to_owned())),
            Box::new(Value::Variant(Box::new(Value::Struct(vec![
                Value::Byte(1),
                Value::Bool(true),
                Value::u32(2),
                Value::Path("/p".to_owned()),
                Value::Signature("s".to_owned()),
            ])))),
        );
        assert_eq!(entry.signature(), "{sv}");
        assert_eq!(
            Value::Struct(vec![
                Value::Byte(1),
                Value::Bytes(Zeroizing::new(Vec::new()))
            ])
            .signature(),
            "(yay)"
        );
        assert_eq!(
            format!("{:?}", Value::Array("{sv}".to_owned(), vec![entry])),
            r#"Array("{sv}", [Entry(Str("k"), Variant(Struct([Byte(1), Bool(true), Number(u, 2), Path("/p"), Signature("s")])))])"#
        );
    }

    #[test]
    fn values_hand_over_what_they_hold_only_when_they_are_that() {
        assert_eq!(Value::Str("a".to_owned()).text(), Some("a"));
        assert_eq!(Value::Path("/a".to_owned()).text(), None);
        assert_eq!(Value::Path("/a".to_owned()).path(), Some("/a"));
        assert_eq!(Value::Str("a".to_owned()).path(), None);
        assert_eq!(
            Value::paths(&["/a".to_owned()]).path_list(),
            Some(vec!["/a".to_owned()])
        );
        assert_eq!(Value::texts(&[("a", "b")]).path_list(), None);
        assert_eq!(Value::Byte(1).path_list(), None);
    }

    #[test]
    fn a_bus_address_names_its_sockets_in_order_and_unescapes_them() {
        assert_eq!(
            addresses("unix:path=/run/user/1000/bus"),
            [Place::Path(PathBuf::from("/run/user/1000/bus"))]
        );
        assert_eq!(
            addresses("tcp:host=localhost,port=1;unix:abstract=/tmp/dbus-%41b,guid=1;unix:path=/x"),
            [
                Place::Abstract(b"/tmp/dbus-Ab".to_vec()),
                Place::Path(PathBuf::from("/x"))
            ]
        );
        for nowhere in [
            "unix:path=/bad%4",
            "unix:path=/bad%zz",
            "unix:tmpdir=/tmp",
            "",
        ] {
            assert_eq!(addresses(nowhere), Vec::<Place>::new(), "{nowhere}");
        }
    }

    #[test]
    fn errors_say_what_went_wrong() {
        let failed = Error::Failed {
            name: "org.freedesktop.Secret.Error.IsLocked".to_owned(),
            message: String::new(),
        };
        assert_eq!(failed.to_string(), "org.freedesktop.Secret.Error.IsLocked");
        let said = Error::Failed {
            name: "a.B".to_owned(),
            message: "why".to_owned(),
        };
        assert_eq!(said.to_string(), "a.B: why");
        assert_eq!(Error::NoBus.to_string(), "no D-Bus session bus");
        assert_eq!(
            Error::Dismissed.to_string(),
            "the prompt to unlock it was dismissed"
        );
        assert_eq!(
            Error::Refused.to_string(),
            "the session bus refused this user"
        );
        assert_eq!(
            Error::from(io::Error::other("gone")).to_string(),
            "the session bus: gone"
        );
        assert_eq!(
            Error::Malformed("x").to_string(),
            "a message from the session bus: x"
        );
    }
}
