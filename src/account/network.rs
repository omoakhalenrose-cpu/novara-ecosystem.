//! NOVARA network identification.

/// Identifies the NOVARA network an account belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NetworkId {
    Development,
    Testnet,
    Mainnet,
}

impl NetworkId {
    /// Returns the stable protocol byte for this network.
    pub const fn as_byte(self) -> u8 {
        match self {
            Self::Development => 0x01,
            Self::Testnet => 0x02,
            Self::Mainnet => 0x03,
        }
    }
}
