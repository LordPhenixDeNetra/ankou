use sha2::{Digest, Sha256};

/// Haché stocké en base pour un secret aléatoire (code d'invitation, jeton).
pub fn hash_secret(secret: &str) -> [u8; 32] {
    Sha256::digest(secret.as_bytes()).into()
}
