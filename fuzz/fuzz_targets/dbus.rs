//! Any bytes from the session bus are a message or refused, never a panic or a runaway: its
//! length is bounded before anything is read, and a message read is one steamship could send.
#![no_main]
#![expect(
    clippy::panic,
    reason = "a panic is how a fuzz target reports what it found"
)]

use libfuzzer_sys::fuzz_target;
use steamship::dbus::{self, Kind};

fuzz_target!(|data: &[u8]| {
    if let Ok(length) = dbus::length(data)
        && length > dbus::LARGEST
    {
        panic!("a length of {length} passed the limit");
    }
    let Ok(message) = dbus::message(data) else {
        return;
    };
    assert!(message.serial != 0, "a message with a serial of 0 was read");
    let answers = matches!(message.kind, Kind::Return | Kind::Error);
    assert!(
        answers == message.reply_to.is_some(),
        "{:?} read with reply serial {:?}",
        message.kind,
        message.reply_to
    );
    let Ok(sent) = dbus::call(1, "d", "/p", "i.f", "M", &message.body) else {
        return;
    };
    match dbus::message(&sent) {
        Ok(read) if read.body == message.body => {}
        other => panic!("a body read did not read back as written: {other:?}"),
    }
});
