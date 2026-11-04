//! NOVARA account model.

pub mod address;

use address::AccountId;

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
    pub id: AccountId,
    pub account_type: AccountType,
}
