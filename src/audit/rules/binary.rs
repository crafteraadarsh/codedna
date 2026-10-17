//! Executables checked in as source.

use crate::audit::{finding, Finding, Severity, BINARY};

/// A finding when `bytes` are a Linux, macOS or Windows executable.
pub(crate) fn check(file: &str, bytes: &[u8]) -> Option<Finding> {
    let kind = executable_kind(bytes)?;
    Some(finding(
        Severity::Medium,
        BINARY,
        file,
        None,
        format!("{kind} executable in the repository: it cannot be reviewed as source"),
    ))
}

fn executable_kind(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(b"\x7fELF") {
        return Some("Linux (ELF)");
    }
    const MACHO: &[[u8; 4]] = &[
        [0xFE, 0xED, 0xFA, 0xCE],
        [0xFE, 0xED, 0xFA, 0xCF],
        [0xCE, 0xFA, 0xED, 0xFE],
        [0xCF, 0xFA, 0xED, 0xFE],
    ];
    if MACHO.iter().any(|magic| bytes.starts_with(magic)) {
        return Some("macOS (Mach-O)");
    }
    if bytes.starts_with(b"MZ") && bytes.len() >= 0x40 {
        let offset = u32::from_le_bytes([bytes[0x3c], bytes[0x3d], bytes[0x3e], bytes[0x3f]]);
        let offset = offset as usize;
        if bytes.get(offset..offset.saturating_add(4)) == Some(b"PE\0\0".as_slice()) {
            return Some("Windows (PE)");
        }
    }
    None
}
