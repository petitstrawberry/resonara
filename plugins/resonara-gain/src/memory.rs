//! Compiler-generated struct copies/zeroing use these freestanding primitives.
//! Volatile byte operations prevent LLVM from rewriting them into calls to
//! themselves. No platform libc, allocator, locks, or TLS are involved.
use core::{
    ffi::{c_int, c_void},
    ptr,
};

#[unsafe(no_mangle)]
unsafe extern "C" fn memcpy(dst: *mut c_void, src: *const c_void, size: usize) -> *mut c_void {
    for i in 0..size {
        let byte = unsafe { ptr::read_volatile(src.cast::<u8>().add(i)) };
        unsafe {
            ptr::write_volatile(dst.cast::<u8>().add(i), byte);
        }
    }
    dst
}
#[unsafe(no_mangle)]
unsafe extern "C" fn memset(dst: *mut c_void, byte: c_int, size: usize) -> *mut c_void {
    for i in 0..size {
        unsafe {
            ptr::write_volatile(dst.cast::<u8>().add(i), byte as u8);
        }
    }
    dst
}
