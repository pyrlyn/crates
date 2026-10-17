//! Artifact format detection by magic bytes, independent of file extension.
//!
//! `.so`, `.so.1.2`, `.dylib`, `.bundle`, `.dll`, `.a`, `.lib` and `.wasm` are
//! all recognised by content, and a shared library built for another OS or CPU
//! is rejected with a precise reason before `dlopen` ever sees it.

use std::fmt;
use std::io::Read;
use std::path::Path;

use plugwright_core::error::{PluginError, Result, io_err};

/// Bytes read from the start of a file; enough for every header we parse
/// except the PE header, which is located through `e_lfanew`.
const SNIFF_LEN: usize = 4096;

/// What a file is, judged by its contents.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Format {
    /// ELF image (`.so`, `.so.N`).
    Elf {
        kind: ElfKind,
        arch: Option<&'static str>,
    },
    /// Mach-O image (`.dylib`, `.bundle`).
    MachO {
        kind: MachKind,
        arch: Option<&'static str>,
    },
    /// Universal (fat) Mach-O with one slice per architecture.
    MachOFat {
        arches: Vec<Option<&'static str>>,
    },
    /// PE image (`.dll`, `.exe`).
    Pe {
        dll: bool,
        arch: Option<&'static str>,
    },
    /// `ar` archive: a static library (`.a`, or `.lib` on Windows).
    Archive,
    /// Core Wasm module (`.wasm`), binary format version 1.
    Wasm,
    /// Wasm component (component model), which plugwright does not run.
    WasmComponent,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ElfKind {
    Shared,
    Executable,
    Relocatable,
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MachKind {
    Dylib,
    Bundle,
    Executable,
    Object,
    Other,
}

impl fmt::Display for Format {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let arch = |a: &Option<&str>| a.unwrap_or("unknown-arch").to_owned();
        match self {
            Format::Elf { kind, arch: a } => write!(f, "{} ELF {kind:?}", arch(a)),
            Format::MachO { kind, arch: a } => write!(f, "{} Mach-O {kind:?}", arch(a)),
            Format::MachOFat { arches } => {
                write!(f, "universal Mach-O ({})", arches.iter().map(arch).collect::<Vec<_>>().join(", "))
            }
            Format::Pe { dll, arch: a } => write!(f, "{} PE {}", arch(a), if *dll { "DLL" } else { "executable" }),
            Format::Archive => f.write_str("static library (ar archive)"),
            Format::Wasm => f.write_str("Wasm module"),
            Format::WasmComponent => f.write_str("Wasm component"),
            Format::Unknown => f.write_str("unknown format"),
        }
    }
}

impl Format {
    /// A native shared library this process can `dlopen`.
    pub fn is_loadable_here(&self) -> bool {
        let here = Some(std::env::consts::ARCH);
        match self {
            // `ET_DYN` also covers PIE executables; `dlopen` rejects those itself.
            Format::Elf { kind: ElfKind::Shared, arch } => {
                cfg!(all(unix, not(target_vendor = "apple"))) && *arch == here
            }
            Format::MachO { kind: MachKind::Dylib | MachKind::Bundle, arch } => {
                cfg!(target_vendor = "apple") && *arch == here
            }
            Format::MachOFat { arches } => cfg!(target_vendor = "apple") && arches.contains(&here),
            Format::Pe { dll: true, arch } => cfg!(windows) && *arch == here,
            _ => false,
        }
    }
}

/// Detect the format of an in-memory prefix of a file (at least the first few
/// hundred bytes; PE needs the bytes up to its `PE\0\0` signature).
pub fn detect(b: &[u8]) -> Format {
    if b.starts_with(b"\x7fELF") {
        return elf(b);
    }
    if b.starts_with(b"!<arch>\n") {
        return Format::Archive;
    }
    if b.starts_with(b"\0asm") && b.len() >= 8 {
        // Core modules are version 1 in layer 0; components have a version of
        // their own in layer 1. Anything else is neither.
        return match (read_u16(b, 4, false), read_u16(b, 6, false)) {
            (Some(1), Some(0)) => Format::Wasm,
            (_, Some(1)) => Format::WasmComponent,
            _ => Format::Unknown,
        };
    }
    if let Some(0xFEED_FACF | 0xFEED_FACE) = read_u32(b, 0, false) {
        return macho(b);
    }
    // Java class files share 0xCAFEBABE; their "count" is a class version >= 45.
    if let Some(0xCAFE_BABE | 0xCAFE_BABF) = read_u32(b, 0, true)
        && read_u32(b, 4, true).is_some_and(|n| (1..=32).contains(&n))
    {
        return fat(b);
    }
    if b.starts_with(b"MZ") {
        return pe(b);
    }
    Format::Unknown
}

/// Read the head of `path` and detect its format.
pub fn sniff(path: &Path) -> Result<Format> {
    let mut f = std::fs::File::open(path).map_err(io_err(path))?;
    let mut buf = Vec::with_capacity(SNIFF_LEN);
    f.by_ref().take(SNIFF_LEN as u64).read_to_end(&mut buf).map_err(io_err(path))?;
    Ok(detect(&buf))
}

/// Fail with a precise reason unless `path` is a shared library for this host.
pub fn check_loadable(path: &Path) -> Result<Format> {
    let format = sniff(path)?;
    if format.is_loadable_here() {
        return Ok(format);
    }
    let host = format!("{}-{}", std::env::consts::ARCH, std::env::consts::OS);
    let reason = match &format {
        Format::Archive => "static library; it has to be relinked into a shared library first".to_owned(),
        Format::Wasm | Format::WasmComponent => format!("{format}; load it with kind = \"wasm\""),
        Format::Unknown => "not a shared library".to_owned(),
        other => format!("{other} cannot be loaded on {host}"),
    };
    Err(PluginError::NotAPlugin { path: path.to_owned(), reason })
}

/// Conventional shared-library extension for a format, used for shadow
/// copies (Windows appends `.dll` to names without an extension).
pub fn shared_ext(format: &Format) -> &'static str {
    match format {
        Format::Elf { .. } => "so",
        Format::MachO { .. } | Format::MachOFat { .. } => "dylib",
        Format::Pe { .. } => "dll",
        _ => std::env::consts::DLL_EXTENSION,
    }
}

fn read_u16(b: &[u8], at: usize, big: bool) -> Option<u16> {
    let s: [u8; 2] = b.get(at..at + 2)?.try_into().ok()?;
    Some(if big { u16::from_be_bytes(s) } else { u16::from_le_bytes(s) })
}

fn read_u32(b: &[u8], at: usize, big: bool) -> Option<u32> {
    let s: [u8; 4] = b.get(at..at + 4)?.try_into().ok()?;
    Some(if big { u32::from_be_bytes(s) } else { u32::from_le_bytes(s) })
}

fn elf(b: &[u8]) -> Format {
    // e_ident[EI_DATA]: 1 = little endian, 2 = big endian.
    let big = b.get(5) == Some(&2);
    let kind = match read_u16(b, 16, big) {
        Some(3) => ElfKind::Shared, // ET_DYN (also PIE executables)
        Some(2) => ElfKind::Executable,
        Some(1) => ElfKind::Relocatable,
        _ => ElfKind::Other,
    };
    let class64 = b.get(4) == Some(&2);
    let arch = match (read_u16(b, 18, big), class64) {
        (Some(0x3E), true) => Some("x86_64"),
        (Some(0x03), false) => Some("x86"),
        (Some(0xB7), true) => Some("aarch64"),
        (Some(0x28), false) => Some("arm"),
        (Some(0xF3), true) => Some("riscv64"),
        (Some(0x15), true) => Some("powerpc64"),
        (Some(0x16), true) => Some("s390x"),
        (Some(0x102), true) => Some("loongarch64"),
        _ => None,
    };
    Format::Elf { kind, arch }
}

fn mach_arch(cputype: u32) -> Option<&'static str> {
    match cputype {
        0x0100_0007 => Some("x86_64"),
        0x0100_000C => Some("aarch64"),
        0x0000_0007 => Some("x86"),
        0x0000_000C => Some("arm"),
        _ => None,
    }
}

fn macho(b: &[u8]) -> Format {
    let arch = read_u32(b, 4, false).and_then(mach_arch);
    let kind = match read_u32(b, 12, false) {
        Some(6) => MachKind::Dylib,
        Some(8) => MachKind::Bundle,
        Some(2) => MachKind::Executable,
        Some(1) => MachKind::Object,
        _ => MachKind::Other,
    };
    Format::MachO { kind, arch }
}

fn fat(b: &[u8]) -> Format {
    let wide = read_u32(b, 0, true) == Some(0xCAFE_BABF);
    let n = read_u32(b, 4, true).unwrap_or(0) as usize;
    let stride = if wide { 32 } else { 20 };
    let arches = (0..n).map_while(|i| read_u32(b, 8 + i * stride, true)).map(mach_arch).collect();
    Format::MachOFat { arches }
}

fn pe(b: &[u8]) -> Format {
    // Bounded by the buffer so `at + 22` cannot overflow a 32-bit `usize`.
    let Some(at) = read_u32(b, 0x3C, false).map(|v| v as usize).filter(|&at| at < b.len()) else {
        return Format::Unknown;
    };
    if b.get(at..at + 4) != Some(b"PE\0\0") {
        return Format::Unknown;
    }
    let arch = match read_u16(b, at + 4, false) {
        Some(0x8664) => Some("x86_64"),
        Some(0xAA64) => Some("aarch64"),
        Some(0x014C) => Some("x86"),
        _ => None,
    };
    // IMAGE_FILE_DLL in the COFF characteristics.
    let dll = read_u16(b, at + 22, false).is_some_and(|c| c & 0x2000 != 0);
    Format::Pe { dll, arch }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ELF_OS: bool = cfg!(all(unix, not(target_vendor = "apple")));
    const APPLE: bool = cfg!(target_vendor = "apple");

    /// `(e_machine, Mach-O cputype, PE Machine)` of the host, if the detector knows its CPU.
    fn host_ids() -> Option<(u16, u32, u16)> {
        match std::env::consts::ARCH {
            "x86_64" => Some((0x3E, 0x0100_0007, 0x8664)),
            "aarch64" => Some((0xB7, 0x0100_000C, 0xAA64)),
            _ => None,
        }
    }

    /// Ids of a CPU the host is not.
    fn foreign_ids() -> (u16, u32, u16) {
        if std::env::consts::ARCH == "x86_64" { (0xB7, 0x0100_000C, 0xAA64) } else { (0x3E, 0x0100_0007, 0x8664) }
    }

    fn elf_raw(class64: bool, big: bool, kind: u16, machine: u16) -> Vec<u8> {
        let mut h = vec![0u8; 64];
        h[..4].copy_from_slice(b"\x7fELF");
        h[4] = if class64 { 2 } else { 1 };
        h[5] = if big { 2 } else { 1 };
        let (kind, machine) =
            if big { (kind.to_be_bytes(), machine.to_be_bytes()) } else { (kind.to_le_bytes(), machine.to_le_bytes()) };
        h[16..18].copy_from_slice(&kind);
        h[18..20].copy_from_slice(&machine);
        h
    }

    fn elf_header(machine: u16, kind: u16) -> Vec<u8> {
        elf_raw(true, false, kind, machine)
    }

    fn macho_header(magic: u32, cputype: u32, filetype: u32) -> Vec<u8> {
        let mut h = vec![0u8; 32];
        h[..4].copy_from_slice(&magic.to_le_bytes());
        h[4..8].copy_from_slice(&cputype.to_le_bytes());
        h[12..16].copy_from_slice(&filetype.to_le_bytes());
        h
    }

    /// Big-endian `fat_header` with one `fat_arch` (20 bytes) or `fat_arch_64`
    /// (32 bytes) per CPU.
    fn fat_header(wide: bool, cputypes: &[u32]) -> Vec<u8> {
        let stride = if wide { 32 } else { 20 };
        let mut h = vec![0u8; 8 + stride * cputypes.len()];
        let magic: u32 = if wide { 0xCAFE_BABF } else { 0xCAFE_BABE };
        h[..4].copy_from_slice(&magic.to_be_bytes());
        h[4..8].copy_from_slice(&(cputypes.len() as u32).to_be_bytes());
        for (i, cpu) in cputypes.iter().enumerate() {
            h[8 + i * stride..12 + i * stride].copy_from_slice(&cpu.to_be_bytes());
        }
        h
    }

    fn pe_header(machine: u16, characteristics: u16) -> Vec<u8> {
        let mut h = vec![0u8; 0x100];
        h[..2].copy_from_slice(b"MZ");
        h[0x3C..0x40].copy_from_slice(&0x80u32.to_le_bytes());
        h[0x80..0x84].copy_from_slice(b"PE\0\0");
        h[0x84..0x86].copy_from_slice(&machine.to_le_bytes());
        h[0x96..0x98].copy_from_slice(&characteristics.to_le_bytes());
        h
    }

    #[test]
    fn detects_every_format() {
        assert_eq!(detect(&elf_header(0xB7, 3)), Format::Elf { kind: ElfKind::Shared, arch: Some("aarch64") });
        assert_eq!(detect(&elf_header(0x3E, 2)), Format::Elf { kind: ElfKind::Executable, arch: Some("x86_64") });
        let mut m = vec![0u8; 32];
        m[..4].copy_from_slice(&0xFEED_FACFu32.to_le_bytes());
        m[4..8].copy_from_slice(&0x0100_000Cu32.to_le_bytes());
        m[12..16].copy_from_slice(&6u32.to_le_bytes());
        assert_eq!(detect(&m), Format::MachO { kind: MachKind::Dylib, arch: Some("aarch64") });
        let mut fat = vec![0u8; 48];
        fat[..4].copy_from_slice(&0xCAFE_BABEu32.to_be_bytes());
        fat[4..8].copy_from_slice(&2u32.to_be_bytes());
        fat[8..12].copy_from_slice(&0x0100_0007u32.to_be_bytes());
        fat[28..32].copy_from_slice(&0x0100_000Cu32.to_be_bytes());
        assert_eq!(detect(&fat), Format::MachOFat { arches: vec![Some("x86_64"), Some("aarch64")] });
        let mut class = fat.clone();
        class[4..8].copy_from_slice(&52u32.to_be_bytes()); // Java 8 class file
        assert_eq!(detect(&class), Format::Unknown);
        assert_eq!(detect(&pe_header(0x8664, 0x2022)), Format::Pe { dll: true, arch: Some("x86_64") });
        assert_eq!(detect(&pe_header(0xAA64, 0x0022)), Format::Pe { dll: false, arch: Some("aarch64") });
        assert_eq!(detect(b"!<arch>\n/               0"), Format::Archive);
        assert_eq!(detect(b"\0asm\x01\0\0\0"), Format::Wasm);
        assert_eq!(detect(b"\0asm\x0d\0\x01\0"), Format::WasmComponent);
        assert_eq!(detect(b"MZ"), Format::Unknown);
        assert_eq!(detect(b"#!/bin/sh\n"), Format::Unknown);
    }

    #[test]
    fn foreign_images_are_not_loadable() {
        let other = if std::env::consts::ARCH == "x86_64" { 0xB7 } else { 0x3E };
        assert!(!detect(&elf_header(other, 3)).is_loadable_here());
        assert!(!detect(&pe_header(0x014C, 0x2000)).is_loadable_here() || cfg!(all(windows, target_arch = "x86")));
        assert!(!Format::Archive.is_loadable_here());
    }

    #[test]
    fn this_test_binary_is_recognised() {
        let exe = std::env::current_exe().unwrap();
        let f = sniff(&exe).unwrap();
        assert!(!matches!(f, Format::Unknown), "{f}");
        // An executable, not a library: refused with a reason. (A Linux PIE is
        // `ET_DYN`, indistinguishable from a library by its header alone.)
        if !cfg!(target_os = "linux") {
            let err = check_loadable(&exe).unwrap_err().to_string();
            assert!(err.contains("cannot be loaded"), "{err}");
        }
    }

    #[test]
    fn elf_kind_follows_e_type() {
        let cases = [
            (3, ElfKind::Shared),
            (2, ElfKind::Executable),
            (1, ElfKind::Relocatable),
            (4, ElfKind::Other), // ET_CORE
            (0, ElfKind::Other),
        ];
        for (e_type, kind) in cases {
            assert_eq!(detect(&elf_header(0x3E, e_type)), Format::Elf { kind, arch: Some("x86_64") }, "{e_type}");
        }
    }

    #[test]
    fn elf_big_endian_fields_are_byte_swapped() {
        assert_eq!(
            detect(&elf_raw(true, true, 3, 0x15)),
            Format::Elf { kind: ElfKind::Shared, arch: Some("powerpc64") }
        );
        assert_eq!(
            detect(&elf_raw(true, true, 2, 0x16)),
            Format::Elf { kind: ElfKind::Executable, arch: Some("s390x") }
        );
        // Little-endian fields under a big-endian EI_DATA read as garbage.
        let mut h = elf_header(0xB7, 3);
        h[5] = 2;
        assert_eq!(detect(&h), Format::Elf { kind: ElfKind::Other, arch: None });
    }

    #[test]
    fn elf_machine_is_matched_together_with_its_class() {
        let cases = [
            (true, 0x3E, Some("x86_64")),
            (true, 0xB7, Some("aarch64")),
            (true, 0xF3, Some("riscv64")),
            (true, 0x102, Some("loongarch64")),
            (false, 0x03, Some("x86")),
            (false, 0x28, Some("arm")),
            // x32 and ILP32 images are not the 64-bit host ABI, and vice versa.
            (false, 0x3E, None),
            (false, 0xB7, None),
            (true, 0x03, None),
            (true, 0x08, None), // MIPS is not mapped
        ];
        for (class64, machine, arch) in cases {
            let f = detect(&elf_raw(class64, false, 3, machine));
            assert_eq!(f, Format::Elf { kind: ElfKind::Shared, arch }, "{machine:#x} class64={class64}");
        }
    }

    #[test]
    fn only_host_elf_shared_objects_are_loadable() {
        let (foreign, ..) = foreign_ids();
        assert!(!detect(&elf_header(foreign, 3)).is_loadable_here());
        let Some((host, ..)) = host_ids() else { return };
        assert_eq!(detect(&elf_header(host, 3)).is_loadable_here(), ELF_OS);
        assert!(!detect(&elf_header(host, 2)).is_loadable_here());
        assert!(!detect(&elf_header(host, 1)).is_loadable_here());
        assert!(!detect(&elf_raw(false, false, 3, host)).is_loadable_here());
    }

    #[test]
    fn macho_kind_follows_filetype_for_both_word_sizes() {
        let cases = [
            (6, MachKind::Dylib),
            (8, MachKind::Bundle),
            (2, MachKind::Executable),
            (1, MachKind::Object),
            (4, MachKind::Other),   // MH_CORE
            (0xB, MachKind::Other), // MH_KEXT_BUNDLE
        ];
        for magic in [0xFEED_FACF, 0xFEED_FACE] {
            for (filetype, kind) in cases {
                let f = detect(&macho_header(magic, 0x0100_0007, filetype));
                assert_eq!(f, Format::MachO { kind, arch: Some("x86_64") }, "{magic:#x} {filetype}");
            }
        }
    }

    #[test]
    fn macho_cputype_maps_to_rust_arch() {
        let cases = [
            (0x0100_0007, Some("x86_64")),
            (0x0100_000C, Some("aarch64")),
            (0x0000_0007, Some("x86")),
            (0x0000_000C, Some("arm")),
            (0x0000_0012, None), // PowerPC
            (0x0200_000C, None), // arm64_32
        ];
        for (cpu, arch) in cases {
            assert_eq!(detect(&macho_header(0xFEED_FACF, cpu, 6)), Format::MachO { kind: MachKind::Dylib, arch });
        }
    }

    #[test]
    fn only_host_macho_dylibs_and_bundles_are_loadable() {
        let (_, foreign, _) = foreign_ids();
        assert!(!detect(&macho_header(0xFEED_FACF, foreign, 6)).is_loadable_here());
        let Some((_, host, _)) = host_ids() else { return };
        assert_eq!(detect(&macho_header(0xFEED_FACF, host, 6)).is_loadable_here(), APPLE);
        assert_eq!(detect(&macho_header(0xFEED_FACF, host, 8)).is_loadable_here(), APPLE);
        assert!(!detect(&macho_header(0xFEED_FACF, host, 2)).is_loadable_here());
        assert!(!detect(&macho_header(0xFEED_FACF, host, 1)).is_loadable_here());
    }

    #[test]
    fn big_endian_macho_is_never_loadable() {
        // Only little-endian Mach-O is parsed; a big-endian header (PowerPC era)
        // must still never pass as loadable, whatever its cputype.
        let (_, cpu, _) = host_ids().unwrap_or(foreign_ids());
        let mut h = vec![0u8; 32];
        h[..4].copy_from_slice(&0xFEED_FACFu32.to_be_bytes());
        h[4..8].copy_from_slice(&cpu.to_be_bytes());
        h[12..16].copy_from_slice(&6u32.to_be_bytes());
        assert!(!detect(&h).is_loadable_here());
    }

    #[test]
    fn fat64_slices_use_the_wide_stride() {
        let f = detect(&fat_header(true, &[0x0100_0007, 0x0100_000C, 0x12]));
        assert_eq!(f, Format::MachOFat { arches: vec![Some("x86_64"), Some("aarch64"), None] });
    }

    #[test]
    fn fat_binary_is_loadable_only_with_a_host_slice() {
        let (_, foreign, _) = foreign_ids();
        for wide in [false, true] {
            assert!(!detect(&fat_header(wide, &[foreign, 0x12])).is_loadable_here());
            if let Some((_, host, _)) = host_ids() {
                assert_eq!(detect(&fat_header(wide, &[foreign, host])).is_loadable_here(), APPLE);
            }
        }
    }

    #[test]
    fn fat_slice_count_outside_1_to_32_is_not_fat() {
        // No slices is meaningless, and a large count is how a Java class file
        // (same magic) looks.
        assert_eq!(detect(&fat_header(false, &[])), Format::Unknown);
        let mut h = fat_header(false, &[0x0100_0007; 33]);
        assert_eq!(detect(&h), Format::Unknown);
        h[4..8].copy_from_slice(&32u32.to_be_bytes());
        assert!(matches!(detect(&h), Format::MachOFat { arches } if arches.len() == 32));
    }

    #[test]
    fn truncated_fat_table_lists_only_the_slices_present() {
        let mut h = fat_header(false, &[0x0100_0007, 0x0100_000C, 0x0100_000C]);
        h.truncate(8 + 20 + 3); // second cputype cut short
        assert_eq!(detect(&h), Format::MachOFat { arches: vec![Some("x86_64")] });
    }

    #[test]
    fn pe_machine_and_dll_flag() {
        let cases =
            [(0x8664, Some("x86_64")), (0xAA64, Some("aarch64")), (0x014C, Some("x86")), (0x01C4, None), (0, None)];
        for (machine, arch) in cases {
            assert_eq!(detect(&pe_header(machine, 0x2102)), Format::Pe { dll: true, arch });
            assert_eq!(detect(&pe_header(machine, 0x0102)), Format::Pe { dll: false, arch });
        }
        // Only IMAGE_FILE_DLL decides; every other characteristic is ignored.
        assert_eq!(detect(&pe_header(0x8664, 0xDFFF)), Format::Pe { dll: false, arch: Some("x86_64") });
        assert_eq!(detect(&pe_header(0x8664, 0x2000)), Format::Pe { dll: true, arch: Some("x86_64") });
    }

    #[test]
    fn mz_without_a_reachable_pe_signature_is_unknown() {
        // A DOS or NE image: the signature is not `PE\0\0`.
        let mut h = pe_header(0x8664, 0x2000);
        h[0x80..0x84].copy_from_slice(b"NE\0\0");
        assert_eq!(detect(&h), Format::Unknown);
        // `e_lfanew` straddling or past the end of the buffer.
        for e_lfanew in [0xFDu32, 0x100, 0x1000, u32::MAX - 3, u32::MAX] {
            let mut h = pe_header(0x8664, 0x2000);
            h[0x3C..0x40].copy_from_slice(&e_lfanew.to_le_bytes());
            assert_eq!(detect(&h), Format::Unknown, "{e_lfanew:#x}");
        }
        // `e_lfanew` itself cut off.
        assert_eq!(detect(&pe_header(0x8664, 0x2000)[..0x3E]), Format::Unknown);
    }

    #[test]
    fn truncated_coff_header_reads_as_unknown_arch_executable() {
        let h = pe_header(0x8664, 0x2000);
        assert_eq!(detect(&h[..0x84]), Format::Pe { dll: false, arch: None });
        assert_eq!(detect(&h[..0x96]), Format::Pe { dll: false, arch: Some("x86_64") });
        assert_eq!(detect(&h[..0x98]), Format::Pe { dll: true, arch: Some("x86_64") });
    }

    #[test]
    fn only_host_pe_dlls_are_loadable() {
        let (.., foreign) = foreign_ids();
        assert!(!detect(&pe_header(foreign, 0x2000)).is_loadable_here());
        let Some((.., host)) = host_ids() else { return };
        assert_eq!(detect(&pe_header(host, 0x2000)).is_loadable_here(), cfg!(windows));
        assert!(!detect(&pe_header(host, 0x0002)).is_loadable_here());
    }

    #[test]
    fn ar_archive_needs_the_full_global_header() {
        assert_eq!(detect(b"!<arch>\n"), Format::Archive);
        assert_eq!(detect(b"!<arch>"), Format::Unknown);
        assert_eq!(detect(b"!<thin>\n"), Format::Unknown);
        assert!(!Format::Archive.is_loadable_here());
    }

    #[test]
    fn wasm_module_and_component_are_told_apart_by_version_and_layer() {
        assert_eq!(detect(b"\0asm\x01\0\0\0"), Format::Wasm);
        // Components carry a pre-1.0 version of their own (0x0d now) in layer 1.
        assert_eq!(detect(b"\0asm\x0d\0\x01\0"), Format::WasmComponent);
        assert_eq!(detect(b"\0asm\x0a\0\x01\0"), Format::WasmComponent);
        // Neither a module nor a component.
        assert_eq!(detect(b"\0asm\x02\0\0\0"), Format::Unknown);
        assert_eq!(detect(b"\0asm\xff\xff\xff\xff"), Format::Unknown);
        assert_eq!(detect(b"\0asm\x01\0"), Format::Unknown);
        assert!(!Format::Wasm.is_loadable_here());
        assert!(!Format::WasmComponent.is_loadable_here());
    }

    #[test]
    fn text_and_empty_input_are_unknown() {
        for b in [&b""[..], b"\n", b"hello world\n", b"{\"name\": \"x\"}", b"\x7fEL", b"MZ", b"\0as"] {
            assert_eq!(detect(b), Format::Unknown, "{b:?}");
        }
        assert!(!Format::Unknown.is_loadable_here());
    }

    #[test]
    fn every_truncated_header_is_handled_without_panicking() {
        let (elf_m, mach_cpu, pe_m) = host_ids().unwrap_or(foreign_ids());
        // Each sample with the length at which its last decisive field ends.
        let samples = [
            (elf_header(elf_m, 3), 20),
            (macho_header(0xFEED_FACF, mach_cpu, 6), 16),
            (fat_header(false, &[mach_cpu]), 12),
            (fat_header(true, &[mach_cpu]), 12),
            (pe_header(pe_m, 0x2000), 0x98),
            (b"\0asm\x01\0\0\0".to_vec(), 8),
            (b"!<arch>\n".to_vec(), 8),
        ];
        for (full, need) in samples {
            for n in 0..full.len() {
                let f = detect(&full[..n]);
                if n < 4 {
                    assert_eq!(f, Format::Unknown, "{n} bytes of {full:x?}");
                }
                if n < need {
                    assert!(!f.is_loadable_here(), "{n} bytes of {full:x?} detected as {f}");
                }
            }
        }
    }

    #[test]
    fn sniff_ignores_misleading_extensions() {
        let dir = tempfile::tempdir().unwrap();
        let cases = [
            ("plugin.wasm", elf_header(0xB7, 3), Format::Elf { kind: ElfKind::Shared, arch: Some("aarch64") }),
            ("libplugin.so", b"\0asm\x01\0\0\0".to_vec(), Format::Wasm),
            ("plugin.dll", b"!<arch>\n".to_vec(), Format::Archive),
            ("libplugin.a", pe_header(0x8664, 0x2000), Format::Pe { dll: true, arch: Some("x86_64") }),
            (
                "plugin",
                macho_header(0xFEED_FACF, 0x0100_000C, 8),
                Format::MachO { kind: MachKind::Bundle, arch: Some("aarch64") },
            ),
            ("libplugin.dylib", b"#!/bin/sh\necho hi\n".to_vec(), Format::Unknown),
            ("libplugin.so.1.2", fat_header(true, &[0x0100_0007]), Format::MachOFat { arches: vec![Some("x86_64")] }),
        ];
        for (name, bytes, expected) in cases {
            let path = dir.path().join(name);
            std::fs::write(&path, &bytes).unwrap();
            assert_eq!(sniff(&path).unwrap(), expected, "{name}");
        }
    }

    #[test]
    fn sniff_reads_only_the_head_of_large_files() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("big");
        let mut elf = elf_header(0x3E, 3);
        elf.resize(1 << 20, 0xAA);
        std::fs::write(&path, &elf).unwrap();
        assert_eq!(sniff(&path).unwrap(), Format::Elf { kind: ElfKind::Shared, arch: Some("x86_64") });

        // A PE signature beyond the sniffed window is out of reach.
        let mut pe = pe_header(0x8664, 0x2000);
        pe.resize(SNIFF_LEN + 0x100, 0);
        pe[0x3C..0x40].copy_from_slice(&(SNIFF_LEN as u32).to_le_bytes());
        pe[SNIFF_LEN..SNIFF_LEN + 4].copy_from_slice(b"PE\0\0");
        std::fs::write(&path, &pe).unwrap();
        assert_eq!(sniff(&path).unwrap(), Format::Unknown);
    }

    #[test]
    fn sniff_reports_a_missing_file_as_io_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("missing.so");
        match sniff(&path) {
            Err(PluginError::Io { path: p, .. }) => assert_eq!(p, path),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn check_loadable_names_the_reason() {
        let dir = tempfile::tempdir().unwrap();
        let (foreign, ..) = foreign_ids();
        let host = format!("cannot be loaded on {}-{}", std::env::consts::ARCH, std::env::consts::OS);
        let cases = [
            (b"!<arch>\n".to_vec(), "static library; it has to be relinked".to_owned()),
            (b"\0asm\x01\0\0\0".to_vec(), "Wasm module; load it with kind = \"wasm\"".to_owned()),
            (b"\0asm\x0d\0\x01\0".to_vec(), "Wasm component; load it with kind = \"wasm\"".to_owned()),
            (b"hello\n".to_vec(), "not a shared library".to_owned()),
            (Vec::new(), "not a shared library".to_owned()),
            (elf_header(foreign, 3), host.clone()),
            (pe_header(0x01C4, 0x2000), format!("unknown-arch PE DLL {host}")),
        ];
        // Named like a library of this host, so only the content can refuse it.
        let path = dir.path().join(format!("plugin.{}", std::env::consts::DLL_EXTENSION));
        for (bytes, reason) in cases {
            std::fs::write(&path, &bytes).unwrap();
            match check_loadable(&path) {
                Err(PluginError::NotAPlugin { path: p, reason: r }) => {
                    assert_eq!(p, path);
                    assert!(r.contains(&reason), "{r:?} lacks {reason:?}");
                }
                other => panic!("{bytes:x?}: {other:?}"),
            }
        }
    }

    #[test]
    fn display_names_arch_and_kind() {
        assert_eq!(Format::Elf { kind: ElfKind::Shared, arch: Some("x86_64") }.to_string(), "x86_64 ELF Shared");
        assert_eq!(Format::MachO { kind: MachKind::Bundle, arch: None }.to_string(), "unknown-arch Mach-O Bundle");
        let fat = Format::MachOFat { arches: vec![Some("x86_64"), None] };
        assert_eq!(fat.to_string(), "universal Mach-O (x86_64, unknown-arch)");
        assert_eq!(Format::Pe { dll: true, arch: Some("aarch64") }.to_string(), "aarch64 PE DLL");
        assert_eq!(Format::Pe { dll: false, arch: None }.to_string(), "unknown-arch PE executable");
        assert_eq!(Format::Archive.to_string(), "static library (ar archive)");
        assert_eq!(Format::Unknown.to_string(), "unknown format");
    }

    #[test]
    fn shared_ext_follows_the_format_not_the_host() {
        assert_eq!(shared_ext(&Format::Elf { kind: ElfKind::Shared, arch: None }), "so");
        assert_eq!(shared_ext(&Format::MachO { kind: MachKind::Bundle, arch: None }), "dylib");
        assert_eq!(shared_ext(&Format::MachOFat { arches: vec![] }), "dylib");
        assert_eq!(shared_ext(&Format::Pe { dll: true, arch: None }), "dll");
        assert_eq!(shared_ext(&Format::Unknown), std::env::consts::DLL_EXTENSION);
    }
}
