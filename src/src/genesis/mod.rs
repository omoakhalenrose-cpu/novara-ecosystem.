//! NOVARA genesis configuration.
//!
//! Defines the initial account and asset allocations
//! before a blockchain network begins operating.

use std::collections::BTreeSet;

use crate::account::{address::AccountId, network::NetworkId};
use crate::transaction::AssetId;

/// One initial asset allocation to an account.
#[derive(Debug)]
pub struct GenesisAllocation {
    pub account_id: AccountId,
    pub asset_id: AssetId,
    pub amount: u128,
}

/// Errors returned during genesis validation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GenesisError {
    ZeroAllocation,
    DuplicateAllocation,
}

/// Initial configuration for a NOVARA network.
#[derive(Debug)]
pub struct GenesisConfig {
    pub network: NetworkId,
    pub allocations: Vec<GenesisAllocation>,
}

impl GenesisConfig {
    /// Creates a new genesis configuration.
    pub fn new(
        network: NetworkId,
        allocations: Vec<GenesisAllocation>,
    ) -> Self {
        Self {
            network,
            allocations,
        }
    }

    /// Validates all initial allocations.
    pub fn validate(&self) -> Result<(), GenesisError> {
        let mut seen = BTreeSet::new();

        for allocation in &self.allocations {
            if allocation.amount == 0 {
                return Err(GenesisError::ZeroAllocation);
            }

            let key = (
                *allocation.account_id.as_bytes(),
                *allocation.asset_id.as_bytes(),
            );

            if !seen.insert(key) {
                return Err(GenesisError::DuplicateAllocation);
            }
        }

        Ok(())
    }
}
