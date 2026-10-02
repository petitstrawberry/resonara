//! Native audio adapters; the render engine remains platform-independent.
#[cfg(not(target_os = "scarlet"))]
mod desktop;
#[cfg(any(target_os = "scarlet", test))]
mod pcm;
#[cfg(target_os = "scarlet")]
mod scarlet;

#[cfg(not(target_os = "scarlet"))]
pub use desktop::Audio;
#[cfg(target_os = "scarlet")]
pub use scarlet::Audio;
