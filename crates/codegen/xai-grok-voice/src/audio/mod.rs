//! Microphone capture (optional `audio` feature).
//!
//! Two backends share one interface (`spawn_pcm_capture`,
//! `capture_pcm_for_duration`, `CaptureHandle`):
//! - macOS/Windows: `cpal` (coreaudio/wasapi), linked into the binary;
//! - Linux and Android: a subprocess recorder (`pw-record`/`parec`/`arecord`),
//!   because the static-musl release binary cannot link `cpal` -> `alsa-sys`.
//!   See [`capture_linux`] for the full rationale. Android's `cpal` backend
//!   pulls in `oboe`, a C++ library — the NDK's static libc++ has known
//!   missing-symbol linker issues that make it not worth chasing (see the
//!   `xai-grok-voice` Cargo.toml `cpal` cfg comment); none of the subprocess
//!   recorders exist under Termux either, so this is "compiles cleanly,
//!   reports unavailable at runtime" rather than working mic capture — same
//!   honest degradation as the clipboard/telemetry/sandbox Android stubs.
//!   A real Termux backend would shell out to `termux-microphone-record`
//!   instead, which nothing here does yet.

#[cfg(not(any(target_os = "linux", target_os = "android")))]
mod capture;
#[cfg(not(any(target_os = "linux", target_os = "android")))]
pub use capture::{CaptureHandle, capture_pcm_for_duration, spawn_pcm_capture};

#[cfg(any(target_os = "linux", target_os = "android"))]
mod capture_linux;
#[cfg(any(target_os = "linux", target_os = "android"))]
pub use capture_linux::{CaptureHandle, capture_pcm_for_duration, spawn_pcm_capture};
