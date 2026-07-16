//! Builds geth `prestateTracer` frames from the executor's per-transaction
//! touched-state snapshot, reusing the revm-inspectors builder for the exact
//! geth default/diff-mode semantics (`retain_changed`, zero-storage removal,
//! Create/SelfDestruct classification, `disableCode` / `disableStorage`).

use crate::utils::{to_alloy_address, to_alloy_h256, to_alloy_u256};
use alloy_primitives::{Address, Bytes, B256, U256};
use alloy_rpc_types_trace::geth::{PreStateConfig, PreStateFrame};
use cfx_executor::state::{AccountSnapshot, TxTouchedAccount};
use cfx_types::Address as CfxAddress;
use revm::{
    bytecode::Bytecode,
    context_interface::result::{
        ExecutionResult, HaltReason, Output, ResultAndState, ResultGas,
        SuccessReason,
    },
    state::{Account, AccountInfo, AccountStatus, EvmState, EvmStorageSlot},
    DatabaseRef,
};
use revm_inspectors::tracing::GethTraceBuilder;
use std::{collections::HashMap, convert::Infallible};

/// In-memory `DatabaseRef` answering pre-transaction lookups from the
/// extracted snapshots. Code is embedded in `AccountInfo.code`, so the
/// builder never falls back to `code_by_hash_ref` (Conflux has no hash→code
/// index).
struct PreStateDb {
    accounts: HashMap<Address, AccountInfo>,
}

impl DatabaseRef for PreStateDb {
    type Error = Infallible;

    fn basic_ref(
        &self, address: Address,
    ) -> Result<Option<AccountInfo>, Infallible> {
        Ok(self.accounts.get(&address).cloned())
    }

    fn code_by_hash_ref(
        &self, _code_hash: B256,
    ) -> Result<Bytecode, Infallible> {
        Ok(Bytecode::new())
    }

    fn storage_ref(
        &self, _address: Address, _index: U256,
    ) -> Result<U256, Infallible> {
        Ok(U256::ZERO)
    }

    fn block_hash_ref(&self, _number: u64) -> Result<B256, Infallible> {
        Ok(B256::ZERO)
    }
}

fn to_account_info(snapshot: &AccountSnapshot) -> AccountInfo {
    let mut info = AccountInfo::default();
    info.balance = to_alloy_u256(snapshot.balance);
    info.nonce = snapshot.nonce.low_u64();
    info.code_hash = to_alloy_h256(snapshot.code_hash);
    info.code = snapshot
        .code
        .as_ref()
        .map(|code| Bytecode::new_legacy(Bytes::copy_from_slice(code)));
    info
}

/// Converts the touched-state snapshot into revm's `EvmState` + a
/// pre-transaction `DatabaseRef` and runs revm-inspectors'
/// `geth_prestate_traces` over them.
pub fn build_prestate_frame(
    touched: HashMap<CfxAddress, TxTouchedAccount>, config: &PreStateConfig,
) -> PreStateFrame {
    let mut state = EvmState::default();
    let mut pre_accounts = HashMap::new();

    for (address, account) in &touched {
        let address = to_alloy_address(*address);

        if let Some(pre) = &account.pre {
            pre_accounts.insert(address, to_account_info(pre));
        }

        let mut evm_account = Account::default();
        match (&account.pre, &account.post) {
            (pre, Some(post)) => {
                evm_account.info = to_account_info(post);
                evm_account.mark_touch();
                if pre.is_none() {
                    evm_account.mark_created();
                }
            }
            (Some(_), None) => {
                evm_account.mark_touch();
                evm_account.mark_selfdestruct();
            }
            (None, None) => {
                evm_account.status = AccountStatus::LoadedAsNotExisting;
            }
        }

        for slot in &account.storage {
            let mut evm_slot = EvmStorageSlot::default();
            evm_slot.original_value = to_alloy_u256(slot.original);
            evm_slot.present_value = to_alloy_u256(slot.present);
            evm_account.storage.insert(
                U256::from_be_bytes(slot.key.to_fixed_bytes()),
                evm_slot,
            );
        }

        state.insert(address, evm_account);
    }

    // The builder only consumes the `state` half of `ResultAndState`.
    let dummy_result = ExecutionResult::<HaltReason>::Success {
        reason: SuccessReason::Stop,
        gas: ResultGas::default(),
        logs: vec![],
        output: Output::Call(Bytes::new()),
    };

    let result: Result<_, Infallible> = GethTraceBuilder::new(vec![])
        .geth_prestate_traces(
            &ResultAndState::new(dummy_result, state),
            config,
            PreStateDb {
                accounts: pre_accounts,
            },
        );
    result.unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;
    use cfx_executor::state::TouchedSlot;
    use cfx_types::{H256 as CfxH256, U256 as CfxU256};
    use revm::primitives::KECCAK_EMPTY;

    fn addr(n: u64) -> CfxAddress { CfxAddress::from_low_u64_be(n) }

    fn eoa(balance: u64, nonce: u64) -> AccountSnapshot {
        AccountSnapshot {
            balance: CfxU256::from(balance),
            nonce: CfxU256::from(nonce),
            code_hash: CfxH256(KECCAK_EMPTY.0),
            code: None,
        }
    }

    fn contract(balance: u64, code: &[u8]) -> AccountSnapshot {
        AccountSnapshot {
            balance: CfxU256::from(balance),
            nonce: CfxU256::from(1),
            code_hash: CfxH256::repeat_byte(0xaa),
            code: Some(code.to_vec()),
        }
    }

    fn slot(key: u64, original: u64, present: u64) -> TouchedSlot {
        TouchedSlot {
            key: CfxH256::from_low_u64_be(key),
            original: CfxU256::from(original),
            present: CfxU256::from(present),
        }
    }

    fn diff_config() -> PreStateConfig {
        PreStateConfig {
            diff_mode: Some(true),
            ..Default::default()
        }
    }

    #[test]
    fn default_mode_includes_read_only_accounts() {
        let mut touched = HashMap::new();
        touched.insert(
            addr(1),
            TxTouchedAccount {
                pre: Some(eoa(100, 1)),
                post: Some(eoa(100, 1)),
                storage: vec![slot(1, 7, 7)],
            },
        );
        // Accessed but nonexistent account.
        touched.insert(
            addr(2),
            TxTouchedAccount {
                pre: None,
                post: None,
                storage: vec![],
            },
        );

        let frame = build_prestate_frame(touched, &PreStateConfig::default());
        let PreStateFrame::Default(mode) = frame else {
            panic!("expected default mode")
        };

        let acc = &mode.0[&to_alloy_address(addr(1))];
        assert_eq!(acc.balance, Some(U256::from(100)));
        assert_eq!(acc.nonce, Some(1));
        assert_eq!(
            acc.storage[&B256::with_last_byte(1)],
            B256::with_last_byte(7)
        );

        // Nonexistent accessed account appears with zero balance, like geth.
        let absent = &mode.0[&to_alloy_address(addr(2))];
        assert_eq!(absent.balance, Some(U256::ZERO));
        assert_eq!(absent.nonce, None);
    }

    #[test]
    fn diff_mode_drops_unchanged_and_keeps_changed() {
        let mut touched = HashMap::new();
        // Read-only account: identical pre/post.
        touched.insert(
            addr(1),
            TxTouchedAccount {
                pre: Some(eoa(100, 1)),
                post: Some(eoa(100, 1)),
                storage: vec![slot(1, 7, 7)],
            },
        );
        // Modified account.
        touched.insert(
            addr(2),
            TxTouchedAccount {
                pre: Some(eoa(100, 1)),
                post: Some(eoa(90, 2)),
                storage: vec![],
            },
        );

        let frame = build_prestate_frame(touched, &diff_config());
        let PreStateFrame::Diff(diff) = frame else {
            panic!("expected diff mode")
        };

        let unchanged = to_alloy_address(addr(1));
        assert!(!diff.pre.contains_key(&unchanged));
        assert!(!diff.post.contains_key(&unchanged));

        let changed = to_alloy_address(addr(2));
        assert_eq!(diff.pre[&changed].balance, Some(U256::from(100)));
        assert_eq!(diff.pre[&changed].nonce, Some(1));
        assert_eq!(diff.post[&changed].balance, Some(U256::from(90)));
        assert_eq!(diff.post[&changed].nonce, Some(2));
    }

    #[test]
    fn diff_mode_create_and_selfdestruct() {
        let mut touched = HashMap::new();
        // Created by this transaction.
        touched.insert(
            addr(1),
            TxTouchedAccount {
                pre: None,
                post: Some(contract(0, b"\x60\x00")),
                storage: vec![],
            },
        );
        // Destroyed by this transaction.
        touched.insert(
            addr(2),
            TxTouchedAccount {
                pre: Some(contract(50, b"\x60\x01")),
                post: None,
                storage: vec![],
            },
        );

        let frame = build_prestate_frame(touched, &diff_config());
        let PreStateFrame::Diff(diff) = frame else {
            panic!("expected diff mode")
        };

        let created = to_alloy_address(addr(1));
        assert!(!diff.pre.contains_key(&created));
        assert_eq!(
            diff.post[&created].code,
            Some(Bytes::from_static(b"\x60\x00"))
        );

        let destroyed = to_alloy_address(addr(2));
        assert!(!diff.post.contains_key(&destroyed));
        assert_eq!(diff.pre[&destroyed].balance, Some(U256::from(50)));
        assert_eq!(
            diff.pre[&destroyed].code,
            Some(Bytes::from_static(b"\x60\x01"))
        );
    }

    #[test]
    fn diff_mode_storage_semantics() {
        let mut touched = HashMap::new();
        touched.insert(
            addr(1),
            TxTouchedAccount {
                pre: Some(contract(10, b"\x60\x00")),
                post: Some(contract(10, b"\x60\x00")),
                storage: vec![
                    // Unchanged slot: dropped entirely.
                    slot(1, 7, 7),
                    // Changed slot: kept on both sides.
                    slot(2, 3, 4),
                    // Zeroed slot: kept in pre, removed from post.
                    slot(3, 5, 0),
                ],
            },
        );

        let frame = build_prestate_frame(touched, &diff_config());
        let PreStateFrame::Diff(diff) = frame else {
            panic!("expected diff mode")
        };

        let address = to_alloy_address(addr(1));
        let pre = &diff.pre[&address];
        let post = &diff.post[&address];

        assert!(!pre.storage.contains_key(&B256::with_last_byte(1)));
        assert!(!post.storage.contains_key(&B256::with_last_byte(1)));

        assert_eq!(
            pre.storage[&B256::with_last_byte(2)],
            B256::with_last_byte(3)
        );
        assert_eq!(
            post.storage[&B256::with_last_byte(2)],
            B256::with_last_byte(4)
        );

        assert_eq!(
            pre.storage[&B256::with_last_byte(3)],
            B256::with_last_byte(5)
        );
        assert!(!post.storage.contains_key(&B256::with_last_byte(3)));
    }
}
