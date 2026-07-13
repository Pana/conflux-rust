//! Conflux-specific trace types retained after the revm-inspectors migration.
//!
//! The vendored call-trace types (`CallTrace`, `CallTraceNode`,
//! `CallTraceStep`, `CallKind`, `StorageChange`, `RecordedMemory`, ...) now
//! come from the upstream `revm-inspectors` crate. Only the Conflux-specific
//! glue types remain here.

use alloy_rpc_types_trace::geth::GethTrace;
use cfx_types::{Space, H256};
use primitives::{block::BlockHeight, BlockNumber};

pub struct GethTraceWithHash {
    pub trace: GethTrace,
    pub tx_hash: H256,
    pub space: Space,
}

#[derive(Clone)]
pub struct TxExecContext {
    pub tx_gas_limit: u64,
    pub block_number: BlockNumber,
    pub block_height: BlockHeight,
}
