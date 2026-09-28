//! Windows process plumbing for the game: no stray console window, output that always goes somewhere, and fatal errors a player can see.
//!
//! `re2` is a console program in the engine's own builds (a terminal waits for it and shows its output, tests read its pipes); `red_engine2 package` flips the copy it
//! ships to the GUI subsystem (`tools::pe`), which never opens a console. Either way [`init`] keeps the same promises:
//! - started from a terminal: the terminal's console shows the output (a GUI-subsystem copy attaches to it only when the command line is a developer's, see
//!   [`wants_terminal`]: attaching from a batch launcher would keep the launcher's console window open for as long as the game runs);
//! - started by a double-click or a shortcut: no console window stays open, and stdout/stderr (a panic message, `RE2_STATS`, the hosted server's log)
//!   go to `re2.log` next to the program (the previous run's log is kept as `re2.previous.log`), and a crash on the main thread says so in a box;
//! - started with its output redirected to a file or a pipe: that redirection is left alone.

use std::fs::OpenOptions;
use std::os::windows::io::IntoRawHandle;
use std::path::{Path, PathBuf};

#[link(name = "kernel32")]
extern "system" {
    fn GetConsoleProcessList(list: *mut u32, count: u32) -> u32;
    fn GetConsoleWindow() -> isize;
    fn AttachConsole(process_id: u32) -> i32;
    fn FreeConsole() -> i32;
    fn SetStdHandle(which: u32, handle: isize) -> i32;
    fn GetStdHandle(which: u32) -> isize;
    fn GetFileType(handle: isize) -> u32;
    fn GetConsoleMode(handle: isize, mode: *mut u32) -> i32;
}

const STD_INPUT: u32 = -10i32 as u32;
const STD_OUTPUT: u32 = -11i32 as u32;
const STD_ERROR: u32 = -12i32 as u32;
const ATTACH_PARENT_PROCESS: u32 = u32::MAX;

/// Where the program's output ended up.
pub enum Output {
    /// A terminal's console.
    Terminal,
    /// A file or pipe the caller chose (or nowhere: a program with no console and no writable log directory).
    Redirected,
    /// `re2.log`, because nobody is watching a console.
    Log(PathBuf),
}

/// True when standard handle `which` is a real, open handle (not the null one a GUI program without redirection gets).
fn handle_is_open(which: u32) -> bool {
    // SAFETY: a plain query of a standard handle number.
    unsafe {
        let h = GetStdHandle(which);
        h != 0 && h != -1 && GetFileType(h) != 0
    }
}

/// True when standard handle `which` is the console itself (not a file or pipe the caller redirected it to).
fn handle_is_console(which: u32) -> bool {
    let mut mode = 0u32;
    // SAFETY: `mode` is a valid out-pointer; the handle is only queried.
    unsafe {
        let h = GetStdHandle(which);
        h != 0 && h != -1 && GetConsoleMode(h, &mut mode) != 0
    }
}

/// Whether the command line is one whose output somebody is waiting to read: asking for help, or one of the flags that make the client a command-line tool
/// (`--headless`, `--playtest`, `--script`, `--dump`), or `RE2_CONSOLE=1`. A GUI-subsystem copy attaches to the launching terminal only then.
pub fn wants_terminal(args: &[String]) -> bool {
    const CLI_FLAGS: [&str; 8] = ["-h", "--help", "-V", "--version", "--debug-help", "--headless", "--playtest", "--script"];
    std::env::var_os("RE2_CONSOLE").is_some() || args.iter().any(|a| CLI_FLAGS.contains(&a.as_str()) || a == "--dump")
}

/// Sets up stdout/stderr for the way the program was started (see the module docs). Call first thing in `main`, before anything prints.
pub fn init(attach_terminal: bool) -> Output {
    // SAFETY: Win32 console calls with valid arguments. The raw handles passed to `SetStdHandle` are leaked on purpose: they are the process's
    // stdout/stderr for its whole life.
    unsafe {
        if GetConsoleWindow() == 0 {
            // No console: a GUI-subsystem program, started by a double-click or from a terminal without redirection.
            if handle_is_open(STD_OUTPUT) {
                return Output::Redirected;
            }
            if attach_terminal && AttachConsole(ATTACH_PARENT_PROCESS) != 0 {
                if let Ok(out) = OpenOptions::new().write(true).open("CONOUT$") {
                    let h = out.into_raw_handle() as isize;
                    SetStdHandle(STD_OUTPUT, h);
                    SetStdHandle(STD_ERROR, h);
                    return Output::Terminal;
                }
            }
            return log_to_file();
        }
        // A console-subsystem program: its console is shared with a terminal (keep it, it shows the output) or was made just for this process
        // by a double-click (close it: the game has its own window).
        let mut pids = [0u32; 4];
        if GetConsoleProcessList(pids.as_mut_ptr(), pids.len() as u32) != 1 {
            return Output::Terminal;
        }
        let console: Vec<u32> = [STD_INPUT, STD_OUTPUT, STD_ERROR].into_iter().filter(|&which| handle_is_console(which)).collect();
        FreeConsole();
        // Those handles now hold the closed console's numbers, and Windows reuses numbers: a later `println!` (the server thread logs when a match ends)
        // could write to whatever got that number, and block for ever. Clear them; Rust drops writes to a missing handle.
        for which in &console {
            SetStdHandle(*which, 0);
        }
        if console.contains(&STD_OUTPUT) || console.contains(&STD_ERROR) {
            log_to_file()
        } else {
            Output::Redirected
        }
    }
}

/// Points stdout and stderr at `re2.log` next to the program (or in the temp directory when that folder is read-only).
fn log_to_file() -> Output {
    let open = |dir: &Path| -> std::io::Result<(PathBuf, std::fs::File)> {
        let path = dir.join("re2.log");
        // A crash is read after the next launch, so the last log is kept.
        let _ = std::fs::rename(&path, dir.join("re2.previous.log"));
        Ok((path.clone(), OpenOptions::new().create(true).write(true).truncate(true).open(&path)?))
    };
    let beside_exe = std::env::current_exe().ok().and_then(|p| p.parent().map(Path::to_path_buf)).unwrap_or_else(std::env::temp_dir);
    match open(&beside_exe).or_else(|_| open(&std::env::temp_dir())) {
        Ok((path, file)) => {
            let h = file.into_raw_handle() as isize;
            // SAFETY: `h` is a valid file handle that is never closed.
            unsafe {
                SetStdHandle(STD_OUTPUT, h);
                SetStdHandle(STD_ERROR, h);
            }
            Output::Log(path)
        }
        Err(_) => {
            // SAFETY: clearing the standard handles is always allowed.
            unsafe {
                SetStdHandle(STD_OUTPUT, 0);
                SetStdHandle(STD_ERROR, 0);
            }
            Output::Redirected
        }
    }
}

/// When the output goes to `re2.log` nobody sees a panic, so a panic on the main thread also opens a box that names the log.
pub fn install_crash_box(output: &Output) {
    let Output::Log(path) = output else { return };
    let path = path.clone();
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        previous(info); // the message and its location go to the log
        if std::thread::current().name() == Some("main") {
            message_box("Red Engine 2 stopped", &format!("The game hit an internal error and has to close.\n\n{info}\n\nDetails are in {}", path.display()));
        }
    }));
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

#[cfg(test)]
mod tests {
    use super::*;

    fn args(a: &[&str]) -> Vec<String> {
        a.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn only_a_developers_command_line_attaches_to_the_terminal() {
        assert!(wants_terminal(&args(&["re2", "--debug-help"])));
        assert!(wants_terminal(&args(&["re2", "map.json", "--headless", "--script", "play.json"])));
        assert!(wants_terminal(&args(&["re2", "map.json", "--dump", "state.json"])));
        if std::env::var_os("RE2_CONSOLE").is_none() {
            assert!(!wants_terminal(&args(&["re2", "arena.json", "--host", "--name", "Sam"])), "a launcher's command line must not hold its console open");
        }
    }
}
