use ankou_core::{fastest, median};
use std::time::Duration;

fn ms(v: &[u64]) -> Vec<Duration> {
    v.iter().map(|&m| Duration::from_millis(m)).collect()
}

#[test]
fn median_of_odd_and_even() {
    assert_eq!(median(&ms(&[30, 10, 20])), Some(Duration::from_millis(20)));
    assert_eq!(median(&ms(&[10, 20])), Some(Duration::from_millis(15)));
    assert_eq!(median(&[]), None);
}

#[test]
fn fastest_uses_median_of_successes() {
    let c = vec![ms(&[50, 60, 55]), ms(&[40]), ms(&[]), ms(&[40])];
    assert_eq!(fastest(&c), Some(1));
    assert_eq!(fastest(&[ms(&[]), ms(&[])]), None);
}
