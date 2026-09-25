//! Windows console attachment and message boxes (the game is a GUI-subsystem binary, so errors need a way out).

use std::os::windows::io::AsRawHandle;

#[link(name = "kernel32")]
extern "system" {
    fn AttachConsole(process_id: u32) -> i32;
    fn GetStdHandle(which: u32) -> isize;
    fn SetStdHandle(which: u32, handle: isize) -> i32;
}
#[link(name = "user32")]
extern "system" {
    fn MessageBoxW(hwnd: isize, text: *const u16, caption: *const u16, kind: u32) -> i32;
}

const ATTACH_PARENT_PROCESS: u32 = u32::MAX;
const STD_OUTPUT_HANDLE: u32 = -11i32 as u32;
const STD_ERROR_HANDLE: u32 = -12i32 as u32;

/// Re-attaches to the launching terminal (if any) and points stdout/stderr at it, unless they
/// were already redirected to a file/pipe. Returns whether a parent console was found.
pub fn attach_console() -> bool {
    unsafe {
        if AttachConsole(ATTACH_PARENT_PROCESS) == 0 {
            return false;
        }
        for which in [STD_OUTPUT_HANDLE, STD_ERROR_HANDLE] {
            let h = GetStdHandle(which);
            if h == 0 || h == -1 {
                if let Ok(con) = std::fs::OpenOptions::new().write(true).open("CONOUT$") {
                    SetStdHandle(which, con.as_raw_handle() as isize);
                    std::mem::forget(con);
                }
            }
        }
    }
    true
}

/// A modal error box, for failures that have nowhere else to be printed.
pub fn message_box(title: &str, text: &str) {
    let wide = |s: &str| s.encode_utf16().chain(std::iter::once(0)).collect::<Vec<u16>>();
    unsafe {
        MessageBoxW(0, wide(text).as_ptr(), wide(title).as_ptr(), 0x10); // MB_ICONERROR
    }
}
