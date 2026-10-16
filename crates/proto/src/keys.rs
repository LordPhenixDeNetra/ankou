use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use std::fmt;
use std::str::FromStr;
use x25519_dalek::StaticSecret;

#[derive(Debug, thiserror::Error)]
#[error("clé WireGuard invalide : base64 de 32 octets attendue")]
pub struct KeyError;

fn decode(s: &str) -> Result<[u8; 32], KeyError> {
    STANDARD
        .decode(s.trim())
        .ok()
        .and_then(|v| v.try_into().ok())
        .ok_or(KeyError)
}

/// Clé privée WireGuard. Jamais affichée par `Debug`.
// ponytail: pas d'effacement mémoire (zeroize) ; à ajouter quand la clé ira dans SecretStore (étape 3).
#[derive(Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct PrivateKey([u8; 32]);

impl PrivateKey {
    pub fn generate() -> Self {
        Self(StaticSecret::random().to_bytes())
    }

    pub fn public_key(&self) -> PublicKey {
        PublicKey(x25519_dalek::PublicKey::from(&StaticSecret::from(self.0)).to_bytes())
    }

    pub fn to_base64(&self) -> String {
        STANDARD.encode(self.0)
    }

    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

impl fmt::Debug for PrivateKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("PrivateKey([redacted])")
    }
}

impl FromStr for PrivateKey {
    type Err = KeyError;
    fn from_str(s: &str) -> Result<Self, KeyError> {
        decode(s).map(Self)
    }
}

impl TryFrom<String> for PrivateKey {
    type Error = KeyError;
    fn try_from(s: String) -> Result<Self, KeyError> {
        s.parse()
    }
}

impl From<PrivateKey> for String {
    fn from(k: PrivateKey) -> String {
        k.to_base64()
    }
}

/// Clé publique WireGuard, affichée en base64.
#[derive(Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct PublicKey([u8; 32]);

impl PublicKey {
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

impl fmt::Display for PublicKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&STANDARD.encode(self.0))
    }
}

impl fmt::Debug for PublicKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "PublicKey({self})")
    }
}

impl FromStr for PublicKey {
    type Err = KeyError;
    fn from_str(s: &str) -> Result<Self, KeyError> {
        decode(s).map(Self)
    }
}

impl TryFrom<String> for PublicKey {
    type Error = KeyError;
    fn try_from(s: String) -> Result<Self, KeyError> {
        s.parse()
    }
}

impl From<PublicKey> for String {
    fn from(k: PublicKey) -> String {
        k.to_string()
    }
}
