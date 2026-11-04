//! NOVARA — Independent Layer-1 blockchain.

pub mod crypto;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

pub fn version() -> &'static str {
    VERSION
}
