//! NOVARA genesis configuration.
//!
//! Defines the initial account and asset allocations
//! before a blockchain network begins operating.

use std::collections::BTreeSet;

use sha2::{Digest, Sha256};

use crate::account::{address::AccountId, network::NetworkId};
use crate::transaction::{AssetId, ChainId};

/// Domain separator for NOVARA genesis encoding.
const GENESIS_DOMAIN: &[u8] = b"NOVARA_GENESIS_V1";

/// Domain separator for NOVARA chain identity.
const CHAIN_ID_DOMAIN: &[u8] = b"NOVARA_CHAIN_ID_V1";

/// One initial asset allocation to an account.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GenesisAllocation {
    pub account_id: AccountId,
    pub asset_id: AssetId,
    pub amount: u128,
}

/// Identifies a NOVARA genesis configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct GenesisHash([u8; 32]);

impl GenesisHash {
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

/// Errors returned during genesis validation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GenesisError {
    ZeroAllocation,
    DuplicateAllocation,
}

/// Initial configuration for a NOVARA network.
#[derive(Debug, Clone)]
pub struct GenesisConfig {
    pub network: NetworkId,
    pub allocations: Vec<GenesisAllocation>,
}

impl GenesisConfig {
    /// Creates a new genesis configuration.
    pub fn new(network: NetworkId, allocations: Vec<GenesisAllocation>) -> Self {
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

    /// Produces deterministic genesis configuration bytes.
    ///
    /// Allocations are sorted so their input order does not
    /// change the resulting genesis identity.
    pub fn encode(&self) -> Vec<u8> {
        let mut allocations: Vec<&GenesisAllocation> = self.allocations.iter().collect();

        allocations.sort_by_key(|allocation| {
            (
                *allocation.account_id.as_bytes(),
                *allocation.asset_id.as_bytes(),
            )
        });

        let mut bytes = Vec::new();

        bytes.extend_from_slice(GENESIS_DOMAIN);
        bytes.push(self.network.as_byte());
        bytes.extend_from_slice(&(allocations.len() as u64).to_be_bytes());

        for allocation in allocations {
            bytes.extend_from_slice(allocation.account_id.as_bytes());
            bytes.extend_from_slice(allocation.asset_id.as_bytes());
            bytes.extend_from_slice(&allocation.amount.to_be_bytes());
        }

        bytes
    }

    /// Derives the chain identity from the genesis configuration.
    pub fn chain_id(&self) -> ChainId {
        let mut hasher = Sha256::new();

        hasher.update(CHAIN_ID_DOMAIN);
        hasher.update(self.encode());

        ChainId::new(hasher.finalize().into())
    }

    /// Calculates the genesis configuration hash.
    pub fn hash(&self) -> GenesisHash {
        let digest = Sha256::digest(self.encode());

        GenesisHash(digest.into())
    }
}

/// A validated genesis configuration with authoritative chain identities.
///
/// The private fields ensure the Chain ID and Genesis Hash
/// cannot be independently replaced after initialization.
#[derive(Debug, Clone)]
pub struct InitializedGenesis {
    config: GenesisConfig,
    chain_id: ChainId,
    genesis_hash: GenesisHash,
}

impl InitializedGenesis {
    /// Validates and initializes a genesis configuration.
    pub fn new(config: GenesisConfig) -> Result<Self, GenesisError> {
        config.validate()?;

        let chain_id = config.chain_id();
        let genesis_hash = config.hash();

        Ok(Self {
            config,
            chain_id,
            genesis_hash,
        })
    }

    /// Returns the validated genesis configuration.
    pub const fn config(&self) -> &GenesisConfig {
        &self.config
    }

    /// Returns the authoritative Chain ID.
    pub const fn chain_id(&self) -> ChainId {
        self.chain_id
    }

    /// Returns the authoritative Genesis Hash.
    pub const fn genesis_hash(&self) -> GenesisHash {
        self.genesis_hash
    }

    /// Consumes the initialized genesis and returns its configuration.
    pub fn into_config(self) -> GenesisConfig {
        self.config
    }
}
