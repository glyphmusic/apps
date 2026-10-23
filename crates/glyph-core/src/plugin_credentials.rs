//! Plugin-owned secrets only. Never falls back to plaintext files or Glyph's keys.
use anyhow::{anyhow, bail, Result};

fn key(plugin: &str, item: &crate::plugins::SettingItem) -> Result<String> {
    if !crate::plugins::valid_id(plugin) || !item.secret || item.id.is_empty() || item.id.len() > 40
        || !item.id.bytes().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || b"_-".contains(&c)) {
        bail!("invalid plugin credential id");
    }
    let scope = item.credential.as_ref().ok_or_else(|| anyhow!("credential has no destination binding"))?;
    use sha2::{Digest, Sha256};
    let scope: String = Sha256::digest(serde_json::to_vec(scope)?).iter().map(|b| format!("{b:02x}")).collect();
    Ok(format!("plugin:{plugin}:{}:{scope}", item.id))
}

pub fn load(plugin: &str, item: &crate::plugins::SettingItem) -> Option<String> {
    let key = key(plugin, item).ok()?;
    if let Some(store) = crate::account::secret_store() {
        return store.get(&key).filter(|s| !s.is_empty());
    }
    #[cfg(any(windows, target_os = "macos", target_os = "ios", target_os = "linux"))]
    return keyring::Entry::new("glyph-plugin", &key).ok()?.get_password().ok().filter(|s| !s.is_empty());
    #[cfg(not(any(windows, target_os = "macos", target_os = "ios", target_os = "linux")))]
    None
}

pub fn store(plugin: &str, item: &crate::plugins::SettingItem, value: &str) -> Result<()> {
    let key = key(plugin, item)?;
    if value.len() > 2000 { bail!("plugin credential is too long"); }
    if let Some(store) = crate::account::secret_store() {
        if value.is_empty() {
            store.delete(&key);
            if store.get(&key).is_none() { return Ok(()); }
        } else if store.set(&key, value) { return Ok(()); }
        bail!("could not save plugin credential in secure storage");
    }
    #[cfg(any(windows, target_os = "macos", target_os = "ios", target_os = "linux"))]
    {
        let entry = keyring::Entry::new("glyph-plugin", &key).map_err(|_| anyhow!("secure credential storage is unavailable"))?;
        if value.is_empty() {
            match entry.delete_credential() {
                Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
                Err(_) => Err(anyhow!("could not delete plugin credential")),
            }
        } else {
            entry.set_password(value).map_err(|_| anyhow!("could not save plugin credential in OS keychain"))
        }
    }
    #[cfg(not(any(windows, target_os = "macos", target_os = "ios", target_os = "linux")))]
    Err(anyhow!("secure credential storage is unavailable"))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn credentials_cannot_name_accounts_or_other_plugins() {
        let mut item: crate::plugins::SettingItem = serde_json::from_value(serde_json::json!({
            "id":"api_key", "type":"text", "secret":true,
            "credential":{"host":"api.example.com", "path":"/v1/", "location":"query", "name":"key"}
        })).unwrap();
        let original = key("one", &item).unwrap();
        assert_ne!(original, key("two", &item).unwrap());
        item.credential.as_mut().unwrap().host = "evil.example.com".into();
        assert_ne!(original, key("one", &item).unwrap());
        for id in ["../account", "account:token", "", "other/key"] { item.id = id.into(); assert!(key("one", &item).is_err()); }
        assert!(key("../one", &item).is_err());
    }
}
