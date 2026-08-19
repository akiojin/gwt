#![cfg(windows)]

use std::{fs, path::Path};

const IMAGE_SUBSYSTEM_WINDOWS_GUI: u16 = 2;
const IMAGE_SUBSYSTEM_WINDOWS_CUI: u16 = 3;

#[test]
fn windows_gwt_binary_uses_gui_subsystem_without_changing_gwtd() {
    assert_eq!(
        pe_subsystem(Path::new(env!("CARGO_BIN_EXE_gwt"))),
        IMAGE_SUBSYSTEM_WINDOWS_GUI,
        "gwt.exe must launch as a Windows GUI app so Explorer/Start Menu startup does not show a console window"
    );
    assert_eq!(
        pe_subsystem(Path::new(env!("CARGO_BIN_EXE_gwtd"))),
        IMAGE_SUBSYSTEM_WINDOWS_CUI,
        "gwtd.exe must remain a console app for hook/headless CLI output"
    );
}

/// Issue #3631: `CreateProcess` only attaches console subsystem images to the
/// pseudoconsole passed through `PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE`. A start
/// gate hosted by the GUI front door therefore has no console, and the agent it
/// releases is given a brand new console that the OS default terminal
/// (Windows Terminal) hosts outside the gwt pane.
#[test]
fn windows_pty_start_gate_runs_in_the_console_subsystem_companion() {
    let gate =
        gwt::pty_start_gate::resolve_pty_start_gate_program(Path::new(env!("CARGO_BIN_EXE_gwt")))
            .expect("resolve the PTY start-gate program");

    assert_eq!(
        gate,
        Path::new(env!("CARGO_BIN_EXE_gwtd")),
        "the Windows start gate must run in the gwtd companion, not the GUI front door"
    );
    assert_eq!(
        pe_subsystem(&gate),
        IMAGE_SUBSYSTEM_WINDOWS_CUI,
        "the PTY start-gate program must be a console app so the pane's pseudoconsole reaches the released agent"
    );
}

fn pe_subsystem(path: &Path) -> u16 {
    let bytes = fs::read(path).unwrap_or_else(|error| {
        panic!("failed to read {}: {error}", path.display());
    });

    let pe_offset = read_u32(&bytes, 0x3c) as usize;
    assert_eq!(
        bytes.get(pe_offset..pe_offset + 4),
        Some(b"PE\0\0".as_slice()),
        "{} is not a PE image",
        path.display()
    );

    let coff_header = pe_offset + 4;
    let optional_header_size = read_u16(&bytes, coff_header + 16) as usize;
    let optional_header = coff_header + 20;
    assert!(
        optional_header_size > 70,
        "{} optional header is too small",
        path.display()
    );

    read_u16(&bytes, optional_header + 68)
}

fn read_u16(bytes: &[u8], offset: usize) -> u16 {
    let Some(raw) = bytes.get(offset..offset + 2) else {
        panic!("PE header read out of bounds at offset {offset}");
    };
    u16::from_le_bytes(raw.try_into().expect("2-byte slice"))
}

fn read_u32(bytes: &[u8], offset: usize) -> u32 {
    let Some(raw) = bytes.get(offset..offset + 4) else {
        panic!("PE header read out of bounds at offset {offset}");
    };
    u32::from_le_bytes(raw.try_into().expect("4-byte slice"))
}
