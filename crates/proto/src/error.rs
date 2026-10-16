/// Codes d'erreur stables, traduits par les interfaces (spec §8.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum ErrorCode {
    #[serde(rename = "E_TUNNEL_PERMISSION")]
    TunnelPermission,
    #[serde(rename = "E_TUNNEL_UNAVAILABLE")]
    TunnelUnavailable,
    #[serde(rename = "E_SERVER_UNREACHABLE")]
    ServerUnreachable,
    #[serde(rename = "E_FIREWALL_UNAVAILABLE")]
    FirewallUnavailable,
    #[serde(rename = "E_PROFILE_MISSING")]
    ProfileMissing,
    #[serde(rename = "E_INTERNAL")]
    Internal,
}

impl ErrorCode {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::TunnelPermission => "E_TUNNEL_PERMISSION",
            Self::TunnelUnavailable => "E_TUNNEL_UNAVAILABLE",
            Self::ServerUnreachable => "E_SERVER_UNREACHABLE",
            Self::FirewallUnavailable => "E_FIREWALL_UNAVAILABLE",
            Self::ProfileMissing => "E_PROFILE_MISSING",
            Self::Internal => "E_INTERNAL",
        }
    }
}
