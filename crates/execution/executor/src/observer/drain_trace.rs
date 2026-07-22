use cfx_statedb::Result as DbResult;
use typemap::ShareDebugMap;

/// Consumes a finalized observer and moves its result into the execution
/// output. State-dependent finalization belongs in `TxTracer::tx_end`.
pub trait DrainTrace {
    fn drain_trace(self, map: &mut ShareDebugMap) -> DbResult<()>;
}

impl<T: DrainTrace> DrainTrace for Option<T> {
    fn drain_trace(self, map: &mut ShareDebugMap) -> DbResult<()> {
        if let Some(x) = self {
            x.drain_trace(map)?;
        }
        Ok(())
    }
}

impl DrainTrace for () {
    fn drain_trace(self, _map: &mut ShareDebugMap) -> DbResult<()> { Ok(()) }
}
