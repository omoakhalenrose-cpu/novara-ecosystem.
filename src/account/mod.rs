//! NOVARA account model.
//!
//! Defines the three account categories supported by NOVARA.

/// Identifies the category of a NOVARA account.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccountType {
    Individual,
    Business,
    SmartContract,
}

/// Represents a NOVARA account.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Account {
    pub address: String,
    pub account_type: AccountType,
}
