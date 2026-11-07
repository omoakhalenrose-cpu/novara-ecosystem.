//! NOVARA in-memory transaction pool (component version 0.3.0).
//!
//! The pool is separate from [`crate::chain::Chain`]. Callers submit signed
//! transactions against the current state and reconcile only after a block has
//! been successfully applied.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, RwLock};

use sha2::{Digest, Sha256};

use crate::account::address::AccountId;
use crate::block::{Block, SignedTransaction};
use crate::state::{State, StateError};
use crate::transaction::validation::{
    ValidationContext, ValidationError, validate_transaction,
};
use crate::transaction::{AssetId, ChainId, Transaction};

/// NOVARA mempool component version. This is independent of the crate version.
pub const MEMPOOL_VERSION: &str = "0.3.0";

const TRANSACTION_ID_DOMAIN: &[u8] = b"NOVARA_MEMPOOL_TRANSACTION_ID_V1";

/// Errors returned by mempool operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MempoolError {
    InvalidTransaction(ValidationError),
    DuplicateTransaction,
    DuplicateSenderNonce,
    NonceMismatch,
    AccountNotFound,
    InsufficientBalance,
    CapacityReached,
    BalanceOverflow,
    NonceOverflow,
    LockPoisoned,
}

/// Thread-safe bounded storage of pending signed transactions.
///
/// Transactions are keyed by a SHA-256 ID over their canonical transaction
/// encoding and signature. Sender queues are ordered by sender bytes and nonce.
#[derive(Debug, Clone)]
pub struct Mempool {
    capacity: usize,
    context: ValidationContext,
    inner: Arc<RwLock<MempoolInner>>,
}

#[derive(Debug, Default)]
struct MempoolInner {
    transactions: BTreeMap<[u8; 32], SignedTransaction>,
    senders: BTreeMap<[u8; 32], BTreeMap<u64, [u8; 32]>>,
}

impl Mempool {
    /// Creates an empty pool with a fixed maximum transaction count.
    pub fn new(capacity: usize, context: ValidationContext) -> Self {
        Self {
            capacity,
            context,
            inner: Arc::new(RwLock::new(MempoolInner::default())),
        }
    }

    /// Returns the configured maximum number of pending transactions.
    pub const fn capacity(&self) -> usize {
        self.capacity
    }

    /// Returns the number of pending transactions.
    pub fn len(&self) -> Result<usize, MempoolError> {
        self.inner
            .read()
            .map(|inner| inner.transactions.len())
            .map_err(|_| MempoolError::LockPoisoned)
    }

    /// Returns whether there are no pending transactions.
    pub fn is_empty(&self) -> Result<bool, MempoolError> {
        self.len().map(|len| len == 0)
    }

    /// Validates and admits a signed transaction against the supplied state.
    ///
    /// The nonce must equal the state's current nonce plus the number of
    /// already-pending transactions from that sender. Pending outgoing amounts
    /// are reserved per asset, preventing the sender from overcommitting its
    /// current on-chain balance. Incoming pending transfers are not spendable.
    pub fn submit(
        &self,
        signed_transaction: SignedTransaction,
        state: &State,
    ) -> Result<[u8; 32], MempoolError> {
        let mut inner = self
            .inner
            .write()
            .map_err(|_| MempoolError::LockPoisoned)?;

        insert_transaction(
            &mut inner,
            self.capacity,
            &self.context,
            state,
            signed_transaction,
        )
    }

    /// Returns pending transactions in deterministic sender-ID, then nonce order.
    pub fn select_transactions(&self) -> Result<Vec<SignedTransaction>, MempoolError> {
        let inner = self
            .inner
            .read()
            .map_err(|_| MempoolError::LockPoisoned)?;

        let mut selected = Vec::with_capacity(inner.transactions.len());
        for nonce_queue in inner.senders.values() {
            for transaction_id in nonce_queue.values() {
                if let Some(transaction) = inner.transactions.get(transaction_id) {
                    selected.push(transaction.clone());
                }
            }
        }
        Ok(selected)
    }

    /// Removes transactions included in an already-applied block and revalidates
    /// all remaining transactions against the resulting state.
    ///
    /// The caller is responsible for invoking this only after successful block
    /// application. The returned count includes included and newly invalidated
    /// transactions removed from the pool.
    pub fn reconcile(&self, block: &Block, resulting_state: &State) -> Result<usize, MempoolError> {
        let mut inner = self
            .inner
            .write()
            .map_err(|_| MempoolError::LockPoisoned)?;

        let included: BTreeSet<[u8; 32]> = block
            .transactions
            .iter()
            .map(transaction_id)
            .collect();

        let remaining: Vec<SignedTransaction> = inner
            .transactions
            .iter()
            .filter(|(id, _)| !included.contains(*id))
            .map(|(_, transaction)| transaction.clone())
            .collect();
        let old_len = inner.transactions.len();

        let mut rebuilt = MempoolInner::default();
        for transaction in remaining {
            // Reconciliation is pruning: entries that no longer fit the new
            // state or valid consecutive nonce sequence are discarded.
            let _ = insert_transaction(
                &mut rebuilt,
                self.capacity,
                &self.context,
                resulting_state,
                transaction,
            );
        }

        let removed = old_len.saturating_sub(rebuilt.transactions.len());
        *inner = rebuilt;
        Ok(removed)
    }
}

/// Computes the stable ID of a signed transaction.
pub fn transaction_id(signed_transaction: &SignedTransaction) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(TRANSACTION_ID_DOMAIN);
    hasher.update(signed_transaction.transaction.encode());
    hasher.update(signed_transaction.signature.public_key);
    hasher.update(signed_transaction.signature.signature);
    hasher.finalize().into()
}

fn insert_transaction(
    inner: &mut MempoolInner,
    capacity: usize,
    context: &ValidationContext,
    state: &State,
    signed_transaction: SignedTransaction,
) -> Result<[u8; 32], MempoolError> {
    let transaction = &signed_transaction.transaction;
    validate_transaction(transaction, &signed_transaction.signature, context)
        .map_err(MempoolError::InvalidTransaction)?;

    let id = transaction_id(&signed_transaction);
    if inner.transactions.contains_key(&id) {
        return Err(MempoolError::DuplicateTransaction);
    }

    if inner.transactions.len() >= capacity {
        return Err(MempoolError::CapacityReached);
    }

    let sender_key = *transaction.sender.as_bytes();
    let sender_queue = inner.senders.get(&sender_key);
    let current_nonce = state
        .nonce_of(&transaction.sender)
        .map_err(map_state_error)?;
    let pending_count = sender_queue.map_or(0, BTreeMap::len);
    let pending_count = u64::try_from(pending_count).map_err(|_| MempoolError::NonceOverflow)?;
    let expected_nonce = current_nonce
        .checked_add(pending_count)
        .ok_or(MempoolError::NonceOverflow)?;
    if transaction.nonce != expected_nonce {
        return Err(MempoolError::NonceMismatch);
    }

    if sender_queue.is_some_and(|queue| queue.contains_key(&transaction.nonce)) {
        return Err(MempoolError::DuplicateSenderNonce);
    }

    let sender_balance = state
        .balance_of(&transaction.sender, &transaction.asset_id)
        .map_err(map_state_error)?;
    if sender_balance < transaction.amount {
        return Err(MempoolError::InsufficientBalance);
    }

    let pending_spent = pending_amount_for_sender_asset(
        inner,
        &transaction.sender,
        &transaction.asset_id,
    )?;
    // A transfer to self advances the nonce but does not reduce the balance in
    // State::execute_transaction, so it is not reserved as cumulative spending.
    let additional_spending = if transaction.sender == transaction.recipient {
        0
    } else {
        transaction.amount
    };
    let total_spending = pending_spent
        .checked_add(additional_spending)
        .ok_or(MempoolError::BalanceOverflow)?;
    if total_spending > sender_balance {
        return Err(MempoolError::InsufficientBalance);
    }

    if transaction.sender != transaction.recipient {
        state
            .balance_of(&transaction.recipient, &transaction.asset_id)
            .map_err(map_state_error)?;
    }

    let nonce_queue = inner.senders.entry(sender_key).or_default();
    nonce_queue.insert(transaction.nonce, id);
    inner.transactions.insert(id, signed_transaction);
    Ok(id)
}

fn pending_amount_for_sender_asset(
    inner: &MempoolInner,
    sender: &AccountId,
    asset_id: &AssetId,
) -> Result<u128, MempoolError> {
    let Some(queue) = inner.senders.get(sender.as_bytes()) else {
        return Ok(0);
    };

    queue.values().try_fold(0u128, |total, id| {
        let Some(pending) = inner.transactions.get(id) else {
            return Ok(total);
        };
        let transaction = &pending.transaction;
        if transaction.asset_id != *asset_id || transaction.sender == transaction.recipient {
            return Ok(total);
        }
        total
            .checked_add(transaction.amount)
            .ok_or(MempoolError::BalanceOverflow)
    })
}

fn map_state_error(error: StateError) -> MempoolError {
    match error {
        StateError::AccountNotFound => MempoolError::AccountNotFound,
        StateError::AccountAlreadyExists => MempoolError::AccountNotFound,
    }
}

#[cfg(test)]
mod tests {
    use std::thread;

    use super::*;
    use crate::account::network::NetworkId;
    use crate::block::{BlockHash, SignedTransaction};
    use crate::crypto::ed25519::Ed25519Identity;
    use crate::genesis::{GenesisAllocation, GenesisConfig, InitializedGenesis};
    use crate::state::execution::ExecutionError;
    use crate::transaction::signing::sign_transaction;
    use crate::transaction::{ChainId, Transaction};

    const CAPACITY: usize = 16;
    const ASSET: AssetId = AssetId::new([7; 32]);
    const RECIPIENT_BYTES: [u8; 32] = [200; 32];

    struct Fixture {
        state: State,
        identity: Ed25519Identity,
        sender: AccountId,
        recipient: AccountId,
        context: ValidationContext,
    }

    fn fixture(balance: u128) -> Fixture {
        let identity = Ed25519Identity::generate();
        let network = NetworkId::Development;
        let sender = AccountId::try_from_ed25519_public_key(network, &identity.public_key()).unwrap();
        let recipient = AccountId::new(RECIPIENT_BYTES);
        let genesis = InitializedGenesis::new(GenesisConfig::new(
            network,
            vec![
                GenesisAllocation {
                    account_id: sender,
                    asset_id: ASSET,
                    amount: balance,
                },
                GenesisAllocation {
                    account_id: recipient,
                    asset_id: ASSET,
                    amount: 10,
                },
            ],
        ))
        .unwrap();
        let state = State::from_genesis(&genesis).unwrap();
        let context = ValidationContext {
            network,
            chain_id: genesis.chain_id(),
        };
        Fixture {
            state,
            identity,
            sender,
            recipient,
            context,
        }
    }

    fn signed(
        identity: &Ed25519Identity,
        sender: AccountId,
        recipient: AccountId,
        context: ValidationContext,
        amount: u128,
        nonce: u64,
    ) -> SignedTransaction {
        let transaction = Transaction::new(
            1,
            context.network,
            context.chain_id,
            sender,
            recipient,
            ASSET,
            amount,
            nonce,
        )
        .unwrap();
        SignedTransaction {
            signature: sign_transaction(&transaction, identity),
            transaction,
        }
    }

    fn pool(context: ValidationContext, capacity: usize) -> Mempool {
        Mempool::new(capacity, context)
    }

    #[test]
    fn admits_valid_transaction() {
        let f = fixture(1_000);
        let mempool = pool(f.context, CAPACITY);
        let transaction = signed(&f.identity, f.sender, f.recipient, f.context, 100, 0);

        assert!(mempool.submit(transaction.clone(), &f.state).is_ok());
        assert_eq!(mempool.len(), Ok(1));
        assert_eq!(mempool.select_transactions(), Ok(vec![transaction]));
    }

    #[test]
    fn rejects_invalid_signature() {
        let f = fixture(1_000);
        let mempool = pool(f.context, CAPACITY);
        let mut transaction = signed(&f.identity, f.sender, f.recipient, f.context, 100, 0);
        transaction.signature.signature = [0; 64];

        assert_eq!(
            mempool.submit(transaction, &f.state),
            Err(MempoolError::InvalidTransaction(
                ValidationError::InvalidSignature
            ))
        );
        assert_eq!(mempool.len(), Ok(0));
    }

    #[test]
    fn rejects_duplicate_transaction() {
        let f = fixture(1_000);
        let mempool = pool(f.context, CAPACITY);
        let transaction = signed(&f.identity, f.sender, f.recipient, f.context, 100, 0);

        mempool.submit(transaction.clone(), &f.state).unwrap();
        assert_eq!(
            mempool.submit(transaction, &f.state),
            Err(MempoolError::DuplicateTransaction)
        );
    }

    #[test]
    fn rejects_wrong_network_and_chain() {
        let f = fixture(1_000);
        let mempool = pool(f.context, CAPACITY);
        let wrong_network_context = ValidationContext {
            network: NetworkId::Testnet,
            ..f.context
        };
        let wrong_network = signed(
            &f.identity,
            f.sender,
            f.recipient,
            wrong_network_context,
            10,
            0,
        );
        assert_eq!(
            mempool.submit(wrong_network, &f.state),
            Err(MempoolError::InvalidTransaction(
                ValidationError::WrongNetwork
            ))
        );

        let wrong_chain_context = ValidationContext {
            chain_id: ChainId::new([99; 32]),
            ..f.context
        };
        let wrong_chain = signed(
            &f.identity,
            f.sender,
            f.recipient,
            wrong_chain_context,
            10,
            0,
        );
        assert_eq!(
            mempool.submit(wrong_chain, &f.state),
            Err(MempoolError::InvalidTransaction(ValidationError::WrongChain))
        );
    }

    #[test]
    fn enforces_consecutive_sender_nonces_and_rejects_gaps() {
        let f = fixture(1_000);
        let mempool = pool(f.context, CAPACITY);
        let nonce_one = signed(&f.identity, f.sender, f.recipient, f.context, 10, 1);
        assert_eq!(
            mempool.submit(nonce_one, &f.state),
            Err(MempoolError::NonceMismatch)
        );

        let nonce_zero = signed(&f.identity, f.sender, f.recipient, f.context, 10, 0);
        let nonce_one = signed(&f.identity, f.sender, f.recipient, f.context, 10, 1);
        mempool.submit(nonce_zero, &f.state).unwrap();
        assert!(mempool.submit(nonce_one, &f.state).is_ok());
    }

    #[test]
    fn rejects_cumulative_pending_spending_over_balance() {
        let f = fixture(100);
        let mempool = pool(f.context, CAPACITY);
        let first = signed(&f.identity, f.sender, f.recipient, f.context, 60, 0);
        let second = signed(&f.identity, f.sender, f.recipient, f.context, 50, 1);

        mempool.submit(first, &f.state).unwrap();
        assert_eq!(
            mempool.submit(second, &f.state),
            Err(MempoolError::InsufficientBalance)
        );
        assert_eq!(mempool.len(), Ok(1));
    }

    #[test]
    fn does_not_count_incoming_pending_funds_as_spendable() {
        let f = fixture(100);
        let mempool = pool(f.context, CAPACITY);
        let recipient_identity = Ed25519Identity::generate();
        let sender_for_recipient = AccountId::try_from_ed25519_public_key(
            f.context.network,
            &recipient_identity.public_key(),
        )
        .unwrap();
        // The extra sender has no balance; this confirms its pending incoming
        // transaction cannot make a later amount spendable before it is applied.
        let incoming = signed(
            &f.identity,
            f.sender,
            sender_for_recipient,
            f.context,
            50,
            0,
        );
        assert!(mempool.submit(incoming, &f.state).is_ok());
        let outgoing = signed(
            &recipient_identity,
            sender_for_recipient,
            f.recipient,
            f.context,
            1,
            0,
        );
        assert_eq!(
            mempool.submit(outgoing, &f.state),
            Err(MempoolError::AccountNotFound)
        );
    }

    #[test]
    fn rejects_new_transactions_when_at_capacity_without_eviction() {
        let f = fixture(1_000);
        let mempool = pool(f.context, 1);
        let first = signed(&f.identity, f.sender, f.recipient, f.context, 10, 0);
        let second = signed(&f.identity, f.sender, f.recipient, f.context, 10, 1);
        let first_id = mempool.submit(first.clone(), &f.state).unwrap();

        assert_eq!(
            mempool.submit(second, &f.state),
            Err(MempoolError::CapacityReached)
        );
        assert_eq!(mempool.select_transactions(), Ok(vec![first]));
        assert_eq!(mempool.select_transactions().unwrap()[0], *mempool.inner.read().unwrap().transactions.get(&first_id).unwrap());
    }

    #[test]
    fn selects_multiple_senders_deterministically_then_nonce() {
        let f = fixture(1_000);
        let second_identity = Ed25519Identity::generate();
        let second_sender = AccountId::try_from_ed25519_public_key(
            f.context.network,
            &second_identity.public_key(),
        )
        .unwrap();
        let third_identity = Ed25519Identity::generate();
        let third_sender = AccountId::try_from_ed25519_public_key(
            f.context.network,
            &third_identity.public_key(),
        )
        .unwrap();
        let mut allocations = vec![
            GenesisAllocation {
                account_id: f.sender,
                asset_id: ASSET,
                amount: 1_000,
            },
            GenesisAllocation {
                account_id: f.recipient,
                asset_id: ASSET,
                amount: 10,
            },
            GenesisAllocation {
                account_id: second_sender,
                asset_id: ASSET,
                amount: 1_000,
            },
            GenesisAllocation {
                account_id: third_sender,
                asset_id: ASSET,
                amount: 1_000,
            },
        ];
        allocations.sort_by_key(|allocation| *allocation.account_id.as_bytes());
        let genesis = InitializedGenesis::new(GenesisConfig::new(f.context.network, allocations)).unwrap();
        let state = State::from_genesis(&genesis).unwrap();
        let context = ValidationContext {
            network: f.context.network,
            chain_id: genesis.chain_id(),
        };
        let mempool = pool(context, CAPACITY);

        let mut senders = vec![
            (f.identity, f.sender),
            (second_identity, second_sender),
            (third_identity, third_sender),
        ];
        senders.sort_by_key(|(_, sender)| *sender.as_bytes());
        for (identity, sender) in senders.iter().rev() {
            let first = signed(identity, *sender, f.recipient, context, 10, 0);
            let next = signed(identity, *sender, f.recipient, context, 10, 1);
            mempool.submit(first, &state).unwrap();
            mempool.submit(next, &state).unwrap();
        }

        let selection = mempool.select_transactions().unwrap();
        let selected_keys: Vec<([u8; 32], u64)> = selection
            .iter()
            .map(|signed| (*signed.transaction.sender.as_bytes(), signed.transaction.nonce))
            .collect();
        let mut expected = Vec::new();
        for (_, sender) in senders {
            expected.push((*sender.as_bytes(), 0));
            expected.push((*sender.as_bytes(), 1));
        }
        assert_eq!(selected_keys, expected);
        assert_eq!(mempool.select_transactions().unwrap(), selection);
    }

    #[test]
    fn reconciles_included_transaction_and_prunes_stale_nonce_entries() {
        let f = fixture(1_000);
        let mempool = pool(f.context, CAPACITY);
        let included = signed(&f.identity, f.sender, f.recipient, f.context, 100, 0);
        let pending = signed(&f.identity, f.sender, f.recipient, f.context, 100, 1);
        mempool.submit(included.clone(), &f.state).unwrap();
        mempool.submit(pending.clone(), &f.state).unwrap();

        let mut resulting_state = f.state.clone();
        resulting_state
            .execute_transaction(
                &included.transaction,
                &included.signature,
                &f.context,
            )
            .unwrap();
        let block = Block::new(
            f.context.network,
            f.context.chain_id,
            InitializedGenesis::new(GenesisConfig::new(f.context.network, Vec::new()))
                .unwrap()
                .genesis_hash(),
            1,
            1,
            BlockHash::new([0; 32]),
            resulting_state.state_root(),
            vec![included.clone()],
        );

        assert_eq!(mempool.reconcile(&block, &resulting_state), Ok(1));
        assert_eq!(mempool.select_transactions(), Ok(vec![pending]));
    }

    #[test]
    fn reconciliation_drops_transactions_that_no_longer_fit_updated_balance() {
        let f = fixture(100);
        let mempool = pool(f.context, CAPACITY);
        let pending = signed(&f.identity, f.sender, f.recipient, f.context, 80, 0);
        mempool.submit(pending, &f.state).unwrap();
        let block = Block::new(
            f.context.network,
            f.context.chain_id,
            InitializedGenesis::new(GenesisConfig::new(f.context.network, Vec::new()))
                .unwrap()
                .genesis_hash(),
            1,
            1,
            BlockHash::new([0; 32]),
            [0; 32],
            Vec::new(),
        );

        let mut updated_state = f.state.clone();
        // Consume part of the sender balance with a valid on-chain transfer.
        let consumed = signed(&f.identity, f.sender, f.recipient, f.context, 50, 0);
        updated_state
            .execute_transaction(&consumed.transaction, &consumed.signature, &f.context)
            .unwrap();

        assert_eq!(mempool.reconcile(&block, &updated_state), Ok(1));
        assert!(mempool.is_empty().unwrap());
    }

    #[test]
    fn cloned_pool_is_thread_safe_for_concurrent_distinct_submissions() {
        let f = fixture(10_000);
        let mempool = pool(f.context, CAPACITY);
        let mut workers = Vec::new();
        for nonce in 0..8 {
            let pool = mempool.clone();
            let state = f.state.clone();
            let identity = Ed25519Identity::generate();
            let sender = AccountId::try_from_ed25519_public_key(
                f.context.network,
                &identity.public_key(),
            )
            .unwrap();
            // Provide this independently signed sender with a state account.
            let mut state = state;
            state.create_account(sender).unwrap();
            let recipient = f.recipient;
            let context = f.context;
            // Only a sender's nonce zero is valid, so use separate senders and
            // a single transaction per worker.
            assert_eq!(nonce, workers.len() as u64);
            workers.push(thread::spawn(move || {
                let transaction = signed(&identity, sender, recipient, context, 1, 0);
                pool.submit(transaction, &state)
            }));
        }

        for worker in workers {
            assert!(worker.join().unwrap().is_ok());
        }
        assert_eq!(mempool.len(), Ok(8));
    }

    #[test]
    fn transaction_id_includes_signature_and_is_stable() {
        let f = fixture(1_000);
        let transaction = signed(&f.identity, f.sender, f.recipient, f.context, 10, 0);
        assert_eq!(transaction_id(&transaction), transaction_id(&transaction));
        let mut altered = transaction.clone();
        altered.signature.signature[0] ^= 1;
        assert_ne!(transaction_id(&transaction), transaction_id(&altered));
    }

    #[test]
    fn reports_state_account_missing() {
        let f = fixture(1_000);
        let absent = AccountId::new([240; 32]);
        let transaction = signed(&f.identity, absent, f.recipient, f.context, 1, 0);
        assert_eq!(
            pool(f.context, CAPACITY).submit(transaction, &f.state),
            Err(MempoolError::AccountNotFound)
        );
    }

    #[test]
    fn cumulative_spending_handles_self_transfers_like_state_execution() {
        let f = fixture(100);
        let mempool = pool(f.context, CAPACITY);
        let self_transfer = signed(&f.identity, f.sender, f.sender, f.context, 100, 0);
        let next_transfer = signed(&f.identity, f.sender, f.recipient, f.context, 100, 1);
        mempool.submit(self_transfer, &f.state).unwrap();
        assert!(mempool.submit(next_transfer, &f.state).is_ok());
    }

    #[test]
    fn submitted_transaction_is_executable_in_state() {
        let f = fixture(100);
        let mempool = pool(f.context, CAPACITY);
        let transaction = signed(&f.identity, f.sender, f.recipient, f.context, 100, 0);
        mempool.submit(transaction.clone(), &f.state).unwrap();
        let mut state = f.state.clone();
        assert_eq!(
            state.execute_transaction(
                &transaction.transaction,
                &transaction.signature,
                &f.context,
            ),
            Ok(())
        );
        let invalid = state.execute_transaction(
            &transaction.transaction,
            &transaction.signature,
            &f.context,
        );
        assert_eq!(invalid, Err(ExecutionError::NonceMismatch));
    }
}
