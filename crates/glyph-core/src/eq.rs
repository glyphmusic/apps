//! Equalizer presets the user saved, and how they are shared.
//!
//! A share code is `glyph:eq:` + base64url(JSON); a preset file (`.glyph-eq`)
//! holds the same JSON, readable. [`decode`] takes either. The JSON shape
//! ([`EqPreset`]) is what other exports (all settings) can embed as is.

use anyhow::{anyhow, Context, Result};
use base64::Engine;
use serde::{Deserialize, Serialize};

pub const PREFIX: &str = "glyph:eq:";
/// File extension for exported presets (without the dot).
pub const FILE_EXT: &str = "glyph-eq";
/// Number of bands (32 Hz .. 16 kHz).
pub const BANDS: usize = 10;
/// Gain range per band (dB).
pub const MAX_DB: f32 = 12.0;
/// Longest preset name kept.
const MAX_NAME: usize = 40;

/// A named equalizer curve.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EqPreset {
    pub name: String,
    /// Gain per band in dB, 32 Hz .. 16 kHz.
    pub gains: Vec<f32>,
}

#[derive(Serialize, Deserialize)]
struct File {
    #[serde(default = "one")]
    v: u8,
    #[serde(flatten)]
    preset: EqPreset,
}

fn one() -> u8 {
    1
}

impl EqPreset {
    pub fn new(name: &str, gains: [f32; BANDS]) -> Self {
        Self { name: clean_name(name), gains: gains.to_vec() }
    }

    /// The ten gains, clamped (missing bands are 0 dB).
    pub fn gains(&self) -> [f32; BANDS] {
        sanitize(&self.gains)
    }
}

/// Ten finite gains within ±[`MAX_DB`], rounded to 0.1 dB.
pub fn sanitize(v: &[f32]) -> [f32; BANDS] {
    let mut g = [0.0; BANDS];
    for (i, x) in v.iter().take(BANDS).enumerate() {
        g[i] = if x.is_finite() { (x.clamp(-MAX_DB, MAX_DB) * 10.0).round() / 10.0 } else { 0.0 };
    }
    g
}

/// The same curve (within 0.05 dB per band).
pub fn same_gains(a: &[f32; BANDS], b: &[f32; BANDS]) -> bool {
    a.iter().zip(b).all(|(x, y)| (x - y).abs() < 0.05)
}

pub fn clean_name(name: &str) -> String {
    name.split_whitespace().collect::<Vec<_>>().join(" ").chars().take(MAX_NAME).collect()
}

/// Share code for the clipboard.
pub fn encode(p: &EqPreset) -> String {
    let json = serde_json::to_vec(&File { v: 1, preset: EqPreset::new(&p.name, p.gains()) }).unwrap_or_default();
    format!("{PREFIX}{}", base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(json))
}

/// Contents of a `.glyph-eq` file.
pub fn to_file(p: &EqPreset) -> String {
    serde_json::to_string_pretty(&File { v: 1, preset: EqPreset::new(&p.name, p.gains()) }).unwrap_or_default()
}

/// Text that looks like an equalizer share code.
pub fn is_code(s: &str) -> bool {
    s.trim().starts_with(PREFIX)
}

/// Reads a share code or a preset file's JSON.
pub fn decode(s: &str) -> Result<EqPreset> {
    let s = s.trim();
    let json = match s.strip_prefix(PREFIX) {
        Some(body) => base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(body.trim().trim_end_matches('='))
            .context("broken equalizer code")?,
        None if s.starts_with('{') => s.as_bytes().to_vec(),
        None => return Err(anyhow!("not a Glyph equalizer code")),
    };
    if json.len() > 16 * 1024 {
        return Err(anyhow!("equalizer code too large"));
    }
    let f: File = serde_json::from_slice(&json).context("broken equalizer code")?;
    if f.preset.gains.len() != BANDS {
        return Err(anyhow!("equalizer code needs {BANDS} bands"));
    }
    let name = clean_name(&f.preset.name);
    Ok(EqPreset { name: if name.is_empty() { "Equalizer".into() } else { name }, gains: f.preset.gains().to_vec() })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let p = EqPreset::new("  My   bass ", [6.0, 5.0, 4.0, 2.5, 1.0, 0.0, 0.0, 0.0, 30.0, f32::NAN]);
        assert_eq!(p.name, "My bass");
        let code = encode(&p);
        assert!(is_code(&format!(" {code}\n")));
        let back = decode(&code).unwrap();
        assert_eq!(back.name, "My bass");
        assert_eq!(back.gains(), [6.0, 5.0, 4.0, 2.5, 1.0, 0.0, 0.0, 0.0, 12.0, 0.0]);
        assert_eq!(decode(&to_file(&p)).unwrap(), back);
        assert!(decode("glyph:eq:@@@").is_err());
        assert!(decode(r#"{"name":"x","gains":[1,2]}"#).is_err());
    }
}
