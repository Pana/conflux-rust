use crate::state::State;
use cfx_statedb::Result as DbResult;
use typemap::ShareDebugMap;

/// Read-only transaction state available when an observer is drained.
///
/// Draining happens after execution has finalized and before the per-
/// transaction state cache is committed, which makes this the canonical seam
/// for observers that need the final transaction state.
pub struct TraceDrainContext<'a> {
    pub state: &'a State,
}

pub trait DrainTrace {
    fn drain_trace(
        self, context: &TraceDrainContext<'_>, map: &mut ShareDebugMap,
    ) -> DbResult<()>;
}

impl<T: DrainTrace> DrainTrace for Option<T> {
    fn drain_trace(
        self, context: &TraceDrainContext<'_>, map: &mut ShareDebugMap,
    ) -> DbResult<()> {
        if let Some(x) = self {
            x.drain_trace(context, map)?;
        }
        Ok(())
    }
}

impl DrainTrace for () {
    fn drain_trace(
        self, _context: &TraceDrainContext<'_>, _map: &mut ShareDebugMap,
    ) -> DbResult<()> {
        Ok(())
    }
}
