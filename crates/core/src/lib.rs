//! Cœur d'aNkou : machine à états de connexion, sans entrée/sortie (spec §4.1, §6.4).

mod backoff;
mod machine;

pub use backoff::backoff;
pub use machine::{Action, CoreSettings, Event, Machine, Policy, State};
