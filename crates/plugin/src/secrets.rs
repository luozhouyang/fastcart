//! Encrypted per-plugin secrets.
//!
//! Secrets are encrypted with an authenticated cipher under a key derived
//! from the deployment's plugin context secret. The admin API is write-only:
//! it can list names and replace or delete values, but never reads them back.
//! A plugin worker reads only the secrets it asks for through the host API and
//! the `secrets.read` scope.

use chacha20poly1305::aead::{Aead as _, AeadCore as _, OsRng};
use chacha20poly1305::{KeyInit as _, XChaCha20Poly1305, XNonce};
use sha2::{Digest, Sha256};

use crate::error::PluginError;

/// Maximum plaintext size of one secret.
pub const MAX_SECRET_BYTES: usize = 16 * 1024;
/// Maximum length of a secret name.
pub const MAX_SECRET_NAME: usize = 64;

/// Encrypts and decrypts plugin secrets at rest.
#[derive(Clone)]
pub struct SecretCipher {
    cipher: XChaCha20Poly1305,
}

impl std::fmt::Debug for SecretCipher {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SecretCipher")
            .finish_non_exhaustive()
    }
}

impl SecretCipher {
    /// Derives the encryption key from the deployment secret.
    #[must_use]
    pub fn new(secret: &[u8]) -> Self {
        let mut hasher = Sha256::new();
        hasher.update(b"fastcart-plugin-secrets-v1\0");
        hasher.update(secret);
        let digest = hasher.finalize();
        let mut key = [0_u8; 32];
        key.copy_from_slice(&digest);
        Self {
            cipher: XChaCha20Poly1305::new((&key).into()),
        }
    }

    /// Encrypts one plaintext, returning `(ciphertext, nonce)`.
    ///
    /// # Errors
    ///
    /// Returns [`PluginError::Crypto`] when encryption fails.
    pub fn encrypt(&self, plaintext: &[u8]) -> Result<(Vec<u8>, Vec<u8>), PluginError> {
        let nonce = XChaCha20Poly1305::generate_nonce(&mut OsRng);
        let ciphertext = self
            .cipher
            .encrypt(&nonce, plaintext)
            .map_err(|_| PluginError::Crypto("secret encryption failed".to_owned()))?;
        Ok((ciphertext, nonce.to_vec()))
    }

    /// Decrypts one stored secret.
    ///
    /// # Errors
    ///
    /// Returns [`PluginError::Crypto`] when the nonce is malformed or the
    /// ciphertext fails authentication.
    pub fn decrypt(&self, nonce: &[u8], ciphertext: &[u8]) -> Result<Vec<u8>, PluginError> {
        let nonce: [u8; 24] = nonce
            .try_into()
            .map_err(|_| PluginError::Crypto("invalid secret nonce".to_owned()))?;
        self.cipher
            .decrypt(XNonce::from_slice(&nonce), ciphertext)
            .map_err(|_| PluginError::Crypto("secret decryption failed".to_owned()))
    }
}

/// Returns true for a valid secret name.
#[must_use]
pub fn valid_secret_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= MAX_SECRET_NAME
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_and_rejects_tampering() {
        let cipher = SecretCipher::new(b"deployment-secret");
        let (ciphertext, nonce) = cipher.encrypt(b"correct horse battery").unwrap();
        assert_eq!(
            cipher.decrypt(&nonce, &ciphertext).unwrap(),
            b"correct horse battery"
        );
        assert_ne!(ciphertext, b"correct horse battery");

        let mut tampered = ciphertext.clone();
        tampered[0] ^= 1;
        assert!(cipher.decrypt(&nonce, &tampered).is_err());
        assert!(cipher.decrypt(b"short", &ciphertext).is_err());
    }

    #[test]
    fn different_deployment_keys_do_not_interoperate() {
        let (ciphertext, nonce) = SecretCipher::new(b"one").encrypt(b"value").unwrap();
        assert!(
            SecretCipher::new(b"two")
                .decrypt(&nonce, &ciphertext)
                .is_err()
        );
    }

    #[test]
    fn validates_secret_names() {
        for name in ["api_key", "stripe.secret-key", "A1"] {
            assert!(valid_secret_name(name), "{name}");
        }
        for name in ["", "../key", "with space", "a/b", "ünicode"] {
            assert!(!valid_secret_name(name), "{name}");
        }
    }
}
