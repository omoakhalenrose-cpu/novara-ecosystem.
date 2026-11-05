//! NOVARA — Independent Layer-1 blockchain.
//!
//! This library contains NOVARA's core protocol components.

pub mod account;
pub mod block;
pub mod chain;
pub mod crypto;
pub mod genesis;
pub mod state;
pub mod transaction;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

pub fn version() -> &'static str {
    VERSION
}
