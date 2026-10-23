//! Hardware id the app sends to Glyph servers (`x-glyph-hardware-id`), so a
//! server operator can ban a device (`glyph remote ban hardware`) and the
//! ban survives reinstalling the app.
//!
//! It is the SHA-256 of the operating system's machine id and a Glyph-only
//! salt: the machine id itself never leaves the device, and the value differs
//! from anything other apps derive from the same id. Sources: Windows
//! `MachineGuid`, macOS `IOPlatformUUID`, Linux `/etc/machine-id`, Android
//! `ANDROID_ID` (handed over by the activity, see [`set_platform_id`]).

use sha2::{Digest, Sha256};
use std::sync::OnceLock;

const SALT: &str = "glyph-hardware-v1";

static PLATFORM_ID: OnceLock<String> = OnceLock::new();
static DESKTOP_ID: OnceLock<Option<String>> = OnceLock::new();

/// Machine id from the platform layer (Android: `Settings.Secure.ANDROID_ID`).
pub fn set_platform_id(id: &str) {
    let id = id.trim();
    if !id.is_empty() {
        let _ = PLATFORM_ID.set(id.to_string());
    }
}

/// Hex hash sent to servers; `None` when the platform has no machine id.
pub fn id() -> Option<String> {
    if let Some(raw) = PLATFORM_ID.get() {
        return Some(hash(raw));
    }
    DESKTOP_ID.get_or_init(|| machine_id().map(|raw| hash(&raw))).clone()
}

fn hash(raw: &str) -> String {
    Sha256::digest(format!("{SALT}|{}", raw.trim().to_ascii_lowercase()).as_bytes()).iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(target_os = "windows")]
fn machine_id() -> Option<String> {
    use winreg::enums::{HKEY_LOCAL_MACHINE, KEY_READ, KEY_WOW64_64KEY};
    let key = winreg::RegKey::predef(HKEY_LOCAL_MACHINE)
        .open_subkey_with_flags(r"SOFTWARE\Microsoft\Cryptography", KEY_READ | KEY_WOW64_64KEY)
        .ok()?;
    key.get_value::<String, _>("MachineGuid").ok().filter(|v| !v.trim().is_empty())
}

#[cfg(target_os = "macos")]
fn machine_id() -> Option<String> {
    let out = std::process::Command::new("/usr/sbin/ioreg").args(["-rd1", "-c", "IOPlatformExpertDevice"]).output().ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    let line = text.lines().find(|l| l.contains("\"IOPlatformUUID\""))?;
    line.rsplit('"').nth(1).map(str::to_string).filter(|v| !v.is_empty())
}

#[cfg(all(unix, not(any(target_os = "macos", target_os = "android", target_os = "ios"))))]
fn machine_id() -> Option<String> {
    ["/etc/machine-id", "/var/lib/dbus/machine-id"]
        .iter()
        .find_map(|p| std::fs::read_to_string(p).ok().map(|s| s.trim().to_string()).filter(|s| !s.is_empty()))
}

#[cfg(not(any(target_os = "windows", target_os = "macos", all(unix, not(any(target_os = "android", target_os = "ios"))))))]
fn machine_id() -> Option<String> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn id_is_a_salted_hash_not_the_machine_id() {
        let h = hash("4c4c4544-0042-3510-8051-b4c04f4e3332");
        assert_eq!(h.len(), 64);
        assert_eq!(h, hash(" 4C4C4544-0042-3510-8051-B4C04F4E3332 "), "case and spaces do not matter");
        assert!(!h.contains("4c4c4544"));
        assert_ne!(h, hash("other"));
        // Whatever this machine reports, it is never sent in the clear.
        if let (Some(id), Some(raw)) = (id(), machine_id()) {
            assert_ne!(id, raw);
        }
    }
}
