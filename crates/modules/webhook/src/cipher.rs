//! AES-256-GCM protection for webhook signing secrets at rest.

use std::fmt;
use std::sync::{Arc, OnceLock};

use fastcart_kernel::AppError;
use rand::RngCore;
use ring::aead::{AES_256_GCM, Aad, LessSafeKey, Nonce, UnboundKey};

/// Environment variable holding the 64 hex character (32 byte) master key.
pub const SECRET_KEY_ENV: &str = "FASTCART_WEBHOOK_SECRET_KEY";
/// Prefix identifying an encrypted secret value.
pub const ENCRYPTED_PREFIX: &str = "enc:v1:";

const NONCE_LEN: usize = 12;
const KEY_LEN: usize = 32;

static ENV_CIPHER: OnceLock<Option<SecretCipher>> = OnceLock::new();

/// Cipher used to encrypt and decrypt webhook signing secrets.
///
/// The key never leaves the process and the `Debug` implementation is
/// redacted so plaintext or key material cannot leak through logs.
#[derive(Clone)]
pub struct SecretCipher {
    key: Arc<LessSafeKey>,
}

impl fmt::Debug for SecretCipher {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("SecretCipher(..)")
    }
}

impl SecretCipher {
    /// Builds a cipher from a 64 hex character (32 byte) key.
    pub fn from_hex(encoded: &str) -> Result<Self, AppError> {
        let key_bytes: [u8; KEY_LEN] = hex::decode(encoded)
            .map_err(|_| invalid_key())?
            .try_into()
            .map_err(|_| invalid_key())?;
        let key = UnboundKey::new(&AES_256_GCM, &key_bytes).map_err(|_| invalid_key())?;
        Ok(Self {
            key: Arc::new(LessSafeKey::new(key)),
        })
    }

    /// Returns the cipher configured by [`SECRET_KEY_ENV`].
    ///
    /// The environment is parsed once per process. When the variable is
    /// absent, `None` keeps legacy plaintext storage for local development;
    /// when it is present but invalid, this panics at first use so a
    /// misconfigured deployment fails fast instead of silently storing
    /// plaintext.
    pub fn from_env() -> Option<Self> {
        ENV_CIPHER
            .get_or_init(|| match std::env::var(SECRET_KEY_ENV) {
                Ok(value) => Some(Self::from_hex(&value).unwrap_or_else(|error| {
                    panic!("{SECRET_KEY_ENV} is invalid: {error}");
                })),
                Err(std::env::VarError::NotPresent) => None,
                Err(error) => panic!("{SECRET_KEY_ENV} could not be read: {error}"),
            })
            .clone()
    }

    /// Returns whether a stored value uses the encrypted format.
    #[must_use]
    pub fn is_encrypted(value: &str) -> bool {
        value.starts_with(ENCRYPTED_PREFIX)
    }

    /// Encrypts a plaintext secret into `enc:v1:<nonce_hex>:<ciphertext_hex>`.
    pub fn encrypt(&self, plaintext: &str) -> Result<String, AppError> {
        let mut nonce_bytes = [0_u8; NONCE_LEN];
        rand::rngs::OsRng.fill_bytes(&mut nonce_bytes);
        let nonce = Nonce::assume_unique_for_key(nonce_bytes);
        let mut in_out = plaintext.as_bytes().to_vec();
        self.key
            .seal_in_place_append_tag(nonce, Aad::empty(), &mut in_out)
            .map_err(|_| AppError::internal("webhook secret encryption failed"))?;
        Ok(format!(
            "{ENCRYPTED_PREFIX}{}:{}",
            hex::encode(nonce_bytes),
            hex::encode(in_out)
        ))
    }

    /// Decrypts a stored secret. Plaintext (legacy) values pass through.
    pub fn decrypt(&self, stored: &str) -> Result<String, AppError> {
        let Some(rest) = stored.strip_prefix(ENCRYPTED_PREFIX) else {
            return Ok(stored.to_owned());
        };
        let (nonce_hex, ciphertext_hex) = rest.split_once(':').ok_or_else(decrypt_error)?;
        let nonce_bytes: [u8; NONCE_LEN] = hex::decode(nonce_hex)
            .map_err(|_| decrypt_error())?
            .try_into()
            .map_err(|_| decrypt_error())?;
        let mut in_out = hex::decode(ciphertext_hex).map_err(|_| decrypt_error())?;
        let nonce = Nonce::assume_unique_for_key(nonce_bytes);
        let plaintext = self
            .key
            .open_in_place(nonce, Aad::empty(), &mut in_out)
            .map_err(|_| decrypt_error())?;
        String::from_utf8(plaintext.to_vec()).map_err(|_| decrypt_error())
    }
}

fn invalid_key() -> AppError {
    AppError::internal(format!(
        "{SECRET_KEY_ENV} must be exactly 64 hex characters (32 bytes)"
    ))
}

fn decrypt_error() -> AppError {
    AppError::internal("stored webhook secret could not be decrypted")
}
