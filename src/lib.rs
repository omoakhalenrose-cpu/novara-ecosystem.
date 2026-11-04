//! NOVARA — Independent Layer-1 blockchain.
//!
//! This library contains NOVARA's core protocol components.

pub mod account;
pub mod crypto;
pub mod transaction;
pub mod state;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

pub fn version() -> &'static str {
    VERSION
}
