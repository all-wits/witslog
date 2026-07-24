//! FR-P9-004: optional encryption-at-rest primitive.
//!
//! Full DB-level (SQLCipher-style) encryption is architecturally in tension
//! with this schema's FTS5 index and `GENERATED ALWAYS AS (json_extract(...))`
//! columns, both of which need plaintext to do their job — encrypting the
//! whole `events` table would silently break search and the hot query axes.
//! Given that and the cross-compile cost of vendoring SQLCipher (same
//! cost-vs-value tradeoff already made for winget/.deb/.rpm in P8), this
//! ships a real, tested field-level cipher (`FieldCipher`, AES-256-GCM) for
//! the free-form `metadata` blob — the one column not read by FTS or any
//! generated column — rather than a fake/partial claim of full-DB encryption.
//! Off by default; callers opt in with `EventBuilder::encrypt_metadata`.

use crate::event::EventBuilder;
use aes_gcm::aead::{Aead, KeyInit, OsRng};
use aes_gcm::{AeadCore, Aes256Gcm, Key, Nonce};
use base64::Engine;
use serde_json::Value as JsonValue;

const ENC_MARKER_KEY: &str = "__witslog_enc";

#[derive(Debug, thiserror::Error)]
pub enum CryptoError {
    #[error("encryption key must be 32 bytes (got {0})")]
    BadKeyLength(usize),
    #[error("key is not valid hex: {0}")]
    BadKeyHex(String),
    #[error("decryption failed (wrong key or corrupted data)")]
    DecryptFailed,
    #[error("payload is not valid base64: {0}")]
    BadPayloadEncoding(String),
}

/// AES-256-GCM field cipher. Construct from a raw 32-byte key or a 64-char
/// hex string (e.g. from config or the `WITSLOG_ENCRYPTION_KEY` env var).
pub struct FieldCipher {
    cipher: Aes256Gcm,
}

impl FieldCipher {
    pub fn new(key_bytes: &[u8]) -> Result<Self, CryptoError> {
        if key_bytes.len() != 32 {
            return Err(CryptoError::BadKeyLength(key_bytes.len()));
        }
        let key = Key::<Aes256Gcm>::from_slice(key_bytes);
        Ok(FieldCipher {
            cipher: Aes256Gcm::new(key),
        })
    }

    pub fn from_hex(hex_key: &str) -> Result<Self, CryptoError> {
        let bytes = hex_decode(hex_key).map_err(|e| CryptoError::BadKeyHex(e))?;
        Self::new(&bytes)
    }

    /// Reads a hex key from the given env var name. Returns `None` (not an
    /// error) when the var is unset — encryption stays off by default
    /// unless a key is explicitly provisioned.
    pub fn from_env(var_name: &str) -> Result<Option<Self>, CryptoError> {
        match std::env::var(var_name) {
            Ok(hex_key) => Ok(Some(Self::from_hex(&hex_key)?)),
            Err(_) => Ok(None),
        }
    }

    /// Encrypts a UTF-8 string, returning a base64 blob of `nonce || ciphertext`.
    pub fn encrypt_str(&self, plaintext: &str) -> String {
        let nonce = Aes256Gcm::generate_nonce(&mut OsRng);
        let ciphertext = self
            .cipher
            .encrypt(&nonce, plaintext.as_bytes())
            .expect("AES-GCM encryption over in-memory buffer cannot fail");

        let mut combined = Vec::with_capacity(nonce.len() + ciphertext.len());
        combined.extend_from_slice(&nonce);
        combined.extend_from_slice(&ciphertext);
        base64::engine::general_purpose::STANDARD.encode(combined)
    }

    pub fn decrypt_str(&self, encoded: &str) -> Result<String, CryptoError> {
        let combined = base64::engine::general_purpose::STANDARD
            .decode(encoded)
            .map_err(|e| CryptoError::BadPayloadEncoding(e.to_string()))?;

        if combined.len() < 12 {
            return Err(CryptoError::DecryptFailed);
        }
        let (nonce_bytes, ciphertext) = combined.split_at(12);
        let nonce = Nonce::from_slice(nonce_bytes);

        let plaintext = self
            .cipher
            .decrypt(nonce, ciphertext)
            .map_err(|_| CryptoError::DecryptFailed)?;

        String::from_utf8(plaintext).map_err(|_| CryptoError::DecryptFailed)
    }

    /// Wraps an entire JSON value as `{"__witslog_enc": "<base64>"}`.
    pub fn encrypt_json(&self, value: &JsonValue) -> JsonValue {
        let serialized = value.to_string();
        serde_json::json!({ ENC_MARKER_KEY: self.encrypt_str(&serialized) })
    }

    /// Reverses `encrypt_json`. If `value` isn't a recognized encrypted
    /// envelope, returns it unchanged (so callers can decrypt-if-needed).
    pub fn decrypt_json(&self, value: &JsonValue) -> Result<JsonValue, CryptoError> {
        match value.get(ENC_MARKER_KEY).and_then(|v| v.as_str()) {
            Some(encoded) => {
                let plaintext = self.decrypt_str(encoded)?;
                serde_json::from_str(&plaintext).map_err(|_| CryptoError::DecryptFailed)
            }
            None => Ok(value.clone()),
        }
    }
}

impl EventBuilder {
    /// Encrypts `metadata` at rest with `cipher` (FR-P9-004). Applied
    /// last, after `.metadata(...)` is set; a no-op if metadata is unset.
    /// Query engine and MCP tools see the opaque envelope for encrypted
    /// events — callers that need plaintext back must decrypt with the
    /// same `FieldCipher` after reading.
    pub fn encrypt_metadata(mut self, cipher: &FieldCipher) -> Self {
        if let Some(meta) = self.metadata_mut() {
            *meta = cipher.encrypt_json(meta);
        }
        self
    }
}

/// Read-side helper for `metadata` display (FR-P9-004 wiring): reverses
/// `encrypt_metadata` for output paths (CLI `get`/`query`/`export`, MCP
/// `get_event`/`explain_error`) without ever failing the read.
///
/// - Not an encrypted envelope (`__witslog_enc` marker absent, or `None`) →
///   returned unchanged. Covers plaintext/legacy rows and events with no
///   metadata at all.
/// - Envelope present, `cipher` given, decrypts successfully → plaintext.
/// - Envelope present but `cipher` is `None` (reader has no key) **or**
///   decryption fails (wrong/rotated key, corrupted data) → the string
///   placeholder `"<encrypted>"`. A reader without the key sees that the
///   field exists and is protected, never a crash and never raw ciphertext.
pub fn decrypt_metadata_for_display(
    metadata: Option<JsonValue>,
    cipher: Option<&FieldCipher>,
) -> Option<JsonValue> {
    let value = metadata?;
    if value.get(ENC_MARKER_KEY).and_then(|v| v.as_str()).is_none() {
        return Some(value);
    }
    match cipher {
        Some(c) => match c.decrypt_json(&value) {
            Ok(plaintext) => Some(plaintext),
            Err(_) => Some(JsonValue::String("<encrypted>".to_string())),
        },
        None => Some(JsonValue::String("<encrypted>".to_string())),
    }
}

fn hex_decode(s: &str) -> Result<Vec<u8>, String> {
    let s = s.trim();
    if s.len() % 2 != 0 {
        return Err("odd-length hex string".to_string());
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).map_err(|e| e.to_string()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_cipher() -> FieldCipher {
        FieldCipher::new(&[7u8; 32]).unwrap()
    }

    #[test]
    fn round_trips_a_string() {
        let cipher = test_cipher();
        let encrypted = cipher.encrypt_str("super secret");
        assert_ne!(encrypted, "super secret");
        assert_eq!(cipher.decrypt_str(&encrypted).unwrap(), "super secret");
    }

    #[test]
    fn wrong_key_fails_to_decrypt() {
        let cipher_a = FieldCipher::new(&[1u8; 32]).unwrap();
        let cipher_b = FieldCipher::new(&[2u8; 32]).unwrap();

        let encrypted = cipher_a.encrypt_str("data");
        let err = cipher_b.decrypt_str(&encrypted).unwrap_err();
        assert!(matches!(err, CryptoError::DecryptFailed));
    }

    #[test]
    fn round_trips_json_envelope() {
        let cipher = test_cipher();
        let original = serde_json::json!({"user_id": 42, "note": "pii here"});

        let encrypted = cipher.encrypt_json(&original);
        assert!(encrypted.get(ENC_MARKER_KEY).is_some());

        let decrypted = cipher.decrypt_json(&encrypted).unwrap();
        assert_eq!(decrypted, original);
    }

    #[test]
    fn decrypt_json_passes_through_non_envelope_values() {
        let cipher = test_cipher();
        let plain = serde_json::json!({"already": "plaintext"});
        assert_eq!(cipher.decrypt_json(&plain).unwrap(), plain);
    }

    #[test]
    fn rejects_wrong_key_length() {
        match FieldCipher::new(&[0u8; 16]) {
            Err(CryptoError::BadKeyLength(16)) => {}
            other => panic!("expected BadKeyLength(16), got {:?}", other.map(|_| ())),
        }
    }

    #[test]
    fn from_hex_round_trips() {
        let hex_key = "07".repeat(32);
        let cipher = FieldCipher::from_hex(&hex_key).unwrap();
        let encrypted = cipher.encrypt_str("hello");
        assert_eq!(cipher.decrypt_str(&encrypted).unwrap(), "hello");
    }

    #[test]
    fn from_env_returns_none_when_unset() {
        // Use a var name that is virtually guaranteed unset.
        assert!(FieldCipher::from_env("WITSLOG_TEST_UNSET_ENCRYPTION_KEY_XYZ")
            .unwrap()
            .is_none());
    }

    #[test]
    fn decrypt_for_display_passes_through_plaintext() {
        let plain = serde_json::json!({"a": 1});
        assert_eq!(
            decrypt_metadata_for_display(Some(plain.clone()), None),
            Some(plain)
        );
    }

    #[test]
    fn decrypt_for_display_passes_through_none() {
        assert_eq!(decrypt_metadata_for_display(None, None), None);
    }

    #[test]
    fn decrypt_for_display_decrypts_with_cipher() {
        let cipher = test_cipher();
        let original = serde_json::json!({"user": "x@y.com"});
        let encrypted = cipher.encrypt_json(&original);
        assert_eq!(
            decrypt_metadata_for_display(Some(encrypted), Some(&cipher)),
            Some(original)
        );
    }

    #[test]
    fn decrypt_for_display_placeholders_without_cipher() {
        let cipher = test_cipher();
        let encrypted = cipher.encrypt_json(&serde_json::json!({"user": "x@y.com"}));
        assert_eq!(
            decrypt_metadata_for_display(Some(encrypted), None),
            Some(JsonValue::String("<encrypted>".to_string()))
        );
    }

    #[test]
    fn decrypt_for_display_placeholders_on_wrong_key() {
        let cipher_a = test_cipher();
        let cipher_b = FieldCipher::new(&[9u8; 32]).unwrap();
        let encrypted = cipher_a.encrypt_json(&serde_json::json!({"user": "x@y.com"}));
        assert_eq!(
            decrypt_metadata_for_display(Some(encrypted), Some(&cipher_b)),
            Some(JsonValue::String("<encrypted>".to_string()))
        );
    }

    #[test]
    fn event_builder_encrypt_metadata_wraps_envelope() {
        let cipher = test_cipher();
        let event = EventBuilder::new("app", "boom")
            .metadata(serde_json::json!({"secret": "value"}))
            .encrypt_metadata(&cipher)
            .build();

        let stored = event.metadata.unwrap();
        assert!(stored.get(ENC_MARKER_KEY).is_some());

        let decrypted = cipher.decrypt_json(&stored).unwrap();
        assert_eq!(decrypted, serde_json::json!({"secret": "value"}));
    }
}
