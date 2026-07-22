//! Builds geth `prestateTracer` frames from tracer-owned account and storage
//! accesses plus the transaction-end state view. The revm-inspectors builder
//! supplies the geth default/diff-mode shaping semantics (`retain_changed`,
//! zero-storage removal, Create/SelfDestruct classification,
//! `disableCode` / `disableStorage`).

use crate::utils::{to_alloy_address, to_alloy_h256, to_alloy_u256};
use alloy_primitives::{Address, Bytes, B256, U256};
use alloy_rpc_types_trace::geth::{PreStateConfig, PreStateFrame};
use cfx_executor::observer::{
    AccountSnapshot, CallTracer, OpcodeTracer, SetAuth, SetAuthOutcome,
    SetAuthTracer, TxEndContext, TxStartContext, TxTracer,
};
use cfx_statedb::{Error as DbError, Result as DbResult};
use cfx_types::{
    u256_to_address_be, Address as CfxAddress, AddressSpaceUtil,
    AddressWithSpace, BigEndianHash, Space, H256 as CfxH256, U256 as CfxU256,
};
use cfx_vm_types::{ActionParams, InterpreterInfo};
use revm::{
    bytecode::Bytecode,
    context_interface::result::{
        ExecutionResult, HaltReason, Output, ResultAndState, ResultGas,
        SuccessReason,
    },
    state::{Account, AccountInfo, AccountStatus, EvmState, EvmStorageSlot},
    DatabaseRef,
};
use revm_bytecode::opcode;
use revm_inspectors::tracing::GethTraceBuilder;
use std::{
    collections::{HashMap, HashSet},
    convert::Infallible,
};

#[derive(Debug, Clone)]
pub(crate) struct TouchedSlot {
    pub key: CfxH256,
    pub original: CfxU256,
    pub present: CfxU256,
}

#[derive(Debug, Clone)]
pub(crate) struct TxTouchedAccount {
    pub pre: Option<AccountSnapshot>,
    pub post: Option<AccountSnapshot>,
    pub storage: Vec<TouchedSlot>,
}

/// Collects the accounts and storage slots needed by geth's prestate tracer
/// and materializes the final frame at the transaction boundary.
pub(crate) struct PrestateCollector {
    config: PreStateConfig,
    tx_space: Space,
    accounts: HashSet<AddressWithSpace>,
    tx_entry_code_account: Option<AddressWithSpace>,
    slots: HashMap<AddressWithSpace, HashSet<CfxH256>>,
    result: Option<PreStateFrame>,
}

impl PrestateCollector {
    pub(crate) fn new(tx_space: Space, config: PreStateConfig) -> Self {
        Self {
            config,
            tx_space,
            accounts: Default::default(),
            tx_entry_code_account: None,
            slots: Default::default(),
            result: None,
        }
    }

    pub(crate) fn into_frame(self) -> DbResult<PreStateFrame> {
        self.result.ok_or_else(|| {
            DbError::Msg(
                "prestate tracer drained before transaction end".into(),
            )
        })
    }
}

impl TxTracer for PrestateCollector {
    fn tx_start(&mut self, context: &TxStartContext<'_>) {
        let tx = context.tx;
        self.accounts.insert(tx.sender());
        match tx.action() {
            primitives::transaction::Action::Call(address) => {
                let address = address.with_space(tx.space());
                self.accounts.insert(address);
                self.tx_entry_code_account = Some(address);
            }
            primitives::transaction::Action::Create => {
                if let Some(address) = tx.cal_created_address() {
                    self.accounts.insert(address);
                }
            }
        }
        self.accounts
            .insert(context.env.author.with_space(tx.space()));
        if let Some(authorizations) = tx.authorization_list() {
            for authorization in authorizations {
                if let Some(authority) = authorization.authority() {
                    self.accounts.insert(authority.with_space(tx.space()));
                }
            }
        }
    }

    fn tx_end(&mut self, context: &TxEndContext<'_>) -> DbResult<()> {
        let diff_mode = self.config.diff_mode.unwrap_or(false);
        let mut touched = HashMap::new();
        let mut accounts = self.accounts.clone();
        if let Some(address) = self.tx_entry_code_account {
            if address.space == self.tx_space {
                if let Some(target) =
                    context.state.pre_account(&address)?.and_then(|snapshot| {
                        snapshot.code.as_deref().and_then(
                            primitives::transaction::extract_7702_payload,
                        )
                    })
                {
                    accounts.insert(target.with_space(address.space));
                }
            }
        }

        for address in &accounts {
            if address.space != self.tx_space {
                continue;
            }
            let pre = context.state.pre_account(address)?;
            let post = if diff_mode {
                context.state.post_account(address)?
            } else {
                pre.clone()
            };
            let mut storage = Vec::new();
            if let Some(keys) = self.slots.get(address) {
                for key in keys {
                    let original = context.state.pre_storage(address, key)?;
                    storage.push(TouchedSlot {
                        key: *key,
                        original,
                        present: if !diff_mode {
                            original
                        } else if post.is_some() {
                            context.state.post_storage(address, key)?
                        } else {
                            CfxU256::zero()
                        },
                    });
                }
            }
            touched.insert(
                address.address,
                TxTouchedAccount { pre, post, storage },
            );
        }
        self.result = Some(build_prestate_frame(touched, &self.config));
        Ok(())
    }
}

impl SetAuthTracer for PrestateCollector {
    fn record_set_auth(&mut self, action: SetAuth) {
        if action.outcome != SetAuthOutcome::Success {
            return;
        }
        if let Some(author) = action.author {
            self.accounts.insert(author.with_space(action.space));
        }
    }
}

impl CallTracer for PrestateCollector {
    fn record_call(&mut self, params: &ActionParams) {
        self.accounts
            .insert(params.address.with_space(params.space));
        self.accounts
            .insert(params.code_address.with_space(params.space));
        self.accounts.insert(params.sender.with_space(params.space));
    }

    fn record_create(&mut self, params: &ActionParams) {
        self.accounts
            .insert(params.address.with_space(params.space));
        self.accounts.insert(params.sender.with_space(params.space));
    }

    fn record_create_attempt(&mut self, space: Space, address: &CfxAddress) {
        self.accounts.insert(address.with_space(space));
    }

    fn do_trace_create_attempt(&self, enabled: &mut bool) { *enabled = true; }
}

impl OpcodeTracer for PrestateCollector {
    fn do_trace_opcode(&self, enabled: &mut bool) { *enabled = true; }

    fn record_account_access(&mut self, space: Space, address: &CfxAddress) {
        self.accounts.insert(address.with_space(space));
    }

    fn step(&mut self, interp: &dyn InterpreterInfo) {
        let stack = interp.stack();
        let top = |index: usize| stack.get(stack.len().checked_sub(index + 1)?);
        match interp.current_opcode() {
            opcode::SLOAD | opcode::SSTORE => {
                let address =
                    interp.contract_address().with_space(self.tx_space);
                self.accounts.insert(address);
                if let Some(key) = top(0) {
                    self.slots
                        .entry(address)
                        .or_default()
                        .insert(BigEndianHash::from_uint(key));
                }
            }
            opcode::BALANCE
            | opcode::EXTCODESIZE
            | opcode::EXTCODECOPY
            | opcode::EXTCODEHASH
            | opcode::SELFDESTRUCT => {
                if let Some(address) = top(0) {
                    self.accounts.insert(
                        u256_to_address_be(*address).with_space(self.tx_space),
                    );
                }
            }
            opcode::CALL
            | opcode::CALLCODE
            | opcode::DELEGATECALL
            | opcode::STATICCALL => {
                // Match geth's prestate tracer: an underflowing call with only
                // the target near the top did not access that account.
                if stack.len() >= 5 {
                    if let Some(address) = top(1) {
                        let address = u256_to_address_be(*address)
                            .with_space(self.tx_space);
                        self.accounts.insert(address);
                    }
                }
            }
            _ => {}
        }
    }

    fn selfdestruct(
        &mut self, space: Space, contract: &CfxAddress, target: &CfxAddress,
        _value: CfxU256,
    ) {
        self.accounts.insert(contract.with_space(space));
        self.accounts.insert(target.with_space(space));
    }
}

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
pub(crate) fn build_prestate_frame(
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
    use cfx_executor::observer::TxStateView;
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

    struct FakeTxStateView;

    impl TxStateView for FakeTxStateView {
        fn pre_account(
            &self, _address: &AddressWithSpace,
        ) -> DbResult<Option<AccountSnapshot>> {
            Ok(Some(eoa(100, 1)))
        }

        fn post_account(
            &self, _address: &AddressWithSpace,
        ) -> DbResult<Option<AccountSnapshot>> {
            Ok(Some(eoa(90, 2)))
        }

        fn pre_storage(
            &self, _address: &AddressWithSpace, _key: &CfxH256,
        ) -> DbResult<CfxU256> {
            Ok(CfxU256::zero())
        }

        fn post_storage(
            &self, _address: &AddressWithSpace, _key: &CfxH256,
        ) -> DbResult<CfxU256> {
            Ok(CfxU256::zero())
        }
    }

    #[test]
    fn collector_materializes_diff_at_transaction_end() {
        let address = addr(1);
        let mut collector =
            PrestateCollector::new(Space::Ethereum, diff_config());
        collector.accounts.insert(address.with_evm_space());

        collector
            .tx_end(&TxEndContext {
                state: &FakeTxStateView,
            })
            .unwrap();

        let PreStateFrame::Diff(diff) = collector.into_frame().unwrap() else {
            panic!("expected diff mode")
        };
        let address = to_alloy_address(address);
        assert_eq!(diff.pre[&address].balance, Some(U256::from(100)));
        assert_eq!(diff.post[&address].balance, Some(U256::from(90)));
    }

    #[test]
    fn collector_records_create_attempt_before_frame_creation() {
        let address = addr(1);
        let mut collector =
            PrestateCollector::new(Space::Ethereum, PreStateConfig::default());

        collector.record_create_attempt(Space::Ethereum, &address);

        assert!(collector.accounts.contains(&address.with_evm_space()));
    }

    struct DelegatedTxStateView {
        authority: AddressWithSpace,
        pre_target: AddressWithSpace,
        post_target: AddressWithSpace,
    }

    impl TxStateView for DelegatedTxStateView {
        fn pre_account(
            &self, address: &AddressWithSpace,
        ) -> DbResult<Option<AccountSnapshot>> {
            if address == &self.authority {
                let mut code =
                    primitives::transaction::CODE_PREFIX_7702.to_vec();
                code.extend_from_slice(self.pre_target.address.as_bytes());
                Ok(Some(contract(0, &code)))
            } else if address == &self.pre_target
                || address == &self.post_target
            {
                Ok(Some(contract(1, b"\x60\x00")))
            } else {
                Ok(None)
            }
        }

        fn post_account(
            &self, address: &AddressWithSpace,
        ) -> DbResult<Option<AccountSnapshot>> {
            if address == &self.authority {
                let mut code =
                    primitives::transaction::CODE_PREFIX_7702.to_vec();
                code.extend_from_slice(self.post_target.address.as_bytes());
                Ok(Some(contract(0, &code)))
            } else {
                self.pre_account(address)
            }
        }

        fn pre_storage(
            &self, _address: &AddressWithSpace, _key: &CfxH256,
        ) -> DbResult<CfxU256> {
            Ok(CfxU256::zero())
        }

        fn post_storage(
            &self, _address: &AddressWithSpace, _key: &CfxH256,
        ) -> DbResult<CfxU256> {
            Ok(CfxU256::zero())
        }
    }

    #[test]
    fn tx_entry_uses_pre_transaction_delegation_target() {
        let authority = addr(1).with_evm_space();
        let pre_target = addr(2).with_evm_space();
        let post_target = addr(3).with_evm_space();
        let state = DelegatedTxStateView {
            authority,
            pre_target,
            post_target,
        };
        let mut collector =
            PrestateCollector::new(Space::Ethereum, PreStateConfig::default());
        collector.accounts.insert(authority);
        collector.tx_entry_code_account = Some(authority);

        collector.tx_end(&TxEndContext { state: &state }).unwrap();

        let PreStateFrame::Default(frame) = collector.into_frame().unwrap()
        else {
            panic!("expected default mode")
        };
        assert!(frame.0.contains_key(&to_alloy_address(pre_target.address)));
        assert!(!frame.0.contains_key(&to_alloy_address(post_target.address)));
    }

    #[test]
    fn collector_records_delegation_target_at_access_time() {
        let target = addr(2);
        let mut collector =
            PrestateCollector::new(Space::Ethereum, PreStateConfig::default());

        collector.record_account_access(Space::Ethereum, &target);

        assert!(collector.accounts.contains(&target.with_evm_space()));
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
