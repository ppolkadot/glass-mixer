//! Generates the application icon from `src/icon_art.rs` and embeds it, the
//! application manifest and version information into the executable.

#[path = "src/icon_art.rs"]
#[allow(dead_code)]
mod icon_art;

use std::{env, fs, path::PathBuf};

fn main() {
    println!("cargo:rerun-if-changed=src/icon_art.rs");
    println!("cargo:rerun-if-changed=assets/app.manifest");
    println!("cargo:rerun-if-changed=build.rs");

    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }

    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());

    let sizes = [16u32, 20, 24, 32, 40, 48, 64, 256];
    let images: Vec<_> = sizes.iter().map(|&s| (s, icon_art::render_app_icon(s, false))).collect();
    let ico_path = out_dir.join("app.ico");
    fs::write(&ico_path, icon_art::encode_ico(&images)).unwrap();

    let version = env::var("CARGO_PKG_VERSION").unwrap();
    let mut parts: Vec<u16> = version.split(['.', '-']).take(3).map(|p| p.parse().unwrap_or(0)).collect();
    parts.resize(4, 0);
    let numeric = parts.iter().map(u16::to_string).collect::<Vec<_>>().join(",");

    let rc = format!(
        r#"#pragma code_page(65001)
1 ICON "{ico}"
1 24 "{manifest}"

1 VERSIONINFO
FILEVERSION {numeric}
PRODUCTVERSION {numeric}
FILEOS 0x40004
FILETYPE 0x1
BEGIN
  BLOCK "StringFileInfo"
  BEGIN
    BLOCK "040904B0"
    BEGIN
      VALUE "CompanyName", "Glass Mixer"
      VALUE "FileDescription", "Glass Mixer"
      VALUE "FileVersion", "{version}"
      VALUE "InternalName", "glass-mixer"
      VALUE "OriginalFilename", "glass-mixer.exe"
      VALUE "ProductName", "Glass Mixer"
      VALUE "ProductVersion", "{version}"
    END
  END
  BLOCK "VarFileInfo"
  BEGIN
    VALUE "Translation", 0x409, 1200
  END
END
"#,
        ico = escape(&ico_path),
        manifest = escape(&manifest_dir.join("assets").join("app.manifest")),
    );
    let rc_path = out_dir.join("app.rc");
    fs::write(&rc_path, rc).unwrap();
    embed_resource::compile(&rc_path, embed_resource::NONE).manifest_required().unwrap();
}

fn escape(path: &std::path::Path) -> String {
    path.display().to_string().replace('\\', "\\\\")
}
