#![allow(clippy::unwrap_used)]

use ankou_proto::api::{ApiErrorCode, EnrollDeviceResponse, RegisterRequest};

#[test]
fn error_codes_are_stable() {
    use ApiErrorCode::*;
    let all = [
        (AuthInvalid, "E_AUTH_INVALID"),
        (AuthExpired, "E_AUTH_EXPIRED"),
        (InvitationInvalid, "E_INVITATION_INVALID"),
        (EmailTaken, "E_EMAIL_TAKEN"),
        (PasswordWeak, "E_PASSWORD_WEAK"),
        (DeviceLimit, "E_DEVICE_LIMIT"),
        (DeviceKeyTaken, "E_DEVICE_KEY_TAKEN"),
        (PoolExhausted, "E_POOL_EXHAUSTED"),
        (NotFound, "E_NOT_FOUND"),
        (RateLimited, "E_RATE_LIMITED"),
        (Validation, "E_VALIDATION"),
        (Internal, "E_INTERNAL"),
    ];
    for (code, text) in all {
        assert_eq!(code.as_str(), text);
        assert_eq!(serde_json::to_string(&code).unwrap(), format!("\"{text}\""));
    }
}

#[test]
fn enroll_response_roundtrip() {
    let json = r#"{"device":{"id":"0192d3a4-5b6c-7d8e-9f00-112233445566","name":"Laptop","public_key":"xTIBA5rboUvnH4htodjb6e697QjLERt1NAB4mZqp8Dg=","ipv4":"10.8.0.2","created_at":"2026-10-16T12:00:00Z"},"address":["10.8.0.2/32"],"servers":[{"id":"0192d3a4-5b6c-7d8e-9f00-aabbccddeeff","name":"vps1","country":"FR","city":"Paris","endpoint":"vps1.example.org:51820","public_key":"HIgo9xNzJMWLKASShiTqIybxZ0U3wGLiUeJ1PKf8ykw=","tags":["p2p"]}]}"#;
    let parsed: EnrollDeviceResponse = serde_json::from_str(json).unwrap();
    assert_eq!(
        parsed.servers[0].endpoint.to_string(),
        "vps1.example.org:51820"
    );
    assert_eq!(parsed.device.ipv4.to_string(), "10.8.0.2");
    assert_eq!(serde_json::to_string(&parsed).unwrap(), json);
}

#[test]
fn register_without_invitation_parses() {
    let r: RegisterRequest =
        serde_json::from_str(r#"{"email":"a@b.c","password":"0123456789ab"}"#).unwrap();
    assert_eq!(r.invitation_code, None);
}
