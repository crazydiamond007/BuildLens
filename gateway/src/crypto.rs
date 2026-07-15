use aes_gcm::{
    Aes256Gcm, KeyInit,
    aead::{Aead, Payload},
};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use sha2::{Digest, Sha256};

const CIPHERTEXT_VERSION: u8 = 1;
const NONCE_LEN: usize = 12;
const TOKEN_BYTES: usize = 32;

#[derive(Clone)]
pub struct TokenCipher {
    cipher: Aes256Gcm,
}

#[derive(Debug, thiserror::Error)]
pub enum CryptoError {
    #[error("secure random generation failed")]
    Random,
    #[error("credential encryption failed")]
    Encryption,
    #[error("credential ciphertext is malformed")]
    MalformedCiphertext,
    #[error("credential decryption failed")]
    Decryption,
}

impl TokenCipher {
    pub fn new(key: &[u8; 32]) -> Self {
        Self {
            cipher: Aes256Gcm::new(key.into()),
        }
    }

    pub fn encrypt(&self, plaintext: &str) -> Result<Vec<u8>, CryptoError> {
        let mut nonce = [0_u8; NONCE_LEN];
        getrandom::fill(&mut nonce).map_err(|_| CryptoError::Random)?;

        let encrypted = self
            .cipher
            .encrypt(
                (&nonce).into(),
                Payload {
                    msg: plaintext.as_bytes(),
                    aad: b"buildlens.github-token.v1",
                },
            )
            .map_err(|_| CryptoError::Encryption)?;

        let mut output = Vec::with_capacity(1 + NONCE_LEN + encrypted.len());
        output.push(CIPHERTEXT_VERSION);
        output.extend_from_slice(&nonce);
        output.extend_from_slice(&encrypted);
        Ok(output)
    }

    #[allow(dead_code)]
    pub fn decrypt(&self, ciphertext: &[u8]) -> Result<String, CryptoError> {
        if ciphertext.len() <= 1 + NONCE_LEN || ciphertext[0] != CIPHERTEXT_VERSION {
            return Err(CryptoError::MalformedCiphertext);
        }

        let (nonce, encrypted) = ciphertext[1..].split_at(NONCE_LEN);
        let plaintext = self
            .cipher
            .decrypt(
                nonce.into(),
                Payload {
                    msg: encrypted,
                    aad: b"buildlens.github-token.v1",
                },
            )
            .map_err(|_| CryptoError::Decryption)?;

        String::from_utf8(plaintext).map_err(|_| CryptoError::Decryption)
    }
}

pub fn random_urlsafe(bytes: usize) -> Result<String, CryptoError> {
    let mut random = vec![0_u8; bytes];
    getrandom::fill(&mut random).map_err(|_| CryptoError::Random)?;
    Ok(URL_SAFE_NO_PAD.encode(random))
}

pub fn new_session_token() -> Result<String, CryptoError> {
    random_urlsafe(TOKEN_BYTES)
}

pub fn new_api_token() -> Result<String, CryptoError> {
    Ok(format!("blq_{}", random_urlsafe(TOKEN_BYTES)?))
}

pub fn sha256(value: &str) -> Vec<u8> {
    Sha256::digest(value.as_bytes()).to_vec()
}

pub fn sha256_hex(value: &str) -> String {
    Sha256::digest(value.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

pub fn verify_github_signature(secret: &str, body: &[u8], signature: &str) -> bool {
    let Some(hex) = signature.strip_prefix("sha256=") else {
        return false;
    };
    if hex.len() != 64 {
        return false;
    }
    let mut supplied = [0_u8; 32];
    for (index, pair) in hex.as_bytes().chunks_exact(2).enumerate() {
        let Some(high) = from_hex(pair[0]) else {
            return false;
        };
        let Some(low) = from_hex(pair[1]) else {
            return false;
        };
        supplied[index] = (high << 4) | low;
    }
    let expected = hmac_sha256(secret.as_bytes(), body);
    expected
        .iter()
        .zip(supplied)
        .fold(0_u8, |difference, (left, right)| {
            difference | (left ^ right)
        })
        == 0
}

fn hmac_sha256(key: &[u8], body: &[u8]) -> [u8; 32] {
    const BLOCK_SIZE: usize = 64;
    let mut normalized = [0_u8; BLOCK_SIZE];
    if key.len() > BLOCK_SIZE {
        normalized[..32].copy_from_slice(&Sha256::digest(key));
    } else {
        normalized[..key.len()].copy_from_slice(key);
    }
    let mut inner_pad = [0x36_u8; BLOCK_SIZE];
    let mut outer_pad = [0x5c_u8; BLOCK_SIZE];
    for index in 0..BLOCK_SIZE {
        inner_pad[index] ^= normalized[index];
        outer_pad[index] ^= normalized[index];
    }
    let mut inner = Sha256::new();
    inner.update(inner_pad);
    inner.update(body);
    let inner = inner.finalize();
    let mut outer = Sha256::new();
    outer.update(outer_pad);
    outer.update(inner);
    outer.finalize().into()
}

fn from_hex(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        b'A'..=b'F' => Some(value - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encrypted_token_round_trips_and_uses_a_fresh_nonce() {
        let cipher = TokenCipher::new(&[7_u8; 32]);
        let first = cipher.encrypt("github-token").unwrap();
        let second = cipher.encrypt("github-token").unwrap();

        assert_ne!(first, second);
        assert_eq!(cipher.decrypt(&first).unwrap(), "github-token");
        assert_eq!(cipher.decrypt(&second).unwrap(), "github-token");
    }

    #[test]
    fn api_tokens_have_the_public_prefix_and_are_not_repeated() {
        let first = new_api_token().unwrap();
        let second = new_api_token().unwrap();

        assert!(first.starts_with("blq_"));
        assert_ne!(first, second);
        assert_eq!(sha256(&first).len(), 32);
    }

    #[test]
    fn verifies_github_hmac_signature_in_constant_time() {
        let signature = "sha256=f7bc83f430538424b13298e6aa6fb143ef4d59a14946175997479dbc2d1a3cd8";
        assert!(verify_github_signature(
            "key",
            b"The quick brown fox jumps over the lazy dog",
            signature
        ));
        assert!(!verify_github_signature("key", b"changed", signature));
        assert!(!verify_github_signature("key", b"body", "sha256=invalid"));
    }
}
