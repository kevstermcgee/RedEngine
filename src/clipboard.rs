//! Reading and writing the system clipboard as text, with no dependency: a join code is 100 characters and nobody types those.
//!
//! Windows talks to `user32` directly; Linux and macOS ask whichever of `wl-paste`/`xclip`/`xsel`/`pbpaste` is installed. Every function
//! returns `None`/`false` when there is nothing to use rather than failing: a missing clipboard only means typing by hand.

/// The clipboard's text, if it holds some.
pub fn get_text() -> Option<String> {
    imp::get_text().map(|s| s.trim_matches(|c: char| c == '\0' || c.is_control() && c != '\n').to_string()).filter(|s| !s.is_empty())
}

/// Puts `text` on the clipboard; `false` when that was not possible.
pub fn set_text(text: &str) -> bool {
    imp::set_text(text)
}

#[cfg(windows)]
mod imp {
    use std::ffi::c_void;

    const CF_UNICODETEXT: u32 = 13;
    const GMEM_MOVEABLE: u32 = 2;

    #[link(name = "user32")]
    extern "system" {
        fn OpenClipboard(owner: *mut c_void) -> i32;
        fn CloseClipboard() -> i32;
        fn EmptyClipboard() -> i32;
        fn GetClipboardData(format: u32) -> *mut c_void;
        fn SetClipboardData(format: u32, mem: *mut c_void) -> *mut c_void;
    }
    #[link(name = "kernel32")]
    extern "system" {
        fn GlobalAlloc(flags: u32, bytes: usize) -> *mut c_void;
        fn GlobalLock(mem: *mut c_void) -> *mut c_void;
        fn GlobalUnlock(mem: *mut c_void) -> i32;
        fn GlobalSize(mem: *mut c_void) -> usize;
    }

    pub fn get_text() -> Option<String> {
        // SAFETY: plain Win32 calls with checked results; the global memory is locked while read and unlocked before the clipboard closes.
        unsafe {
            if OpenClipboard(std::ptr::null_mut()) == 0 {
                return None;
            }
            let mut out = None;
            let handle = GetClipboardData(CF_UNICODETEXT);
            if !handle.is_null() {
                let ptr = GlobalLock(handle) as *const u16;
                if !ptr.is_null() {
                    let max = GlobalSize(handle) / 2;
                    let mut len = 0;
                    while len < max && *ptr.add(len) != 0 {
                        len += 1;
                    }
                    out = Some(String::from_utf16_lossy(std::slice::from_raw_parts(ptr, len)));
                    GlobalUnlock(handle);
                }
            }
            CloseClipboard();
            out
        }
    }

    pub fn set_text(text: &str) -> bool {
        let wide: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
        // SAFETY: the allocation is exactly `wide.len()` u16s, filled while locked, then handed to the clipboard, which owns it afterwards.
        unsafe {
            if OpenClipboard(std::ptr::null_mut()) == 0 {
                return false;
            }
            EmptyClipboard();
            let mem = GlobalAlloc(GMEM_MOVEABLE, wide.len() * 2);
            let mut ok = false;
            if !mem.is_null() {
                let ptr = GlobalLock(mem) as *mut u16;
                if !ptr.is_null() {
                    std::ptr::copy_nonoverlapping(wide.as_ptr(), ptr, wide.len());
                    GlobalUnlock(mem);
                    ok = !SetClipboardData(CF_UNICODETEXT, mem).is_null();
                }
            }
            CloseClipboard();
            ok
        }
    }
}

#[cfg(not(windows))]
mod imp {
    use std::io::Write;
    use std::process::{Command, Stdio};

    pub fn get_text() -> Option<String> {
        for (cmd, args) in
            [("wl-paste", &["--no-newline"][..]), ("xclip", &["-selection", "clipboard", "-o"][..]), ("xsel", &["-b", "-o"][..]), ("pbpaste", &[][..])]
        {
            if let Ok(out) = Command::new(cmd).args(args).stderr(Stdio::null()).output() {
                if out.status.success() {
                    return String::from_utf8(out.stdout).ok();
                }
            }
        }
        None
    }

    pub fn set_text(text: &str) -> bool {
        for (cmd, args) in [("wl-copy", &[][..]), ("xclip", &["-selection", "clipboard"][..]), ("xsel", &["-b", "-i"][..]), ("pbcopy", &[][..])] {
            if let Ok(mut child) = Command::new(cmd).args(args).stdin(Stdio::piped()).stdout(Stdio::null()).stderr(Stdio::null()).spawn() {
                if let Some(mut stdin) = child.stdin.take() {
                    if stdin.write_all(text.as_bytes()).is_err() {
                        continue;
                    }
                }
                return child.wait().is_ok_and(|s| s.success());
            }
        }
        false
    }
}
