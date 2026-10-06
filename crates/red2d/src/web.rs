//! The WebAssembly surface: a small hand-written C ABI over [`crate::host::Host`], called by `web/runtime.js`.
//!
//! There is no wasm-bindgen: the whole boundary is the list below, so it is auditable at a glance. Bytes go in through `alloc` + a write by JavaScript + a call; text and samples come out
//! through one output buffer (`out_ptr`/`out_len`, valid until the next call that fills it) and the picture through `frame_ptr`/`frame_len`. Nothing here touches a clock, a file, the
//! network, a thread or the environment: those do not exist in the module, which is why it needs no imports.
//!
//! | export | meaning |
//! |---|---|
//! | `alloc(n)`, `dealloc(p, n)` | a buffer JavaScript fills |
//! | `init(p, n, seed) -> 0/1` | load the game text; on 1, `error` holds every problem |
//! | `step(n)`, `render()` | run `n` ticks; draw the frame |
//! | `view_w()`, `view_h()`, `frame_ptr()`, `frame_len()` | the picture |
//! | `key(p, n, down)`, `action(p, n, down)`, `pointer(x, y)`, `click(x, y)` | input |
//! | `window(ww, wh)`, `layout_x/y/w/h()`, `to_view(wx, wy) -> 0/1`, `view_x()`, `view_y()` | resolution independence |
//! | `snapshot() -> len`, `save_take() -> len`, `save_load(p, n) -> code`, `scenarios() -> len` | text out (in `out`) |
//! | `sounds_take() -> count`, `sound_pcm(i) -> bytes`, `music_pcm() -> bytes`, `music_on()`, `sample_rate()` | audio (f32 LE samples in `out`) |

use crate::host::Host;
use std::cell::RefCell;

thread_local! {
    static HOST: RefCell<Host> = RefCell::new(Host::default());
    static OUT: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) };
    static ERR: RefCell<String> = const { RefCell::new(String::new()) };
    static VIEW: RefCell<[f32; 2]> = const { RefCell::new([0.0; 2]) };
    static LAYOUT: RefCell<[i32; 4]> = const { RefCell::new([0; 4]) };
}

fn with<R>(f: impl FnOnce(&mut Host) -> R) -> R {
    HOST.with(|h| f(&mut h.borrow_mut()))
}

fn text<'a>(p: *const u8, n: usize) -> &'a str {
    // SAFETY: JavaScript wrote `n` bytes at `p` (obtained from `alloc`) before calling.
    let bytes = unsafe { std::slice::from_raw_parts(p, n) };
    std::str::from_utf8(bytes).unwrap_or("")
}

fn put_text(s: &str) -> usize {
    OUT.with(|o| {
        let mut o = o.borrow_mut();
        o.clear();
        o.extend_from_slice(s.as_bytes());
        o.len()
    })
}

fn put_f32(samples: &[f32]) -> usize {
    OUT.with(|o| {
        let mut o = o.borrow_mut();
        o.clear();
        for s in samples {
            o.extend_from_slice(&s.to_le_bytes());
        }
        o.len()
    })
}

/// A buffer of `n` bytes for JavaScript to fill.
#[no_mangle]
pub extern "C" fn alloc(n: usize) -> *mut u8 {
    let mut v = Vec::<u8>::with_capacity(n.max(1));
    let p = v.as_mut_ptr();
    std::mem::forget(v);
    p
}

/// Frees a buffer from [`alloc`].
///
/// The host calls it exactly once per buffer, with the length it allocated; an exported C function cannot be `unsafe` in the module's ABI listing.
#[allow(clippy::not_unsafe_ptr_arg_deref)]
#[no_mangle]
pub extern "C" fn dealloc(p: *mut u8, n: usize) {
    // SAFETY: `p` came from `alloc(n)` with this capacity.
    unsafe { drop(Vec::from_raw_parts(p, 0, n.max(1))) }
}

/// Loads the game; 0 = ready, 1 = refused (see [`error_len`]).
#[no_mangle]
pub extern "C" fn init(p: *const u8, n: usize, seed: u32) -> i32 {
    let t = text(p, n).to_string();
    match with(|h| h.init(&t, u64::from(seed))) {
        Ok(()) => 0,
        Err(e) => {
            ERR.with(|x| *x.borrow_mut() = e);
            1
        }
    }
}

/// Hot reload: replaces the running game with new text, carrying over the named entities' places (see `Sim::carry_over`); 0 = applied, 1 = refused (the old game keeps running;
/// see [`error`]).
#[no_mangle]
pub extern "C" fn reload(p: *const u8, n: usize, seed: u32) -> i32 {
    let t = text(p, n).to_string();
    match with(|h| h.reload(&t, u64::from(seed))) {
        Ok(_) => 0,
        Err(e) => {
            ERR.with(|x| *x.borrow_mut() = e);
            1
        }
    }
}

/// The refusal text of the last failed [`init`], placed in the output buffer; its length.
#[no_mangle]
pub extern "C" fn error() -> usize {
    ERR.with(|e| put_text(&e.borrow()))
}

/// Runs `n` ticks.
#[no_mangle]
pub extern "C" fn step(n: u32) {
    with(|h| h.step(n));
}

/// Draws the frame.
#[no_mangle]
pub extern "C" fn render() {
    with(Host::render);
}

/// Virtual screen width.
#[no_mangle]
pub extern "C" fn view_w() -> u32 {
    with(|h| h.view_size().0)
}

/// Virtual screen height.
#[no_mangle]
pub extern "C" fn view_h() -> u32 {
    with(|h| h.view_size().1)
}

/// Address of the RGBA picture (valid until the next `init`).
#[no_mangle]
pub extern "C" fn frame_ptr() -> *const u8 {
    with(|h| h.frame().as_ptr())
}

/// Bytes in the picture.
#[no_mangle]
pub extern "C" fn frame_len() -> usize {
    with(|h| h.frame().len())
}

/// A key (`KeyboardEvent.code`) went down (1) or up (0).
#[no_mangle]
pub extern "C" fn key(p: *const u8, n: usize, down: i32) {
    let c = text(p, n).to_string();
    with(|h| h.key(&c, down != 0));
}

/// An action went down (1) or up (0); 0 returned when there is no such action.
#[no_mangle]
pub extern "C" fn action(p: *const u8, n: usize, down: i32) -> i32 {
    let c = text(p, n).to_string();
    i32::from(with(|h| h.action(&c, down != 0)))
}

/// The pointer moved (virtual pixels).
#[no_mangle]
pub extern "C" fn pointer(x: f32, y: f32) {
    with(|h| h.pointer(x, y));
}

/// The pointer was pressed (virtual pixels).
#[no_mangle]
pub extern "C" fn click(x: f32, y: f32) {
    with(|h| h.click(x, y));
}

/// The window is `ww x wh` CSS pixels: computes where the screen lands.
#[no_mangle]
pub extern "C" fn window(ww: u32, wh: u32) {
    let l = with(|h| h.set_window(ww, wh));
    LAYOUT.with(|x| *x.borrow_mut() = [l.x, l.y, l.w as i32, l.h as i32]);
}

/// Left edge of the screen in the window.
#[no_mangle]
pub extern "C" fn layout_x() -> i32 {
    LAYOUT.with(|l| l.borrow()[0])
}
/// Top edge.
#[no_mangle]
pub extern "C" fn layout_y() -> i32 {
    LAYOUT.with(|l| l.borrow()[1])
}
/// On-screen width.
#[no_mangle]
pub extern "C" fn layout_w() -> i32 {
    LAYOUT.with(|l| l.borrow()[2])
}
/// On-screen height.
#[no_mangle]
pub extern "C" fn layout_h() -> i32 {
    LAYOUT.with(|l| l.borrow()[3])
}

/// Maps a window position to the virtual screen: 1 = on the screen (read [`view_x`]/[`view_y`]), 0 = in the bars.
#[no_mangle]
pub extern "C" fn to_view(wx: f32, wy: f32) -> i32 {
    match with(|h| h.window_to_view(wx, wy)) {
        Some(p) => {
            VIEW.with(|v| *v.borrow_mut() = p);
            1
        }
        None => 0,
    }
}
/// x from the last successful [`to_view`].
#[no_mangle]
pub extern "C" fn view_x() -> f32 {
    VIEW.with(|v| v.borrow()[0])
}
/// y from the last successful [`to_view`].
#[no_mangle]
pub extern "C" fn view_y() -> f32 {
    VIEW.with(|v| v.borrow()[1])
}

/// The state as JSON text in the output buffer; its length.
#[no_mangle]
pub extern "C" fn snapshot() -> usize {
    let s = with(|h| h.snapshot_json());
    put_text(&s)
}

/// The save text, if there is something new to keep, in the output buffer; 0 = nothing.
#[no_mangle]
pub extern "C" fn save_take() -> usize {
    match with(Host::take_save) {
        Some(s) => put_text(&s),
        None => 0,
    }
}

/// Restores saved text; returns 0 fresh, 1 loaded, 2 incompatible, 3 unreadable. A sentence is in the output buffer.
#[no_mangle]
pub extern "C" fn save_load(p: *const u8, n: usize) -> i32 {
    let t = text(p, n).to_string();
    let (code, msg) = with(|h| h.load_save(&t));
    put_text(&msg);
    code
}

/// Runs the game's scenarios and puts their JSON report in the output buffer; its length.
#[no_mangle]
pub extern "C" fn scenarios() -> usize {
    let s = with(|h| h.scenarios_json());
    put_text(&s)
}

/// Sounds to play now: u32 LE indices in the output buffer; the count.
#[no_mangle]
pub extern "C" fn sounds_take() -> usize {
    let v = with(Host::take_sounds);
    OUT.with(|o| {
        let mut o = o.borrow_mut();
        o.clear();
        for i in &v {
            o.extend_from_slice(&i.to_le_bytes());
        }
    });
    v.len()
}

/// Samples of sound `i` (mono f32 LE) in the output buffer; the byte count, 0 if it cannot be rendered.
#[no_mangle]
pub extern "C" fn sound_pcm(i: u32) -> usize {
    match with(|h| h.sound_pcm(i as usize)) {
        Ok(p) => put_f32(&p),
        Err(e) => {
            ERR.with(|x| *x.borrow_mut() = e);
            0
        }
    }
}

/// Samples of the first music track (stereo interleaved f32 LE) in the output buffer; 0 = the game has none.
#[no_mangle]
pub extern "C" fn music_pcm() -> usize {
    match with(|h| h.music_pcm()) {
        Ok(Some(p)) => put_f32(&p),
        Ok(None) => 0,
        Err(e) => {
            ERR.with(|x| *x.borrow_mut() = e);
            0
        }
    }
}

/// 1 when the game wants music playing.
#[no_mangle]
pub extern "C" fn music_on() -> i32 {
    i32::from(with(|h| h.music_on()))
}

/// Samples per second of rendered audio.
#[no_mangle]
pub extern "C" fn sample_rate() -> u32 {
    crate::synth::SAMPLE_RATE
}

/// Address of the output buffer.
#[no_mangle]
pub extern "C" fn out_ptr() -> *const u8 {
    OUT.with(|o| o.borrow().as_ptr())
}

/// Length of the output buffer's current contents.
#[no_mangle]
pub extern "C" fn out_len() -> usize {
    OUT.with(|o| o.borrow().len())
}
