//! LCD wire format. Port of `transferToLcd`, `setLcdBrightness`, `setLcdRotation` and the LCD
//! feature reports in `Stop`/`StopDirty` from OpenLinkHub `src/devices/lsh/lsh.go`.
//!
//! Every buffer starts with its HID report id, exactly as upstream passes it to hidapi:
//! frames are output report `0x02`, settings are feature report `0x03`.

/// Corsair.
pub const VENDOR_ID: u16 = 0x1b1c;
/// iCUE LINK AIO LCD pump cover (Go: 3150).
pub const PRODUCT_AIO: u16 = 0x0c4e;
/// iCUE LINK XD5 Elite LCD pump/reservoir (Go: 3139).
pub const PRODUCT_XD5: u16 = 0x0c43;

/// Output report size, including the report id byte.
pub const BUFFER_SIZE: usize = 1024;
pub const HEADER_SIZE: usize = 8;
/// JPEG bytes carried by one packet.
pub const MAX_CHUNK: usize = BUFFER_SIZE - HEADER_SIZE;

/// Backlight off (Go: cmdLcdOff). Sent after a brightness of 0.
pub const REPORT_OFF: [u8; 4] = [0x03, 0x0b, 0x00, 0x01];
/// Sent on a clean shutdown (Go: Stop), handing the panel back to its firmware.
pub const REPORTS_STOP: [[u8; 4]; 3] = [[0x03, 0x1e, 0x01, 0x01], [0x03, 0x1d, 0x00, 0x01], [0x03, 0x0b, 0x64, 0x01]];
/// Sent on a dirty shutdown (Go: StopDirty): the first two `REPORTS_STOP`, brightness untouched.
pub const REPORTS_STOP_DIRTY: [[u8; 4]; 2] = [[0x03, 0x1e, 0x01, 0x01], [0x03, 0x1d, 0x00, 0x01]];

/// Backlight brightness, 0-100 (Go: cmdLcdBrightness with byte 2 replaced).
pub fn brightness_report(brightness: u8) -> [u8; 4] {
    [0x03, 0x0b, brightness, 0x01]
}

/// Panel rotation: 0 = default, 1 = 90°, 2 = 180°, 3 = 270°. The panel rotates the image
/// itself; frames are always rendered upright.
pub fn rotation_report(rotation: u8) -> [u8; 4] {
    [0x03, 0x0c, rotation, 0x01]
}

/// Split a JPEG into 1024-byte output reports:
/// `02 05 01 <last> <index> 00 <len u16 LE> <up to 1016 bytes of JPEG>`, zero padded.
///
/// As upstream, `last` is 0x01 on a packet shorter than 1016 bytes (the display renders the
/// frame on that packet), and the index byte wraps at 256.
pub fn packets(jpeg: &[u8]) -> Vec<Vec<u8>> {
    jpeg.chunks(MAX_CHUNK)
        .enumerate()
        .map(|(i, chunk)| {
            let mut p = vec![0u8; BUFFER_SIZE];
            p[0] = 0x02;
            p[1] = 0x05;
            p[2] = 0x01;
            if chunk.len() < MAX_CHUNK {
                p[3] = 0x01;
            }
            p[4] = i as u8;
            p[6..8].copy_from_slice(&(chunk.len() as u16).to_le_bytes());
            p[HEADER_SIZE..HEADER_SIZE + chunk.len()].copy_from_slice(chunk);
            p
        })
        .collect()
}
