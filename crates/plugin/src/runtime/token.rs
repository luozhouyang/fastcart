//! Short-lived plugin context tokens.
//!
//! Tokens are minted by the host for one request and forwarded to the plugin in
//! `x-fastcart-context-token`. They are never stored in the plugin environment.

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

const PREFIX: &str = "fc_ctx_";
const ISSUER: &str = "fastcart";
const AUDIENCE: &str = "plugin-api";
const VERSION: u8 = 1;

/// Actor bound to a context token.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TokenActor {
    /// An administrator acting in the admin UI.
    Admin {
        /// User id.
        id: String,
        /// Permissions held by the user at mint time.
        permissions: Vec<String>,
    },
    /// An anonymous or signed-in storefront visitor.
    Storefront {
        /// Customer id when signed in.
        id: Option<String>,
    },
    /// An event delivery.
    Event {
        /// Event id.
        id: String,
    },
    /// A scheduled or enqueued job.
    Job {
        /// Job id.
        id: String,
    },
}

impl TokenActor {
    /// Stable actor kind.
    #[must_use]
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Admin { .. } => "admin",
            Self::Storefront { .. } => "storefront",
            Self::Event { .. } => "event",
            Self::Job { .. } => "job",
        }
    }
}

/// Claims carried by a context token.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContextClaims {
    /// Token format version.
    pub v: u8,
    /// Issuer.
    pub iss: String,
    /// Plugin id.
    pub sub: String,
    /// Audience.
    pub aud: String,
    /// Store id.
    pub store: String,
    /// Artifact digest the token is bound to.
    pub digest: String,
    /// Effective scopes for this request.
    pub scopes: Vec<String>,
    /// Acting principal.
    pub actor: TokenActor,
    /// Single-use identifier.
    pub jti: String,
    /// Expiry, seconds since the Unix epoch.
    pub exp: i64,
}

impl ContextClaims {
    /// Builds claims for one request with a 30 second lifetime.
    #[must_use]
    pub fn new(
        plugin_id: impl Into<String>,
        store_id: impl Into<String>,
        digest: impl Into<String>,
        scopes: Vec<String>,
        actor: TokenActor,
        now: i64,
    ) -> Self {
        Self {
            v: VERSION,
            iss: ISSUER.to_owned(),
            sub: plugin_id.into(),
            aud: AUDIENCE.to_owned(),
            store: store_id.into(),
            digest: digest.into(),
            scopes,
            actor,
            jti: format!("jti_{}", uuid::Uuid::now_v7().simple()),
            exp: now + 30,
        }
    }
}

/// Token errors.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum TokenError {
    /// Malformed or unsigned token.
    #[error("malformed plugin token")]
    Malformed,
    /// Signature mismatch.
    #[error("invalid plugin token signature")]
    Signature,
    /// Token expired.
    #[error("plugin token expired")]
    Expired,
    /// Wrong issuer, audience or version.
    #[error("plugin token claims are not accepted")]
    Claims,
}

/// HMAC-SHA256 signer and verifier.
#[derive(Clone)]
pub struct TokenSigner {
    key: [u8; 32],
}

impl std::fmt::Debug for TokenSigner {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("TokenSigner")
            .finish_non_exhaustive()
    }
}

impl TokenSigner {
    /// Derives a signing key from the deployment secret.
    #[must_use]
    pub fn new(secret: &[u8]) -> Self {
        let mut hasher = Sha256::new();
        hasher.update(b"fastcart-plugin-context-v1\0");
        hasher.update(secret);
        let digest = hasher.finalize();
        let mut key = [0_u8; 32];
        key.copy_from_slice(&digest);
        Self { key }
    }

    /// Derives a signer bound to one installation at one digest.
    #[must_use]
    pub fn derive(&self, store: &str, plugin: &str, digest: &str) -> Self {
        let mut mac = Hmac::<Sha256>::new_from_slice(&self.key).expect("HMAC accepts any key size");
        mac.update(b"fastcart-plugin-installation-v1\0");
        mac.update(store.as_bytes());
        mac.update(b"\0");
        mac.update(plugin.as_bytes());
        mac.update(b"\0");
        mac.update(digest.as_bytes());
        let digest = mac.finalize().into_bytes();
        let mut key = [0_u8; 32];
        key.copy_from_slice(&digest);
        Self { key }
    }

    /// Decodes claims without checking the signature.
    ///
    /// Only use this to look up the installation key; never trust the result
    /// until [`TokenSigner::verify`] succeeds with the derived signer.
    ///
    /// # Errors
    ///
    /// Returns [`TokenError::Malformed`] for an unparsable token.
    pub fn inspect(token: &str) -> Result<ContextClaims, TokenError> {
        let rest = token.strip_prefix(PREFIX).ok_or(TokenError::Malformed)?;
        let (encoded, _) = rest.split_once('.').ok_or(TokenError::Malformed)?;
        let payload = URL_SAFE_NO_PAD
            .decode(encoded)
            .map_err(|_| TokenError::Malformed)?;
        serde_json::from_slice(&payload).map_err(|_| TokenError::Malformed)
    }

    /// Signs claims.
    ///
    /// # Errors
    ///
    /// Returns [`TokenError::Malformed`] if serialization fails.
    pub fn mint(&self, claims: &ContextClaims) -> Result<String, TokenError> {
        let payload = serde_json::to_vec(claims).map_err(|_| TokenError::Malformed)?;
        let encoded = URL_SAFE_NO_PAD.encode(payload);
        let signature = self.sign(encoded.as_bytes());
        Ok(format!("{PREFIX}{encoded}.{signature}"))
    }

    /// Verifies a token at `now` (seconds since the epoch).
    ///
    /// # Errors
    ///
    /// Returns signature, expiry or claim errors.
    pub fn verify(&self, token: &str, now: i64) -> Result<ContextClaims, TokenError> {
        let rest = token.strip_prefix(PREFIX).ok_or(TokenError::Malformed)?;
        let (encoded, signature) = rest.split_once('.').ok_or(TokenError::Malformed)?;
        let mut mac =
            Hmac::<Sha256>::new_from_slice(&self.key).map_err(|_| TokenError::Malformed)?;
        mac.update(encoded.as_bytes());
        let signature = URL_SAFE_NO_PAD
            .decode(signature)
            .map_err(|_| TokenError::Malformed)?;
        mac.verify_slice(&signature)
            .map_err(|_| TokenError::Signature)?;

        let payload = URL_SAFE_NO_PAD
            .decode(encoded)
            .map_err(|_| TokenError::Malformed)?;
        let claims: ContextClaims =
            serde_json::from_slice(&payload).map_err(|_| TokenError::Malformed)?;
        if claims.v != VERSION || claims.iss != ISSUER || claims.aud != AUDIENCE {
            return Err(TokenError::Claims);
        }
        if claims.exp <= now {
            return Err(TokenError::Expired);
        }
        if claims.sub.is_empty()
            || claims.store.is_empty()
            || claims.digest.is_empty()
            || claims.jti.is_empty()
        {
            return Err(TokenError::Claims);
        }
        Ok(claims)
    }

    fn sign(&self, payload: &[u8]) -> String {
        let mut mac = Hmac::<Sha256>::new_from_slice(&self.key).expect("HMAC accepts any key size");
        mac.update(payload);
        URL_SAFE_NO_PAD.encode(mac.finalize().into_bytes())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn claims(now: i64) -> ContextClaims {
        ContextClaims {
            v: VERSION,
            iss: ISSUER.to_owned(),
            sub: "chat".to_owned(),
            aud: AUDIENCE.to_owned(),
            store: "store_1".to_owned(),
            digest: "a".repeat(64),
            scopes: vec!["catalog.read".to_owned()],
            actor: TokenActor::Admin {
                id: "user_1".to_owned(),
                permissions: vec!["orders.read".to_owned()],
            },
            jti: "jti_1".to_owned(),
            exp: now + 30,
        }
    }

    #[test]
    fn round_trips_claims() {
        let signer = TokenSigner::new(b"secret");
        let token = signer.mint(&claims(100)).unwrap();
        assert!(token.starts_with(PREFIX));
        let verified = signer.verify(&token, 100).unwrap();
        assert_eq!(verified, claims(100));
    }

    #[test]
    fn rejects_tampering_and_expiry() {
        let signer = TokenSigner::new(b"secret");
        let token = signer.mint(&claims(100)).unwrap();
        let (encoded, signature) = token.strip_prefix(PREFIX).unwrap().split_once('.').unwrap();
        let payload = URL_SAFE_NO_PAD.decode(encoded).unwrap();
        let mut value: serde_json::Value = serde_json::from_slice(&payload).unwrap();
        value["store"] = serde_json::json!("store_2");
        let forged = format!(
            "{PREFIX}{}.{signature}",
            URL_SAFE_NO_PAD.encode(serde_json::to_vec(&value).unwrap())
        );
        assert_eq!(signer.verify(&forged, 100), Err(TokenError::Signature));
        assert_eq!(signer.verify(&token, 200), Err(TokenError::Expired));

        let other = TokenSigner::new(b"other");
        assert_eq!(other.verify(&token, 100), Err(TokenError::Signature));

        let master = TokenSigner::new(b"master");
        let bound = master.derive("store_1", "chat", &"a".repeat(64));
        let bound_token = bound.mint(&claims(100)).unwrap();
        assert!(
            master
                .derive("store_2", "chat", &"a".repeat(64))
                .verify(&bound_token, 100)
                .is_err()
        );
        assert!(
            master
                .derive("store_1", "chat", &"b".repeat(64))
                .verify(&bound_token, 100)
                .is_err()
        );
        let inspected = TokenSigner::inspect(&bound_token).unwrap();
        assert_eq!(inspected, claims(100));
    }
}
