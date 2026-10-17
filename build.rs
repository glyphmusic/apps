fn main() {
    println!("cargo:rerun-if-changed=assets/icons/glyph.ico");
    #[cfg(windows)]
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        // Icon resource id 1 is what GPUI's Windows backend loads as the window icon.
        let dir = std::path::PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
        let ico = dir.join("assets").join("icons").join("glyph.ico");
        let out = std::path::PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("glyph.rc");
        let ico = ico.display().to_string().replace('\\', "/");
        // Version info: Windows shows FileDescription as the app's name (taskbar
        // jump list, Task Manager) instead of the file name.
        let version = std::env::var("CARGO_PKG_VERSION").unwrap();
        let nums: Vec<u32> = version.split(['.', '-']).take(3).map(|p| p.parse().unwrap_or(0)).collect();
        let n = |i: usize| nums.get(i).copied().unwrap_or(0);
        let (a, b, c) = (n(0), n(1), n(2));
        let rc = format!(
            r#"1 ICON "{ico}"
1 VERSIONINFO
FILEVERSION {a},{b},{c},0
PRODUCTVERSION {a},{b},{c},0
BEGIN
  BLOCK "StringFileInfo"
  BEGIN
    BLOCK "040904B0"
    BEGIN
      VALUE "FileDescription", "Glyph"
      VALUE "ProductName", "Glyph"
      VALUE "CompanyName", "glyphmusic"
      VALUE "FileVersion", "{version}"
      VALUE "ProductVersion", "{version}"
      VALUE "OriginalFilename", "glyph.exe"
    END
  END
  BLOCK "VarFileInfo"
  BEGIN
    VALUE "Translation", 0x409, 1200
  END
END
"#
        );
        std::fs::write(&out, rc).unwrap();
        let _ = embed_resource::compile(&out, embed_resource::NONE);
    }
}
