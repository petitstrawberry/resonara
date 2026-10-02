//! Experimental embedded Scarlet/SWS CLAP window API. This is not a CLAP standard API.
//! `clap_window.specific.ptr` points to ParentV1, valid until gui.destroy returns.
//! All calls are on the CLAP main thread. Buffers are borrowed only for present().
#![no_std]
use core::ffi::{CStr, c_void};
pub const API: &CStr = c"org.scarlet-os.sws/1";
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct Input {
    /// 1 mouse down, 2 mouse up, 3 move, 4 key down, 5 key up, 6 character,
    /// 7 cancel mouse capture.
    pub kind: u32,
    pub x: i32,
    pub y: i32,
    /// 1 Escape, 2 Enter, 3 Tab, 4 Backspace, 5 Delete, 6 Left, 7 Right,
    /// 8 Up, 9 Down, 10 Home, 11 End, 12 Space; otherwise 0x10000 + Unicode.
    pub key: u32,
    /// Shift=1, Control=2, Alt=4, Super=8.
    pub modifiers: u32,
    /// Mouse click count (at least 1) for down/up; zero for other events.
    pub click_count: u32,
}
#[repr(C)]
#[derive(Clone, Copy)]
pub struct ParentV1 {
    pub version: u32,
    pub context: *mut c_void,
    pub scale_milli: unsafe extern "C" fn(*mut c_void) -> u32,
    pub poll_input: unsafe extern "C" fn(*mut c_void, *mut Input) -> bool,
    /// BGRA8, tightly packed physical pixels. Host copies before returning.
    pub present: unsafe extern "C" fn(*mut c_void, *const u8, u32, u32, u32) -> bool,
}
