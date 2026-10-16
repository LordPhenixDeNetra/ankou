//! Règles pures du serveur aNkou (spec §7) : aucune E/S, aucune horloge.

mod pool;
mod secret;
mod validate;

pub use pool::allocate_ipv4;
pub use secret::hash_secret;
pub use validate::{
    InvitationState, check_device_limit, check_invitation, normalize_email, validate_device_name,
    validate_password,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum DomainError {
    #[error("adresse e-mail invalide")]
    InvalidEmail,
    #[error("mot de passe trop court ou trop long")]
    PasswordWeak,
    #[error("nom d'appareil invalide")]
    InvalidDeviceName,
    #[error("invitation invalide, expirée ou déjà utilisée")]
    InvitationInvalid,
    #[error("nombre maximal d'appareils atteint")]
    DeviceLimit,
    #[error("plus d'adresse disponible")]
    PoolExhausted,
}
