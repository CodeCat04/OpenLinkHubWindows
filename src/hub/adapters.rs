//! iCUE LINK Adapter: lighting attached through the adapter (LS350/LS430 strips, case
//! lighting, QX/LL/8-LED fans, LC100). Port of the LINK adapter handling in upstream
//! `lsh.go` (`UpdateLinkAdapter`, `getLedDeviceTypes`, `getDevicesValue`).
//!
//! The hub keeps a per-channel "device command code" table and an LED-count table. Choosing
//! what is connected to an adapter rewrites both and power-cycles the LED ports; the hub
//! remembers the configuration across restarts.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::sync::OnceLock;

const ADAPTERS_JSON: &str = include_str!("../../data/linkadapter.json");

#[derive(Debug, Clone, Deserialize)]
pub struct AdapterPart {
    pub name: String,
    pub total: u8,
}

/// One selectable configuration, e.g. "2x LS350 Aurora".
#[derive(Debug, Clone, Deserialize)]
pub struct AdapterDef {
    pub index: u32,
    pub name: String,
    pub command: u8,
    #[serde(default)]
    pub devices: BTreeMap<u8, AdapterPart>,
}

impl AdapterDef {
    pub fn total_leds(&self) -> u8 {
        self.devices.values().map(|d| d.total as u32).sum::<u32>().min(255) as u8
    }
}

/// For the UI: (index, name).
#[derive(Debug, Clone, Serialize)]
pub struct AdapterChoice {
    pub index: u32,
    pub name: String,
}

pub fn all() -> &'static [AdapterDef] {
    static DEFS: OnceLock<Vec<AdapterDef>> = OnceLock::new();
    DEFS.get_or_init(|| serde_json::from_str(ADAPTERS_JSON).expect("embedded linkadapter.json is valid"))
}

pub fn get(index: u32) -> Option<&'static AdapterDef> {
    all().iter().find(|a| a.index == index)
}

pub fn choices() -> Vec<AdapterChoice> {
    all().iter().map(|a| AdapterChoice { index: a.index, name: a.name.clone() }).collect()
}

/// The hub's LED type tables, read at start-up (Go: `getLedDeviceTypes`).
#[derive(Debug, Clone, Default)]
pub struct LedTables {
    /// Command code entries: `0x00` (none) or `0x01 <code>`, one per channel from 0.
    pub codes: Vec<u8>,
    /// Number of channels the code table covers.
    pub length: u8,
    /// LED-count table, written back with an adapter's LED total.
    pub leds: Vec<u8>,
}

impl LedTables {
    /// Parse replies to `read_device_data(0x1e)` and `read_device_data(0x1d)`.
    pub fn parse(codes: &[u8], leds: &[u8]) -> LedTables {
        let mut t = LedTables::default();
        if codes.len() > 7 {
            t.length = codes[6];
            t.codes = codes[7..].to_vec();
        }
        if leds.len() > 8 && leds[2] == 0x08 && leds[6] > 0 {
            let n = (leds[6] as usize * 2 - 1).min(leds.len() - 8);
            t.leds.extend_from_slice(&leds[6..8]);
            t.leds.extend_from_slice(&leds[8..8 + n]);
        }
        t
    }

    /// Command code of the device on `channel`, 0 if none (Go: `getDevicesValue`).
    pub fn code_for(&self, channel: u8) -> u8 {
        let (mut pos, mut index) = (0usize, 0u8);
        while pos < self.codes.len() {
            let has_code = self.codes[pos] == 0x01 && pos + 1 < self.codes.len();
            if index == channel {
                return if has_code { self.codes[pos + 1] } else { 0 };
            }
            pos += if has_code { 2 } else { 1 };
            index = index.wrapping_add(1);
        }
        0
    }
}

/// What occupies a channel when rebuilding the command code table.
#[derive(Debug, Clone, Copy)]
pub enum ChannelKind {
    Empty,
    /// A LINK adapter with this configuration index (0 = nothing connected).
    Adapter(u32),
    CommanderDuo,
    Device(u8),
}

/// Build the command code table for channels `1..=max_channel` (Go: `UpdateLinkAdapter`).
/// Upstream skips adapters that have never been configured, which misaligns the table when
/// a hub has several adapters; here every channel always gets exactly one entry.
pub fn command_table(channels: &BTreeMap<u8, ChannelKind>, max_channel: u8) -> Vec<u8> {
    let mut buf = vec![max_channel + 1, 0x00];
    for ch in 1..=max_channel {
        match channels.get(&ch).copied().unwrap_or(ChannelKind::Empty) {
            ChannelKind::Adapter(index) => match get(index).filter(|_| index > 0) {
                Some(def) => buf.extend_from_slice(&[0x01, def.command]),
                None => buf.push(0x00),
            },
            // Commander DUO with automatic LED detection (upstream's override disabled).
            ChannelKind::CommanderDuo => buf.extend_from_slice(&[0x01, 0x00]),
            ChannelKind::Device(code) if code > 0 => buf.extend_from_slice(&[0x01, code]),
            _ => buf.push(0x00),
        }
    }
    buf
}

/// Set the LED count for an adapter's channel in the LED table (Go: `updateAdapterLedsAmount`).
pub fn set_led_count(leds: &mut [u8], channel: u8, count: u8) -> bool {
    let idx = channel as usize * 2 + 1;
    match leds.get_mut(idx) {
        Some(v) => {
            *v = count;
            true
        }
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn definitions_parse() {
        assert_eq!(all().len(), 37);
        let two_ls350 = get(2).unwrap();
        assert_eq!(two_ls350.command, 1);
        assert_eq!(two_ls350.total_leds(), 80);
        assert_eq!(get(10).unwrap().total_leds(), 204); // 5000T case lighting
    }

    #[test]
    fn code_lookup_walks_variable_length_entries() {
        // ch0: none, ch1: code 0x07, ch2: none, ch3: code 0x01
        let t = LedTables { codes: vec![0x00, 0x01, 0x07, 0x00, 0x01, 0x01, 0x00], length: 4, leds: vec![] };
        assert_eq!(t.code_for(0), 0);
        assert_eq!(t.code_for(1), 0x07);
        assert_eq!(t.code_for(2), 0);
        assert_eq!(t.code_for(3), 0x01);
        assert_eq!(t.code_for(9), 0);
    }

    #[test]
    fn command_table_has_one_entry_per_channel() {
        let channels = BTreeMap::from([
            (1, ChannelKind::Device(0x07)),
            (2, ChannelKind::Adapter(2)),
            (3, ChannelKind::Adapter(0)),
            (5, ChannelKind::CommanderDuo),
        ]);
        assert_eq!(command_table(&channels, 5), vec![6, 0x00, 0x01, 0x07, 0x01, 0x01, 0x00, 0x00, 0x01, 0x00]);
    }

    #[test]
    fn led_table_parse_and_update() {
        let mut leds_reply = vec![0u8; 20];
        leds_reply[2] = 0x08;
        leds_reply[6] = 3; // 3 channels → 2 header bytes + 5 data bytes
        leds_reply[7] = 0x00;
        leds_reply[8..13].copy_from_slice(&[0xAA, 0, 0xBB, 0, 0xCC]);
        let mut t = LedTables::parse(&[0; 8], &leds_reply);
        assert_eq!(t.leds, vec![3, 0, 0xAA, 0, 0xBB, 0, 0xCC]);
        assert!(set_led_count(&mut t.leds, 2, 80));
        assert_eq!(t.leds[5], 80);
        assert!(!set_led_count(&mut t.leds, 9, 1));
    }
}
