//! Pare-feu du kill switch sous macOS : règles `pf` dans l'ancre `com.apple/ankou`
//! (ADR 0010). Elles survivent à un plantage d'`ankoud`, pas au redémarrage de la machine.

use crate::{Firewall, FirewallPolicy, PF_ANCHOR, PlatformError, pf_ruleset};
use std::io::Write;
use std::process::{Command, Stdio};

/// Jeton de `pfctl -E` : `pf` reste activé tant qu'un jeton est tenu, et revient à son
/// état d'avant (souvent désactivé) quand le dernier est rendu.
#[derive(Default)]
pub struct PfFirewall {
    token: Option<String>,
}

/// `pfctl` : sortie standard, ou erreur (sortie d'erreur).
fn pfctl(args: &[&str], stdin: Option<&str>) -> Result<(String, String), String> {
    let mut child = Command::new("pfctl")
        .args(args)
        .stdin(if stdin.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("pfctl introuvable : {e}"))?;
    if let (Some(text), Some(mut input)) = (stdin, child.stdin.take()) {
        input
            .write_all(text.as_bytes())
            .map_err(|e| e.to_string())?;
    }
    let out = child.wait_with_output().map_err(|e| e.to_string())?;
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    if out.status.success() {
        Ok((String::from_utf8_lossy(&out.stdout).into_owned(), stderr))
    } else {
        Err(stderr.trim().to_owned())
    }
}

impl Firewall for PfFirewall {
    /// Les nouvelles règles remplacent les anciennes d'un coup (`-f`), puis `pf` est activé.
    fn apply(&mut self, policy: &FirewallPolicy) -> Result<(), PlatformError> {
        pfctl(&["-a", PF_ANCHOR, "-f", "-"], Some(&pf_ruleset(policy)))
            .map_err(PlatformError::Unavailable)?;
        if self.token.is_none() {
            let (stdout, stderr) = pfctl(&["-E"], None).map_err(PlatformError::Unavailable)?;
            self.token = stdout
                .lines()
                .chain(stderr.lines())
                .find_map(|l| l.strip_prefix("Token : "))
                .map(|t| t.trim().to_owned());
        }
        Ok(())
    }

    // ponytail: après un plantage, le jeton est perdu et `pf` reste activé ; sans effet,
    // l'ancre étant vide. Le garder dans le dossier d'état si cela gêne.
    fn clear(&mut self) -> Result<(), PlatformError> {
        if self.is_active() {
            pfctl(&["-a", PF_ANCHOR, "-F", "rules"], None).map_err(PlatformError::Backend)?;
        }
        if let Some(token) = self.token.take() {
            let _ = pfctl(&["-X", &token], None);
        }
        Ok(())
    }

    fn is_active(&self) -> bool {
        pfctl(&["-a", PF_ANCHOR, "-s", "rules"], None).is_ok_and(|(out, _)| !out.trim().is_empty())
    }
}
