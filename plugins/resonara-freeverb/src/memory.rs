//! Compiler-generated struct copies/zeroing use these freestanding primitives.
//! Volatile byte operations prevent LLVM from rewriting them into calls to
//! themselves. No allocator, locks, or TLS are involved. Darwin also links its
//! mandatory libSystem; native Scarlet/Linux builds remain freestanding.
use core::{
    ffi::{c_int, c_void},
    ptr,
};

// Darwin requires this dependency for every dylib, even with freestanding
// compiler primitives. Native Scarlet/Linux builds remain libc-free.
#[cfg(target_os = "macos")]
#[link(name = "System")]
unsafe extern "C" {}

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

#[unsafe(no_mangle)]
unsafe extern "C" fn memcmp(a: *const c_void, b: *const c_void, size: usize) -> c_int {
    for i in 0..size {
        let x = unsafe { ptr::read_volatile(a.cast::<u8>().add(i)) };
        let y = unsafe { ptr::read_volatile(b.cast::<u8>().add(i)) };
        if x != y {
            return x as c_int - y as c_int;
        }
    }
    0
}
#[unsafe(no_mangle)]
unsafe extern "C" fn memmove(dst: *mut c_void, src: *const c_void, size: usize) -> *mut c_void {
    if (dst as usize) < (src as usize) {
        unsafe { memcpy(dst, src, size) };
    } else {
        for i in (0..size).rev() {
            let b = unsafe { ptr::read_volatile(src.cast::<u8>().add(i)) };
            unsafe {
                ptr::write_volatile(dst.cast::<u8>().add(i), b);
            }
        }
    }
    dst
}
