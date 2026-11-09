//! NOVARA in-memory transaction pool (component version 0.3.0).
//!
//! The pool is separate from [`crate::chain::Chain`]. Callers submit signed
//! transactions against current state and reconcile after successful block
//! application.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, RwLock};

use sha2::{Digest, Sha256};

use crate::account::address::AccountId;
use crate::block::{Block, SignedTransaction};
use crate::state::{State, StateError};
use crate::transaction::AssetId;
use crate::transaction::validation::{ValidationContext, ValidationError, validate_transaction};

/// NOVARA mempool component version, independent of the crate version.
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
    /// Nonces must be consecutive from the state's current nonce. Pending
    /// outgoing transfers reserve the sender's balance; pending incoming funds
    /// are not available for spending. Recipient existence and balance overflow
    /// are checked to match [`State::execute_transaction`].
    pub fn submit(
        &self,
        signed_transaction: SignedTransaction,
        state: &State,
    ) -> Result<[u8; 32], MempoolError> {
        let mut inner = self.inner.write().map_err(|_| MempoolError::LockPoisoned)?;
        insert_transaction(
            &mut inner,
            self.capacity,
            &self.context,
            state,
            signed_transaction,
        )
    }

    /// Returns transactions in deterministic sender-ID, then nonce order.
    pub fn select_transactions(&self) -> Result<Vec<SignedTransaction>, MempoolError> {
        let inner = self.inner.read().map_err(|_| MempoolError::LockPoisoned)?;
        let mut selected = Vec::with_capacity(inner.transactions.len());
        for nonce_queue in inner.senders.values() {
            for id in nonce_queue.values() {
                if let Some(transaction) = inner.transactions.get(id) {
                    selected.push(transaction.clone());
                }
            }
        }
        Ok(selected)
    }

    /// Removes included transactions and revalidates the rest against the
    /// resulting state. Call only after the block has been successfully applied.
    /// Returns the number of removed included or invalidated transactions.
    pub fn reconcile(&self, block: &Block, resulting_state: &State) -> Result<usize, MempoolError> {
        let mut inner = self.inner.write().map_err(|_| MempoolError::LockPoisoned)?;
        let included: BTreeSet<[u8; 32]> = block.transactions.iter().map(transaction_id).collect();
        let mut remaining: Vec<([u8; 32], SignedTransaction)> = inner
            .transactions
            .iter()
            .filter(|(id, _)| !included.contains(*id))
            .map(|(id, transaction)| (*id, transaction.clone()))
            .collect();

        // Rebuild from state nonce forward; hash-map order is unrelated to nonce
        // order and could otherwise cause valid later transactions to be dropped.
        remaining.sort_by_key(|(id, signed)| {
            (
                *signed.transaction.sender.as_bytes(),
                signed.transaction.nonce,
                *id,
            )
        });

        let old_len = inner.transactions.len();
        let mut rebuilt = MempoolInner::default();
        for (_, transaction) in remaining {
            // Reconciliation is pruning: transactions that do not remain valid
            // against the committed state are discarded, not surfaced as errors.
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
    let pending_spent =
        pending_amount_for_sender_asset(inner, &transaction.sender, &transaction.asset_id)?;
    // State execution advances nonce but leaves balance unchanged on self-send.
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
        let recipient_balance = state
            .balance_of(&transaction.recipient, &transaction.asset_id)
            .map_err(map_state_error)?;
        let pending_received = pending_amount_for_recipient_asset(
            inner,
            &transaction.recipient,
            &transaction.asset_id,
        )?;
        recipient_balance
            .checked_add(pending_received)
            .and_then(|balance| balance.checked_add(transaction.amount))
            .ok_or(MempoolError::BalanceOverflow)?;
    }

    inner
        .senders
        .entry(sender_key)
        .or_default()
        .insert(transaction.nonce, id);
    inner.transactions.insert(id, signed_transaction);
    Ok(id)
}

fn pending_amount_for_sender_asset(
    inner: &MempoolInner,
    sender: &AccountId,
    asset_id: &AssetId,
) -> Result<u128, MempoolError> {
    pending_amount(inner, sender, asset_id, true)
}

fn pending_amount_for_recipient_asset(
    inner: &MempoolInner,
    recipient: &AccountId,
    asset_id: &AssetId,
) -> Result<u128, MempoolError> {
    pending_amount(inner, recipient, asset_id, false)
}

fn pending_amount(
    inner: &MempoolInner,
    account: &AccountId,
    asset_id: &AssetId,
    outgoing: bool,
) -> Result<u128, MempoolError> {
    let Some(queue) = inner.senders.get(account.as_bytes()) else {
        return Ok(0);
    };
    queue.values().try_fold(0u128, |total, id| {
        let Some(signed) = inner.transactions.get(id) else {
            return Ok(total);
        };
        let transaction = &signed.transaction;
        let applies = if outgoing {
            transaction.sender == *account && transaction.recipient != *account
        } else {
            transaction.recipient == *account && transaction.sender != *account
        };
        if transaction.asset_id != *asset_id || !applies {
            return Ok(total);
        }
        total
            .checked_add(transaction.amount)
            .ok_or(MempoolError::BalanceOverflow)
    })
}

fn map_state_error(error: StateError) -> MempoolError {
    match error {
        StateError::AccountNotFound | StateError::AccountAlreadyExists => {
            MempoolError::AccountNotFound
        }
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

    const CAPACITY: usize = 32;
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
        let sender =
            AccountId::try_from_ed25519_public_key(network, &identity.public_key()).unwrap();
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
        Fixture {
            state: State::from_genesis(&genesis).unwrap(),
            identity,
            sender,
            recipient,
            context: ValidationContext {
                network,
                chain_id: genesis.chain_id(),
            },
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
    fn admits_valid_transaction_and_rejects_bad_signature() {
        let f = fixture(1_000);
        let mempool = pool(f.context, CAPACITY);
        let transaction = signed(&f.identity, f.sender, f.recipient, f.context, 100, 0);
        assert!(mempool.submit(transaction.clone(), &f.state).is_ok());
        assert_eq!(mempool.select_transactions(), Ok(vec![transaction]));

        let mut invalid = signed(&f.identity, f.sender, f.recipient, f.context, 100, 0);
        invalid.signature.signature = [0; 64];
        assert_eq!(
            pool(f.context, CAPACITY).submit(invalid, &f.state),
            Err(MempoolError::InvalidTransaction(
                ValidationError::InvalidSignature
            ))
        );
    }

    #[test]
    fn rejects_duplicates_wrong_network_and_wrong_chain() {
        let f = fixture(1_000);
        let mempool = pool(f.context, CAPACITY);
        let transaction = signed(&f.identity, f.sender, f.recipient, f.context, 100, 0);
        mempool.submit(transaction.clone(), &f.state).unwrap();
        assert_eq!(
            mempool.submit(transaction, &f.state),
            Err(MempoolError::DuplicateTransaction)
        );

        let wrong_network = ValidationContext {
            network: NetworkId::Testnet,
            ..f.context
        };
        let transaction = signed(&f.identity, f.sender, f.recipient, wrong_network, 10, 0);
        assert_eq!(
            mempool.submit(transaction, &f.state),
            Err(MempoolError::InvalidTransaction(
                ValidationError::WrongNetwork
            ))
        );
        let wrong_chain = ValidationContext {
            chain_id: ChainId::new([99; 32]),
            ..f.context
        };
        let transaction = signed(&f.identity, f.sender, f.recipient, wrong_chain, 10, 0);
        assert_eq!(
            mempool.submit(transaction, &f.state),
            Err(MempoolError::InvalidTransaction(
                ValidationError::WrongChain
            ))
        );
    }

    #[test]
    fn requires_consecutive_nonces_and_rejects_gaps() {
        let f = fixture(1_000);
        let mempool = pool(f.context, CAPACITY);
        let nonce_one = signed(&f.identity, f.sender, f.recipient, f.context, 10, 1);
        assert_eq!(
            mempool.submit(nonce_one, &f.state),
            Err(MempoolError::NonceMismatch)
        );
        mempool
            .submit(
                signed(&f.identity, f.sender, f.recipient, f.context, 10, 0),
                &f.state,
            )
            .unwrap();
        assert!(
            mempool
                .submit(
                    signed(&f.identity, f.sender, f.recipient, f.context, 10, 1),
                    &f.state
                )
                .is_ok()
        );
    }

    #[test]
    fn reserves_cumulative_pending_spending() {
        let f = fixture(100);
        let mempool = pool(f.context, CAPACITY);
        mempool
            .submit(
                signed(&f.identity, f.sender, f.recipient, f.context, 60, 0),
                &f.state,
            )
            .unwrap();
        assert_eq!(
            mempool.submit(
                signed(&f.identity, f.sender, f.recipient, f.context, 50, 1),
                &f.state
            ),
            Err(MempoolError::InsufficientBalance)
        );
    }

    #[test]
    fn requires_existing_recipient_and_pending_incoming_does_not_make_funds_spendable() {
        let f = fixture(100);
        let mempool = pool(f.context, CAPACITY);
        let recipient_identity = Ed25519Identity::generate();
        let recipient = AccountId::try_from_ed25519_public_key(
            f.context.network,
            &recipient_identity.public_key(),
        )
        .unwrap();
        let incoming = signed(&f.identity, f.sender, recipient, f.context, 50, 0);

        // Match State::execute_transaction: transfers to absent accounts fail.
        assert_eq!(
            mempool.submit(incoming.clone(), &f.state),
            Err(MempoolError::AccountNotFound)
        );
        let mut state = f.state.clone();
        state.create_account(recipient).unwrap();
        mempool.submit(incoming, &state).unwrap();

        // Existing account with zero balance is valid as a recipient, but its
        // unconfirmed incoming transfer cannot be spent by its own transaction.
        let outgoing = signed(&recipient_identity, recipient, f.recipient, f.context, 1, 0);
        assert_eq!(
            mempool.submit(outgoing, &state),
            Err(MempoolError::InsufficientBalance)
        );
    }

    #[test]
    fn rejects_capacity_without_evicting_existing_transactions() {
        let f = fixture(1_000);
        let mempool = pool(f.context, 1);
        let first = signed(&f.identity, f.sender, f.recipient, f.context, 10, 0);
        mempool.submit(first.clone(), &f.state).unwrap();
        assert_eq!(
            mempool.submit(
                signed(&f.identity, f.sender, f.recipient, f.context, 10, 1),
                &f.state
            ),
            Err(MempoolError::CapacityReached)
        );
        assert_eq!(mempool.select_transactions(), Ok(vec![first]));
    }

    #[test]
    fn selects_multiple_senders_deterministically_then_nonce() {
        let f = fixture(1_000);
        let identities: Vec<Ed25519Identity> =
            (0..2).map(|_| Ed25519Identity::generate()).collect();
        let senders: Vec<AccountId> = identities
            .iter()
            .map(|identity| {
                AccountId::try_from_ed25519_public_key(f.context.network, &identity.public_key())
                    .unwrap()
            })
            .collect();
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
        ];
        allocations.extend(senders.iter().map(|sender| GenesisAllocation {
            account_id: *sender,
            asset_id: ASSET,
            amount: 100,
        }));
        let genesis =
            InitializedGenesis::new(GenesisConfig::new(f.context.network, allocations)).unwrap();
        let state = State::from_genesis(&genesis).unwrap();
        let context = ValidationContext {
            network: f.context.network,
            chain_id: genesis.chain_id(),
        };
        let mempool = pool(context, CAPACITY);
        for (identity, sender) in identities.iter().zip(&senders).rev() {
            mempool
                .submit(
                    signed(identity, *sender, f.recipient, context, 1, 0),
                    &state,
                )
                .unwrap();
            mempool
                .submit(
                    signed(identity, *sender, f.recipient, context, 1, 1),
                    &state,
                )
                .unwrap();
        }
        let selected = mempool.select_transactions().unwrap();
        let keys: Vec<([u8; 32], u64)> = selected
            .iter()
            .map(|tx| (*tx.transaction.sender.as_bytes(), tx.transaction.nonce))
            .collect();
        let mut expected = Vec::new();
        let mut sorted_senders: Vec<[u8; 32]> =
            senders.iter().map(|sender| *sender.as_bytes()).collect();
        sorted_senders.sort();
        for sender in sorted_senders {
            expected.push((sender, 0));
            expected.push((sender, 1));
        }
        assert_eq!(keys, expected);
        assert_eq!(mempool.select_transactions().unwrap(), selected);
    }

    #[test]
    fn reconciliation_rebuilds_consecutive_transactions_in_sender_nonce_order() {
        let f = fixture(1_000);
        let mempool = pool(f.context, CAPACITY);
        let included = signed(&f.identity, f.sender, f.recipient, f.context, 1, 0);
        mempool.submit(included.clone(), &f.state).unwrap();

        let nonce_one = signed(&f.identity, f.sender, f.recipient, f.context, 1, 1);
        // Find an amount for nonce two whose ID sorts before nonce one's ID,
        // making the regression independent of accidental hash ordering.
        let nonce_two = (1..1_000)
            .map(|amount| signed(&f.identity, f.sender, f.recipient, f.context, amount, 2))
            .find(|candidate| transaction_id(candidate) < transaction_id(&nonce_one))
            .expect("a candidate transaction ID should sort before nonce one's ID");
        mempool.submit(nonce_one.clone(), &f.state).unwrap();
        mempool.submit(nonce_two.clone(), &f.state).unwrap();
        assert!(transaction_id(&nonce_two) < transaction_id(&nonce_one));

        let mut resulting_state = f.state.clone();
        resulting_state
            .execute_transaction(&included.transaction, &included.signature, &f.context)
            .unwrap();
        let genesis =
            InitializedGenesis::new(GenesisConfig::new(f.context.network, Vec::new())).unwrap();
        let block = Block::new(
            f.context.network,
            f.context.chain_id,
            genesis.genesis_hash(),
            1,
            1,
            BlockHash::new([0; 32]),
            resulting_state.state_root(),
            vec![included],
        );
        assert_eq!(mempool.reconcile(&block, &resulting_state), Ok(1));
        assert_eq!(
            mempool.select_transactions(),
            Ok(vec![nonce_one, nonce_two])
        );
    }

    #[test]
    fn reconciliation_prunes_entries_that_no_longer_fit_updated_balance() {
        let f = fixture(100);
        let mempool = pool(f.context, CAPACITY);
        mempool
            .submit(
                signed(&f.identity, f.sender, f.recipient, f.context, 80, 0),
                &f.state,
            )
            .unwrap();
        let mut updated_state = f.state.clone();
        let consumed = signed(&f.identity, f.sender, f.recipient, f.context, 50, 0);
        updated_state
            .execute_transaction(&consumed.transaction, &consumed.signature, &f.context)
            .unwrap();
        let genesis =
            InitializedGenesis::new(GenesisConfig::new(f.context.network, Vec::new())).unwrap();
        let block = Block::new(
            f.context.network,
            f.context.chain_id,
            genesis.genesis_hash(),
            1,
            1,
            BlockHash::new([0; 32]),
            [0; 32],
            Vec::new(),
        );
        assert_eq!(mempool.reconcile(&block, &updated_state), Ok(1));
        assert!(mempool.is_empty().unwrap());
    }

    #[test]
    fn concurrent_submissions_from_funded_senders_are_thread_safe() {
        let network = NetworkId::Development;
        let recipient = AccountId::new(RECIPIENT_BYTES);
        let identities: Vec<Ed25519Identity> =
            (0..8).map(|_| Ed25519Identity::generate()).collect();
        let senders: Vec<AccountId> = identities
            .iter()
            .map(|identity| {
                AccountId::try_from_ed25519_public_key(network, &identity.public_key()).unwrap()
            })
            .collect();
        let mut allocations = vec![GenesisAllocation {
            account_id: recipient,
            asset_id: ASSET,
            amount: 1,
        }];
        allocations.extend(senders.iter().map(|sender| GenesisAllocation {
            account_id: *sender,
            asset_id: ASSET,
            amount: 10,
        }));
        let genesis = InitializedGenesis::new(GenesisConfig::new(network, allocations)).unwrap();
        let state = State::from_genesis(&genesis).unwrap();
        let context = ValidationContext {
            network,
            chain_id: genesis.chain_id(),
        };
        let mempool = pool(context, 8);
        let workers: Vec<_> = identities
            .into_iter()
            .zip(senders)
            .map(|(identity, sender)| {
                let mempool = mempool.clone();
                let state = state.clone();
                thread::spawn(move || {
                    mempool.submit(signed(&identity, sender, recipient, context, 1, 0), &state)
                })
            })
            .collect();
        for worker in workers {
            assert!(worker.join().unwrap().is_ok());
        }
        assert_eq!(mempool.len(), Ok(8));
    }

    #[test]
    fn pool_submission_matches_state_execution_nonce_and_self_transfer_balance() {
        let f = fixture(100);
        let mempool = pool(f.context, CAPACITY);
        let self_transfer = signed(&f.identity, f.sender, f.sender, f.context, 100, 0);
        mempool.submit(self_transfer.clone(), &f.state).unwrap();
        let next = signed(&f.identity, f.sender, f.recipient, f.context, 100, 1);
        assert!(mempool.submit(next, &f.state).is_ok());
        let mut state = f.state.clone();
        state
            .execute_transaction(
                &self_transfer.transaction,
                &self_transfer.signature,
                &f.context,
            )
            .unwrap();
        assert_eq!(
            state.execute_transaction(
                &self_transfer.transaction,
                &self_transfer.signature,
                &f.context
            ),
            Err(ExecutionError::NonceMismatch)
        );
    }
}
