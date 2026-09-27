//! Gives steamship.exe its icon, the details Windows shows under Properties and in Task Manager,
//! its application manifest, and the linker's mitigations that rustc leaves off. The resources
//! are written here in the compiled `.res` form, which Microsoft's linker takes as it is, so
//! building needs neither a resource compiler nor a crate to drive one.

use std::env;
use std::fs;
use std::path::PathBuf;

const ICON: &str = "assets/steamship.ico";

const RT_ICON: u16 = 3;
const RT_GROUP_ICON: u16 = 14;
const RT_VERSION: u16 = 16;
const RT_MANIFEST: u16 = 24;
/// English (United States), with the Unicode code page the strings are written in.
const LANGUAGE: u16 = 0x0409;
const CODE_PAGE: u16 = 1200;

fn main() -> Result<(), String> {
    tell(&format!("cargo::rerun-if-changed={ICON}"));
    tell("cargo::rerun-if-changed=Cargo.toml");
    let windows_msvc = env::var("CARGO_CFG_TARGET_OS").is_ok_and(|os| os == "windows")
        && env::var("CARGO_CFG_TARGET_ENV").is_ok_and(|abi| abi == "msvc");
    if !windows_msvc {
        return Ok(());
    }
    let icon = fs::read(ICON).map_err(|error| format!("{ICON}: {error}"))?;
    let mut res = Vec::new();
    // A compiled resource file opens with an empty entry, which is how the linker knows it.
    resource(&mut res, 0, 0, 0, &[])?;
    icons(&mut res, &icon)?;
    resource(&mut res, RT_VERSION, 1, 0x0030, &version_info()?)?;
    // Resource 1 is the manifest Windows reads when it creates the process.
    resource(&mut res, RT_MANIFEST, 1, 0x0030, manifest()?.as_bytes())?;
    let out = PathBuf::from(env::var("OUT_DIR").map_err(|error| error.to_string())?)
        .join("steamship.res");
    fs::write(&out, res).map_err(|error| format!("{}: {error}", out.display()))?;
    tell(&format!("cargo::rustc-link-arg-bins={}", out.display()));
    // Compatible with the hardware shadow stack, which then guards every return address. And the
    // DLLs steamship imports are looked up in System32 alone: wintrust.dll and the crypto
    // libraries are not among Windows' known DLLs, so without this a copy planted beside the
    // program, in a Downloads folder say, would be loaded in their place.
    tell("cargo::rustc-link-arg-bins=/CETCOMPAT");
    tell("cargo::rustc-link-arg-bins=/DEPENDENTLOADFLAG:0x800");
    Ok(())
}

/// Runs as whoever started it, on Windows 10 and later, with UTF-8 as the code page any
/// narrow-string API uses and paths longer than 260 characters where the system allows them.
fn manifest() -> Result<String, String> {
    let version = env::var("CARGO_PKG_VERSION").map_err(|error| error.to_string())?;
    Ok(format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<assembly xmlns="urn:schemas-microsoft-com:asm.v1" manifestVersion="1.0">
  <assemblyIdentity type="win32" name="Aureliolo.steamship" version="{version}.0"/>
  <trustInfo xmlns="urn:schemas-microsoft-com:asm.v3">
    <security>
      <requestedPrivileges>
        <requestedExecutionLevel level="asInvoker" uiAccess="false"/>
      </requestedPrivileges>
    </security>
  </trustInfo>
  <compatibility xmlns="urn:schemas-microsoft-com:compatibility.v1">
    <application>
      <supportedOS Id="{{8e0f7a12-bfb3-4fe8-b9a5-48fd50a15a9a}}"/>
    </application>
  </compatibility>
  <application xmlns="urn:schemas-microsoft-com:asm.v3">
    <windowsSettings>
      <activeCodePage xmlns="http://schemas.microsoft.com/SMI/2019/WindowsSettings">UTF-8</activeCodePage>
      <longPathAware xmlns="http://schemas.microsoft.com/SMI/2016/WindowsSettings">true</longPathAware>
    </windowsSettings>
  </application>
</assembly>
"#
    ))
}

fn tell(line: &str) {
    println!("{line}");
}

/// One resource, identified by number, in the header the `.res` format gives each.
fn resource(res: &mut Vec<u8>, kind: u16, id: u16, flags: u16, data: &[u8]) -> Result<(), String> {
    let size = u32::try_from(data.len()).map_err(|error| error.to_string())?;
    res.extend_from_slice(&size.to_le_bytes());
    res.extend_from_slice(&32_u32.to_le_bytes());
    for number in [kind, id] {
        res.extend_from_slice(&0xFFFF_u16.to_le_bytes());
        res.extend_from_slice(&number.to_le_bytes());
    }
    res.extend_from_slice(&0_u32.to_le_bytes());
    res.extend_from_slice(&flags.to_le_bytes());
    let language = if kind == 0 { 0 } else { LANGUAGE };
    res.extend_from_slice(&language.to_le_bytes());
    res.extend_from_slice(&[0; 8]);
    res.extend_from_slice(data);
    align(res);
    Ok(())
}

/// Each image of the `.ico` as its own resource, and the group that lists them, which is the
/// `.ico`'s own directory with each image's file offset replaced by its resource number.
fn icons(res: &mut Vec<u8>, icon: &[u8]) -> Result<(), String> {
    let bad = || format!("{ICON} is not an icon file");
    let count = icon
        .get(4..6)
        .and_then(|bytes| bytes.try_into().ok())
        .map(u16::from_le_bytes)
        .ok_or_else(bad)?;
    let mut group = icon.get(..6).ok_or_else(bad)?.to_vec();
    for number in 1..=count {
        let at = usize::from(number)
            .checked_mul(16)
            .and_then(|offset| offset.checked_sub(10))
            .ok_or_else(bad)?;
        let entry = icon
            .get(at..at.checked_add(16).ok_or_else(bad)?)
            .ok_or_else(bad)?;
        let field = |from: usize| -> Result<usize, String> {
            entry
                .get(from..from.checked_add(4).ok_or_else(bad)?)
                .and_then(|bytes| bytes.try_into().ok())
                .map(u32::from_le_bytes)
                .and_then(|value| usize::try_from(value).ok())
                .ok_or_else(bad)
        };
        let (length, offset) = (field(8)?, field(12)?);
        let image = icon
            .get(offset..offset.checked_add(length).ok_or_else(bad)?)
            .ok_or_else(bad)?;
        resource(res, RT_ICON, number, 0x1010, image)?;
        group.extend_from_slice(entry.get(..12).ok_or_else(bad)?);
        group.extend_from_slice(&number.to_le_bytes());
    }
    resource(res, RT_GROUP_ICON, 1, 0x1030, &group)
}

/// The version block: numbers Windows compares, and the strings Properties shows.
fn version_info() -> Result<Vec<u8>, String> {
    let part = |name: &str| -> Result<u32, String> {
        env::var(name)
            .map_err(|error| format!("{name}: {error}"))?
            .parse::<u16>()
            .map(u32::from)
            .map_err(|error| format!("{name}: {error}"))
    };
    let (major, minor, patch) = (
        part("CARGO_PKG_VERSION_MAJOR")?,
        part("CARGO_PKG_VERSION_MINOR")?,
        part("CARGO_PKG_VERSION_PATCH")?,
    );
    let (high, low) = ((major << 16_u32) | minor, patch << 16_u32);
    let mut fixed = Vec::new();
    for word in [
        0xFEEF_04BD, // the signature Windows looks for
        0x0001_0000, // the structure's version
        high,
        low,
        high,
        low,
        0x3F,        // every flag bit is meaningful
        0,           // and none is set: not a debug, patched or pre-release build
        0x0004_0004, // for Windows NT
        1,           // an application
        0,
        0,
        0,
    ] {
        fixed.extend_from_slice(&u32::to_le_bytes(word));
    }
    let version = env::var("CARGO_PKG_VERSION").map_err(|error| error.to_string())?;
    let mut strings = Vec::new();
    for (key, value) in [
        ("CompanyName", "Aurelio Amoroso"),
        ("FileDescription", "steamship"),
        ("FileVersion", version.as_str()),
        ("InternalName", "steamship"),
        ("LegalCopyright", "Copyright (c) 2026 Aurelio Amoroso"),
        ("OriginalFilename", "steamship.exe"),
        ("ProductName", "steamship"),
        ("ProductVersion", version.as_str()),
    ] {
        strings.push(block(key, Value::Text(value), &[])?);
    }
    let table = block(
        &format!("{LANGUAGE:04X}{CODE_PAGE:04X}"),
        Value::None,
        &strings,
    )?;
    let mut translation = LANGUAGE.to_le_bytes().to_vec();
    translation.extend_from_slice(&CODE_PAGE.to_le_bytes());
    block(
        "VS_VERSION_INFO",
        Value::Binary(&fixed),
        &[
            block("StringFileInfo", Value::None, &[table])?,
            block(
                "VarFileInfo",
                Value::None,
                &[block("Translation", Value::Binary(&translation), &[])?],
            )?,
        ],
    )
}

#[derive(Clone, Copy)]
enum Value<'value> {
    None,
    Text(&'value str),
    Binary(&'value [u8]),
}

/// One node of the version block: its length, its value's length and kind, its name, its value,
/// then its children, each starting on a four-byte boundary.
fn block(key: &str, value: Value<'_>, children: &[Vec<u8>]) -> Result<Vec<u8>, String> {
    let (bytes, value_length, text) = match value {
        Value::None => (Vec::new(), 0, 1_u16),
        Value::Text(text) => {
            let wide = utf16(text);
            let characters = wide.len().checked_div(2).unwrap_or_default();
            (wide, characters, 1)
        }
        Value::Binary(binary) => (binary.to_vec(), binary.len(), 0),
    };
    let mut node = vec![0; 2];
    node.extend_from_slice(
        &u16::try_from(value_length)
            .map_err(|error| error.to_string())?
            .to_le_bytes(),
    );
    node.extend_from_slice(&text.to_le_bytes());
    node.extend_from_slice(&utf16(key));
    align(&mut node);
    node.extend_from_slice(&bytes);
    for child in children {
        align(&mut node);
        node.extend_from_slice(child);
    }
    let length = u16::try_from(node.len()).map_err(|error| error.to_string())?;
    if let Some(slot) = node.get_mut(..2) {
        slot.copy_from_slice(&length.to_le_bytes());
    }
    Ok(node)
}

/// `text` as Windows keeps it: UTF-16, little-endian, ending in a zero.
fn utf16(text: &str) -> Vec<u8> {
    text.encode_utf16()
        .chain([0])
        .flat_map(u16::to_le_bytes)
        .collect()
}

fn align(bytes: &mut Vec<u8>) {
    while !bytes.len().is_multiple_of(4) {
        bytes.push(0);
    }
}
