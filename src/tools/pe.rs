//! Reading and flipping the Windows *subsystem* of an executable (console <-> GUI), so a shipped game never opens a console window.
//!
//! A console-subsystem program started by a double-click owns a console window for as long as it runs; a GUI-subsystem one owns none, and attaches the
//! console of the terminal it was started from, if any (`bin/re2/win.rs`). The engine builds `re2` as a console program (a terminal, a test or a CI job waits
//! for it and reads its output) and `red_engine2 package` flips the copy it ships. Pure functions over the image bytes: no Windows API, testable anywhere.

/// Which kind of Windows program an image is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Subsystem {
    /// `IMAGE_SUBSYSTEM_WINDOWS_CUI`: runs in a console (a window of its own when double-clicked).
    Console,
    /// `IMAGE_SUBSYSTEM_WINDOWS_GUI`: has no console of its own.
    Gui,
}

const IMAGE_SUBSYSTEM_WINDOWS_GUI: u16 = 2;
const IMAGE_SUBSYSTEM_WINDOWS_CUI: u16 = 3;
/// Offset of `Subsystem` inside the optional header (the same in PE32 and PE32+).
const OPTIONAL_HEADER_SUBSYSTEM: usize = 68;
/// Size of the PE signature plus the COFF file header, which sit between `e_lfanew` and the optional header.
const PE_SIGNATURE_AND_FILE_HEADER: usize = 24;

/// Byte offset of the `Subsystem` field of `image`, or what is wrong with it.
fn subsystem_offset(image: &[u8]) -> Result<usize, String> {
    let u16_at = |at: usize| image.get(at..at + 2).map(|b| u16::from_le_bytes([b[0], b[1]]));
    let u32_at = |at: usize| image.get(at..at + 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]));
    if u16_at(0) != Some(0x5A4D) {
        return Err("not a Windows executable (no MZ header)".into());
    }
    let pe = u32_at(0x3c).ok_or("truncated DOS header")? as usize;
    if u32_at(pe) != Some(0x0000_4550) {
        return Err("not a PE image (no PE signature at e_lfanew)".into());
    }
    let optional = pe + PE_SIGNATURE_AND_FILE_HEADER;
    if !matches!(u16_at(optional), Some(0x10b | 0x20b)) {
        return Err("not a PE32 or PE32+ image (unknown optional header)".into());
    }
    let at = optional + OPTIONAL_HEADER_SUBSYSTEM;
    if at + 2 > image.len() {
        return Err("truncated optional header".into());
    }
    Ok(at)
}

/// The subsystem an executable image declares; `Err` when the bytes are not a PE image or the subsystem is neither console nor GUI.
pub fn subsystem(image: &[u8]) -> Result<Subsystem, String> {
    let at = subsystem_offset(image)?;
    match u16::from_le_bytes([image[at], image[at + 1]]) {
        IMAGE_SUBSYSTEM_WINDOWS_GUI => Ok(Subsystem::Gui),
        IMAGE_SUBSYSTEM_WINDOWS_CUI => Ok(Subsystem::Console),
        other => Err(format!("unsupported subsystem {other} (only console and GUI programs are handled)")),
    }
}

/// Rewrites the subsystem of `image` in place. Windows does not verify the image checksum of an ordinary program, so nothing else changes; an image that was
/// signed with Authenticode would need signing again.
pub fn set_subsystem(image: &mut [u8], to: Subsystem) -> Result<(), String> {
    let at = subsystem_offset(image)?;
    let value = match to {
        Subsystem::Gui => IMAGE_SUBSYSTEM_WINDOWS_GUI,
        Subsystem::Console => IMAGE_SUBSYSTEM_WINDOWS_CUI,
    };
    image[at..at + 2].copy_from_slice(&value.to_le_bytes());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A minimal image: DOS header pointing at a PE header at 0x80, an optional header with `magic` and `subsystem`.
    fn image(magic: u16, subsystem: u16) -> Vec<u8> {
        let mut b = vec![0u8; 0x200];
        b[0..2].copy_from_slice(&0x5A4Du16.to_le_bytes());
        b[0x3c..0x40].copy_from_slice(&0x80u32.to_le_bytes());
        b[0x80..0x84].copy_from_slice(&0x0000_4550u32.to_le_bytes());
        b[0x98..0x9a].copy_from_slice(&magic.to_le_bytes());
        b[0x98 + 68..0x98 + 70].copy_from_slice(&subsystem.to_le_bytes());
        b
    }

    #[test]
    fn a_console_program_becomes_a_gui_program_and_back_without_touching_anything_else() {
        for magic in [0x20b, 0x10b] {
            let original = image(magic, 3);
            let mut b = original.clone();
            assert_eq!(subsystem(&b), Ok(Subsystem::Console));
            set_subsystem(&mut b, Subsystem::Gui).unwrap();
            assert_eq!(subsystem(&b), Ok(Subsystem::Gui));
            assert_eq!(b.iter().zip(&original).filter(|(a, o)| a != o).count(), 1, "only the low byte of the subsystem changes");
            set_subsystem(&mut b, Subsystem::Console).unwrap();
            assert_eq!(b, original);
        }
    }

    #[test]
    fn things_that_are_not_ordinary_windows_programs_are_refused_with_a_reason() {
        assert!(subsystem(b"#!/bin/sh\n").unwrap_err().contains("MZ"));
        assert!(subsystem(&[]).unwrap_err().contains("MZ"));
        let mut no_pe = image(0x20b, 3);
        no_pe[0x80] = 0;
        assert!(subsystem(&no_pe).unwrap_err().contains("PE signature"));
        assert!(subsystem(&image(0x107, 3)).unwrap_err().contains("optional header"));
        assert!(subsystem(&image(0x20b, 1)).unwrap_err().contains("unsupported subsystem 1"), "native drivers are not touched");
        assert!(set_subsystem(&mut image(0x20b, 1), Subsystem::Gui).is_err());
        let cut = &image(0x20b, 3)[..0x98 + 70 - 1];
        assert!(subsystem(cut).unwrap_err().contains("truncated"));
    }

    #[test]
    fn a_pe_header_pointing_outside_the_file_is_an_error_not_a_panic() {
        let mut b = image(0x20b, 3);
        b[0x3c..0x40].copy_from_slice(&0xFFFF_FFF0u32.to_le_bytes());
        assert!(subsystem(&b).is_err());
    }

    #[cfg(windows)]
    #[test]
    fn this_very_test_program_is_a_console_program() {
        let exe = std::fs::read(std::env::current_exe().unwrap()).unwrap();
        assert_eq!(subsystem(&exe), Ok(Subsystem::Console));
    }
}
