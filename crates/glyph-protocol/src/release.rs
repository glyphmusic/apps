//! Signed releases.
//!
//! Every release of the app, the server and r-dlp ships `SHA256SUMS.txt`
//! and [`SIGNATURE_ASSET`], a minisign signature by the Glyph release key.
//! Its trusted comment names the repository and the tag, so a signature of
//! one release cannot vouch for another. Updaters check the signature first
//! and trust a checksum only from a list that passed: GitHub alone (an
//! account, a token, a workflow) is not enough to ship code to devices.

/// minisign public keys of the Glyph release key (base64, as in the `.pub`
/// file). During a key change both keys are listed for one release.
pub const PUBLIC_KEYS: &[&str] = &["RWQ2xWZMYa2l251YUhrekvWRC+kaD698lXz0itKGjvNYAauLRTWCzg9o"];

/// The checksum list every release publishes.
pub const CHECKSUM_ASSET: &str = "SHA256SUMS.txt";
/// Its signature.
pub const SIGNATURE_ASSET: &str = "SHA256SUMS.txt.minisig";

/// The trusted comment a release of `repo` (`owner/name`) at `tag` carries.
pub fn trusted_comment(repo: &str, tag: &str) -> String {
    format!("glyph-release repo={repo} tag={tag}")
}

/// Checks `minisig` over `sums` against [`PUBLIC_KEYS`] and the trusted
/// comment for `repo` and `tag`. `Err` holds a reason for the user.
#[cfg(feature = "release-verify")]
pub fn verify_checksums(sums: &[u8], minisig: &str, repo: &str, tag: &str) -> Result<(), String> {
    verify_with_keys(PUBLIC_KEYS, sums, minisig, repo, tag)
}

#[cfg(feature = "release-verify")]
fn verify_with_keys(keys: &[&str], sums: &[u8], minisig: &str, repo: &str, tag: &str) -> Result<(), String> {
    let sig = minisign_verify::Signature::decode(minisig).map_err(|_| "the release signature cannot be read".to_string())?;
    let want = trusted_comment(repo, tag);
    if sig.trusted_comment() != want {
        return Err(format!("the release signature is for \"{}\", not \"{want}\"", sig.trusted_comment()));
    }
    let signed = keys.iter().filter_map(|k| minisign_verify::PublicKey::from_base64(k).ok()).any(|k| k.verify(sums, &sig, false).is_ok());
    if signed {
        Ok(())
    } else {
        Err("the checksum list is not signed by the Glyph release key".into())
    }
}

#[cfg(all(test, feature = "release-verify"))]
mod tests {
    use super::*;

    // Test-only key pair, generated for these tests and used nowhere else.
    const TEST_KEY: &str = "RWSyLHYJmF9zPkyJKmuEUDg25SafKAKbKehYL948tB+qqAWrqAlLE+w5";
    const SUMS: &[u8] = b"0000000000000000000000000000000000000000000000000000000000000000  glyph.zip\n";
    const SIG: &str = "untrusted comment: signature from minisign secret key
RUSyLHYJmF9zPtzqACmbHTadkZw1GT8yRZL6KqZfrwqxw72Q0HnRHfzUiCZngHOHWGfp9iqorZF+CZ6gYE2yeTsXp0pGD86/jQY=
trusted comment: glyph-release repo=glyphmusic/apps tag=v9.9.9
ho77B/jSKGQjjlcZeRYypTgbSeZNnG+4sya6k5qbd0Rpq8R1ywkJ/gnNA2/Gq1+Kr2pvRy7+zBLTRJa1JpS8CA==
";

    #[test]
    fn signature_must_match_key_repo_and_tag() {
        assert_eq!(verify_with_keys(&[TEST_KEY], SUMS, SIG, "glyphmusic/apps", "v9.9.9"), Ok(()));
        assert!(verify_with_keys(&[TEST_KEY], SUMS, SIG, "glyphmusic/server", "v9.9.9").is_err(), "another repository");
        assert!(verify_with_keys(&[TEST_KEY], SUMS, SIG, "glyphmusic/apps", "v9.9.8").is_err(), "another tag");
        assert!(verify_with_keys(&[TEST_KEY], b"changed", SIG, "glyphmusic/apps", "v9.9.9").is_err(), "changed list");
        assert!(verify_with_keys(PUBLIC_KEYS, SUMS, SIG, "glyphmusic/apps", "v9.9.9").is_err(), "another key");
        assert!(verify_with_keys(&[TEST_KEY], SUMS, "garbage", "glyphmusic/apps", "v9.9.9").is_err());
        let forged = SIG.replace("tag=v9.9.9", "tag=v9.9.8");
        assert!(verify_with_keys(&[TEST_KEY], SUMS, &forged, "glyphmusic/apps", "v9.9.8").is_err(), "edited trusted comment");
    }
}
