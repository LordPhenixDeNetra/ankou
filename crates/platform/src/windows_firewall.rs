//! Kill switch Windows par WFP (Windows Filtering Platform), bibliothèque `wfp` (ADR 0009).
//!
//! Fournisseur, sous-couche et filtres **persistants**, à nos GUID : la protection reste
//! après un plantage ou un redémarrage, et `is_active` la retrouve (fail-closed, spec §8.1).

use crate::PlatformError;
use crate::firewall::{Firewall, FirewallPolicy, WfpRule, wfp_rules};
use std::io;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use wfp::{
    ActionType, FilterBuilder, FilterEngine, FilterEngineBuilder, FilterEnumerator, FilterLifetime,
    FilterWeight, GUID, IcmpConditionBuilder, InterfaceConditionBuilder, IpAddressConditionBuilder,
    Layer, PortConditionBuilder, ProtocolConditionBuilder, ProviderBuilder, SubLayerBuilder,
    SubLayerEnumerator, Transaction, delete_filter, delete_provider, delete_sublayer,
};

const PROVIDER: GUID = GUID::from_u128(0x6a1d_8c2e_4f3b_4e5a_9c7d_1b2e_3f4a_5b01);
const SUBLAYER: GUID = GUID::from_u128(0x6a1d_8c2e_4f3b_4e5a_9c7d_1b2e_3f4a_5b02);
/// Une permission l'emporte toujours sur un blocage de notre sous-couche.
const PERMIT: u64 = 1_000;
const BLOCK: u64 = 1;

fn same(a: &GUID, b: &GUID) -> bool {
    a.data1 == b.data1 && a.data2 == b.data2 && a.data3 == b.data3 && a.data4 == b.data4
}

fn map_err(e: io::Error) -> PlatformError {
    if e.kind() == io::ErrorKind::PermissionDenied {
        PlatformError::Permission
    } else {
        PlatformError::Unavailable(format!("WFP : {e}"))
    }
}

fn engine() -> Result<FilterEngine, PlatformError> {
    FilterEngineBuilder::default().open().map_err(map_err)
}

fn sublayer_exists(tx: &Transaction<'_>) -> io::Result<bool> {
    let mut list = SubLayerEnumerator::new(tx)?;
    while let Some(item) = list.next() {
        if same(&item?.guid(), &SUBLAYER) {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Supprime nos filtres (ceux de notre fournisseur).
fn delete_ours(tx: &Transaction<'_>) -> io::Result<()> {
    let mut ids = Vec::new();
    {
        let mut list = FilterEnumerator::new(tx)?;
        while let Some(item) = list.next() {
            let item = item?;
            if item.provider().is_some_and(|p| same(&p, &PROVIDER)) {
                ids.push(item.id());
            }
        }
    }
    for id in ids {
        delete_filter(tx, id)?;
    }
    Ok(())
}

fn filter(
    tx: &Transaction<'_>,
    layer: Layer,
    action: ActionType,
    conditions: Vec<wfp::Condition>,
) -> io::Result<()> {
    let weight = if matches!(action, ActionType::Permit) {
        PERMIT
    } else {
        BLOCK
    };
    let mut builder = FilterBuilder::default()
        .name("aNkou")
        .description("Kill switch d'aNkou")
        .action(action)
        .layer(layer)
        .provider(PROVIDER)
        .sublayer(SUBLAYER)
        .weight(FilterWeight::Exact(weight))
        .lifetime(FilterLifetime::Persistent);
    for c in conditions {
        builder = builder.condition(c);
    }
    builder.add(tx).map(|_| ())
}

fn host(ip: IpAddr) -> wfp::Condition {
    match ip {
        IpAddr::V4(a) => IpAddressConditionBuilder::remote().subnet_v4(a, 32).build(),
        IpAddr::V6(a) => IpAddressConditionBuilder::remote()
            .subnet_v6(a, 128)
            .build(),
    }
}

fn add_rule(tx: &Transaction<'_>, rule: &WfpRule) -> io::Result<()> {
    use ActionType::{Block, Permit};
    use Layer::{ConnectV4, ConnectV6};
    match rule {
        WfpRule::PermitInterface { name } => {
            // La carte n'existe qu'après la montée du tunnel : le service réapplique alors
            // la politique (ADR 0009). Avant, rien à autoriser.
            let Ok(cond) = InterfaceConditionBuilder::local().alias(name) else {
                return Ok(());
            };
            let cond = cond.build();
            filter(tx, ConnectV4, Permit, vec![cond.clone()])?;
            filter(tx, ConnectV6, Permit, vec![cond])
        }
        WfpRule::PermitLoopback => {
            let v4 = IpAddressConditionBuilder::remote()
                .subnet_v4(Ipv4Addr::new(127, 0, 0, 0), 8)
                .build();
            filter(tx, ConnectV4, Permit, vec![v4])?;
            filter(
                tx,
                ConnectV6,
                Permit,
                vec![host(IpAddr::V6(Ipv6Addr::LOCALHOST))],
            )
        }
        WfpRule::PermitEndpoint(ep) => {
            let layer = if ep.is_ipv4() { ConnectV4 } else { ConnectV6 };
            let conditions = vec![
                host(ep.ip()),
                PortConditionBuilder::remote().equal(ep.port()).build(),
                ProtocolConditionBuilder::udp().build(),
            ];
            filter(tx, layer, Permit, conditions)
        }
        WfpRule::PermitDhcpV4 => {
            let conditions = vec![
                PortConditionBuilder::remote().equal(67).build(),
                ProtocolConditionBuilder::udp().build(),
            ];
            filter(tx, ConnectV4, Permit, conditions)
        }
        WfpRule::PermitNeighbourDiscovery => {
            // Sollicitation de routeur (133), sollicitation et annonce de voisin (135, 136).
            for icmp_type in [133, 135, 136] {
                let conditions = vec![
                    ProtocolConditionBuilder::icmpv6().build(),
                    IcmpConditionBuilder::r#type().equal(icmp_type).build(),
                ];
                filter(tx, ConnectV6, Permit, conditions)?;
            }
            Ok(())
        }
        WfpRule::BlockAllV6 => filter(tx, ConnectV6, Block, Vec::new()),
        WfpRule::BlockAllV4 => filter(tx, ConnectV4, Block, Vec::new()),
    }
}

pub struct WfpFirewall;

impl Firewall for WfpFirewall {
    /// Une seule transaction : anciens filtres retirés, nouveaux posés, jamais d'instant sans.
    fn apply(&mut self, policy: &FirewallPolicy) -> Result<(), PlatformError> {
        let mut engine = engine()?;
        let tx = Transaction::new(&mut engine).map_err(map_err)?;
        let result = (|| {
            delete_ours(&tx)?;
            if !sublayer_exists(&tx)? {
                ProviderBuilder::default()
                    .name("aNkou")
                    .description("Client VPN aNkou")
                    .guid(PROVIDER)
                    .persistent()
                    .add(&tx)?;
                SubLayerBuilder::default()
                    .name("aNkou")
                    .description("Kill switch d'aNkou")
                    .guid(SUBLAYER)
                    .provider(PROVIDER)
                    .weight(u16::MAX)
                    .persistent()
                    .add(&tx)?;
            }
            for rule in wfp_rules(policy) {
                add_rule(&tx, &rule)?;
            }
            Ok(())
        })();
        match result {
            Ok(()) => tx.commit().map_err(map_err),
            Err(e) => {
                let _ = tx.abort();
                Err(map_err(e))
            }
        }
    }

    fn clear(&mut self) -> Result<(), PlatformError> {
        let mut engine = engine()?;
        let tx = Transaction::new(&mut engine).map_err(map_err)?;
        let result = (|| {
            delete_ours(&tx)?;
            if sublayer_exists(&tx)? {
                delete_sublayer(&tx, &SUBLAYER)?;
                delete_provider(&tx, &PROVIDER)?;
            }
            Ok(())
        })();
        match result {
            Ok(()) => tx.commit().map_err(map_err),
            Err(e) => {
                let _ = tx.abort();
                Err(map_err(e))
            }
        }
    }

    fn is_active(&self) -> bool {
        let Ok(mut engine) = engine() else {
            return false;
        };
        let Ok(tx) = Transaction::new(&mut engine) else {
            return false;
        };
        let found = sublayer_exists(&tx).unwrap_or(false);
        let _ = tx.abort();
        found
    }
}
