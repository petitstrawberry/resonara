//! The native loader ABI is intentionally isolated from CLAP and the audio path.
use crate::{Error, Result};
use std::{
    ffi::{CStr, CString, c_char, c_int, c_void},
    path::Path,
};

#[cfg(any(target_os = "linux", target_os = "macos", target_os = "scarlet"))]
unsafe extern "C" {
    #[cfg_attr(target_os = "linux", link_name = "dlopen")]
    fn dlopen(path: *const c_char, flags: c_int) -> *mut c_void;
    fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
    fn dlclose(handle: *mut c_void) -> c_int;
    fn dlerror() -> *const c_char;
}
// Scarlet imports these symbols from /bin/scarlet-ld. Linking scarlet-dl here
// would create a second, incorrect loader context. Darwin supplies them in System.
#[cfg(target_os = "linux")]
#[link(name = "dl")]
unsafe extern "C" {}

pub(crate) struct Library(*mut c_void);
// A library handle is never used in processing. All opens/closes/CLAP entry
// init/deinit calls are serialized by the library registry in lib.rs.
unsafe impl Send for Library {}
unsafe impl Sync for Library {}
impl Library {
    pub(crate) fn open(path: &Path) -> Result<Self> {
        #[cfg(any(target_os = "linux", target_os = "macos", target_os = "scarlet"))]
        {
            // Native binaries only; macOS .clap bundles need an explicitly chosen
            // Contents/MacOS binary. Do not guess an executable from untrusted metadata.
            let path = CString::new(path.as_os_str().as_encoded_bytes())
                .map_err(|_| Error::new("Plugin path contains a NUL byte"))?;
            #[cfg(target_os = "scarlet")]
            const FLAGS: c_int = 0x102; // Scarlet requires RTLD_NOW | RTLD_GLOBAL.
            #[cfg(target_os = "linux")]
            const FLAGS: c_int = 2; // RTLD_NOW | RTLD_LOCAL (LOCAL = 0).
            #[cfg(target_os = "macos")]
            const FLAGS: c_int = 2 | 4; // RTLD_NOW | RTLD_LOCAL.
            // SAFETY: valid terminated path; no loader work runs on the audio thread.
            let handle = unsafe { dlopen(path.as_ptr(), FLAGS) };
            if handle.is_null() {
                return Err(last_error("Cannot load CLAP library"));
            }
            Ok(Self(handle))
        }
        #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "scarlet")))]
        {
            let _ = path;
            Err(Error::new("CLAP loading is unsupported on this OS"))
        }
    }
    pub(crate) fn symbol(&self, name: &CStr) -> Result<*mut c_void> {
        #[cfg(any(target_os = "linux", target_os = "macos", target_os = "scarlet"))]
        {
            // SAFETY: self owns a live handle and name is NUL terminated.
            let ptr = unsafe {
                dlerror();
                dlsym(self.0, name.as_ptr())
            };
            if ptr.is_null() {
                return Err(last_error("CLAP entry symbol is missing"));
            }
            Ok(ptr)
        }
        #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "scarlet")))]
        {
            let _ = name;
            Err(Error::new("CLAP loading is unsupported on this OS"))
        }
    }
}
impl Drop for Library {
    fn drop(&mut self) {
        #[cfg(any(target_os = "linux", target_os = "macos", target_os = "scarlet"))]
        // SAFETY: all instances and entry callbacks have finished before this drop.
        unsafe {
            dlclose(self.0);
        }
    }
}
#[cfg(any(target_os = "linux", target_os = "macos", target_os = "scarlet"))]
fn last_error(prefix: &str) -> Error {
    // SAFETY: dlerror owns a thread-local NUL terminated string, copied immediately.
    let ptr = unsafe { dlerror() };
    let detail = if ptr.is_null() {
        "unknown loader error".into()
    } else {
        unsafe { CStr::from_ptr(ptr) }.to_string_lossy()
    };
    Error::new(format!("{prefix}: {detail}"))
}

/// Non-allocating OS-thread identity. On Scarlet the native std runtime establishes
/// one nonzero thread pointer per thread; no ELF TLS is required in the plug-in.
#[inline]
pub(crate) fn thread_token() -> usize {
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    {
        unsafe extern "C" {
            fn pthread_self() -> usize;
        }
        unsafe { pthread_self() }
    }
    #[cfg(all(target_os = "scarlet", target_arch = "aarch64"))]
    {
        let ptr: usize;
        unsafe {
            core::arch::asm!("mrs {p}, tpidr_el0", p = out(reg) ptr, options(nomem, nostack, preserves_flags));
        }
        ptr
    }
    #[cfg(all(target_os = "scarlet", target_arch = "riscv64"))]
    {
        let ptr: usize;
        unsafe {
            core::arch::asm!("mv {p}, tp", p = out(reg) ptr, options(nomem, nostack, preserves_flags));
        }
        ptr
    }
    #[cfg(not(any(
        target_os = "linux",
        target_os = "macos",
        all(
            target_os = "scarlet",
            any(target_arch = "aarch64", target_arch = "riscv64")
        )
    )))]
    {
        0
    }
}
