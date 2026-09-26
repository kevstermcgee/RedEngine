//! Windows message boxes for fatal failures that also print to the attached console.

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
