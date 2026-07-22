use crate::stack::FrameResult;
use cfx_types::{Address, Space};
use cfx_vm_types::ActionParams;

use impl_tools::autoimpl;
use impl_trait_for_tuples::impl_for_tuples;

#[impl_for_tuples(4)]
#[autoimpl(for<T: trait + ?Sized> &mut T)]
#[allow(unused_variables)]
pub trait CallTracer {
    /// Prepares call trace for given params.
    fn record_call(&mut self, params: &ActionParams) {}

    /// Prepares call result trace
    fn record_call_result(&mut self, result: &FrameResult) {}

    /// Prepares create trace for given params.
    fn record_create(&mut self, params: &ActionParams) {}

    /// Records a create destination before conflict checks can abort frame
    /// creation.
    fn record_create_attempt(&mut self, space: Space, address: &Address) {}

    /// Enables create-attempt address derivation for interested tracers.
    fn do_trace_create_attempt(&self, enabled: &mut bool) {}

    /// Prepares create result trace
    fn record_create_result(&mut self, result: &FrameResult) {}
}
