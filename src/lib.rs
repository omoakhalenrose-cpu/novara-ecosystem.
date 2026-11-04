//! NOVARA — Independent Layer-1 blockchain.
//!
//! This library will contain NOVARA's core protocol components.

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

pub fn version() -> &'static str {
    VERSION
}
