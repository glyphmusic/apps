//! Exposes the compile target triple to the crate (`GLYPH_TARGET`), used by
//! the updater to pick the matching release asset.
fn main() {
    let target = std::env::var("TARGET").unwrap_or_else(|_| "unknown".into());
    println!("cargo:rustc-env=GLYPH_TARGET={target}");
    println!("cargo:rerun-if-changed=build.rs");
}
