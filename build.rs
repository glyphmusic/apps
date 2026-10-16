fn main() {
    println!("cargo:rerun-if-changed=assets/icons/glyph.ico");
    #[cfg(windows)]
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        // Icon resource id 1 is what GPUI's Windows backend loads as the window icon.
        let dir = std::path::PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
        let ico = dir.join("assets").join("icons").join("glyph.ico");
        let out = std::path::PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("glyph.rc");
        let ico = ico.display().to_string().replace('\\', "/");
        std::fs::write(&out, format!("1 ICON \"{ico}\"\n")).unwrap();
        let _ = embed_resource::compile(&out, embed_resource::NONE);
    }
}
