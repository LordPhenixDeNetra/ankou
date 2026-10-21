//! Kill switch Windows par WFP (Windows Filtering Platform), bibliothèque `wfp` (ADR 0009).
//!
//! Fournisseur, sous-couche et filtres **persistants**, à nos GUID : la protection reste
//! après un plantage ou un redémarrage, et `is_active` la retrouve (fail-closed, spec §8.1).
//! Chaque règle vaut pour les connexions sortantes (`Connect*`) et entrantes (`Accept*`) :
//! les réponses à une connexion acceptée ne repassent pas par la couche de connexion.

use crate::PlatformError;
use crate::firewall::{Firewall, FirewallPolicy, WfpRule, wfp_rules};
use std::io;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::time::Duration;
use wfp::{
    ActionType, Condition, FilterBuilder, FilterEngine, FilterEngineBuilder, FilterEnumerator,
    FilterLifetime, FilterWeight, GUID, IcmpConditionBuilder, InterfaceConditionBuilder,
    IpAddressConditionBuilder, Layer, PortConditionBuilder, ProtocolConditionBuilder,
    ProviderBuilder, SubLayerBuilder, SubLayerEnumerator, Transaction, delete_filter,
    delete_provider, delete_sublayer,
};

const PROVIDER: GUID = GUID::from_u128(0x6a1d_8c2e_4f3b_4e5a_9c7d_1b2e_3f4a_5b01);
const SUBLAYER: GUID = GUID::from_u128(0x6a1d_8c2e_4f3b_4e5a_9c7d_1b2e_3f4a_5b02);
/// Une permission l'emporte toujours sur un blocage de notre sous-couche.
const PERMIT: u64 = 1_000;
const BLOCK: u64 = 1;
/// `FWP_E_ALREADY_EXISTS` : fournisseur laissé par un retrait incomplet.
const ALREADY_EXISTS: i32 = 0x8032_0009_u32 as i32;
/// Au démarrage de Windows, le moteur de filtrage (BFE) peut arriver après le service.
const ENGINE_WAIT: Duration = Duration::from_secs(30);

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
    conditions: &[Condition],
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
        builder = builder.condition(c.clone());
    }
    builder.add(tx).map(|_| ())
}

/// Même filtre en sortie et en entrée, pour une famille.
fn both(
    tx: &Transaction<'_>,
    v6: bool,
    action: ActionType,
    conditions: &[Condition],
) -> io::Result<()> {
    let (connect, accept) = if v6 {
        (Layer::ConnectV6, Layer::AcceptV6)
    } else {
        (Layer::ConnectV4, Layer::AcceptV4)
    };
    filter(tx, connect, action, conditions)?;
    filter(tx, accept, action, conditions)
}

fn host(ip: IpAddr) -> Condition {
    match ip {
        IpAddr::V4(a) => IpAddressConditionBuilder::remote().subnet_v4(a, 32).build(),
        IpAddr::V6(a) => IpAddressConditionBuilder::remote()
            .subnet_v6(a, 128)
            .build(),
    }
}

fn add_rule(tx: &Transaction<'_>, rule: &WfpRule) -> io::Result<()> {
    use ActionType::{Block, Permit};
    match rule {
        WfpRule::PermitInterface { name } => {
            // La carte n'existe qu'après la montée du tunnel : le service réapplique alors
            // la politique (ADR 0009). Avant, rien à autoriser.
            let Ok(cond) = InterfaceConditionBuilder::local().alias(name) else {
                return Ok(());
            };
            let cond = [cond.build()];
            both(tx, false, Permit, &cond)?;
            both(tx, true, Permit, &cond)
        }
        WfpRule::PermitLoopback => {
            let v4 = IpAddressConditionBuilder::remote()
                .subnet_v4(Ipv4Addr::new(127, 0, 0, 0), 8)
                .build();
            both(tx, false, Permit, &[v4])?;
            both(tx, true, Permit, &[host(IpAddr::V6(Ipv6Addr::LOCALHOST))])
        }
        WfpRule::PermitEndpoint(ep) => {
            let conditions = [
                host(ep.ip()),
                PortConditionBuilder::remote().equal(ep.port()).build(),
                ProtocolConditionBuilder::udp().build(),
            ];
            both(tx, ep.is_ipv6(), Permit, &conditions)
        }
        WfpRule::PermitDhcpV4 => {
            // Client DHCP : port local 68 vers le port 67 du serveur, comme sous Linux.
            let conditions = [
                PortConditionBuilder::local().equal(68).build(),
                PortConditionBuilder::remote().equal(67).build(),
                ProtocolConditionBuilder::udp().build(),
            ];
            both(tx, false, Permit, &conditions)
        }
        WfpRule::PermitNeighbourDiscovery => {
            // Sollicitation et annonce de routeur (133, 134), de voisin (135, 136).
            for icmp_type in [133, 134, 135, 136] {
                let conditions = [
                    ProtocolConditionBuilder::icmpv6().build(),
                    IcmpConditionBuilder::r#type().equal(icmp_type).build(),
                ];
                both(tx, true, Permit, &conditions)?;
            }
            Ok(())
        }
        WfpRule::BlockAllV6 => both(tx, true, Block, &[]),
        WfpRule::BlockAllV4 => both(tx, false, Block, &[]),
    }
}

/// Exécute `work` dans une transaction : validée si tout réussit, annulée sinon.
fn transaction(work: impl FnOnce(&Transaction<'_>) -> io::Result<()>) -> Result<(), PlatformError> {
    let mut engine = engine()?;
    let tx = Transaction::new(&mut engine).map_err(map_err)?;
    match work(&tx) {
        Ok(()) => tx.commit().map_err(map_err),
        Err(e) => {
            let _ = tx.abort();
            Err(map_err(e))
        }
    }
}

pub struct WfpFirewall;

impl Firewall for WfpFirewall {
    /// Une seule transaction : anciens filtres retirés, nouveaux posés, jamais d'instant sans.
    fn apply(&mut self, policy: &FirewallPolicy) -> Result<(), PlatformError> {
        transaction(|tx| {
            delete_ours(tx)?;
            if !sublayer_exists(tx)? {
                let added = ProviderBuilder::default()
                    .name("aNkou")
                    .description("Client VPN aNkou")
                    .guid(PROVIDER)
                    .persistent()
                    .add(tx);
                match added {
                    Err(e) if e.raw_os_error() != Some(ALREADY_EXISTS) => return Err(e),
                    _ => {}
                }
                SubLayerBuilder::default()
                    .name("aNkou")
                    .description("Kill switch d'aNkou")
                    .guid(SUBLAYER)
                    .provider(PROVIDER)
                    .weight(u16::MAX)
                    .persistent()
                    .add(tx)?;
            }
            for rule in wfp_rules(policy) {
                add_rule(tx, &rule)?;
            }
            Ok(())
        })
    }

    fn clear(&mut self) -> Result<(), PlatformError> {
        transaction(|tx| {
            delete_ours(tx)?;
            if sublayer_exists(tx)? {
                delete_sublayer(tx, &SUBLAYER)?;
            }
            // Fournisseur seul (retrait précédent incomplet) : supprimé aussi.
            let _ = delete_provider(tx, &PROVIDER);
            Ok(())
        })
    }

    /// Attend le moteur de filtrage s'il démarre après le service : un « non » trop tôt
    /// ferait oublier une protection restée, qui bloquerait ensuite tout sans recours.
    fn is_active(&self) -> bool {
        let deadline = std::time::Instant::now() + ENGINE_WAIT;
        let mut engine = loop {
            match engine() {
                Ok(engine) => break engine,
                Err(_) if std::time::Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_secs(1));
                }
                Err(e) => {
                    tracing::warn!(error = %e, "moteur de filtrage indisponible");
                    return false;
                }
            }
        };
        let Ok(tx) = Transaction::new(&mut engine) else {
            return false;
        };
        let found = sublayer_exists(&tx).unwrap_or(false);
        let _ = tx.abort();
        found
    }
}
