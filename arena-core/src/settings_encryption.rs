//! AES-256-GCM encryption for sensitive settings (NFR-003).
//!
//! Key derivation: HKDF-SHA256 with JWT_SIGNING_KEY as IKM,
//! no salt, info = b"arena-ses-key-v1".
//!
//! Storage format: hex(12-byte-nonce || ciphertext)
//!
//! Shared by `server` and `game-server` so both can decrypt secrets
//! (e.g. the `openrouter_api_key` app_setting) from the same key.

use aes_gcm::{
    Aes256Gcm, Key, Nonce,
    aead::{Aead, KeyInit},
};
use hkdf::Hkdf;
use rand::{RngCore, rngs::OsRng};
use sha2::Sha256;

/// Error type for encryption/decryption operations.
#[derive(Debug, thiserror::Error)]
pub enum EncryptionError {
    #[error("hex decode error: {0}")]
    HexDecode(#[from] hex::FromHexError),
    #[error("stored value too short (need at least 12 bytes for nonce)")]
    TooShort,
    #[error("AES-GCM decryption failed (wrong key or corrupt data)")]
    DecryptionFailed,
}

/// Handles encryption and decryption of sensitive settings values.
///
/// Constructed once at startup from `JWT_SIGNING_KEY` and stored in `AppState`.
pub struct SettingsEncryption {
    key: Key<Aes256Gcm>,
}

impl SettingsEncryption {
    /// Derive encryption key from JWT signing key bytes via HKDF-SHA256.
    pub fn new(jwt_signing_key: &[u8]) -> Self {
        let hk = Hkdf::<Sha256>::new(None, jwt_signing_key);
        let mut key_bytes = [0u8; 32];
        hk.expand(b"arena-ses-key-v1", &mut key_bytes)
            .expect("HKDF expand failed (output too long)");
        Self {
            key: Key::<Aes256Gcm>::from(key_bytes),
        }
    }

    /// Encrypt a plaintext string.
    ///
    /// Returns `hex(nonce || ciphertext)`. A fresh 12-byte nonce is generated
    /// from `OsRng` on every call, so repeated encryptions of the same value
    /// produce different outputs.
    pub fn encrypt(&self, plaintext: &str) -> String {
        let cipher = Aes256Gcm::new(&self.key);
        let mut nonce_bytes = [0u8; 12];
        OsRng.fill_bytes(&mut nonce_bytes);
        let nonce = Nonce::from_slice(&nonce_bytes);
        let ciphertext = cipher
            .encrypt(nonce, plaintext.as_bytes())
            .expect("AES-GCM encryption failed");
        let mut combined = Vec::with_capacity(12 + ciphertext.len());
        combined.extend_from_slice(&nonce_bytes);
        combined.extend_from_slice(&ciphertext);
        hex::encode(combined)
    }

    /// Decrypt a stored value produced by [`encrypt`].
    ///
    /// Returns the original plaintext string on success.
    pub fn decrypt(&self, stored: &str) -> Result<String, EncryptionError> {
        let bytes = hex::decode(stored)?;
        if bytes.len() < 12 {
            return Err(EncryptionError::TooShort);
        }
        let (nonce_bytes, ciphertext) = bytes.split_at(12);
        let cipher = Aes256Gcm::new(&self.key);
        let nonce = Nonce::from_slice(nonce_bytes);
        let plaintext_bytes = cipher
            .decrypt(nonce, ciphertext)
            .map_err(|_| EncryptionError::DecryptionFailed)?;
        Ok(String::from_utf8_lossy(&plaintext_bytes).into_owned())
    }
}
