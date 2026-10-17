//! Embeds the app icon, version info and manifest into OpenLinkHub.exe.

include!("src/icon.rs");

use std::path::Path;

fn main() {
    println!("cargo:rerun-if-changed=src/icon.rs");
    println!("cargo:rerun-if-changed=app.manifest");
    let out = std::env::var("OUT_DIR").unwrap();
    let out = Path::new(&out);

    std::fs::write(out.join("app.ico"), ico(&[16, 24, 32, 48, 64, 256])).unwrap();

    let version = std::env::var("CARGO_PKG_VERSION").unwrap();
    let mut parts: Vec<u16> = version.split('.').map(|p| p.parse().unwrap_or(0)).collect();
    parts.resize(4, 0);
    let v = format!("{},{},{},{}", parts[0], parts[1], parts[2], parts[3]);
    let manifest = Path::new(&std::env::var("CARGO_MANIFEST_DIR").unwrap()).join("app.manifest");
    let rc = format!(
        r#"#include <winver.h>
1 ICON "{ico}"
1 24 "{manifest}"
VS_VERSION_INFO VERSIONINFO
FILEVERSION {v}
PRODUCTVERSION {v}
FILEOS VOS_NT_WINDOWS32
FILETYPE VFT_APP
BEGIN
  BLOCK "StringFileInfo"
  BEGIN
    BLOCK "040904b0"
    BEGIN
      VALUE "FileDescription", "OpenLinkHub"
      VALUE "ProductName", "OpenLinkHub"
      VALUE "FileVersion", "{version}"
      VALUE "ProductVersion", "{version}"
      VALUE "OriginalFilename", "OpenLinkHub.exe"
      VALUE "LegalCopyright", "GPL-3.0-or-later"
    END
  END
  BLOCK "VarFileInfo"
  BEGIN
    VALUE "Translation", 0x409, 1200
  END
END
"#,
        ico = out.join("app.ico").display().to_string().replace('\\', "/"),
        manifest = manifest.display().to_string().replace('\\', "/"),
    );
    let rc_path = out.join("app.rc");
    std::fs::write(&rc_path, rc).unwrap();
    embed_resource::compile(&rc_path, embed_resource::NONE).manifest_required().unwrap();
}

/// Build a .ico with 32-bit BMP images at the given sizes.
fn ico(sizes: &[u32]) -> Vec<u8> {
    let images: Vec<Vec<u8>> = sizes.iter().map(|&s| bmp(s)).collect();
    let mut out = Vec::new();
    out.extend_from_slice(&[0, 0, 1, 0]);
    out.extend_from_slice(&(sizes.len() as u16).to_le_bytes());
    let mut offset = 6 + 16 * sizes.len() as u32;
    for (&s, img) in sizes.iter().zip(&images) {
        let dim = if s >= 256 { 0 } else { s as u8 };
        out.extend_from_slice(&[dim, dim, 0, 0, 1, 0, 32, 0]);
        out.extend_from_slice(&(img.len() as u32).to_le_bytes());
        out.extend_from_slice(&offset.to_le_bytes());
        offset += img.len() as u32;
    }
    for img in images {
        out.extend_from_slice(&img);
    }
    out
}

fn bmp(size: u32) -> Vec<u8> {
    let (px, _) = rgba(size);
    let mask_row = size.div_ceil(32) * 4;
    let mut out = Vec::new();
    for v in [40u32, size, size * 2] {
        out.extend_from_slice(&v.to_le_bytes());
    }
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&32u16.to_le_bytes());
    for v in [0u32, size * size * 4 + mask_row * size, 0, 0, 0, 0] {
        out.extend_from_slice(&v.to_le_bytes());
    }
    for y in (0..size).rev() {
        for x in 0..size {
            let i = ((y * size + x) * 4) as usize;
            out.extend_from_slice(&[px[i + 2], px[i + 1], px[i], px[i + 3]]);
        }
    }
    out.extend(std::iter::repeat_n(0u8, (mask_row * size) as usize));
    out
}
