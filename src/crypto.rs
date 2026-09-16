//! Just enough crypto to trust a self-update.
//!
//! The free app has no module, no session, no token - the one thing it
//! needs to verify is that an update installer it downloaded was built by
//! us. That is an Ed25519 signature over the file's bytes, checked against
//! the public key below. The private half lives only in CI, so a man in
//! the middle who swaps the installer cannot forge it.

use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use sha2::{Digest, Sha256};

/// The public half of the release key - the same key the Pro app carries,
/// and public by nature: it verifies, it cannot sign. Kept in the source
/// rather than a build secret so a bad value cannot ship silently.
const RELEASE_PUBLIC_KEY_B64: &str = "M6+3EG/4JwpZMpX8/ibvYjoqJBrEhgvcy3k+Qqt0lsk=";

/// Verify an Ed25519 signature over the sha256 of some bytes. Ok only when
/// the signature was made by the release key over exactly these bytes.
pub fn verify_release(bytes: &[u8], signature: &[u8]) -> Result<(), String> {
    let key_bytes = base64_decode(RELEASE_PUBLIC_KEY_B64)
        .ok_or("the compiled-in release key is not base64")?;
    if key_bytes.len() != 32 {
        return Err("the compiled-in release key is not 32 bytes".into());
    }
    let mut key = [0u8; 32];
    key.copy_from_slice(&key_bytes);
    let verifying =
        VerifyingKey::from_bytes(&key).map_err(|_| "the compiled-in release key is not a key")?;
    if signature.len() != 64 {
        return Err("the signature is not 64 bytes".into());
    }
    let mut sig = [0u8; 64];
    sig.copy_from_slice(signature);
    let signature = Signature::from_bytes(&sig);
    let digest = Sha256::digest(bytes);
    verifying
        .verify(&digest, &signature)
        .map_err(|_| "the download is not signed by the release key".to_string())
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

pub fn base64_decode(text: &str) -> Option<Vec<u8>> {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.decode(text.trim()).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_release_key_is_a_valid_ed25519_key() {
        let bytes = base64_decode(RELEASE_PUBLIC_KEY_B64).expect("base64");
        assert_eq!(bytes.len(), 32);
        let mut key = [0u8; 32];
        key.copy_from_slice(&bytes);
        VerifyingKey::from_bytes(&key).expect("valid key");
    }
}
