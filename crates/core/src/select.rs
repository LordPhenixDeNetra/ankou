//! Choix du serveur le plus rapide (spec §6.5), sans E/S : les mesures arrivent toutes faites.

use std::time::Duration;

/// Médiane ; nombre pair d'échantillons : moyenne des deux du milieu.
pub fn median(samples: &[Duration]) -> Option<Duration> {
    let mut s = samples.to_vec();
    s.sort();
    let mid = s.len() / 2;
    match s.len() {
        0 => None,
        n if n % 2 == 1 => s.get(mid).copied(),
        _ => Some((s[mid - 1] + s[mid]) / 2),
    }
}

/// Indice du candidat de plus petite médiane (charge non publiée : score = médiane) ;
/// candidats sans mesure ignorés ; égalité → le premier.
pub fn fastest(candidates: &[Vec<Duration>]) -> Option<usize> {
    candidates
        .iter()
        .enumerate()
        .filter_map(|(i, c)| median(c).map(|m| (m, i)))
        .min()
        .map(|(_, i)| i)
}
