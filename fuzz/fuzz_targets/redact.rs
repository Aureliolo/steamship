//! A steamcmd file and an output, split at the first NUL. Reading the file never panics, no
//! secret it holds is left in the output, and redacting again changes nothing.
#![no_main]

use libfuzzer_sys::fuzz_target;
use steamship::redact::Redactor;

fuzz_target!(|data: &[u8]| {
    let split = data
        .iter()
        .position(|&byte| byte == 0)
        .unwrap_or(data.len());
    let (file, output) = data.split_at(split);
    let redactor = Redactor::from_texts(&[file.to_vec()]);
    let redacted = redactor.redact(output);
    for secret in redactor.secrets() {
        assert!(
            !redacted
                .windows(secret.len())
                .any(|window| window == secret.as_slice()),
            "{secret:?} is left in {redacted:?}"
        );
    }
    assert_eq!(
        redactor.redact(&redacted),
        redacted,
        "redacting again changed it"
    );
});
