//! Embeds the icon, version info and manifest into OpenLinkHub.exe, and version info into
//! OpenLinkHubSensors.exe.

use std::path::Path;

fn main() {
    println!("cargo:rerun-if-changed=assets/OpenLinkHub.ico");
    println!("cargo:rerun-if-changed=app.manifest");
    println!("cargo:rerun-if-changed=build.rs");
    let root = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    let root = Path::new(&root);
    let out = std::env::var("OUT_DIR").unwrap();

    let version = std::env::var("CARGO_PKG_VERSION").unwrap();
    let mut parts: Vec<u16> = version.split('.').map(|p| p.parse().unwrap_or(0)).collect();
    parts.resize(4, 0);
    let v = format!("{},{},{},{}", parts[0], parts[1], parts[2], parts[3]);
    // rc.exe accepts forward slashes, which avoids escaping backslashes.
    let path = |p: &Path| p.display().to_string().replace('\\', "/");

    // OpenLinkHub.exe gets the icon and manifest; the service only needs version info.
    for (bin, description, extra) in [
        (
            "OpenLinkHub",
            "OpenLinkHub",
            format!(
                "1 ICON \"{}\"\n1 24 \"{}\"\n",
                path(&root.join("assets").join("OpenLinkHub.ico")),
                path(&root.join("app.manifest"))
            ),
        ),
        ("OpenLinkHubSensors", "OpenLinkHub Sensors", String::new()),
    ] {
        let rc = format!(
            r#"#include <winver.h>
{extra}VS_VERSION_INFO VERSIONINFO
FILEVERSION {v}
PRODUCTVERSION {v}
FILEOS VOS_NT_WINDOWS32
FILETYPE VFT_APP
BEGIN
  BLOCK "StringFileInfo"
  BEGIN
    BLOCK "040904b0"
    BEGIN
      VALUE "FileDescription", "{description}"
      VALUE "ProductName", "OpenLinkHub"
      VALUE "FileVersion", "{version}"
      VALUE "ProductVersion", "{version}"
      VALUE "OriginalFilename", "{bin}.exe"
      VALUE "LegalCopyright", "GPL-3.0-or-later"
    END
  END
  BLOCK "VarFileInfo"
  BEGIN
    VALUE "Translation", 0x409, 1200
  END
END
"#
        );
        let rc_path = Path::new(&out).join(format!("{bin}.rc"));
        std::fs::write(&rc_path, rc).unwrap();
        embed_resource::compile_for(&rc_path, [bin], embed_resource::NONE).manifest_required().unwrap();
    }
}
