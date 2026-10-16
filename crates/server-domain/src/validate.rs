use crate::DomainError;
use time::OffsetDateTime;

const EMAIL_MAX: usize = 254;
const PASSWORD_MIN: usize = 12;
const PASSWORD_MAX: usize = 128;
const DEVICE_NAME_MAX: usize = 64;

/// E-mail sans espaces autour, en minuscules ; un seul `@`, parties non vides.
pub fn normalize_email(raw: &str) -> Result<String, DomainError> {
    let email = raw.trim().to_lowercase();
    let valid = email.chars().count() <= EMAIL_MAX
        && !email.chars().any(char::is_whitespace)
        && matches!(email.split_once('@'), Some((l, d)) if !l.is_empty() && !d.is_empty() && !d.contains('@'));
    if valid {
        Ok(email)
    } else {
        Err(DomainError::InvalidEmail)
    }
}

/// Longueur en caractères (pas en octets).
pub fn validate_password(pw: &str) -> Result<(), DomainError> {
    let n = pw.chars().count();
    if (PASSWORD_MIN..=PASSWORD_MAX).contains(&n) {
        Ok(())
    } else {
        Err(DomainError::PasswordWeak)
    }
}

pub fn validate_device_name(raw: &str) -> Result<String, DomainError> {
    let name = raw.trim();
    let n = name.chars().count();
    if (1..=DEVICE_NAME_MAX).contains(&n) && !name.chars().any(char::is_control) {
        Ok(name.to_owned())
    } else {
        Err(DomainError::InvalidDeviceName)
    }
}

pub struct InvitationState {
    pub expires_at: OffsetDateTime,
    pub used: bool,
}

pub fn check_invitation(inv: &InvitationState, now: OffsetDateTime) -> Result<(), DomainError> {
    if inv.used || inv.expires_at <= now {
        Err(DomainError::InvitationInvalid)
    } else {
        Ok(())
    }
}

pub fn check_device_limit(active_devices: u32, max: Option<u32>) -> Result<(), DomainError> {
    match max {
        Some(max) if active_devices >= max => Err(DomainError::DeviceLimit),
        _ => Ok(()),
    }
}
