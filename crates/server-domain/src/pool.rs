use crate::DomainError;
use ipnet::Ipv4Net;
use std::collections::BTreeSet;
use std::net::Ipv4Addr;

/// Plus petite adresse libre du pool ; le réseau, la diffusion et la 1ʳᵉ adresse
/// (réservée aux serveurs) ne sont jamais attribués.
// ponytail: parcours linéaire, O(taille du pool) ; suffisant pour un /16 familial.
pub fn allocate_ipv4(pool: Ipv4Net, used: &BTreeSet<Ipv4Addr>) -> Result<Ipv4Addr, DomainError> {
    let reserved = pool.hosts().next();
    // `hosts()` exclut déjà le réseau et la diffusion.
    pool.hosts()
        .find(|ip| Some(*ip) != reserved && !used.contains(ip))
        .ok_or(DomainError::PoolExhausted)
}
