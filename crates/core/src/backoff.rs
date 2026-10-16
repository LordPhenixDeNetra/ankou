use std::time::Duration;

const BASE_MS: u64 = 1_000;
const CAP_MS: u64 = 30_000;

/// Délai avant la tentative `attempt` (1-based) : 1 s × 2^(n−1) plafonné à 30 s,
/// gigue uniforme ±20 %, résultat plafonné à 30 s. `rng` : état xorshift64.
pub fn backoff(attempt: u32, rng: &mut u64) -> Duration {
    let shift = attempt.saturating_sub(1).min(16);
    let base = (BASE_MS << shift).min(CAP_MS);
    // Facteur dans [0,8 ; 1,2] en millièmes : 800 + (0..=400).
    let factor = 800 + next(rng) % 401;
    Duration::from_millis((base * factor / 1000).min(CAP_MS))
}

fn next(state: &mut u64) -> u64 {
    if *state == 0 {
        *state = 0x9E37_79B9_7F4A_7C15;
    }
    let mut x = *state;
    x ^= x << 13;
    x ^= x >> 7;
    x ^= x << 17;
    *state = x;
    x
}
