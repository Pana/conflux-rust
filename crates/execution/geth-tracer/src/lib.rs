#![allow(unused)]
mod fourbyte;
mod gas;
mod geth_tracer;
mod prestate;
mod tracing_inspector;
mod types;
mod utils;

// Trace recording config, style, arena and trace types now all come from the
// upstream `revm-inspectors` crate; the vendored
// `config`/`geth_builder`/`arena` modules and the duplicated trace types in
// `types` have been removed in favour of the upstream equivalents.
use revm_inspectors::tracing::{TraceStyle, TracingInspectorConfig};

pub use geth_tracer::{GethTraceKey, GethTracer};
pub use types::{GethTraceWithHash, TxExecContext};
pub use utils::{
    from_alloy_address, to_alloy_address, to_alloy_h256, to_alloy_u256,
};
