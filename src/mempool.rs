use std::collections::{BTreeMap, HashMap};
use std::error::Error;
use std::fmt;

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
    pub fn size(&self) -> usize {
        self.payload.len()
    }

    pub fn is_expired(&self, current_time: u64, ttl: u64) -> bool {
        current_time.saturating_sub(self.received_at) >= ttl
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

impl fmt::Display for MempoolError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DuplicateTransaction => {
                write!(f, "Transaction already exists in mempool")
            }
            Self::TransactionTooLarge => {
                write!(f, "Transaction exceeds maximum permitted size")
            }
            Self::MempoolFull => write!(f, "Mempool capacity reached"),
            Self::SenderLimitReached => {
                write!(f, "Sender transaction limit reached")
            }
            Self::InvalidTransactionSize => {
                write!(f, "Transaction payload cannot be empty")
            }
        }
    }
}

impl Error for MempoolError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MempoolStats {
    pub transaction_count: usize,
    pub total_bytes: usize,
    pub sender_count: usize,
}

#[derive(Debug)]
pub struct Mempool {
    config: MempoolConfig,
    transactions: HashMap<[u8; 32], PendingTransaction>,
    sender_counts: HashMap<[u8; 32], usize>,
    total_bytes: usize,
}

impl Mempool {
    pub fn new(config: MempoolConfig) -> Self {
        Self {
            config,
            transactions: HashMap::new(),
            sender_counts: HashMap::new(),
            total_bytes: 0,
        }
    }

    pub fn admit(
        &mut self,
        transaction: PendingTransaction,
    ) -> Result<(), MempoolError> {
        if transaction.payload.is_empty() {
            return Err(MempoolError::InvalidTransactionSize);
        }

        if transaction.size() > self.config.max_transaction_bytes {
            return Err(MempoolError::TransactionTooLarge);
        }

        if self.transactions.contains_key(&transaction.id) {
            return Err(MempoolError::DuplicateTransaction);
        }

        if self.transactions.len() >= self.config.max_transactions {
            return Err(MempoolError::MempoolFull);
        }

        let new_total = self
            .total_bytes
            .checked_add(transaction.size())
            .ok_or(MempoolError::MempoolFull)?;

        if new_total > self.config.max_total_bytes {
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

        self.total_bytes = new_total;
        *self.sender_counts.entry(transaction.sender).or_insert(0) += 1;

        self.transactions.insert(transaction.id, transaction);

        Ok(())
    }

    pub fn remove(&mut self, id: &[u8; 32]) -> Option<PendingTransaction> {
        let transaction = self.transactions.remove(id)?;

        self.total_bytes = self.total_bytes.saturating_sub(transaction.size());

        if let Some(count) = self.sender_counts.get_mut(&transaction.sender) {
            *count = count.saturating_sub(1);

            if *count == 0 {
                self.sender_counts.remove(&transaction.sender);
            }
        }

        Some(transaction)
    }

    pub fn expire(&mut self, current_time: u64) -> usize {
        let expired_ids: Vec<[u8; 32]> = self
            .transactions
            .values()
            .filter(|transaction| {
                transaction.is_expired(current_time, self.config.transaction_ttl)
            })
            .map(|transaction| transaction.id)
            .collect();

        let expired_count = expired_ids.len();

        for id in expired_ids {
            self.remove(&id);
        }

        expired_count
    }

    pub fn get(&self, id: &[u8; 32]) -> Option<&PendingTransaction> {
        self.transactions.get(id)
    }

    pub fn contains(&self, id: &[u8; 32]) -> bool {
        self.transactions.contains_key(id)
    }

    pub fn is_empty(&self) -> bool {
        self.transactions.is_empty()
    }

    pub fn stats(&self) -> MempoolStats {
        MempoolStats {
            transaction_count: self.transactions.len(),
            total_bytes: self.total_bytes,
            sender_count: self.sender_counts.len(),
        }
    }

    pub fn select_for_block(
        &self,
        maximum_transactions: usize,
        current_time: u64,
        expected_nonces: &HashMap<[u8; 32], u64>,
    ) -> Vec<PendingTransaction> {
        if maximum_transactions == 0 {
            return Vec::new();
        }

        let mut sender_queues: BTreeMap<[u8; 32], BTreeMap<u64, PendingTransaction>> =
            BTreeMap::new();

        for transaction in self.transactions.values() {
            if transaction.is_expired(current_time, self.config.transaction_ttl) {
                continue;
            }

            let Some(expected_nonce) = expected_nonces.get(&transaction.sender) else {
                continue;
            };

            if transaction.nonce < *expected_nonce {
                continue;
            }

            let nonce_queue = sender_queues.entry(transaction.sender).or_default();

            match nonce_queue.get(&transaction.nonce) {
                Some(existing) if existing.id <= transaction.id => {}
                _ => {
                    nonce_queue.insert(transaction.nonce, transaction.clone());
                }
            }
        }

        let mut eligible_queues: BTreeMap<[u8; 32], Vec<PendingTransaction>> = BTreeMap::new();

        for (sender, nonce_queue) in sender_queues {
            let Some(expected_nonce) = expected_nonces.get(&sender) else {
                continue;
            };

            let mut eligible = Vec::new();
            let mut next_nonce = *expected_nonce;

            while let Some(transaction) = nonce_queue.get(&next_nonce) {
                eligible.push(transaction.clone());

                let Some(incremented_nonce) = next_nonce.checked_add(1) else {
                    break;
                };

                next_nonce = incremented_nonce;
            }

            if !eligible.is_empty() {
                eligible_queues.insert(sender, eligible);
            }
        }

        let mut selected = Vec::new();
        let mut selected_per_sender: HashMap<[u8; 32], usize> = HashMap::new();

        while selected.len() < maximum_transactions {
            let mut made_progress = false;

            for (sender, queue) in eligible_queues.iter_mut() {
                if selected.len() >= maximum_transactions {
                    break;
                }

                let selected_count = selected_per_sender.get(sender).copied().unwrap_or(0);

                if selected_count >= self.config.max_per_sender_per_block {
                    continue;
                }

                if queue.is_empty() {
                    continue;
                }

                let transaction = queue.remove(0);

                selected.push(transaction);

                *selected_per_sender.entry(*sender).or_insert(0) += 1;

                made_progress = true;
            }

            if !made_progress {
                break;
            }
        }

        selected
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_transaction(
        id: u8,
        sender: u8,
        nonce: u64,
        received_at: u64,
    ) -> PendingTransaction {
        PendingTransaction {
            id: [id; 32],
            sender: [sender; 32],
            nonce,
            fee: 100,
            received_at,
            payload: vec![1, 2, 3],
        }
    }

    fn test_config() -> MempoolConfig {
        MempoolConfig {
            max_transactions: 100,
            max_total_bytes: 10_000,
            max_transaction_bytes: 1_000,
            max_per_sender: 10,
            transaction_ttl: 100,
            max_per_sender_per_block: 10,
        }
    }

    #[test]
    fn admits_transaction() {
        let mut mempool = Mempool::new(test_config());

        let transaction = test_transaction(1, 1, 0, 10);

        assert!(mempool.admit(transaction.clone()).is_ok());
        assert!(mempool.contains(&transaction.id));
    }

    #[test]
    fn rejects_duplicate_transaction() {
        let mut mempool = Mempool::new(test_config());

        let transaction = test_transaction(1, 1, 0, 10);

        assert!(mempool.admit(transaction.clone()).is_ok());
        assert_eq!(
            mempool.admit(transaction),
            Err(MempoolError::DuplicateTransaction)
        );
    }

    #[test]
    fn rejects_empty_payload() {
        let mut mempool = Mempool::new(test_config());

        let mut transaction = test_transaction(1, 1, 0, 10);
        transaction.payload.clear();

        assert_eq!(
            mempool.admit(transaction),
            Err(MempoolError::InvalidTransactionSize)
        );
    }

    #[test]
    fn enforces_sender_limit() {
        let mut config = test_config();
        config.max_per_sender = 1;

        let mut mempool = Mempool::new(config);

        assert!(mempool.admit(test_transaction(1, 1, 0, 10)).is_ok());

        assert_eq!(
            mempool.admit(test_transaction(2, 1, 1, 10)),
            Err(MempoolError::SenderLimitReached)
        );
    }

    #[test]
    fn enforces_transaction_limit() {
        let mut config = test_config();
        config.max_transactions = 1;

        let mut mempool = Mempool::new(config);

        assert!(mempool.admit(test_transaction(1, 1, 0, 10)).is_ok());

        assert_eq!(
            mempool.admit(test_transaction(2, 2, 0, 10)),
            Err(MempoolError::MempoolFull)
        );
    }

    #[test]
    fn expires_old_transactions() {
        let mut mempool = Mempool::new(test_config());

        mempool
            .admit(test_transaction(1, 1, 0, 10))
            .unwrap();

        let expired = mempool.expire(111);

        assert_eq!(expired, 1);
        assert!(mempool.is_empty());
    }

    #[test]
    fn removes_transaction() {
        let mut mempool = Mempool::new(test_config());

        let transaction = test_transaction(1, 1, 0, 10);
        let id = transaction.id;

        mempool.admit(transaction).unwrap();

        assert!(mempool.remove(&id).is_some());
        assert!(!mempool.contains(&id));
    }

    #[test]
    fn selects_consecutive_nonces() {
        let mut mempool = Mempool::new(test_config());

        mempool
            .admit(test_transaction(1, 1, 0, 10))
            .unwrap();

        mempool
            .admit(test_transaction(2, 1, 1, 10))
            .unwrap();

        mempool
            .admit(test_transaction(3, 1, 2, 10))
            .unwrap();

        let mut expected = HashMap::new();
        expected.insert([1; 32], 0);

        let selected = mempool.select_for_block(10, 20, &expected);

        assert_eq!(selected.len(), 3);
        assert_eq!(selected[0].nonce, 0);
        assert_eq!(selected[1].nonce, 1);
        assert_eq!(selected[2].nonce, 2);
    }

    #[test]
    fn excludes_stale_nonces() {
        let mut mempool = Mempool::new(test_config());

        mempool
            .admit(test_transaction(1, 1, 0, 10))
            .unwrap();

        mempool
            .admit(test_transaction(2, 1, 1, 10))
            .unwrap();

        let mut expected = HashMap::new();
        expected.insert([1; 32], 1);

        let selected = mempool.select_for_block(10, 20, &expected);

        assert_eq!(selected.len(), 1);
        assert_eq!(selected[0].nonce, 1);
    }

    #[test]
    fn nonce_gap_blocks_later_transactions() {
        let mut mempool = Mempool::new(test_config());

        mempool
            .admit(test_transaction(1, 1, 0, 10))
            .unwrap();

        mempool
            .admit(test_transaction(3, 1, 2, 10))
            .unwrap();

        let mut expected = HashMap::new();
        expected.insert([1; 32], 0);

        let selected = mempool.select_for_block(10, 20, &expected);

        assert_eq!(selected.len(), 1);
        assert_eq!(selected[0].nonce, 0);
    }

    #[test]
    fn excludes_senders_without_expected_nonce() {
        let mut mempool = Mempool::new(test_config());

        mempool
            .admit(test_transaction(1, 1, 0, 10))
            .unwrap();

        let expected = HashMap::new();

        let selected = mempool.select_for_block(10, 20, &expected);

        assert!(selected.is_empty());
    }

    #[test]
    fn duplicate_sender_nonce_selection_is_deterministic() {
        let mut mempool = Mempool::new(test_config());

        mempool
            .admit(test_transaction(2, 1, 0, 10))
            .unwrap();

        mempool
            .admit(test_transaction(1, 1, 0, 10))
            .unwrap();

        let mut expected = HashMap::new();
        expected.insert([1; 32], 0);

        let selected = mempool.select_for_block(10, 20, &expected);

        assert_eq!(selected.len(), 1);
        assert_eq!(selected[0].id, [1; 32]);
    }

    #[test]
    fn schedules_senders_round_robin() {
        let mut mempool = Mempool::new(test_config());

        mempool
            .admit(test_transaction(1, 1, 0, 10))
            .unwrap();

        mempool
            .admit(test_transaction(2, 1, 1, 10))
            .unwrap();

        mempool
            .admit(test_transaction(3, 2, 0, 10))
            .unwrap();

        mempool
            .admit(test_transaction(4, 2, 1, 10))
            .unwrap();

        let mut expected = HashMap::new();
        expected.insert([1; 32], 0);
        expected.insert([2; 32], 0);

        let selected = mempool.select_for_block(4, 20, &expected);

        assert_eq!(selected.len(), 4);
        assert_eq!(selected[0].sender, [1; 32]);
        assert_eq!(selected[1].sender, [2; 32]);
        assert_eq!(selected[2].sender, [1; 32]);
        assert_eq!(selected[3].sender, [2; 32]);
    }

    #[test]
    fn selection_is_deterministic() {
        let mut mempool = Mempool::new(test_config());

        mempool
            .admit(test_transaction(1, 1, 0, 10))
            .unwrap();

        mempool
            .admit(test_transaction(2, 2, 0, 10))
            .unwrap();

        let mut expected = HashMap::new();
        expected.insert([1; 32], 0);
        expected.insert([2; 32], 0);

        let first = mempool.select_for_block(10, 20, &expected);
        let second = mempool.select_for_block(10, 20, &expected);

        assert_eq!(first, second);
    }

    #[test]
    fn selection_does_not_remove_transactions() {
        let mut mempool = Mempool::new(test_config());

        let transaction = test_transaction(1, 1, 0, 10);
        let id = transaction.id;

        mempool.admit(transaction).unwrap();

        let mut expected = HashMap::new();
        expected.insert([1; 32], 0);

        mempool.select_for_block(10, 20, &expected);

        assert!(mempool.contains(&id));
    }

    #[test]
    fn enforces_per_sender_block_limit() {
        let mut config = test_config();
        config.max_per_sender_per_block = 1;

        let mut mempool = Mempool::new(config);

        mempool
            .admit(test_transaction(1, 1, 0, 10))
            .unwrap();

        mempool
            .admit(test_transaction(2, 1, 1, 10))
            .unwrap();

        let mut expected = HashMap::new();
        expected.insert([1; 32], 0);

        let selected = mempool.select_for_block(10, 20, &expected);

        assert_eq!(selected.len(), 1);
    }

    #[test]
    fn excludes_expired_transactions_during_selection() {
        let mut mempool = Mempool::new(test_config());

        mempool
            .admit(test_transaction(1, 1, 0, 10))
            .unwrap();

        let mut expected = HashMap::new();
        expected.insert([1; 32], 0);

        let selected = mempool.select_for_block(10, 111, &expected);

        assert!(selected.is_empty());
    }
}
