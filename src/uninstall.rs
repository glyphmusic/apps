//! `glyph --uninstall-cleanup`: run by the Windows uninstaller when the user
//! chooses to delete all app data (`packaging/windows/glyph.iss`). The
//! folders are deleted by the uninstaller itself; this removes what lives
//! outside them: sign-in tokens, settings keys and server credentials in the
//! Windows credential store.

/// Command-line flag.
pub const FLAG: &str = "--uninstall-cleanup";

/// Asked for on this start.
pub fn requested() -> bool {
    std::env::args().skip(1).any(|a| a == FLAG)
}

/// Glyph's entries as `keyring` names them (`<user>.<service>`).
fn is_glyph_target(target: &str) -> bool {
    if let Some(user) = target.strip_suffix(".glyph") {
        return user.starts_with("account:") || user.starts_with("settings-key:");
    }
    target.strip_suffix(".glyph-server-access").is_some_and(|h| !h.is_empty() && h.bytes().all(|b| b.is_ascii_hexdigit()))
}

/// Deletes Glyph's entries from the credential store.
pub fn forget_credentials() {
    #[cfg(windows)]
    unsafe {
        use windows::core::{PCWSTR, PWSTR};
        use windows::Win32::Security::Credentials::{CredDeleteW, CredEnumerateW, CredFree, CREDENTIALW, CRED_TYPE_GENERIC};

        let read = |p: PWSTR| if p.is_null() { String::new() } else { p.to_string().unwrap_or_default() };
        let mut count = 0u32;
        let mut list: *mut *mut CREDENTIALW = std::ptr::null_mut();
        if CredEnumerateW(PCWSTR::null(), None, &mut count, &mut list).is_err() {
            return;
        }
        let mut targets = Vec::new();
        for i in 0..count as usize {
            let c = &**list.add(i);
            let target = read(c.TargetName);
            // Written by `keyring` (its comment), not by another program.
            if c.Type == CRED_TYPE_GENERIC && read(c.Comment).starts_with("keyring") && is_glyph_target(&target) {
                targets.push(target);
            }
        }
        CredFree(list as *const _);
        for t in targets {
            let _ = CredDeleteW(&windows::core::HSTRING::from(t.as_str()), CRED_TYPE_GENERIC, None);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_glyph_entries() {
        assert!(is_glyph_target("account:4f2a.glyph"));
        assert!(is_glyph_target("settings-key:acc_1.glyph"));
        assert!(is_glyph_target("0a1b2c.glyph-server-access"));
        assert!(!is_glyph_target("someone.glyph"));
        assert!(!is_glyph_target("account:x.glyphs"));
        assert!(!is_glyph_target("not-hex!.glyph-server-access"));
        assert!(!is_glyph_target("git:https://github.com"));
    }
}
