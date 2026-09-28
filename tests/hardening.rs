//! The exploit mitigations each system's loader enforces, read from the steamship program as
//! built, the way that loader reads them: a mitigation lost from the build fails here, in CI and
//! in every release build, rather than shipping unnoticed.
#![expect(
    clippy::unwrap_used,
    reason = "a test reports failure by panicking, its helpers included"
)]

use std::fs;

/// The program's bytes, read as its own little-endian headers lay them out.
struct Program(Vec<u8>);

impl Program {
    fn built() -> Self {
        Self(fs::read(env!("CARGO_BIN_EXE_steamship")).unwrap())
    }

    fn bytes(&self, at: usize, length: usize) -> &[u8] {
        self.0
            .get(at..at.checked_add(length).unwrap())
            .ok_or("past the end of the program")
            .unwrap()
    }

    #[cfg(any(windows, target_os = "linux"))]
    fn u16(&self, at: usize) -> u16 {
        u16::from_le_bytes(self.bytes(at, 2).try_into().unwrap())
    }

    fn u32(&self, at: usize) -> usize {
        usize::try_from(u32::from_le_bytes(self.bytes(at, 4).try_into().unwrap())).unwrap()
    }

    #[cfg(target_os = "linux")]
    fn u64(&self, at: usize) -> usize {
        usize::try_from(u64::from_le_bytes(self.bytes(at, 8).try_into().unwrap())).unwrap()
    }

    /// Where the 64-bit optional header starts.
    #[cfg(windows)]
    fn optional(&self) -> usize {
        let pe = self.u32(0x3C);
        assert_eq!(self.bytes(pe, 4), b"PE\0\0", "a Windows program");
        let optional = plus(pe, 24);
        assert_eq!(self.u16(optional), 0x20B, "a 64-bit program");
        optional
    }

    /// The file offset of a data directory's contents, and their size.
    #[cfg(windows)]
    fn directory(&self, index: usize) -> (usize, usize) {
        let entry = plus(plus(self.optional(), 112), index.checked_mul(8).unwrap());
        (self.offset(self.u32(entry)), self.u32(plus(entry, 4)))
    }

    /// The file offset of an address as loaded, found through the section it falls in.
    #[cfg(windows)]
    fn offset(&self, address: usize) -> usize {
        let pe = self.u32(0x3C);
        let sections = usize::from(self.u16(plus(pe, 6)));
        let first = plus(self.optional(), usize::from(self.u16(plus(pe, 20))));
        records(first, sections, 40)
            .find_map(|header| {
                let (size, start, raw) = (
                    self.u32(plus(header, 8)),
                    self.u32(plus(header, 12)),
                    self.u32(plus(header, 20)),
                );
                let inside = address.checked_sub(start).filter(|within| *within < size)?;
                raw.checked_add(inside)
            })
            .unwrap()
    }
}

/// `at` moved on by `by`, which must stay inside the address space.
const fn plus(at: usize, by: usize) -> usize {
    at.checked_add(by).unwrap()
}

/// The start of each of `count` records of `size` bytes, the first at `first`.
#[cfg(any(windows, target_os = "linux"))]
fn records(first: usize, count: usize, size: usize) -> impl Iterator<Item = usize> {
    (0..count).map(move |index| plus(first, index.checked_mul(size).unwrap()))
}

/// ASLR, DEP, Control Flow Guard, the hardware shadow stack on x86-64, imports from System32
/// alone, and the manifest Windows reads when it creates the process.
#[cfg(windows)]
#[test]
fn the_windows_program_has_its_mitigations_and_manifest() {
    let program = Program::built();
    let characteristics = program.u16(plus(program.optional(), 70));
    for (flag, name) in [
        (0x0020, "high-entropy ASLR"),
        (0x0040, "ASLR"),
        (0x0100, "DEP"),
        (0x4000, "Control Flow Guard"),
    ] {
        assert_ne!(characteristics & flag, 0, "{name}");
    }

    let (debug, size) = program.directory(6);
    let shadow_stack = records(debug, size.checked_div(28).unwrap(), 28)
        .filter(|&entry| program.u32(plus(entry, 12)) == 20)
        .any(|entry| program.u32(program.u32(plus(entry, 24))) & 1 != 0);
    assert_eq!(
        shadow_stack,
        cfg!(target_arch = "x86_64"),
        "compatible with the hardware shadow stack, which Arm lacks"
    );

    let (load_config, _) = program.directory(10);
    assert_eq!(
        program.u16(plus(load_config, 78)),
        0x800,
        "imports looked up in System32 alone"
    );

    let text = String::from_utf8_lossy(&program.0);
    for setting in [
        r#"<requestedExecutionLevel level="asInvoker" uiAccess="false"/>"#,
        ">UTF-8</activeCodePage>",
        ">true</longPathAware>",
    ] {
        assert!(text.contains(setting), "the manifest says {setting}");
    }
}

/// Loaded at a random address, its relocations made read-only before it runs and every symbol
/// bound up front so none are written later, and a stack that cannot be executed.
#[cfg(target_os = "linux")]
#[test]
fn the_linux_program_is_position_independent_with_full_relro_and_no_executable_stack() {
    const PT_DYNAMIC: usize = 2;
    const PT_GNU_STACK: usize = 0x6474_E551;
    const PT_GNU_RELRO: usize = 0x6474_E552;
    const DT_FLAGS: usize = 30;
    const DT_FLAGS_1: usize = 0x6FFF_FFFB;

    let program = Program::built();
    assert_eq!(program.bytes(0, 4), b"\x7FELF", "an ELF program");
    assert_eq!(program.u16(16), 3, "position-independent (ET_DYN)");
    let headers: Vec<usize> = records(
        program.u64(32),
        usize::from(program.u16(56)),
        usize::from(program.u16(54)),
    )
    .collect();
    let of_type = |kind: usize| {
        headers
            .iter()
            .copied()
            .find(|&header| program.u32(header) == kind)
    };

    let stack = of_type(PT_GNU_STACK).ok_or("no stack header").unwrap();
    assert_eq!(
        program.u32(plus(stack, 4)) & 1,
        0,
        "a stack that cannot be executed"
    );
    assert!(
        of_type(PT_GNU_RELRO).is_some(),
        "relocations made read-only"
    );

    let dynamic = of_type(PT_DYNAMIC).ok_or("no dynamic section").unwrap();
    let (start, size) = (
        program.u64(plus(dynamic, 8)),
        program.u64(plus(dynamic, 32)),
    );
    let entries: Vec<(usize, usize)> = records(start, size.checked_div(16).unwrap(), 16)
        .map(|entry| (program.u64(entry), program.u64(plus(entry, 8))))
        .collect();
    let flags = |tag: usize| {
        entries
            .iter()
            .filter(|&&(found, _)| found == tag)
            .fold(0, |all, &(_, value)| all | value)
    };
    assert!(
        flags(DT_FLAGS) & 0x8 != 0 || flags(DT_FLAGS_1) & 0x1 != 0,
        "every symbol bound before it runs"
    );
}

/// Loaded at a random address, no stack that can be executed, and signed, which Apple silicon
/// requires of anything it runs.
#[cfg(target_os = "macos")]
#[test]
fn the_macos_program_is_position_independent_signed_and_has_no_executable_stack() {
    const MH_PIE: usize = 0x0020_0000;
    const MH_ALLOW_STACK_EXECUTION: usize = 0x0002_0000;
    const LC_CODE_SIGNATURE: usize = 0x1D;

    let program = Program::built();
    assert_eq!(program.u32(0), 0xFEED_FACF, "a 64-bit Mach-O program");
    let flags = program.u32(24);
    assert_ne!(flags & MH_PIE, 0, "position-independent");
    assert_eq!(
        flags & MH_ALLOW_STACK_EXECUTION,
        0,
        "a stack that cannot be executed"
    );
    let mut command = 32;
    let signed = (0..program.u32(16)).any(|_| {
        let (kind, size) = (program.u32(command), program.u32(plus(command, 4)));
        command = plus(command, size);
        kind == LC_CODE_SIGNATURE
    });
    assert!(signed, "signed");
}
