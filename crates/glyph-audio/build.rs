//! `cfg(glyph_mobile)` on Android/iOS: there the app shell (Compose/SwiftUI)
//! provides media keys and the embed webview, so the desktop backends are off.

fn main() {
    println!("cargo::rustc-check-cfg=cfg(glyph_mobile)");
    let os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    if os == "android" || os == "ios" {
        println!("cargo::rustc-cfg=glyph_mobile");
    }
}
