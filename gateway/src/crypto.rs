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
}
