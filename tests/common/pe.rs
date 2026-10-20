//! Windows programs and libraries as small as the tests need them.

/// A 64-bit Windows library importing `dlls`, laid out as a linker lays one out: the headers, then
/// one section holding the import table and the names it points to.
///
/// # Panics
///
/// When the names are too many or too long for the section.
#[must_use]
pub fn windows_library(dlls: &[&str]) -> Vec<u8> {
    fn put(bytes: &mut Vec<u8>, at: usize, value: &[u8]) {
        bytes.resize(at, 0);
        bytes.extend_from_slice(value);
    }
    let mut bytes = Vec::new();
    put(&mut bytes, 0, b"MZ");
    put(&mut bytes, 0x3C, &64_u32.to_le_bytes());
    put(&mut bytes, 64, b"PE\0\0");
    put(&mut bytes, 68, &0x8664_u16.to_le_bytes());
    put(&mut bytes, 70, &1_u16.to_le_bytes());
    put(&mut bytes, 84, &240_u16.to_le_bytes());
    put(&mut bytes, 88, &0x20B_u16.to_le_bytes());
    put(&mut bytes, 196, &16_u32.to_le_bytes());
    put(&mut bytes, 208, &0x1000_u32.to_le_bytes());
    put(&mut bytes, 340, &0x1000_u32.to_le_bytes());
    put(&mut bytes, 344, &0x1000_u32.to_le_bytes());
    put(&mut bytes, 348, &0x200_u32.to_le_bytes());
    put(&mut bytes, 0x200, &[]);
    let descriptors = dlls.len().checked_add(1).unwrap().checked_mul(20).unwrap();
    let mut names = Vec::new();
    for dll in dlls {
        let name = descriptors.checked_add(names.len()).unwrap();
        let address = 0x1000_u32
            .checked_add(u32::try_from(name).unwrap())
            .unwrap();
        bytes.extend_from_slice(&[0; 12]);
        bytes.extend_from_slice(&address.to_le_bytes());
        bytes.extend_from_slice(&[0; 4]);
        names.extend_from_slice(dll.as_bytes());
        names.push(0);
    }
    bytes.extend_from_slice(&[0; 20]);
    bytes.extend(names);
    bytes
}

/// A 64-bit Windows library importing `dlls` as [`windows_library`] does, and delay-loading
/// `delayed`, whose table is further into the same section.
///
/// # Panics
///
/// When the names are too many or too long for the section.
#[must_use]
pub fn windows_library_delaying(dlls: &[&str], delayed: &[&str]) -> Vec<u8> {
    let mut bytes = windows_library(dlls);
    assert!(
        bytes.len() <= 0xA00,
        "the import table runs into the delay-load table"
    );
    drop(bytes.splice(304..308, 0x1800_u32.to_le_bytes()));
    bytes.resize(0xA00, 0);
    let descriptors = delayed
        .len()
        .checked_add(1)
        .unwrap()
        .checked_mul(32)
        .unwrap();
    let mut names = Vec::new();
    for dll in delayed {
        let name = descriptors.checked_add(names.len()).unwrap();
        let address = 0x1800_u32
            .checked_add(u32::try_from(name).unwrap())
            .unwrap();
        bytes.extend_from_slice(&1_u32.to_le_bytes());
        bytes.extend_from_slice(&address.to_le_bytes());
        bytes.extend_from_slice(&[0; 24]);
        names.extend_from_slice(dll.as_bytes());
        names.push(0);
    }
    bytes.extend_from_slice(&[0; 32]);
    bytes.extend(names);
    assert!(
        bytes.len() <= 0x1200,
        "the delay-load table runs out of the section"
    );
    bytes
}

/// A redistributable an app ticks, depot `DEPOT` of app 228980, among the app's depots.
const TICKED: &str = "\t\t\"DEPOT\"\n\t\t{\n\t\t\t\"config\"\n\t\t\t{\n\t\t\t\t\"oslist\"\t\t\
                      \"windows\"\n\t\t\t}\n\t\t\t\"depotfromapp\"\t\t\"228980\"\n\t\t\t\
                      \"sharedinstall\"\t\t\"1\"\n\t\t}\n";

/// App 1000's settings as `app_info_print` shows them, ticking each depot of app 228980 in
/// `ticked`.
///
/// Depot 1001 is for Windows and 1002 for Linux. Each redistributable is written as Steam shows
/// one an app ticks, copied from what Steam showed of app 8930 on 2026-10-19.
#[must_use]
pub fn settings_ticking(ticked: &[&str]) -> String {
    let redistributables = ticked
        .iter()
        .map(|depot| TICKED.replace("DEPOT", depot))
        .collect::<Vec<_>>()
        .concat();
    format!(
        "\"1000\"\n{{\n\t\"common\"\n\t{{\n\t\t\"name\"\t\t\"Spacewar\"\n\t}}\n\t\"depots\"\n\t{{\n\
         \t\t\"1001\"\n\t\t{{\n\t\t\t\"config\"\n\t\t\t{{\n\t\t\t\t\"oslist\"\t\t\"windows\"\n\
         \t\t\t}}\n\t\t}}\n\t\t\"1002\"\n\t\t{{\n\t\t\t\"config\"\n\t\t\t{{\n\t\t\t\t\"oslist\"\
         \t\t\"linux\"\n\t\t\t}}\n\t\t}}\n{redistributables}\t}}\n}}\n"
    )
}
