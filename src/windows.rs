//! What only Windows does. Kept in a module of its own so that everything here is compiled,
//! tested and mutation-tested on the system where it runs.
#![expect(
    unsafe_code,
    reason = "Windows' own APIs are the only way to ask Windows these questions"
)]

use std::io;
use std::os::windows::ffi::OsStrExt as _;
use std::path::Path;
use std::ptr;

use windows_sys::Win32::Foundation::{HANDLE, INVALID_HANDLE_VALUE};
use windows_sys::Win32::Security::Cryptography::{
    CERT_NAME_SIMPLE_DISPLAY_TYPE, CertGetNameStringW,
};
use windows_sys::Win32::Security::WinTrust::{
    WINTRUST_ACTION_GENERIC_VERIFY_V2, WINTRUST_DATA, WINTRUST_DATA_0, WINTRUST_FILE_INFO,
    WTD_CHOICE_FILE, WTD_REVOCATION_CHECK_CHAIN_EXCLUDE_ROOT, WTD_REVOKE_WHOLECHAIN,
    WTD_STATEACTION_CLOSE, WTD_STATEACTION_VERIFY, WTD_UI_NONE, WTHelperGetProvSignerFromChain,
    WTHelperProvDataFromStateData, WinVerifyTrust,
};

/// Windows has no executable bit, so there is nothing to mark.
///
/// # Errors
///
/// Never; the signature is the one every system shares.
pub const fn mark_if_program(_: &Path) -> io::Result<()> {
    Ok(())
}

/// Only macOS packages hold links, and they are only ever unpacked on macOS, so a link in a
/// package for Windows is refused rather than guessed at.
///
/// # Errors
///
/// Always.
pub fn make_link(_: &str, _: &Path) -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "a package for this system holds a link",
    ))
}

/// Who signed steamcmd's own programs and libraries.
pub const VALVE: &str = "Valve Corp.";

/// Valve ships a web UI for its content server alongside steamcmd, bundling Microsoft's runtime
/// (signed by Microsoft) and a few third-party libraries (not signed at all). steamship never
/// runs it; its files are held to their pinned SHA-256 like every other, but not to Valve's
/// signature, which they do not carry.
const NOT_VALVES: &str = "siteserverui/";

/// Every one of `files` under `root` that is steamcmd's own program or library and does not carry
/// a valid signature from Valve, each said as a change to the install.
pub fn unsigned<'file, Files>(root: &Path, files: Files) -> Vec<String>
where
    Files: IntoIterator<Item = &'file String>,
{
    files
        .into_iter()
        .filter(|relative| !relative.starts_with(NOT_VALVES))
        .filter(|relative| {
            Path::new(relative.as_str())
                .extension()
                .is_some_and(|extension| {
                    extension.eq_ignore_ascii_case("exe") || extension.eq_ignore_ascii_case("dll")
                })
        })
        .filter_map(|relative| match signature(&root.join(relative)) {
            Signature::Signed(name) if name == VALVE => None,
            Signature::Signed(name) => Some(format!("{relative} is signed by {name}, not {VALVE}")),
            Signature::Refused(status) => Some(format!(
                "{relative} has no valid signature (Windows says {status:#010x})"
            )),
        })
        .collect()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Signature {
    /// Windows verified the signature, and this is the signer's name.
    Signed(String),
    /// Windows did not accept the file's signature, or found none; this is its answer.
    Refused(u32),
}

/// What Windows makes of `path`'s Authenticode signature. No window is ever shown, and the
/// revocation of every certificate in the chain but its root is checked.
#[must_use]
pub fn signature(path: &Path) -> Signature {
    let wide: Vec<u16> = path.as_os_str().encode_wide().chain([0]).collect();
    let mut file = WINTRUST_FILE_INFO {
        cbStruct: size_of_u32::<WINTRUST_FILE_INFO>(),
        pcwszFilePath: wide.as_ptr(),
        ..WINTRUST_FILE_INFO::default()
    };
    let mut data = WINTRUST_DATA {
        cbStruct: size_of_u32::<WINTRUST_DATA>(),
        dwUIChoice: WTD_UI_NONE,
        fdwRevocationChecks: WTD_REVOKE_WHOLECHAIN,
        dwUnionChoice: WTD_CHOICE_FILE,
        Anonymous: WINTRUST_DATA_0 {
            pFile: &raw mut file,
        },
        dwStateAction: WTD_STATEACTION_VERIFY,
        dwProvFlags: WTD_REVOCATION_CHECK_CHAIN_EXCLUDE_ROOT,
        ..WINTRUST_DATA::default()
    };
    let mut action = WINTRUST_ACTION_GENERIC_VERIFY_V2;
    // SAFETY: `data` and the `file` it points to are fully initialised and outlive the call, and
    // `wide` is a NUL-terminated path that outlives both. An invalid window handle and
    // WTD_UI_NONE together tell WinTrust there is nobody to show anything to.
    let status = unsafe {
        WinVerifyTrust(
            INVALID_HANDLE_VALUE,
            &raw mut action,
            (&raw mut data).cast(),
        )
    };
    let answer = if status == 0_i32 {
        signer_name(data.hWVTStateData).map_or(Signature::Refused(0), Signature::Signed)
    } else {
        Signature::Refused(status.cast_unsigned())
    };
    data.dwStateAction = WTD_STATEACTION_CLOSE;
    // SAFETY: the same structures, now asking WinTrust to free the state the call above left in
    // `data.hWVTStateData`, which is what that call's documentation requires.
    let _: i32 = unsafe {
        WinVerifyTrust(
            INVALID_HANDLE_VALUE,
            &raw mut action,
            (&raw mut data).cast(),
        )
    };
    answer
}

/// The simple display name, in practice the common name, of the certificate that signed.
fn signer_name(state: HANDLE) -> Option<String> {
    // SAFETY: `state` is the state a successful verification left open; it stays valid until
    // the closing call, which comes after this function returns.
    let provider = unsafe { WTHelperProvDataFromStateData(state) };
    if provider.is_null() {
        return None;
    }
    // SAFETY: `provider` is non-null and belongs to the same open state.
    let signer = unsafe { WTHelperGetProvSignerFromChain(provider, 0, 0, 0) };
    // SAFETY: a non-null signer points at a structure WinTrust owns for as long as the state.
    let chain = unsafe { signer.as_ref() }?.pasCertChain;
    // SAFETY: the first element of a non-null chain is the signing certificate's entry.
    let certificate = unsafe { chain.as_ref() }?.pCert;
    if certificate.is_null() {
        return None;
    }
    let mut name = vec![0_u16; 256];
    let capacity = u32::try_from(name.len()).ok()?;
    // SAFETY: `certificate` is valid while the state is open, and `name` holds `capacity` units.
    let written = unsafe {
        CertGetNameStringW(
            certificate,
            CERT_NAME_SIMPLE_DISPLAY_TYPE,
            0,
            ptr::null(),
            name.as_mut_ptr(),
            capacity,
        )
    };
    // The count includes the terminating NUL, so one means an empty name.
    let length = usize::try_from(written).ok()?.checked_sub(1)?;
    name.truncate(length);
    String::from_utf16(&name)
        .ok()
        .filter(|name| !name.is_empty())
}

/// The size of a Win32 structure, as the `cbStruct` fields want it.
const fn size_of_u32<Structure>() -> u32 {
    #[expect(
        clippy::as_conversions,
        clippy::cast_possible_truncation,
        reason = "a Win32 structure is a few hundred bytes, nowhere near 4 GiB"
    )]
    let size = size_of::<Structure>() as u32;
    size
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;
    use std::fs;
    use std::path::PathBuf;

    /// A library Windows installs with a signature of its own inside it, rather than in a
    /// catalogue, which is the kind steamcmd's files carry.
    fn signed_by_microsoft() -> PathBuf {
        Path::new(&env::var_os("SystemRoot").unwrap()).join("System32/vcruntime140.dll")
    }

    /// What Windows answers for a file that carries no signature.
    const TRUST_E_NOSIGNATURE: u32 = 0x800b_0100;

    #[test]
    fn names_the_signer_of_a_signed_file() {
        let found = signature(&signed_by_microsoft());
        assert!(
            matches!(&found, Signature::Signed(name) if name.starts_with("Microsoft")),
            "{found:?}"
        );
    }

    #[test]
    fn refuses_a_file_with_no_signature() {
        let unsigned = env::current_exe().unwrap();
        assert_eq!(
            signature(&unsigned),
            Signature::Refused(TRUST_E_NOSIGNATURE)
        );
    }

    #[test]
    fn holds_steamcmds_own_files_to_valves_signature_and_nothing_else() {
        let root = tempfile::tempdir().unwrap();
        let copy = |relative: &str, from: &Path| {
            let to = root.path().join(relative);
            fs::create_dir_all(to.parent().unwrap()).unwrap();
            let _: u64 = fs::copy(from, to).unwrap();
        };
        copy("steamcmd.exe", &signed_by_microsoft());
        copy("bin/unsigned.DLL", &env::current_exe().unwrap());
        copy("siteserverui/win32/node.dll", &env::current_exe().unwrap());
        copy("public/strings.txt", &env::current_exe().unwrap());
        let files: Vec<String> = [
            "steamcmd.exe",
            "bin/unsigned.DLL",
            "siteserverui/win32/node.dll",
            "public/strings.txt",
        ]
        .map(str::to_owned)
        .into();
        let found = unsigned(root.path(), &files);
        let [microsoft, bare] = found.as_slice() else {
            panic!("expected two findings, got {found:?}");
        };
        assert!(
            microsoft.starts_with("steamcmd.exe is signed by Microsoft"),
            "{microsoft}"
        );
        assert!(microsoft.ends_with(", not Valve Corp."), "{microsoft}");
        assert_eq!(
            bare,
            "bin/unsigned.DLL has no valid signature (Windows says 0x800b0100)"
        );
    }
}
