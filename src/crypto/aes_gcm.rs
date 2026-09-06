use aes_gcm::{
    aead::{Aead, KeyInit},
    Aes256Gcm, Nonce,
};
use rand::RngCore;

/// Encrypt is only used by legacy tooling/tests; new uploads are stored plaintext.
#[cfg_attr(not(test), allow(dead_code))]
pub fn encrypt(data: &[u8], key: &[u8; 32]) -> Result<(Vec<u8>, [u8; 12]), String> {
    let cipher = Aes256Gcm::new(key.into());
    let mut nonce_bytes = [0u8; 12];
    rand::thread_rng().fill_bytes(&mut nonce_bytes);

    let nonce = Nonce::from_slice(&nonce_bytes);
    let ciphertext = cipher
        .encrypt(nonce, data)
        .map_err(|e| format!("AES-256-GCM Encryption failed: {:?}", e))?;

    Ok((ciphertext, nonce_bytes))
}

pub fn decrypt(ciphertext: &[u8], nonce_bytes: &[u8; 12], key: &[u8; 32]) -> Result<Vec<u8>, String> {
    let cipher = Aes256Gcm::new(key.into());
    let nonce = Nonce::from_slice(nonce_bytes);

    let plaintext = cipher
        .decrypt(nonce, ciphertext)
        .map_err(|e| format!("AES-256-GCM Decryption failed: {:?}", e))?;

    Ok(plaintext)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_encrypt_decrypt_cycle() {
        let key = [42u8; 32];
        let original_data = b"Hello, Telegram S3 Storage with Zero-Knowledge Encryption!";

        let (encrypted, nonce) = encrypt(original_data, &key).expect("Encryption failed");
        assert_ne!(original_data.as_slice(), encrypted.as_slice());

        let decrypted = decrypt(&encrypted, &nonce, &key).expect("Decryption failed");
        assert_eq!(original_data.as_slice(), decrypted.as_slice());
    }
}
