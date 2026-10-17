//! iCUE LINK System Hub wire protocol. Port of the transfer/read/write helpers in
//! OpenLinkHub `src/devices/lsh/lsh.go`.
//!
//! Every packet is a 512-byte output report on interface 0 (report id 0):
//! `[0x00, header, 0x01, command..., payload...]`, answered by one 512-byte input report.

use hidapi::HidDevice;
use std::io;

pub const BUFFER_SIZE: usize = 512;
const HEADER_SIZE: usize = 3;
const HEADER_WRITE_SIZE: usize = 4;
const READ_TIMEOUT_MS: i32 = 1000;
pub const MAX_BUFFER_PER_REQUEST: usize = 508;

pub const CMD_OPEN_ENDPOINT: &[u8] = &[0x0d, 0x01];
pub const CMD_OPEN_COLOR_ENDPOINT: &[u8] = &[0x0d, 0x00];
pub const CMD_CLOSE_ENDPOINT: &[u8] = &[0x05, 0x01, 0x01];
pub const CMD_CLOSE_COLOR_ENDPOINT: &[u8] = &[0x05, 0x01];
pub const CMD_GET_FIRMWARE: &[u8] = &[0x02, 0x13];
pub const CMD_SOFTWARE_MODE: &[u8] = &[0x01, 0x03, 0x00, 0x02];
pub const CMD_HARDWARE_MODE: &[u8] = &[0x01, 0x03, 0x00, 0x01];
pub const CMD_WRITE: &[u8] = &[0x06, 0x01];
pub const CMD_WRITE_COLOR: &[u8] = &[0x06, 0x00];
pub const CMD_READ: &[u8] = &[0x08, 0x01];
pub const CMD_READ_COLOR: &[u8] = &[0x08, 0x00];
pub const CMD_DEVICE_COMMAND_CODES: &[u8] = &[0x1e];
pub const CMD_DEVICE_COMMAND_LEDS: &[u8] = &[0x1d];

pub const MODE_GET_LEDS: &[u8] = &[0x20];
pub const MODE_GET_DEVICES: &[u8] = &[0x36];
pub const MODE_GET_TEMPERATURES: &[u8] = &[0x21];
pub const MODE_GET_SPEEDS: &[u8] = &[0x17];
pub const MODE_SET_SPEED: &[u8] = &[0x18];
pub const MODE_SET_COLOR: &[u8] = &[0x22];
pub const MODE_GET_PSU_VOLTS: &[u8] = &[0x28];
pub const MODE_GET_PSU_AMPS: &[u8] = &[0x29];
pub const MODE_GET_DEVICE_MODELS: &[u8] = &[0x40];

pub const DATA_TYPE_GET_DEVICES: &[u8] = &[0x21, 0x00];
pub const DATA_TYPE_GET_TEMPERATURES: &[u8] = &[0x10, 0x00];
pub const DATA_TYPE_GET_SPEEDS: &[u8] = &[0x25, 0x00];
pub const DATA_TYPE_SET_SPEED: &[u8] = &[0x07, 0x00];
pub const DATA_TYPE_SET_COLOR: &[u8] = &[0x12, 0x00];
pub const DATA_TYPE_SUB_COLOR: &[u8] = &[0x07, 0x00];

/// Header byte used for packets addressed to a PSU on the LINK chain.
pub const PSU_HEADER: u8 = 0x19;

/// Raw device handle. Callers serialise access through the hub's I/O mutex, which also
/// guards multi-packet sequences (open endpoint, read, close) against interleaving.
pub struct Transport {
    dev: HidDevice,
    pub debug: bool,
}

impl Transport {
    pub fn new(dev: HidDevice, debug: bool) -> Self {
        Transport { dev, debug }
    }

    pub fn product(&self) -> String {
        self.dev.get_product_string().ok().flatten().unwrap_or_default()
    }

    /// Send one command and return the device's reply. `header` is 0 for the hub itself,
    /// `PSU_HEADER` for a LINK PSU, or a channel id for V2 device model queries.
    pub fn transfer(&self, command: &[u8], payload: &[u8], header: u8) -> io::Result<[u8; BUFFER_SIZE]> {
        let mut w = [0u8; BUFFER_SIZE + 1];
        w[1] = header;
        w[2] = 0x01;
        let cmd_end = HEADER_SIZE + command.len();
        w[HEADER_SIZE..cmd_end].copy_from_slice(command);
        let n = payload.len().min(w.len() - cmd_end);
        w[cmd_end..cmd_end + n].copy_from_slice(&payload[..n]);

        self.dev.write(&w).map_err(io::Error::other)?;
        let mut r = [0u8; BUFFER_SIZE];
        let got = self.dev.read_timeout(&mut r, READ_TIMEOUT_MS).map_err(io::Error::other)?;
        if got == 0 {
            return Err(io::Error::new(io::ErrorKind::TimedOut, "no reply from hub"));
        }
        if self.debug {
            log::debug!("tx {:02x?} rx {:02x?}", &w[1..cmd_end + n.min(16)], &r[..16]);
        }
        Ok(r)
    }

    /// Fire-and-forget variant used during shutdown (Go: `transfer` with `d.Exit` set).
    pub fn send(&self, command: &[u8]) -> io::Result<()> {
        let mut w = [0u8; BUFFER_SIZE + 1];
        w[2] = 0x01;
        w[HEADER_SIZE..HEADER_SIZE + command.len()].copy_from_slice(command);
        self.dev.write(&w).map(|_| ()).map_err(io::Error::other)
    }

    /// Read a data endpoint: close, open, read (twice if the reply spans two packets), close.
    pub fn read(&self, endpoint: &[u8], data_type: &[u8], header: u8) -> io::Result<Vec<u8>> {
        let _ = self.transfer(CMD_CLOSE_ENDPOINT, endpoint, header);
        self.transfer(CMD_OPEN_ENDPOINT, endpoint, header)?;
        let first = self.transfer(CMD_READ, endpoint, header)?;
        let mut buffer = first.to_vec();
        if !data_type.is_empty() && first[4..6] == *data_type {
            let next = self.transfer(CMD_READ, endpoint, header)?;
            buffer.extend_from_slice(&next[4..]);
        }
        let _ = self.transfer(CMD_CLOSE_ENDPOINT, endpoint, header);
        Ok(buffer)
    }

    /// Read a device-data endpoint through the color channel (Go: `readDeviceData`).
    pub fn read_device_data(&self, endpoint: &[u8]) -> io::Result<[u8; BUFFER_SIZE]> {
        let mut open = CMD_OPEN_COLOR_ENDPOINT.to_vec();
        open.extend_from_slice(endpoint);
        self.transfer(&open, &[], 0)?;
        let buffer = self.transfer(CMD_READ_COLOR, &[], 0)?;
        let _ = self.transfer(CMD_CLOSE_COLOR_ENDPOINT, &[], 0);
        Ok(buffer)
    }

    /// Write `data` of type `data_type` to `endpoint`, returning the device's reply.
    pub fn write(&self, endpoint: &[u8], data_type: &[u8], data: &[u8]) -> io::Result<[u8; BUFFER_SIZE]> {
        let buffer = framed(data_type, data);
        self.transfer(CMD_CLOSE_ENDPOINT, endpoint, 0)?;
        self.transfer(CMD_OPEN_ENDPOINT, endpoint, 0)?;
        let reply = self.transfer(CMD_WRITE, &buffer, 0)?;
        self.transfer(CMD_CLOSE_ENDPOINT, endpoint, 0)?;
        Ok(reply)
    }

    /// Stream a color frame (packed RGB for every LED on the chain) to the open color endpoint.
    pub fn write_color(&self, data: &[u8]) -> io::Result<()> {
        let buffer = framed(DATA_TYPE_SET_COLOR, data);
        for (i, chunk) in buffer.chunks(MAX_BUFFER_PER_REQUEST).enumerate() {
            let command = if i == 0 { CMD_WRITE_COLOR } else { DATA_TYPE_SUB_COLOR };
            self.transfer(command, chunk, 0)?;
        }
        Ok(())
    }

    /// Re-open the color endpoint (Go: `setColorEndpoint`).
    pub fn open_color_endpoint(&self) -> io::Result<()> {
        let _ = self.transfer(CMD_CLOSE_ENDPOINT, MODE_SET_COLOR, 0);
        self.transfer(CMD_OPEN_COLOR_ENDPOINT, MODE_SET_COLOR, 0)?;
        Ok(())
    }
}

/// `[len+2 (u16 LE), 0x00, 0x00, data_type..., data...]`
fn framed(data_type: &[u8], data: &[u8]) -> Vec<u8> {
    let mut buffer = vec![0u8; HEADER_WRITE_SIZE + data_type.len() + data.len()];
    buffer[0..2].copy_from_slice(&((data.len() + 2) as u16).to_le_bytes());
    buffer[HEADER_WRITE_SIZE..HEADER_WRITE_SIZE + data_type.len()].copy_from_slice(data_type);
    buffer[HEADER_WRITE_SIZE + data_type.len()..].copy_from_slice(data);
    buffer
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn framing_matches_go() {
        // Go: buffer[0:2] = len(data)+2 (LE), bufferType at [4:], data after it.
        let f = framed(DATA_TYPE_SET_SPEED, &[1, 2, 0, 70, 0]);
        assert_eq!(f, vec![7, 0, 0, 0, 0x07, 0x00, 1, 2, 0, 70, 0]);
    }

    #[test]
    fn color_chunking_matches_go() {
        // 200 LEDs = 600 bytes + 6 header = 606 → chunks of 508 + 98.
        let buffer = framed(DATA_TYPE_SET_COLOR, &[0u8; 600]);
        let chunks: Vec<_> = buffer.chunks(MAX_BUFFER_PER_REQUEST).collect();
        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[0].len(), 508);
        assert_eq!(chunks[1].len(), 98);
    }
}
