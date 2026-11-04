//! NOVARA account identifiers.

/// A unique 32-byte identifier for a NOVARA account.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct AccountId(pub [u8; 32]);

impl AccountId {
    /// Creates an account identifier from 32 bytes.
    pub const fn new(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    /// Returns the identifier's underlying bytes.
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}
