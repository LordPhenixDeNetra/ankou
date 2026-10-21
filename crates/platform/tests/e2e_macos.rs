//! Vrai pare-feu `pf` (root requis) : lancés par le job CI `e2e-macos`.
#![cfg(target_os = "macos")]
#![allow(clippy::unwrap_used, clippy::expect_used)]

use ankou_platform::{FirewallPolicy, default_firewall};
use std::net::{SocketAddr, TcpStream};
use std::time::Duration;

fn policy() -> FirewallPolicy {
    FirewallPolicy {
        kill_switch: true,
        interface: "utun100".into(),
        endpoints: vec!["192.0.2.1:51820".parse().unwrap()],
    }
}

fn reachable() -> bool {
    let addr: SocketAddr = "1.1.1.1:443".parse().unwrap();
    TcpStream::connect_timeout(&addr, Duration::from_secs(3)).is_ok()
}

#[test]
#[ignore = "root requis"]
fn firewall_blocks_then_clears() {
    assert!(reachable(), "Internet doit répondre avant le test");
    let mut fw = default_firewall();
    fw.apply(&policy()).unwrap();
    let blocked = !reachable();
    fw.clear().unwrap();
    assert!(blocked, "le kill switch doit tout bloquer hors tunnel");
    assert!(reachable(), "Internet doit revenir après clear");
}

/// Règles du noyau : une autre instance (le service relancé) les retrouve.
#[test]
#[ignore = "root requis"]
fn pf_survives_new_instance() {
    let mut fw = default_firewall();
    fw.apply(&policy()).unwrap();
    let seen = default_firewall().is_active();
    fw.clear().unwrap();
    assert!(seen);
    assert!(!default_firewall().is_active());
}
