//! NOVARA transaction mempool.
//!
//! Provides bounded pending-transaction storage, expiration,
//! removal, statistics, and deterministic block selection.
//!
//! Transactions must be validated by the caller before admission.
//! Expected account nonces must be supplied during block selection.

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
    /// Creates an empty mempool.
    pub fn new(config: MempoolConfig) -> Self {
        Self {
            config,
            transactions: BTreeMap::new(),
            sender_counts: BTreeMap::new(),
            total_bytes: 0,
        }
    }

    /// Adds a transaction after caller-side validation.
    pub fn admit(&mut self, transaction: PendingTransaction) -> Result<(), MempoolError> {
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
            || self.total_bytes.saturating_add(size) > self.config.max_total_bytes
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

    /// Removes transactions that have reached their configured lifetime.
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

    /// Removes a transaction by identifier.
    pub fn remove(&mut self, id: &[u8; 32]) -> Option<PendingTransaction> {
        let transaction = self.transactions.remove(id)?;

        self.total_bytes = self.total_bytes.saturating_sub(transaction.size_bytes());

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

    /// Selects transactions using expected account nonces.
    ///
    /// For each sender, selection starts at the expected nonce and
    /// proceeds only through consecutive available nonces.
    ///
    /// Transactions with stale nonces or gaps are not selected.
    /// If multiple transactions share the same sender and nonce,
    /// the transaction with the lowest ID is chosen deterministically.
    ///
    /// Senders are served in deterministic round-robin order.
    /// Selection does not remove transactions from the mempool.
    pub fn select_for_block(
        &self,
        maximum_transactions: usize,
        current_time: u64,
        expected_nonces: &BTreeMap<[u8; 32], u64>,
    ) -> Vec<PendingTransaction> {
        if maximum_transactions == 0 {
            return Vec::new();
        }

        let mut sender_queues: BTreeMap<
            [u8; 32],
            BTreeMap<u64, PendingTransaction>,
        > = BTreeMap::new();

        for transaction in self.transactions.values() {
            let age = current_time.saturating_sub(transaction.received_at);

            if age >= self.config.transaction_ttl {
                continue;
            }

            let Some(expected_nonce) = expected_nonces.get(&transaction.sender) else {
                continue;
            };

            if transaction.nonce < *expected_nonce {
                continue;
            }

            let nonce_queue = sender_queues
                .entry(transaction.sender)
                .or_default();

            nonce_queue
                .entry(transaction.nonce)
                .or_insert_with(|| transaction.clone());
        }

        let mut eligible_queues: BTreeMap<
            [u8; 32],
            Vec<PendingTransaction>,
        > = BTreeMap::new();

        for (sender, nonce_queue) in sender_queues {
            let Some(expected_nonce) = expected_nonces.get(&sender) else {
                continue;
            };

            let mut next_nonce = *expected_nonce;
            let mut queue = Vec::new();

            while let Some(transaction) = nonce_queue.get(&next_nonce) {
                queue.push(transaction.clone());

                let Some(incremented_nonce) = next_nonce.checked_add(1) else {
                    break;
                };

                next_nonce = incremented_nonce;
            }

            if !queue.is_empty() {
                eligible_queues.insert(sender, queue);
            }
        }

        let mut positions: BTreeMap<[u8; 32], usize> = BTreeMap::new();
        let mut selected_per_sender: BTreeMap<[u8; 32], usize> = BTreeMap::new();
        let mut selected = Vec::new();

        while selected.len() < maximum_transactions {
            let mut made_progress = false;

            for (sender, queue) in &eligible_queues {
                if selected.len() >= maximum_transactions {
                    break;
                }

                let selected_count = selected_per_sender
                    .get(sender)
                    .copied()
                    .unwrap_or(0);

                if selected_count >= self.config.max_per_sender_per_block {
                    continue;
                }

                let position = positions.get(sender).copied().unwrap_or(0);

                if let Some(transaction) = queue.get(position) {
                    selected.push(transaction.clone());

                    positions.insert(*sender, position + 1);

                    selected_per_sender.insert(*sender, selected_count + 1);

                    made_progress = true;
                }
            }

            if !made_progress {
                break;
            }
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

    fn expected_nonce(sender: u8, nonce: u64) -> BTreeMap<[u8; 32], u64> {
        BTreeMap::from([([sender; 32], nonce)])
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

        assert_eq!(pool.admit(tx), Err(MempoolError::InvalidTransactionSize));
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
    fn selects_consecutive_nonces_in_order() {
        let mut pool = Mempool::new(MempoolConfig::default());

        assert!(pool.admit(transaction(1, 1, 5, 10, 100)).is_ok());
        assert!(pool.admit(transaction(2, 1, 6, 10, 100)).is_ok());
        assert!(pool.admit(transaction(3, 1, 7, 10, 100)).is_ok());

        let nonces = expected_nonce(1, 5);
        let selected = pool.select_for_block(10, 100, &nonces);

        assert_eq!(
            selected.iter().map(|tx| tx.nonce).collect::<Vec<_>>(),
            vec![5, 6, 7]
        );
    }

    #[test]
    fn does_not_select_stale_nonces() {
        let mut pool = Mempool::new(MempoolConfig::default());

        assert!(pool.admit(transaction(1, 1, 4, 10, 100)).is_ok());
        assert!(pool.admit(transaction(2, 1, 5, 10, 100)).is_ok());

        let nonces = expected_nonce(1, 5);
        let selected = pool.select_for_block(10, 100, &nonces);

        assert_eq!(selected.len(), 1);
        assert_eq!(selected[0].nonce, 5);
    }

    #[test]
    fn does_not_skip_missing_nonce() {
        let mut pool = Mempool::new(MempoolConfig::default());

        assert!(pool.admit(transaction(1, 1, 5, 10, 100)).is_ok());
        assert!(pool.admit(transaction(2, 1, 7, 10, 100)).is_ok());

        let nonces = expected_nonce(1, 5);
        let selected = pool.select_for_block(10, 100, &nonces);

        assert_eq!(selected.len(), 1);
        assert_eq!(selected[0].nonce, 5);
    }

    #[test]
    fn does_not_select_transactions_without_expected_nonce() {
        let mut pool = Mempool::new(MempoolConfig::default());

        assert!(pool.admit(transaction(1, 1, 0, 10, 100)).is_ok());

        let selected = pool.select_for_block(10, 100, &BTreeMap::new());

        assert!(selected.is_empty());
    }

    #[test]
    fn duplicate_sender_nonce_uses_lowest_id() {
        let mut pool = Mempool::new(MempoolConfig::default());

        assert!(pool.admit(transaction(2, 1, 5, 100, 100)).is_ok());
        assert!(pool.admit(transaction(1, 1, 5, 10, 100)).is_ok());

        let nonces = expected_nonce(1, 5);
        let selected = pool.select_for_block(10, 100, &nonces);

        assert_eq!(selected.len(), 1);
        assert_eq!(selected[0].id, [1; 32]);
    }

    #[test]
    fn senders_receive_round_robin_selection() {
        let mut pool = Mempool::new(MempoolConfig::default());

        assert!(pool.admit(transaction(1, 1, 0, 100, 100)).is_ok());
        assert!(pool.admit(transaction(2, 1, 1, 100, 100)).is_ok());
        assert!(pool.admit(transaction(3, 2, 0, 100, 100)).is_ok());
        assert!(pool.admit(transaction(4, 2, 1, 100, 100)).is_ok());

        let mut nonces = BTreeMap::new();
        nonces.insert([1; 32], 0);
        nonces.insert([2; 32], 0);

        let selected = pool.select_for_block(4, 100, &nonces);

        assert_eq!(selected.len(), 4);
        assert_eq!(selected[0].sender, [1; 32]);
        assert_eq!(selected[1].sender, [2; 32]);
        assert_eq!(selected[2].sender, [1; 32]);
        assert_eq!(selected[3].sender, [2; 32]);
    }

    #[test]
    fn selection_is_deterministic() {
        let mut pool = Mempool::new(MempoolConfig::default());

        assert!(pool.admit(transaction(1, 1, 0, 100, 100)).is_ok());
        assert!(pool.admit(transaction(2, 1, 1, 100, 100)).is_ok());
        assert!(pool.admit(transaction(3, 2, 0, 100, 100)).is_ok());

        let mut nonces = BTreeMap::new();
        nonces.insert([1; 32], 0);
        nonces.insert([2; 32], 0);

        let first = pool.select_for_block(10, 100, &nonces);
        let second = pool.select_for_block(10, 100, &nonces);

        assert_eq!(first, second);
    }

    #[test]
    fn selection_does_not_remove_transactions() {
        let mut pool = Mempool::new(MempoolConfig::default());
        let tx = transaction(1, 1, 0, 10, 100);
        let id = tx.id;

        assert!(pool.admit(tx).is_ok());

        let nonces = expected_nonce(1, 0);
        let _ = pool.select_for_block(10, 100, &nonces);

        assert!(pool.contains(&id));
        assert_eq!(pool.stats().transaction_count, 1);
    }

    #[test]
    fn respects_per_sender_block_limit() {
        let config = MempoolConfig {
            max_per_sender_per_block: 1,
            ..MempoolConfig::default()
        };

        let mut pool = Mempool::new(config);

        assert!(pool.admit(transaction(1, 1, 0, 10, 100)).is_ok());
        assert!(pool.admit(transaction(2, 1, 1, 10, 100)).is_ok());

        let nonces = expected_nonce(1, 0);
        let selected = pool.select_for_block(10, 100, &nonces);

        assert_eq!(selected.len(), 1);
        assert_eq!(selected[0].nonce, 0);
    }

    #[test]
    fn does_not_select_expired_transactions() {
        let config = MempoolConfig {
            transaction_ttl: 10,
            ..MempoolConfig::default()
        };

        let mut pool = Mempool::new(config);

        assert!(pool.admit(transaction(1, 1, 0, 10, 100)).is_ok());

        let nonces = expected_nonce(1, 0);
        let selected = pool.select_for_block(10, 110, &nonces);

        assert!(selected.is_empty());
    }
}
