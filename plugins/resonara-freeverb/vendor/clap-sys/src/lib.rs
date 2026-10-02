#![no_std]
#![allow(non_camel_case_types)]
#![allow(clippy::legacy_numeric_constants)] // Preserve upstream API source spelling.

pub mod audio_buffer;
pub mod entry;
pub mod events;
pub mod ext;
pub mod factory;
pub mod fixedpoint;
pub mod host;
pub mod id;
pub mod plugin;
pub mod plugin_features;
pub mod process;
pub mod stream;
pub mod string_sizes;
pub mod version;

macro_rules! cstr {
    ($str:literal) => {
        unsafe { core::ffi::CStr::from_bytes_with_nul_unchecked(concat!($str, "\0").as_bytes()) }
    };
}
pub(crate) use cstr;
