#![allow(clippy::unwrap_used)]

use ankou_server_domain::*;
use std::collections::BTreeSet;
use std::net::Ipv4Addr;
use time::{Duration, OffsetDateTime};

#[test]
fn normalizes_emails() {
    assert_eq!(
        normalize_email("  Alice@Example.ORG ").unwrap(),
        "alice@example.org"
    );
    let long = format!("{}@b.c", "a".repeat(251));
    for bad in ["a@@b", "@b", "a@", "", "ab", long.as_str()] {
        assert_eq!(
            normalize_email(bad),
            Err(DomainError::InvalidEmail),
            "{bad}"
        );
    }
}

#[test]
fn password_length_bounds() {
    assert_eq!(
        validate_password(&"a".repeat(11)),
        Err(DomainError::PasswordWeak)
    );
    assert_eq!(validate_password(&"a".repeat(12)), Ok(()));
    assert_eq!(validate_password(&"é".repeat(12)), Ok(()));
    assert_eq!(
        validate_password(&"a".repeat(129)),
        Err(DomainError::PasswordWeak)
    );
}

#[test]
fn device_name_rules() {
    assert_eq!(validate_device_name(" Laptop ").unwrap(), "Laptop");
    for bad in ["", "   ", "a\nb", &"x".repeat(65)] {
        assert_eq!(
            validate_device_name(bad),
            Err(DomainError::InvalidDeviceName),
            "{bad:?}"
        );
    }
}

#[test]
fn invitation_rules() {
    let now = OffsetDateTime::now_utc();
    let inv = |expires_at, used| InvitationState { expires_at, used };
    assert_eq!(
        check_invitation(&inv(now + Duration::days(1), true), now),
        Err(DomainError::InvitationInvalid)
    );
    assert_eq!(
        check_invitation(&inv(now, false), now),
        Err(DomainError::InvitationInvalid)
    );
    assert_eq!(
        check_invitation(&inv(now + Duration::seconds(1), false), now),
        Ok(())
    );
}

#[test]
fn device_limit_rules() {
    assert_eq!(check_device_limit(3, None), Ok(()));
    assert_eq!(check_device_limit(2, Some(3)), Ok(()));
    assert_eq!(
        check_device_limit(3, Some(3)),
        Err(DomainError::DeviceLimit)
    );
}

#[test]
fn allocates_lowest_free_address() {
    let pool = "10.8.0.0/16".parse().unwrap();
    assert_eq!(
        allocate_ipv4(pool, &BTreeSet::new()).unwrap(),
        Ipv4Addr::new(10, 8, 0, 2)
    );
    let used = BTreeSet::from([Ipv4Addr::new(10, 8, 0, 2), Ipv4Addr::new(10, 8, 0, 4)]);
    assert_eq!(
        allocate_ipv4(pool, &used).unwrap(),
        Ipv4Addr::new(10, 8, 0, 3)
    );
}

#[test]
fn pool_exhaustion() {
    let p30 = "10.9.0.0/30".parse().unwrap();
    assert_eq!(
        allocate_ipv4(p30, &BTreeSet::from([Ipv4Addr::new(10, 9, 0, 2)])),
        Err(DomainError::PoolExhausted)
    );
    let p29: ipnet::Ipv4Net = "10.9.0.0/29".parse().unwrap();
    let all: BTreeSet<_> = p29.hosts().collect();
    assert_eq!(allocate_ipv4(p29, &all), Err(DomainError::PoolExhausted));
}

#[test]
fn hashes_secrets() {
    let h = hash_secret("abc");
    let hex: String = h.iter().map(|b| format!("{b:02x}")).collect();
    assert_eq!(
        hex,
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
    assert_ne!(hash_secret("abd"), h);
}
