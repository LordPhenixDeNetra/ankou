use ankou_proto::ErrorCode;
use lpdnsoft_i18n::{Localizer, t};

/// Message traduit pour un code d'erreur stable.
pub fn error_message(loc: &Localizer, code: ErrorCode, server: &str, detail: &str) -> String {
    match code {
        ErrorCode::TunnelPermission => t!(loc, "error-tunnel-permission"),
        ErrorCode::TunnelUnavailable => t!(loc, "error-tunnel-unavailable", detail = detail),
        ErrorCode::ServerUnreachable => t!(loc, "error-server-unreachable", server = server),
        ErrorCode::FirewallUnavailable => t!(loc, "error-firewall-unavailable", detail = detail),
        ErrorCode::ProfileMissing => t!(loc, "error-profile-missing"),
        ErrorCode::Internal => t!(loc, "error-internal", detail = detail),
    }
}
