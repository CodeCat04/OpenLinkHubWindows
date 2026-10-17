//! Embeds the app icon, version info and manifest into OpenLinkHub.exe.

use std::path::Path;

fn main() {
    println!("cargo:rerun-if-changed=assets/OpenLinkHub.ico");
    println!("cargo:rerun-if-changed=app.manifest");
    let root = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    let root = Path::new(&root);
    let out = std::env::var("OUT_DIR").unwrap();

    let version = std::env::var("CARGO_PKG_VERSION").unwrap();
    let mut parts: Vec<u16> = version.split('.').map(|p| p.parse().unwrap_or(0)).collect();
    parts.resize(4, 0);
    let v = format!("{},{},{},{}", parts[0], parts[1], parts[2], parts[3]);
    // rc.exe accepts forward slashes, which avoids escaping backslashes.
    let path = |p: &Path| p.display().to_string().replace('\\', "/");

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
        ico = path(&root.join("assets").join("OpenLinkHub.ico")),
        manifest = path(&root.join("app.manifest")),
    );
    let rc_path = Path::new(&out).join("app.rc");
    std::fs::write(&rc_path, rc).unwrap();
    embed_resource::compile(&rc_path, embed_resource::NONE).manifest_required().unwrap();
}
