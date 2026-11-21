//! SnowFishEmu Core
//!
//! Platform-independent emulator core for SnowFish (SFA) feature-phone
//! applications. This crate provides:
//! - `.app` package loading and relocation processing
//! - ARM/Thumb guest execution and platform service bridging
//! - Display, input, file, and timer services
//! - Framebuffer and runtime state management

mod compat;
pub mod consts;
pub mod cpu;
pub mod font;
pub mod gdi;
pub mod lz;
pub mod machine;
pub mod mem;
pub mod plat;
pub mod save;
pub mod sfa;

pub use cpu::ArmCpu;
pub use lz::LzError;
pub use machine::SnowFishMachine;
pub use mem::GuestMem;
pub use save::{SaveStateError, SAVE_STATE_FORMAT};
pub use sfa::{SfaError, SfaHeader, SfaImage};
