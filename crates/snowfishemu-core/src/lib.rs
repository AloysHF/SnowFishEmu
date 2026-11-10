//! SnowFishEmu Core
//!
//! Platform-independent emulator core for SnowFish (SFA) feature-phone
//! applications. This crate provides:
//! - `.app` package loading and relocation processing
//! - ARM/Thumb guest execution and platform service bridging
//! - Display, input, file, and timer services
//! - Framebuffer and runtime state management

pub mod consts;
pub mod sfa;

pub use sfa::{SfaError, SfaHeader, SfaImage};
