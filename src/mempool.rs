//! NOVARA Mempool
//!
//! Manages pending transactions before block inclusion.
//! This module is intentionally independent of consensus.
//!
//! All time values are supplied by the caller so that tests
//! remain deterministic.

use std::collections::{BTreeMap, BTreeSet};

/// Configuration for the mempool.
#[derive(Debug, Clone)]
pub struct MempoolConfig {
    pub max_transactions: usize,
    pub max_total_bytes: usize,
    pub max_transaction_bytes: usize,
    pub max_per_sender: usize,
    pub transaction_ttl: u64,
    pub max_per_sender_per_block: usize,
}

impl Default for MempoolConfig {
    fn default() -> Self {
        Self {
            max_transactions: 10_000,
            max_total_bytes: 64 * 1024 * 1024,
            max_transaction_bytes: 128 * 1024,
            max_per_sender: 100,
            transaction_ttl: 60 * 60,
            max_per_sender_per_block: 10,
        }
    }
}

/// A transaction waiting to be included in a block.
///
/// The caller must validate the actual transaction signature,
/// network, chain, nonce and balance before admission.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingTransaction {
    pub id: [u8; 32],
    pub sender: [u8; 32],
    pub nonce: u64,
    pub fee: u128,
    pub received_at: u64,
    pub payload: Vec<u8>,
}

impl PendingTransaction {
    pub fn size_bytes(&self) -> usize {
        self.payload.len()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MempoolError {
    DuplicateTransaction,
    TransactionTooLarge,
    MempoolFull,
    SenderLimitReached,
    InvalidTransactionSize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MempoolStats {
    pub transaction_count: usize,
    pub total_bytes: usize,
    pub sender_count: usize,
}

/// Pending transaction pool.
#[derive(Debug)]
pub struct Mempool {
    config: MempoolConfig,
    transactions: BTreeMap<[u8; 32], PendingTransaction>,
    sender_counts: BTreeMap<[u8; 32], usize>,
    total_bytes: usize,
}

impl Mempool {
    pub fn new(config: MempoolConfig) -> Self {
        Self {
            config,
            transactions: BTreeMap::new(),
            sender_counts: BTreeMap::new(),
            total_bytes: 0,
        }
    }

    /// Admit a transaction after external protocol validation.
    pub fn admit(
        &mut self,
        transaction: PendingTransaction,
    ) -> Result<(), MempoolError> {
        let size = transaction.size_bytes();

        if size == 0 {
            return Err(MempoolError::InvalidTransactionSize);
        }

        if size > self.config.max_transaction_bytes {
            return Err(MempoolError::TransactionTooLarge);
        }

        if self.transactions.contains_key(&transaction.id) {
            return Err(MempoolError::DuplicateTransaction);
        }

        if self.transactions.len() >= self.config.max_transactions
            || self.total_bytes.saturating_add(size)
                > self.config.max_total_bytes
        {
            return Err(MempoolError::MempoolFull);
        }

        let sender_count = self
            .sender_counts
            .get(&transaction.sender)
            .copied()
            .unwrap_or(0);

        if sender_count >= self.config.max_per_sender {
            return Err(MempoolError::SenderLimitReached);
        }

        self.total_bytes += size;

        *self.sender_counts.entry(transaction.sender).or_insert(0) += 1;

        self.transactions.insert(transaction.id, transaction);

        Ok(())
    }

    /// Remove transactions that have exceeded their lifetime.
    pub fn expire(&mut self, current_time: u64) -> usize {
        let ttl = self.config.transaction_ttl;

        let expired: Vec<[u8; 32]> = self
            .transactions
            .iter()
            .filter_map(|(id, transaction)| {
                let age = current_time.saturating_sub(transaction.received_at);

                if age >= ttl {
                    Some(*id)
                } else {
                    None
                }
            })
            .collect();

        let count = expired.len();

        for id in expired {
            self.remove(&id);
        }

        count
    }

    /// Remove a transaction and update accounting.
    pub fn remove(
        &mut self,
        id: &[u8; 32],
    ) -> Option<PendingTransaction> {
        let transaction = self.transactions.remove(id)?;

        self.total_bytes = self
            .total_bytes
            .saturating_sub(transaction.size_bytes());

        if let Some(count) = self.sender_counts.get_mut(&transaction.sender) {
            *count -= 1;

            if *count == 0 {
                self.sender_counts.remove(&transaction.sender);
            }
        }

        Some(transaction)
    }

    /// Select transactions for a block.
    ///
    /// Selection is deterministic for the same input and time.
    /// Higher fee-per-byte and older transactions receive priority.
    /// A sender cannot monopolise the selection.
    ///
    /// This method does not remove selected transactions.
    pub fn select_for_block(
        &self,
        maximum_transactions: usize,
        current_time: u64,
    ) -> Vec<PendingTransaction> {
        let mut candidates: Vec<&PendingTransaction> =
            self.transactions.values().collect();

        candidates.sort_by(|a, b| {
            let age_a = current_time.saturating_sub(a.received_at);
            let age_b = current_time.saturating_sub(b.received_at);

            let size_a = a.size_bytes().max(1) as u128;
            let size_b = b.size_bytes().max(1) as u128;

            // Compare fee-per-byte without floating-point arithmetic.
            let fee_a = a.fee.saturating_mul(size_b);
            let fee_b = b.fee.saturating_mul(size_a);

            // Age breaks fee-rate ties.
            fee_b
                .cmp(&fee_a)
                .then_with(|| age_b.cmp(&age_a))
                .then_with(|| a.id.cmp(&b.id))
        });

        let mut selected = Vec::new();
        let mut sender_counts: BTreeMap<[u8; 32], usize> =
            BTreeMap::new();

        // Enforce sequential nonces within each sender's selected
        // transactions. The caller supplies the expected account nonce
        // in a later integration stage.
        let mut last_nonce: BTreeMap<[u8; 32], u64> =
            BTreeMap::new();

        for transaction in candidates {
            if selected.len() >= maximum_transactions {
                break;
            }

            let sender_count = sender_counts
                .get(&transaction.sender)
                .copied()
                .unwrap_or(0);

            if sender_count >= self.config.max_per_sender_per_block {
                continue;
            }

            if let Some(previous_nonce) = last_nonce.get(&transaction.sender) {
                if transaction.nonce != previous_nonce.saturating_add(1) {
                    continue;
                }
            }

            last_nonce.insert(transaction.sender, transaction.nonce);

            *sender_counts.entry(transaction.sender).or_insert(0) += 1;

            selected.push(transaction.clone());
        }

        selected
    }

    pub fn get(&self, id: &[u8; 32]) -> Option<&PendingTransaction> {
        self.transactions.get(id)
    }

    pub fn stats(&self) -> MempoolStats {
        MempoolStats {
            transaction_count: self.transactions.len(),
            total_bytes: self.total_bytes,
            sender_count: self.sender_counts.len(),
        }
    }

    pub fn contains(&self, id: &[u8; 32]) -> bool {
        self.transactions.contains_key(id)
    }

    pub fn is_empty(&self) -> bool {
        self.transactions.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn transaction(
        id: u8,
        sender: u8,
        nonce: u64,
        fee: u128,
        received_at: u64,
    ) -> PendingTransaction {
        PendingTransaction {
            id: [id; 32],
            sender: [sender; 32],
            nonce,
            fee,
            received_at,
            payload: vec![1, 2, 3, 4],
        }
    }

    #[test]
    fn admits_valid_pending_transaction() {
        let mut pool = Mempool::new(MempoolConfig::default());

        assert!(pool.admit(transaction(1, 1, 0, 10, 100)).is_ok());
        assert_eq!(pool.stats().transaction_count, 1);
    }

    #[test]
    fn rejects_duplicate_transaction() {
        let mut pool = Mempool::new(MempoolConfig::default());
        let tx = transaction(1, 1, 0, 10, 100);

        assert!(pool.admit(tx.clone()).is_ok());
        assert_eq!(
            pool.admit(tx),
            Err(MempoolError::DuplicateTransaction)
        );
    }

    #[test]
    fn rejects_empty_payload() {
        let mut pool = Mempool::new(MempoolConfig::default());
        let mut tx = transaction(1, 1, 0, 10, 100);
        tx.payload.clear();

        assert_eq!(
            pool.admit(tx),
            Err(MempoolError::InvalidTransactionSize)
        );
    }

    #[test]
    fn enforces_sender_limit() {
        let config = MempoolConfig {
            max_per_sender: 1,
            ..MempoolConfig::default()
        };

        let mut pool = Mempool::new(config);

        assert!(pool.admit(transaction(1, 1, 0, 10, 100)).is_ok());

        assert_eq!(
            pool.admit(transaction(2, 1, 1, 10, 100)),
            Err(MempoolError::SenderLimitReached)
        );
    }

    #[test]
    fn enforces_transaction_count_limit() {
        let config = MempoolConfig {
            max_transactions: 1,
            ..MempoolConfig::default()
        };

        let mut pool = Mempool::new(config);

        assert!(pool.admit(transaction(1, 1, 0, 10, 100)).is_ok());

        assert_eq!(
            pool.admit(transaction(2, 2, 0, 10, 100)),
            Err(MempoolError::MempoolFull)
        );
    }

    #[test]
    fn expires_old_transactions() {
        let config = MempoolConfig {
            transaction_ttl: 100,
            ..MempoolConfig::default()
        };

        let mut pool = Mempool::new(config);

        assert!(pool.admit(transaction(1, 1, 0, 10, 100)).is_ok());

        assert_eq!(pool.expire(200), 1);
        assert!(pool.is_empty());
    }

    #[test]
    fn removal_updates_accounting() {
        let mut pool = Mempool::new(MempoolConfig::default());
        let tx = transaction(1, 1, 0, 10, 100);
        let id = tx.id;

        assert!(pool.admit(tx).is_ok());
        assert!(pool.remove(&id).is_some());

        assert_eq!(pool.stats().transaction_count, 0);
        assert_eq!(pool.stats().total_bytes, 0);
    }

    #[test]
    fn higher_fee_per_byte_gets_priority() {
        let mut pool = Mempool::new(MempoolConfig::default());

        assert!(pool.admit(transaction(1, 1, 0, 10, 100)).is_ok());
        assert!(pool.admit(transaction(2, 2, 0, 100, 100)).is_ok());

        let selected = pool.select_for_block(2, 100);

        assert_eq!(selected[0].id, [2; 32]);
    }

    #[test]
    fn sender_cannot_monopolise_block_selection() {
        let config = MempoolConfig {
            max_per_sender_per_block: 1,
            ..MempoolConfig::default()
        };

        let mut pool = Mempool::new(config);

        assert!(pool.admit(transaction(1, 1, 0, 100, 100)).is_ok());
        assert!(pool.admit(transaction(2, 1, 1, 90, 100)).is_ok());
        assert!(pool.admit(transaction(3, 2, 0, 80, 100)).is_ok());

        let selected = pool.select_for_block(3, 100);

        assert_eq!(selected.len(), 2);
    }

    #[test]
    fn selection_does_not_remove_transactions() {
        let mut pool = Mempool::new(MempoolConfig::default());

        assert!(pool.admit(transaction(1, 1, 0, 10, 100)).is_ok());

        let _ = pool.select_for_block(1, 100);

        assert_eq!(pool.stats().transaction_count, 1);
    }
}
