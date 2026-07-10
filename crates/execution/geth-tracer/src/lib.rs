#![allow(unused)]
mod arena;
mod fourbyte;
mod gas;
mod geth_builder;
mod geth_tracer;
mod tracing_inspector;
mod types;
mod utils;

use arena::CallTraceArena;
use geth_builder::GethTraceBuilder;

// Step 2 (revm-inspectors migration): config now comes from the upstream crate.
// The local `config` module (vendored copy) has been removed. `TraceStyle` is
// also taken from upstream now that it is exported publicly there.
use revm_inspectors::tracing::{TraceStyle, TracingInspectorConfig};

// Reachable-but-unused upstream types staged for later steps (types/builder
// swap). Kept behind the crate-wide `#![allow(unused)]`.
#[allow(unused_imports)]
use revm_inspectors::tracing::{
    types::{CallTrace, CallTraceNode, CallTraceStep, RecordedMemory},
    GethTraceBuilder as UpstreamGethTraceBuilder,
};

pub use geth_tracer::{GethTraceKey, GethTracer};
pub use types::{GethTraceWithHash, TxExecContext};
pub use utils::{
    from_alloy_address, to_alloy_address, to_alloy_h256, to_alloy_u256,
};
