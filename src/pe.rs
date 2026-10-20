//! Just enough of the PE format to list the DLLs a Windows program or library imports.
//!
//! Windows has to find every DLL of the import table before the file runs, in Windows itself or
//! beside the file. One of the delay-load table is looked for when the file first calls into it,
//! so the file starts without it and fails only then, unless it is written to do without. A DLL
//! loaded by name is in neither table, and is one the file has to find for itself.

use std::io::{self, Read, Seek, SeekFrom};

const DOS_MAGIC: [u8; 2] = *b"MZ";
const PE_MAGIC: [u8; 4] = *b"PE\0\0";
const PE32: u16 = 0x10B;
const PE32_PLUS: u16 = 0x20B;
/// The signature and file header, 24 bytes, then the longest optional header, 240: PE32+ with all
/// 16 data directories.
const HEADERS: u64 = 264;
const SECTION: u8 = 40;
/// Far more DLLs than any file imports, so that a table with no end is not read for ever.
const DESCRIPTOR_LIMIT: usize = 4096;
/// A DLL's name is a file's, which no path on Windows is longer than.
const NAME_LIMIT: u64 = 260;

/// The DLLs a Windows program or library names, each spelt as it names it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Imports {
    /// Those Windows loads before the file runs.
    pub loaded: Vec<String>,
    /// Those Windows loads when the file first calls into one.
    pub delayed: Vec<String>,
}

/// One of the two tables of the DLLs a file imports.
struct Table {
    /// Its entry's place among the data directories.
    directory: u8,
    /// How long each of its entries is, and where in one the DLL's name is.
    descriptor: u8,
    name_at: usize,
    called: &'static str,
}

const LOADED: Table = Table {
    directory: 1,
    descriptor: 20,
    name_at: 12,
    called: "import table",
};

const DELAYED: Table = Table {
    directory: 13,
    descriptor: 32,
    name_at: 4,
    called: "delay-load table",
};

/// The DLLs `file` imports, or `None` when it is not a Windows program or library.
///
/// # Errors
///
/// When reading fails, or, as `InvalidData`, when the file says it is a Windows program or library
/// but a table of its imports cannot be read.
pub fn imports<File>(file: &mut File) -> io::Result<Option<Imports>>
where
    File: Read + Seek,
{
    let dos = read_up_to(file, 64)?;
    if dos.get(..2) != Some(&DOS_MAGIC[..]) {
        return Ok(None);
    }
    let Some(at) = u32_at(&dos, 0x3C) else {
        return Ok(None);
    };
    let _: u64 = file.seek(SeekFrom::Start(u64::from(at)))?;
    let headers = read_up_to(file, HEADERS)?;
    if headers.get(..4) != Some(&PE_MAGIC[..]) {
        return Ok(None);
    }
    let (Some(sections), Some(optional), Some(kind)) = (
        u16_at(&headers, 6),
        u16_at(&headers, 20),
        u16_at(&headers, 24),
    ) else {
        return Err(invalid("its headers are cut short"));
    };
    // From the signature: where the count of data directories is, where they start, and where a
    // 32-bit file says the address it is loaded at, which Visual C++ 6 wrote into the delay-load
    // table's addresses.
    let (count_at, directories, base) = match kind {
        PE32 => (116, 120, u32_at(&headers, 52)),
        PE32_PLUS => (132, 136, Some(0)),
        _ => {
            return Err(invalid(
                "its optional header is of a kind Windows does not load",
            ));
        }
    };
    let (Some(count), Some(base)) = (u32_at(&headers, count_at), base) else {
        return Err(invalid("its optional header is cut short"));
    };
    // A table's address, or `None` when the optional header holds no entry for it or the entry is
    // empty.
    let entry = |table: &Table| {
        let entry_at = usize::from(table.directory)
            .saturating_mul(8)
            .saturating_add(directories);
        if count <= u32::from(table.directory)
            || usize::from(optional).saturating_add(24) < entry_at.saturating_add(8)
        {
            return Ok(None);
        }
        u32_at(&headers, entry_at)
            .map(|address| (address != 0).then_some(address))
            .ok_or_else(|| invalid("its optional header is cut short"))
    };
    let tables = [(&LOADED, entry(&LOADED)?), (&DELAYED, entry(&DELAYED)?)];
    if tables.iter().all(|(_, address)| address.is_none()) {
        return Ok(Some(Imports::default()));
    }
    let sections_at = u64::from(at)
        .saturating_add(24)
        .saturating_add(u64::from(optional));
    let _: u64 = file.seek(SeekFrom::Start(sections_at))?;
    let section_table = read_up_to(file, u64::from(sections).saturating_mul(SECTION.into()))?;
    let found: Vec<Section> = section_table
        .chunks_exact(SECTION.into())
        .filter_map(Section::of)
        .collect();
    if found.len() < usize::from(sections) {
        return Err(invalid("its section table is cut short"));
    }
    let mut imports = Imports::default();
    for (table, address) in tables {
        let Some(address) = address else {
            continue;
        };
        let names = named(file, &found, table, address, base)?;
        if table.directory == DELAYED.directory {
            imports.delayed = names;
        } else {
            imports.loaded = names;
        }
    }
    Ok(Some(imports))
}

/// The DLLs `table`, at `address`, names. A 32-bit file's delay-load entry whose first bit is
/// clear gives addresses in memory, from `base`, as Visual C++ 6 wrote them.
fn named<File>(
    file: &mut File,
    sections: &[Section],
    table: &Table,
    address: u32,
    base: u32,
) -> io::Result<Vec<String>>
where
    File: Read + Seek,
{
    let outside = || invalid(&format!("its {} points outside its sections", table.called));
    let offset = |into: u32| {
        sections
            .iter()
            .find_map(|section| section.offset(into))
            .ok_or_else(outside)
    };
    let _: u64 = file.seek(SeekFrom::Start(offset(address)?))?;
    let mut names = Vec::new();
    loop {
        let descriptor = read_up_to(file, table.descriptor.into())?;
        let Some(name) = u32_at(&descriptor, table.name_at)
            .filter(|_| descriptor.len() == usize::from(table.descriptor))
        else {
            return Err(invalid(&format!("its {} is cut short", table.called)));
        };
        if name == 0 {
            break;
        }
        if names.len() == DESCRIPTOR_LIMIT {
            return Err(invalid(&format!("its {} has no end", table.called)));
        }
        let in_memory = table.directory == DELAYED.directory
            && u32_at(&descriptor, 0).is_some_and(|attributes| attributes & 1 == 0);
        names.push(if in_memory {
            name.checked_sub(base).ok_or_else(outside)?
        } else {
            name
        });
    }
    let mut dlls = Vec::with_capacity(names.len());
    for name in names {
        let _: u64 = file.seek(SeekFrom::Start(offset(name)?))?;
        let mut bytes = read_up_to(file, NAME_LIMIT)?;
        let Some(end) = bytes.iter().position(|&byte| byte == 0) else {
            return Err(invalid("it names a DLL without ending the name"));
        };
        bytes.truncate(end);
        dlls.push(String::from_utf8_lossy(&bytes).into_owned());
    }
    Ok(dlls)
}

/// Where a section's bytes are, in memory and in the file.
struct Section {
    address: u32,
    size: u32,
    at: u32,
}

impl Section {
    fn of(bytes: &[u8]) -> Option<Self> {
        Some(Self {
            address: u32_at(bytes, 12)?,
            size: u32_at(bytes, 16)?,
            at: u32_at(bytes, 20)?,
        })
    }

    /// Where in the file `address` is, if this section's bytes in the file hold it.
    fn offset(&self, address: u32) -> Option<u64> {
        let into = address.checked_sub(self.address)?;
        u64::from(self.at)
            .checked_add(u64::from(into))
            .filter(|_| into < self.size)
    }
}

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

/// Up to `limit` bytes from where `file` stands, fewer only at its end.
fn read_up_to<File>(file: &mut File, limit: u64) -> io::Result<Vec<u8>>
where
    File: Read,
{
    let mut bytes = Vec::new();
    file.by_ref()
        .take(limit)
        .read_to_end(&mut bytes)
        .map(|_| bytes)
}

fn bytes<const N: usize>(bytes: &[u8], at: usize) -> Option<[u8; N]> {
    bytes.get(at..at.checked_add(N)?)?.try_into().ok()
}

fn u16_at(from: &[u8], at: usize) -> Option<u16> {
    bytes(from, at).map(u16::from_le_bytes)
}

fn u32_at(from: &[u8], at: usize) -> Option<u32> {
    bytes(from, at).map(u32::from_le_bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    /// Writes `value` at `at`, zero-filling up to it; fields are written in ascending order.
    fn put(bytes: &mut Vec<u8>, at: usize, value: &[u8]) {
        bytes.resize(at, 0);
        bytes.extend_from_slice(value);
    }

    /// Where the import table starts in the file, and in memory, and how long its section is.
    const TABLE: usize = 0x200;
    const ADDRESS: u32 = 0x1000;
    const LENGTH: u32 = 0x0010_0000;

    /// A Windows file of the optional header `kind` and length `optional`, with one section
    /// holding an import table at its start that names `dlls`, and as many data directories as
    /// `count` says.
    fn pe(kind: u16, optional: u16, count: u32, dlls: &[&str]) -> Vec<u8> {
        let (count_at, entry_at) = if kind == PE32 { (180, 192) } else { (196, 208) };
        let mut bytes = Vec::new();
        put(&mut bytes, 0, &DOS_MAGIC);
        put(&mut bytes, 0x3C, &64_u32.to_le_bytes());
        put(&mut bytes, 64, &PE_MAGIC);
        put(&mut bytes, 68, &0x8664_u16.to_le_bytes());
        put(&mut bytes, 70, &1_u16.to_le_bytes());
        put(&mut bytes, 84, &optional.to_le_bytes());
        put(&mut bytes, 88, &kind.to_le_bytes());
        put(&mut bytes, count_at, &count.to_le_bytes());
        put(&mut bytes, entry_at, &ADDRESS.to_le_bytes());
        let section = 88_usize.checked_add(optional.into()).unwrap();
        put(&mut bytes, section, &[0; 12]);
        bytes.extend_from_slice(&ADDRESS.to_le_bytes());
        bytes.extend_from_slice(&LENGTH.to_le_bytes());
        bytes.extend_from_slice(&u32::try_from(TABLE).unwrap().to_le_bytes());
        put(&mut bytes, TABLE, &import_table(dlls));
        bytes
    }

    /// An import table at `ADDRESS` naming `dlls`, followed by their names.
    fn import_table(dlls: &[&str]) -> Vec<u8> {
        let descriptors = dlls.len().checked_add(1).unwrap().checked_mul(20).unwrap();
        let mut table = Vec::new();
        let mut names = Vec::new();
        for dll in dlls {
            let name = descriptors.checked_add(names.len()).unwrap();
            let address = ADDRESS.checked_add(u32::try_from(name).unwrap()).unwrap();
            table.extend_from_slice(&[0; 12]);
            table.extend_from_slice(&address.to_le_bytes());
            table.extend_from_slice(&[0; 4]);
            names.extend_from_slice(dll.as_bytes());
            names.push(0);
        }
        table.extend_from_slice(&[0; 20]);
        table.extend(names);
        table
    }

    fn pe64(dlls: &[&str]) -> Vec<u8> {
        pe(PE32_PLUS, 240, 16, dlls)
    }

    fn read(bytes: Vec<u8>) -> Option<Vec<String>> {
        imports(&mut Cursor::new(bytes))
            .unwrap()
            .map(|found| found.loaded)
    }

    /// Where the delay-load table starts in the file, and in memory, past the import table.
    const DELAY_TABLE: usize = 0x0A00;
    const DELAY_ADDRESS: u32 = 0x1800;

    /// `bytes`, a file `pe` made with the optional header `kind`, with a delay-load table naming
    /// `dlls` and each entry's first bit as `attributes` says, its addresses given from `base`.
    fn delaying(bytes: Vec<u8>, kind: u16, dlls: &[&str], attributes: u32, base: u32) -> Vec<u8> {
        let entry_at = if kind == PE32 { 64 + 224 } else { 64 + 240 };
        let mut bytes = altered(bytes, entry_at, &DELAY_ADDRESS.to_le_bytes());
        let descriptors = dlls.len().checked_add(1).unwrap().checked_mul(32).unwrap();
        let mut table = Vec::new();
        let mut names = Vec::new();
        for dll in dlls {
            let name = descriptors.checked_add(names.len()).unwrap();
            let address = DELAY_ADDRESS
                .checked_add(u32::try_from(name).unwrap())
                .and_then(|address| address.checked_add(base))
                .unwrap();
            table.extend_from_slice(&attributes.to_le_bytes());
            table.extend_from_slice(&address.to_le_bytes());
            table.extend_from_slice(&[0; 24]);
            names.extend_from_slice(dll.as_bytes());
            names.push(0);
        }
        table.extend_from_slice(&[0; 32]);
        table.extend(names);
        put(&mut bytes, DELAY_TABLE, &table);
        bytes
    }

    fn delayed(bytes: Vec<u8>) -> Imports {
        imports(&mut Cursor::new(bytes)).unwrap().unwrap()
    }

    fn refused(bytes: Vec<u8>) -> String {
        let error = imports(&mut Cursor::new(bytes)).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        error.to_string()
    }

    /// `bytes` with the bytes at `at` replaced by `value`.
    fn altered(mut bytes: Vec<u8>, at: usize, value: &[u8]) -> Vec<u8> {
        let tail = bytes.split_off(at);
        bytes.extend_from_slice(value);
        bytes.extend(tail.into_iter().skip(value.len()));
        bytes
    }

    #[test]
    fn a_64_bit_file_lists_the_dlls_it_imports_as_it_spells_them() {
        assert_eq!(
            read(pe64(&["KERNEL32.dll", "VCRUNTIME140.dll"])),
            Some(vec![
                "KERNEL32.dll".to_owned(),
                "VCRUNTIME140.dll".to_owned()
            ])
        );
    }

    #[test]
    fn a_32_bit_file_lists_them_too() {
        assert_eq!(
            read(pe(PE32, 224, 16, &["msvcrt.dll"])),
            Some(vec!["msvcrt.dll".to_owned()])
        );
    }

    #[test]
    fn the_dlls_a_file_delay_loads_are_listed_apart_from_those_it_imports() {
        let both = delaying(
            pe64(&["KERNEL32.dll"]),
            PE32_PLUS,
            &["VCRUNTIME140.dll", "nvapi64.dll"],
            1,
            0,
        );
        assert_eq!(
            delayed(both),
            Imports {
                loaded: vec!["KERNEL32.dll".to_owned()],
                delayed: vec!["VCRUNTIME140.dll".to_owned(), "nvapi64.dll".to_owned()],
            }
        );
        let alone = altered(
            delaying(pe64(&["KERNEL32.dll"]), PE32_PLUS, &["d3d12.dll"], 1, 0),
            208,
            &0_u32.to_le_bytes(),
        );
        assert_eq!(
            delayed(alone),
            Imports {
                loaded: Vec::new(),
                delayed: vec!["d3d12.dll".to_owned()],
            },
            "a delay-load table with no import table"
        );
    }

    #[test]
    fn a_32_bit_file_s_delay_loads_are_read_from_either_kind_of_address() {
        let file = |attributes: u32, base: u32| {
            let bytes = altered(
                pe(PE32, 224, 16, &["msvcrt.dll"]),
                116,
                &0x0040_0000_u32.to_le_bytes(),
            );
            delaying(bytes, PE32, &["xinput1_4.dll"], attributes, base)
        };
        let xinput = vec!["xinput1_4.dll".to_owned()];
        assert_eq!(delayed(file(1, 0)).delayed, xinput, "from the base");
        assert_eq!(
            delayed(file(0, 0x0040_0000)).delayed,
            xinput,
            "in memory, as Visual C++ 6 wrote them"
        );
        assert_eq!(
            refused(file(0, 0)),
            "its delay-load table points outside its sections",
            "an address in memory below the base"
        );
    }

    #[test]
    fn a_delay_load_table_is_read_only_where_the_optional_header_holds_its_entry() {
        let file = |optional: u16, count: u32| {
            delaying(
                pe(PE32_PLUS, optional, count, &["user32.dll"]),
                PE32_PLUS,
                &["d3d12.dll"],
                1,
                0,
            )
        };
        let d3d12 = vec!["d3d12.dll".to_owned()];
        assert_eq!(delayed(file(224, 14)).delayed, d3d12, "just held");
        assert_eq!(delayed(file(223, 16)).delayed, Vec::<String>::new());
        assert_eq!(delayed(file(240, 13)).delayed, Vec::<String>::new());
    }

    #[test]
    fn a_delay_load_table_that_cannot_be_read_is_refused() {
        let file = || delaying(pe64(&["user32.dll"]), PE32_PLUS, &["d3d12.dll"], 1, 0);
        let mut cut = file();
        cut.truncate(DELAY_TABLE + 40);
        assert_eq!(refused(cut), "its delay-load table is cut short");
        assert_eq!(
            refused(altered(file(), 304, &(ADDRESS + LENGTH).to_le_bytes())),
            "its delay-load table points outside its sections"
        );
        let mut unended = file();
        let _last: Option<u8> = unended.pop();
        assert_eq!(refused(unended), "it names a DLL without ending the name");
        let many = vec!["a.dll"; 4097];
        assert_eq!(
            refused(delaying(pe64(&[]), PE32_PLUS, &many, 1, 0)),
            "its delay-load table has no end"
        );
    }

    #[test]
    fn a_file_with_no_import_table_imports_nothing() {
        assert_eq!(read(pe64(&[])), Some(Vec::new()), "an empty table");
        assert_eq!(
            read(altered(pe64(&["user32.dll"]), 208, &0_u32.to_le_bytes())),
            Some(Vec::new()),
            "no table"
        );
        assert_eq!(
            read(pe(PE32_PLUS, 240, 1, &["user32.dll"])),
            Some(Vec::new()),
            "one data directory, which is not the import table's"
        );
        assert_eq!(
            read(pe(PE32_PLUS, 120, 16, &["user32.dll"])),
            Some(Vec::new()),
            "an optional header too short to hold the table's entry"
        );
    }

    #[test]
    fn the_least_that_holds_the_import_table_is_enough() {
        let user32 = Some(vec!["user32.dll".to_owned()]);
        assert_eq!(read(pe(PE32_PLUS, 240, 2, &["user32.dll"])), user32);
        assert_eq!(read(pe(PE32_PLUS, 128, 16, &["user32.dll"])), user32);
        assert_eq!(read(pe(PE32, 112, 16, &["user32.dll"])), user32);
    }

    #[test]
    fn anything_but_a_windows_file_is_not_one() {
        assert_eq!(read(Vec::new()), None, "nothing");
        assert_eq!(read(b"\x7fELF a Linux program".to_vec()), None, "ELF");
        assert_eq!(read(b"MZ, then nothing".to_vec()), None, "cut short");
        let mut dos = pe64(&["user32.dll"]);
        dos.truncate(64);
        assert_eq!(read(dos), None, "a DOS program alone");
        assert_eq!(
            read(altered(pe64(&["user32.dll"]), 64, b"NE\0\0")),
            None,
            "a 16-bit program"
        );
        assert_eq!(
            read(altered(
                pe64(&["user32.dll"]),
                0x3C,
                &u32::MAX.to_le_bytes()
            )),
            None,
            "a signature past the end of the file"
        );
    }

    #[test]
    fn a_windows_file_whose_import_table_cannot_be_read_is_refused() {
        let file = || pe64(&["user32.dll"]);
        let cut = |length: usize| {
            let mut bytes = file();
            bytes.truncate(length);
            bytes
        };
        assert_eq!(refused(cut(80)), "its headers are cut short");
        assert_eq!(
            refused(altered(file(), 88, &0x107_u16.to_le_bytes())),
            "its optional header is of a kind Windows does not load"
        );
        assert_eq!(refused(cut(200)), "its optional header is cut short");
        assert_eq!(
            refused(altered(file(), 70, &u16::MAX.to_le_bytes())),
            "its section table is cut short"
        );
        for length in [TABLE + 30, TABLE + 37] {
            assert_eq!(
                refused(cut(length)),
                "its import table is cut short",
                "{length}"
            );
        }
        assert_eq!(
            refused(altered(
                file(),
                TABLE + 12,
                &(ADDRESS + LENGTH).to_le_bytes()
            )),
            "its import table points outside its sections"
        );
        let mut unended = file();
        let _last: Option<u8> = unended.pop();
        assert_eq!(refused(unended), "it names a DLL without ending the name");
    }

    #[test]
    fn an_import_table_is_found_only_inside_a_section_in_the_file() {
        for (what, address) in [
            ("just before the section", ADDRESS - 1),
            (
                "just past the section's bytes in the file",
                ADDRESS + LENGTH,
            ),
        ] {
            assert_eq!(
                refused(altered(pe64(&["user32.dll"]), 208, &address.to_le_bytes())),
                "its import table points outside its sections",
                "{what}"
            );
        }
    }

    /// Written out rather than taken from the constants, so that the limits themselves are under
    /// test.
    #[test]
    fn a_name_is_read_as_long_as_a_path_can_be_and_a_table_to_4096_dlls() {
        let longest = "a".repeat(259);
        assert_eq!(read(pe64(&[&longest])), Some(vec![longest.clone()]));
        assert_eq!(
            refused(pe64(&[&format!("{longest}a")])),
            "it names a DLL without ending the name"
        );
        let many = vec!["a.dll"; 4096];
        assert_eq!(read(pe64(&many)).map(|dlls| dlls.len()), Some(4096));
        let too_many = vec!["a.dll"; 4097];
        assert_eq!(refused(pe64(&too_many)), "its import table has no end");
    }
}
