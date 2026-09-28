//! Windows message boxes for fatal failures that also print to the attached console.

#[link(name = "kernel32")]
extern "system" {
    fn GetConsoleProcessList(list: *mut u32, count: u32) -> u32;
    fn FreeConsole() -> i32;
    fn SetStdHandle(which: u32, handle: isize) -> i32;
    fn GetStdHandle(which: u32) -> isize;
    fn GetConsoleMode(handle: isize, mode: *mut u32) -> i32;
}

/// Started by a double-click or a shortcut, Windows opens a console window just for the game and the game has its own; close that one.
/// Started from a terminal the console is shared with the shell, so it stays and keeps showing the output.
pub fn hide_own_console() {
    let mut pids = [0u32; 4];
    // SAFETY: `pids` is a valid buffer of the length passed; FreeConsole takes no arguments.
    unsafe {
        if GetConsoleProcessList(pids.as_mut_ptr(), pids.len() as u32) == 1 {
            // Which standard handles are the console itself (not a file or pipe the caller redirected them to)?
            let console: Vec<u32> = [-10i32, -11, -12]
                .into_iter()
                .map(|w| w as u32)
                .filter(|&which| {
                    let (h, mut mode) = (GetStdHandle(which), 0u32);
                    h != 0 && h != -1 && GetConsoleMode(h, &mut mode) != 0
                })
                .collect();
            FreeConsole();
            // Those now hold the closed console's numbers, and Windows reuses numbers: a later `println!` (the server thread logs when a match ends)
            // could write to whatever got that number, and block for ever. Clear them; Rust drops writes to a missing handle.
            for which in console {
                SetStdHandle(which, 0);
            }
        }
    }
}

#[link(name = "user32")]
extern "system" {
    fn MessageBoxW(hwnd: isize, text: *const u16, caption: *const u16, kind: u32) -> i32;
}

/// A modal error box, for failures that have nowhere else to be printed.
pub fn message_box(title: &str, text: &str) {
    let wide = |s: &str| s.encode_utf16().chain(std::iter::once(0)).collect::<Vec<u16>>();
    unsafe {
        MessageBoxW(0, wide(text).as_ptr(), wide(title).as_ptr(), 0x10); // MB_ICONERROR
    }
}
