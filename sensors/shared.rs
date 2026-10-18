//! Shared-memory block written by the sensor service and read by OpenLinkHub.
//!
//! Only SYSTEM and Administrators can write it; any signed-in user can read it. Readers use
//! the sequence number like a seqlock: odd means a write is in progress, and a read is only
//! valid if the number is even and unchanged across the copy.

#![allow(dead_code)]

pub const MAPPING_NAME: &str = "Global\\OpenLinkHubSensors";
pub const MAGIC: u32 = 0x5348_4C4F; // "OLHS"
pub const VERSION: u32 = 1;

pub const STATUS_STARTING: u32 = 1;
pub const STATUS_OK: u32 = 0;
pub const STATUS_NO_PAWNIO: u32 = 2;
pub const STATUS_UNSUPPORTED_CPU: u32 = 3;
pub const STATUS_MODULE_FAILED: u32 = 4;
pub const STATUS_READ_FAILED: u32 = 5;

pub const VENDOR_UNKNOWN: u32 = 0;
pub const VENDOR_AMD: u32 = 1;
pub const VENDOR_INTEL: u32 = 2;

#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct Shared {
    pub magic: u32,
    pub version: u32,
    pub seq: u32,
    pub status: u32,
    /// CPU package temperature, °C.
    pub cpu_temp: f32,
    pub vendor: u32,
    /// `GetTickCount64()` at the last update (system-wide, comparable across sessions).
    pub updated_tick_ms: u64,
    pub reserved: [u32; 8],
}

pub fn status_text(status: u32) -> &'static str {
    match status {
        STATUS_OK => "ok",
        STATUS_STARTING => "starting",
        STATUS_NO_PAWNIO => "PawnIO driver not installed",
        STATUS_UNSUPPORTED_CPU => "CPU not supported",
        STATUS_MODULE_FAILED => "PawnIO module failed to load",
        STATUS_READ_FAILED => "temperature read failed",
        _ => "unknown",
    }
}
