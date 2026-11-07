//! NOVARA transaction mempool.
//!
//! This module provides bounded pending-transaction storage,
//! expiration, removal, statistics, and block-selection helpers.
//!
//! IMPORTANT:
//! Transactions must be validated by the caller before admission.
//! This module does not independently verify signatures, balances,
//! network identity, chain identity, or account nonces.

use std::collections::BTreeMap;

/// Configuration limits for the mempool.
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
            transaction_ttl: 3_600,
            max_per_sender_per_block: 10,
        }
    }
}

/// A transaction waiting to be included in a block.
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

/// Errors returned by mempool operations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MempoolError {
    DuplicateTransaction,
    TransactionTooLarge,
    MempoolFull,
    SenderLimitReached,
    InvalidTransactionSize,
}

/// Current mempool statistics.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MempoolStats {
    pub transaction_count: usize,
    pub total_bytes: usize,
    pub sender_count: usize,
}

/// Bounded storage for pending NOVARA transactions.
pub struct Mempool {
    config: MempoolConfig,
    transactions: BTreeMap<[u8; 32], PendingTransaction>,
    sender_counts: BTreeMap<[u8; 32], usize>,
    total_bytes: usize,
}

impl Mempool {
    /// Creates an empty mempool with the supplied configuration.
    pub fn new(config: MempoolConfig) -> Self {
        Self {
            config,
            transactions: BTreeMap::new(),
            sender_counts: BTreeMap::new(),
            total_bytes: 0,
        }
    }

    /// Adds a transaction after the caller has validated it.
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

        *self
            .sender_counts
            .entry(transaction.sender)
            .or_insert(0) += 1;

        self.transactions.insert(transaction.id, transaction);

        Ok(())
    }

    /// Removes transactions that have reached their configured lifetime.
    ///
    /// Returns the number of expired transactions removed.
    pub fn expire(&mut self, current_time: u64) -> usize {
        let expired_ids: Vec<[u8; 32]> = self
            .transactions
            .iter()
            .filter_map(|(id, transaction)| {
                let age = current_time.saturating_sub(transaction.received_at);

                if age >= self.config.transaction_ttl {
                    Some(*id)
                } else {
                    None
                }
            })
            .collect();

        let mut removed = 0;

        for id in expired_ids {
            if self.remove(&id).is_some() {
                removed += 1;
            }
        }

        removed
    }

    /// Removes a transaction by its identifier.
    ///
    /// Returns the removed transaction, if it existed.
    pub fn remove(
        &mut self,
        id: &[u8; 32],
    ) -> Option<PendingTransaction> {
        let transaction = self.transactions.remove(id)?;

        self.total_bytes = self
            .total_bytes
            .saturating_sub(transaction.size_bytes());

        let sender = transaction.sender;

        let remove_sender = if let Some(count) = self.sender_counts.get_mut(&sender) {
            *count = count.saturating_sub(1);
            *count == 0
        } else {
            false
        };

        if remove_sender {
            self.sender_counts.remove(&sender);
        }

        Some(transaction)
    }

    /// Selects transactions for a candidate block.
    ///
    /// Higher fee-per-byte transactions are prioritised.
    /// Older transactions break equal-priority ties.
    /// The sender limit prevents one sender from monopolising selection.
    ///
    /// This method does not remove selected transactions.
    /// Expected account nonces must be checked by the caller.
    pub fn select_for_block(
        &self,
        maximum_transactions: usize,
        current_time: u64,
    ) -> Vec<PendingTransaction> {
        let mut candidates: Vec<&PendingTransaction> =
            self.transactions.values().collect();

        candidates.sort_by(|a, b| {
            let a_size = a.size_bytes().max(1) as u128;
            let b_size = b.size_bytes().max(1) as u128;

            let a_priority = a.fee.saturating_mul(b_size);
            let b_priority = b.fee.saturating_mul(a_size);

            b_priority
                .cmp(&a_priority)
                .then_with(|| {
                    let a_age = current_time.saturating_sub(a.received_at);
                    let b_age = current_time.saturating_sub(b.received_at);
                    b_age.cmp(&a_age)
                })
                .then_with(|| a.id.cmp(&b.id))
        });

        let mut selected = Vec::new();
        let mut selected_per_sender: BTreeMap<[u8; 32], usize> =
            BTreeMap::new();

        for transaction in candidates {
            if selected.len() >= maximum_transactions {
                break;
            }

            let sender_count = selected_per_sender
                .get(&transaction.sender)
                .copied()
                .unwrap_or(0);

            if sender_count >= self.config.max_per_sender_per_block {
                continue;
            }

            *selected_per_sender
                .entry(transaction.sender)
                .or_insert(0) += 1;

            selected.push(transaction.clone());
        }

        selected
    }

    /// Returns a pending transaction by identifier.
    pub fn get(&self, id: &[u8; 32]) -> Option<&PendingTransaction> {
        self.transactions.get(id)
    }

    /// Returns current mempool statistics.
    pub fn stats(&self) -> MempoolStats {
        MempoolStats {
            transaction_count: self.transactions.len(),
            total_bytes: self.total_bytes,
            sender_count: self.sender_counts.len(),
        }
    }

    /// Checks whether a transaction exists.
    pub fn contains(&self, id: &[u8; 32]) -> bool {
        self.transactions.contains_key(id)
    }

    /// Checks whether the mempool is empty.
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
        let tx = transaction(1, 1, 0, 10, 100);

        assert!(pool.admit(tx).is_ok());
        assert_eq!(pool.stats().transaction_count, 1);
    }

    #[test]
    fn rejects_duplicate_transaction() {
        let mut pool = Mempool::new(MempoolConfig::default());
        let tx = transaction(1, 1, 0, 10, 100);

        assert!(pool.admit(tx.clone()).is_ok());
        assert_eq!(pool.admit(tx), Err(MempoolError::DuplicateTransaction));
    }

    #[test]
    fn rejects_empty_transaction_payload() {
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
            transaction_ttl: 10,
            ..MempoolConfig::default()
        };

        let mut pool = Mempool::new(config);

        assert!(pool.admit(transaction(1, 1, 0, 10, 100)).is_ok());

        assert_eq!(pool.expire(110), 1);
        assert!(pool.is_empty());
    }

    #[test]
    fn removal_updates_accounting() {
        let mut pool = Mempool::new(MempoolConfig::default());
        let tx = transaction(1, 1, 0, 10, 100);
        let id = tx.id;

        assert!(pool.admit(tx).is_ok());
        assert!(pool.remove(&id).is_some());

        let stats = pool.stats();

        assert_eq!(stats.transaction_count, 0);
        assert_eq!(stats.total_bytes, 0);
        assert_eq!(stats.sender_count, 0);
    }

    #[test]
    fn higher_fee_per_byte_gets_priority() {
        let mut pool = Mempool::new(MempoolConfig::default());

        let low_fee = PendingTransaction {
            fee: 10,
            ..transaction(1, 1, 0, 10, 100)
        };

        let high_fee = PendingTransaction {
            fee: 100,
            ..transaction(2, 2, 0, 100, 100)
        };

        assert!(pool.admit(low_fee).is_ok());
        assert!(pool.admit(high_fee).is_ok());

        let selected = pool.select_for_block(1, 100);

        assert_eq!(selected.len(), 1);
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
        assert_ne!(selected[0].sender, selected[1].sender);
    }

    #[test]
    fn selection_does_not_remove_transactions() {
        let mut pool = Mempool::new(MempoolConfig::default());
        let tx = transaction(1, 1, 0, 10, 100);
        let id = tx.id;

        assert!(pool.admit(tx).is_ok());

        let _ = pool.select_for_block(10, 100);

        assert!(pool.contains(&id));
        assert_eq!(pool.stats().transaction_count, 1);
    }
}
