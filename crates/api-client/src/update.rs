//! Dernière release d'aNkou publiée sur GitHub (ADR 0011) : signalée, jamais installée seule.

use std::time::Duration;

const LATEST_API: &str = "https://api.github.com/repos/LordPhenixDeNetra/ankou/releases/latest";
/// Page de la dernière release, à ouvrir pour télécharger.
pub const RELEASES_PAGE: &str = "https://github.com/LordPhenixDeNetra/ankou/releases/latest";

/// `1.2.3` ou `v1.2.3` ; une préversion (`1.0.0-rc1`) ou un tag illisible → `None`.
fn parse(version: &str) -> Option<[u64; 3]> {
    let mut parts = version.strip_prefix('v').unwrap_or(version).split('.');
    let mut out = [0; 3];
    for slot in &mut out {
        *slot = parts.next()?.parse().ok()?;
    }
    parts.next().is_none().then_some(out)
}

/// Version du tag (sans `v`) si elle est plus récente que `current`.
pub fn newer_version(tag: &str, current: &str) -> Option<String> {
    let (latest, now) = (parse(tag)?, parse(current)?);
    (latest > now).then(|| tag.trim_start_matches('v').to_owned())
}

#[derive(serde::Deserialize)]
struct Release {
    tag_name: String,
}

/// Version plus récente que celle-ci, si GitHub en publie une. Toute erreur (hors ligne,
/// limite de l'API) donne `None` : rien à signaler.
pub async fn latest_release() -> Option<String> {
    let http = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .user_agent(concat!("ankou/", env!("CARGO_PKG_VERSION")))
        .build()
        .ok()?;
    let release: Release = http
        .get(LATEST_API)
        .send()
        .await
        .ok()?
        .error_for_status()
        .ok()?
        .json()
        .await
        .ok()?;
    newer_version(&release.tag_name, ankou_proto::VERSION)
}

#[cfg(test)]
mod tests {
    use super::newer_version;

    #[test]
    fn compares_versions() {
        assert_eq!(newer_version("v1.0.1", "1.0.0").as_deref(), Some("1.0.1"));
        assert_eq!(newer_version("1.10.0", "1.9.0").as_deref(), Some("1.10.0"));
        assert_eq!(newer_version("v1.0.0", "1.0.0"), None);
        assert_eq!(newer_version("v0.9.0", "1.0.0"), None);
        assert_eq!(newer_version("v1.1.0-rc1", "1.0.0"), None);
        assert_eq!(newer_version("nightly", "1.0.0"), None);
    }
}
