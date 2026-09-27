//! Platform boundaries: selection, replacement, application picking and system translation.
//! The shared translator never needs AppKit, UI Automation or clipboard details.
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "windows")]
mod windows;
#[cfg(target_os = "macos")]
pub use macos::Platform;
#[cfg(target_os = "windows")]
pub use windows::Platform;
