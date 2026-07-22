//! Per-transaction touched-state extraction for the geth `prestateTracer`.
//!
//! Between `transact()` and `update_state_post_tx_execution()`, `State.cache`
//! holds exactly the accounts loaded by the executed transaction (the
//! analogue of revm's `ResultAndState.state`). The values before the
//! transaction come from `committed_cache` — the state left by preceding
//! transactions of the same epoch — with the backing db as fallback.

use super::State;
use crate::state::overlay_account::OverlayAccount;
use cfx_bytes::Bytes;
use cfx_statedb::{Result as DbResult, StateDbExt};
use cfx_types::{Address, AddressWithSpace, Space, H256, U256};
use keccak_hash::KECCAK_EMPTY;
use primitives::{StorageKey, StorageValue};
use std::collections::{HashMap, HashSet};

/// Storage slots observed by a tracer during one transaction. This access
/// set is intentionally owned by the tracer so checkpoint reverts cannot
/// remove entries from it.
pub type PreStateStorageAccesses = HashMap<AddressWithSpace, HashSet<H256>>;

/// Basic fields of an account at a fixed point in time.
#[derive(Debug, Clone)]
pub struct AccountSnapshot {
    pub balance: U256,
    pub nonce: U256,
    pub code_hash: H256,
    pub code: Option<Bytes>,
}

/// A storage slot accessed by the transaction.
#[derive(Debug, Clone)]
pub struct TouchedSlot {
    pub key: H256,
    /// Value at transaction start.
    pub original: U256,
    /// Value at transaction end.
    pub present: U256,
}

/// Pre/post images of one account touched by a transaction.
#[derive(Debug, Clone)]
pub struct TxTouchedAccount {
    /// `None` = the account did not exist before the transaction.
    pub pre: Option<AccountSnapshot>,
    /// `None` = the account does not exist after the transaction.
    pub post: Option<AccountSnapshot>,
    pub storage: Vec<TouchedSlot>,
}

impl State {
    /// Collects every account of the given space loaded by the transaction
    /// just executed, along with its accessed storage slots.
    ///
    /// Must be called after `transact()` returns and BEFORE
    /// `update_state_post_tx_execution()` (which drains `cache`).
    pub fn collect_tx_touched_state(
        &self, space: Space, accessed_slots: &PreStateStorageAccesses,
    ) -> DbResult<HashMap<Address, TxTouchedAccount>> {
        let cache = self.cache.read();
        let mut touched = HashMap::with_capacity(cache.len());
        for (addr, entry) in cache.iter() {
            if addr.space != space {
                continue;
            }
            let (post, storage) = match entry.account() {
                Some(acc) => {
                    let post = if acc.removed_without_update() {
                        None
                    } else {
                        Some(self.snapshot_account(addr, acc)?)
                    };
                    (
                        post,
                        self.touched_slots(
                            addr,
                            acc,
                            accessed_slots.get(addr),
                        )?,
                    )
                }
                // Loaded but absent from the db (revm's
                // `LoadedAsNotExisting`).
                None => (
                    None,
                    accessed_slots
                        .get(addr)
                        .map(|slots| {
                            slots
                                .iter()
                                .map(|key| TouchedSlot {
                                    key: *key,
                                    original: U256::zero(),
                                    present: U256::zero(),
                                })
                                .collect()
                        })
                        .unwrap_or_default(),
                ),
            };
            let pre = self.pre_tx_snapshot(addr)?;
            touched
                .insert(addr.address, TxTouchedAccount { pre, post, storage });
        }
        Ok(touched)
    }

    /// Account state before the transaction: `committed_cache` holds the
    /// state left by the preceding transactions of the same epoch; on miss
    /// the account has not been modified in this epoch and is read from the
    /// backing db.
    fn pre_tx_snapshot(
        &self, address: &AddressWithSpace,
    ) -> DbResult<Option<AccountSnapshot>> {
        Ok(match self.committed_cache.get(address) {
            Some(entry) => match entry.account() {
                Some(acc) if !acc.removed_without_update() => {
                    Some(self.snapshot_account(address, acc)?)
                }
                _ => None,
            },
            None => match self.db.get_account(address)? {
                Some(acc) => {
                    let code =
                        self.load_code_from_db(address, &acc.code_hash)?;
                    Some(AccountSnapshot {
                        balance: acc.balance,
                        nonce: acc.nonce,
                        code_hash: acc.code_hash,
                        code,
                    })
                }
                None => None,
            },
        })
    }

    fn snapshot_account(
        &self, address: &AddressWithSpace, account: &OverlayAccount,
    ) -> DbResult<AccountSnapshot> {
        let code_hash = account.code_hash();
        // Code of contracts created by this epoch's transactions is not in
        // the db yet, but their `OverlayAccount`s keep it in memory.
        let code = match account.code_opt() {
            Some(code) => Some((*code).clone()),
            None => self.load_code_from_db(address, &code_hash)?,
        };
        Ok(AccountSnapshot {
            balance: *account.balance(),
            nonce: *account.nonce(),
            code_hash,
            code,
        })
    }

    fn load_code_from_db(
        &self, address: &AddressWithSpace, code_hash: &H256,
    ) -> DbResult<Option<Bytes>> {
        if *code_hash == KECCAK_EMPTY {
            return Ok(None);
        }
        Ok(self
            .db
            .get_code(address, code_hash)?
            .map(|info| (*info.code).clone()))
    }

    fn touched_slots(
        &self, address: &AddressWithSpace, account: &OverlayAccount,
        accessed_slots: Option<&HashSet<H256>>,
    ) -> DbResult<Vec<TouchedSlot>> {
        let Some(accessed_slots) = accessed_slots else {
            return Ok(vec![]);
        };

        let mut slots = Vec::with_capacity(accessed_slots.len());
        for key in accessed_slots {
            let original = match account.origin_storage_at(key.as_bytes()) {
                Some(value) => value,
                None => self.storage_from_db(address, key.as_bytes())?,
            };
            let present = account.storage_at(&self.db, key.as_bytes())?;
            slots.push(TouchedSlot {
                key: *key,
                original,
                present,
            });
        }
        Ok(slots)
    }

    fn storage_from_db(
        &self, address: &AddressWithSpace, key: &[u8],
    ) -> DbResult<U256> {
        let storage_key = StorageKey::new_storage_key(&address.address, key)
            .with_space(address.space);
        Ok(self
            .db
            .get::<StorageValue>(storage_key)?
            .map_or_else(U256::zero, |v| v.value))
    }
}
