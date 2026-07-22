//! Transaction-boundary state view used by tracers.
//!
//! Pre-transaction values come from `committed_cache` — the state left by
//! preceding transactions of the same epoch — with the backing db as fallback.
//! Post-transaction values come from the active transaction cache.

use super::State;
use crate::{
    executive_observer::{AccountSnapshot, TxStateView},
    state::overlay_account::OverlayAccount,
};
use cfx_bytes::Bytes;
use cfx_statedb::{Result as DbResult, StateDbExt};
use cfx_types::{AddressWithSpace, H256, U256};
use keccak_hash::KECCAK_EMPTY;
use primitives::{StorageKey, StorageValue};

impl State {
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

impl TxStateView for State {
    fn pre_account(
        &self, address: &AddressWithSpace,
    ) -> DbResult<Option<AccountSnapshot>> {
        self.pre_tx_snapshot(address)
    }

    fn post_account(
        &self, address: &AddressWithSpace,
    ) -> DbResult<Option<AccountSnapshot>> {
        let entry = self.read_account_lock(address)?;
        match entry.as_ref() {
            Some(account) if !account.removed_without_update() => {
                Ok(Some(self.snapshot_account(address, account)?))
            }
            _ => Ok(None),
        }
    }

    fn pre_storage(
        &self, address: &AddressWithSpace, key: &H256,
    ) -> DbResult<U256> {
        match self.committed_cache.get(address) {
            Some(entry) => match entry.account() {
                Some(account) if !account.removed_without_update() => {
                    account.storage_at(&self.db, key.as_bytes())
                }
                _ => Ok(U256::zero()),
            },
            None => self.storage_from_db(address, key.as_bytes()),
        }
    }

    fn post_storage(
        &self, address: &AddressWithSpace, key: &H256,
    ) -> DbResult<U256> {
        self.storage_at(address, key.as_bytes())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{state::get_state_for_genesis_write, substate::Substate};
    use cfx_rpc_eth_types::{
        AccountOverride, AccountStateOverrideMode, StateOverride,
    };
    use cfx_types::{
        address_util::AddressUtil, Address, AddressSpaceUtil, Space,
    };
    use std::collections::HashMap;

    #[test]
    fn state_override_is_the_pre_transaction_baseline() {
        let base = get_state_for_genesis_write();
        let address = Address::from_low_u64_be(1);
        let key = H256::from_low_u64_be(2);
        let value = H256::from_low_u64_be(9);
        let mut storage = HashMap::new();
        storage.insert(key, value);
        let mut overrides = StateOverride::new();
        overrides.insert(
            address,
            AccountOverride {
                balance: Some(U256::from(7)),
                nonce: Some(3.into()),
                code: None,
                state: AccountStateOverrideMode::State(storage),
                move_precompile_to: None,
            },
        );

        let state =
            State::new_with_override(base.db, &overrides, Space::Ethereum)
                .unwrap();
        let address = address.with_evm_space();

        let account = state.pre_account(&address).unwrap().unwrap();
        assert_eq!(account.balance, U256::from(7));
        assert_eq!(account.nonce, U256::from(3));
        assert_eq!(state.pre_storage(&address, &key).unwrap(), U256::from(9));
    }

    #[test]
    fn pre_storage_survives_contract_removal() {
        let mut state = get_state_for_genesis_write();
        let mut address = Address::zero();
        address.set_contract_type_bits();
        let address = address.with_native_space();
        let key = H256::from_low_u64_be(1);

        state
            .new_contract_with_code(&address, U256::zero())
            .unwrap();
        state
            .set_storage(
                &address,
                key.as_bytes().to_vec(),
                U256::from(7),
                address.address,
                &mut Substate::new(),
            )
            .unwrap();
        state.update_state_post_tx_execution(false);

        state.remove_contract(&address).unwrap();

        assert_eq!(state.pre_storage(&address, &key).unwrap(), U256::from(7));
        assert!(state.post_account(&address).unwrap().is_none());
    }
}
