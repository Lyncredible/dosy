//! Platform-agnostic, deterministic PC / MS-DOS emulator core.
//!
//! No windowing, GPU, audio-device, thread or wall-clock dependencies live here.
//! See `docs/ARCHITECTURE.md` for the module layout and interfaces.

pub mod audio;
pub mod bus;
pub mod cpu;
pub mod hle;
pub mod video;

/// Drive abstraction used by the DOS kernel.
pub use dosy_vfs as vfs;

/// Crate version.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
