//! Optional AEAD for tunnel payloads (ChaCha20-Poly1305).

use crate::{ProtocolError, Result};
use chacha20poly1305::{
    aead::{Aead, KeyInit},
    ChaCha20Poly1305, Nonce,
};
use sha2::{Digest, Sha256};

/// Session crypto derived from a pre-shared password.
#[derive(Clone)]
pub struct SessionCrypto {
    cipher: ChaCha20Poly1305,
}

impl std::fmt::Debug for SessionCrypto {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SessionCrypto(<redacted>)")
    }
}

impl SessionCrypto {
    /// Create from a raw 32-byte key.
    pub fn from_key(key: [u8; 32]) -> Self {
        Self {
            cipher: ChaCha20Poly1305::new((&key).into()),
        }
    }

    /// Encrypt payload; nonce is derived from sequence (unique per direction if keys differ,
    /// or we use sequence in nonce — both peers use independent TX sequences).
    pub fn encrypt(&self, sequence: u64, plaintext: &[u8]) -> Result<Vec<u8>> {
        let nonce = nonce_from_sequence(sequence);
        self.cipher
            .encrypt(&nonce, plaintext)
            .map_err(|_| ProtocolError::CryptoFailed)
    }

    /// Decrypt and authenticate payload.
    pub fn decrypt(&self, sequence: u64, ciphertext: &[u8]) -> Result<Vec<u8>> {
        let nonce = nonce_from_sequence(sequence);
        self.cipher
            .decrypt(&nonce, ciphertext)
            .map_err(|_| ProtocolError::CryptoFailed)
    }
}

fn nonce_from_sequence(sequence: u64) -> Nonce {
    let mut n = [0u8; 12];
    n[4..12].copy_from_slice(&sequence.to_be_bytes());
    Nonce::from(n)
}

/// Derive a 32-byte key from a password using SHA-256 (simple PSK KDF).
///
/// For production Internet exposure prefer WireGuard; this PSK is for LAN obfuscation.
pub fn derive_key_from_password(password: &str) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(b"bmw-enet-gateway-v1:");
    hasher.update(password.as_bytes());
    let out = hasher.finalize();
    let mut key = [0u8; 32];
    key.copy_from_slice(&out);
    key
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encrypt_roundtrip() {
        let key = derive_key_from_password("test-secret");
        let crypto = SessionCrypto::from_key(key);
        let pt = b"ethernet-frame-bytes";
        let ct = crypto.encrypt(7, pt).unwrap();
        assert_ne!(&ct, pt);
        let rt = crypto.decrypt(7, &ct).unwrap();
        assert_eq!(rt, pt);
    }

    #[test]
    fn tamper_detected() {
        let crypto = SessionCrypto::from_key(derive_key_from_password("x"));
        let mut ct = crypto.encrypt(1, b"abc").unwrap();
        ct[0] ^= 0xff;
        assert!(crypto.decrypt(1, &ct).is_err());
    }
}
