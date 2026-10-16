#![allow(clippy::unwrap_used)]

use ankou_platform::fake::{FakeDns, FakeFirewall};
use ankou_platform::{Dns, Firewall, FirewallPolicy, nft_ruleset, resolvectl_commands};
use std::net::IpAddr;

fn policy(kill_switch: bool) -> FirewallPolicy {
    FirewallPolicy {
        kill_switch,
        interface: "ankou0".into(),
        endpoints: vec!["203.0.113.7:51820".parse().unwrap()],
    }
}

#[test]
fn ruleset_without_kill_switch_blocks_only_ipv6() {
    let r = nft_ruleset(&policy(false));
    assert!(r.contains("table inet ankou"), "{r}");
    assert!(r.contains("oifname \"lo\" accept"), "{r}");
    assert!(r.contains("oifname \"ankou0\" accept"), "{r}");
    assert!(r.contains("meta nfproto ipv6 drop"), "{r}");
    assert!(!r.contains("meta nfproto ipv4 drop"), "{r}");
}

#[test]
fn ruleset_with_kill_switch_allows_endpoint_and_dhcp() {
    let r = nft_ruleset(&policy(true));
    let endpoint = r
        .find("ip daddr 203.0.113.7 udp dport 51820 accept")
        .unwrap();
    let drop4 = r.find("meta nfproto ipv4 drop").unwrap();
    assert!(endpoint < drop4, "{r}");
    assert!(r.contains("udp sport 68 udp dport 67 accept"), "{r}");
    assert!(r.find("oifname \"ankou0\" accept").unwrap() < drop4, "{r}");
}

#[test]
fn ipv6_endpoint_is_allowed_before_ipv6_drop() {
    let mut p = policy(true);
    p.endpoints = vec!["[2001:db8::7]:51820".parse().unwrap()];
    let r = nft_ruleset(&p);
    let allow = r
        .find("ip6 daddr 2001:db8::7 udp dport 51820 accept")
        .unwrap();
    assert!(allow < r.find("meta nfproto ipv6 drop").unwrap(), "{r}");
}

#[test]
fn ruleset_replaces_table_atomically() {
    let r = nft_ruleset(&policy(true));
    assert!(
        r.starts_with("add table inet ankou\ndelete table inet ankou\n"),
        "{r}"
    );
}

#[test]
fn resolvectl_routes_all_domains_to_tunnel() {
    let servers: Vec<IpAddr> = vec![
        "9.9.9.9".parse().unwrap(),
        "149.112.112.112".parse().unwrap(),
    ];
    assert_eq!(
        resolvectl_commands("ankou0", &servers),
        vec![
            vec!["resolvectl", "dns", "ankou0", "9.9.9.9", "149.112.112.112"],
            vec!["resolvectl", "domain", "ankou0", "~."],
            vec!["resolvectl", "default-route", "ankou0", "yes"],
        ]
    );
}

#[test]
fn fake_firewall_records_policy() {
    let mut f = FakeFirewall::default();
    let handle = f.clone();
    assert!(!f.is_active());
    f.apply(&policy(true)).unwrap();
    assert_eq!(handle.active(), Some(policy(true)));
    assert!(f.is_active());
    f.clear().unwrap();
    assert_eq!(handle.active(), None);
    handle.fail(true);
    assert!(f.apply(&policy(false)).is_err());
    handle.set_leftover(policy(true));
    assert!(f.is_active());
}

#[test]
fn fake_dns_records_servers() {
    let mut d = FakeDns::default();
    let handle = d.clone();
    let servers: Vec<IpAddr> = vec!["1.1.1.1".parse().unwrap()];
    d.set("ankou0", &servers).unwrap();
    assert_eq!(handle.current(), Some(("ankou0".into(), servers)));
    d.revert("ankou0");
    assert_eq!(handle.current(), None);
    handle.fail(true);
    assert!(d.set("ankou0", &[]).is_err());
}

/// Conteneurs et VM (trafic routé par l'hôte) : même blocage que l'hôte.
#[test]
fn forwarded_traffic_is_filtered_too() {
    let on = nft_ruleset(&policy(true));
    let forward = &on[on.find("chain forward").unwrap()..];
    assert!(
        forward.contains("type filter hook forward priority 0"),
        "{on}"
    );
    assert!(forward.contains("iifname \"ankou0\" accept"), "{on}");
    assert!(forward.contains("oifname \"ankou0\" accept"), "{on}");
    assert!(forward.contains("meta nfproto ipv6 drop"), "{on}");
    assert!(forward.contains("meta nfproto ipv4 drop"), "{on}");
    let off = nft_ruleset(&policy(false));
    let forward = &off[off.find("chain forward").unwrap()..];
    assert!(forward.contains("meta nfproto ipv6 drop"), "{off}");
    assert!(!forward.contains("meta nfproto ipv4 drop"), "{off}");
}

/// Endpoint IPv6 : joignable avec ou sans kill switch (règle + découverte des voisins).
#[test]
fn ipv6_endpoint_reachable_without_kill_switch() {
    let mut p = policy(false);
    p.endpoints = vec!["[2001:db8::7]:51820".parse().unwrap()];
    let r = nft_ruleset(&p);
    let drop6 = r.find("meta nfproto ipv6 drop").unwrap();
    assert!(
        r.find("ip6 daddr 2001:db8::7 udp dport 51820 accept")
            .unwrap()
            < drop6,
        "{r}"
    );
    assert!(r.find("nd-neighbor-solicit").unwrap() < drop6, "{r}");
    assert!(!nft_ruleset(&policy(false)).contains("nd-neighbor-solicit"));
}
