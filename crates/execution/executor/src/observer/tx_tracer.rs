use cfx_bytes::Bytes;
use cfx_statedb::Result as DbResult;
use cfx_types::{AddressWithSpace, H256, U256};
use cfx_vm_types::Env;
use impl_tools::autoimpl;
use impl_trait_for_tuples::impl_for_tuples;
use primitives::SignedTransaction;

pub struct TxStartContext<'a> {
    pub tx: &'a SignedTransaction,
    pub env: &'a Env,
}

/// Basic fields of an account at a transaction boundary.
#[derive(Debug, Clone)]
pub struct AccountSnapshot {
    pub balance: U256,
    pub nonce: U256,
    pub code_hash: H256,
    pub code: Option<Bytes>,
}

pub trait TxStateView {
    fn pre_account(
        &self, address: &AddressWithSpace,
    ) -> DbResult<Option<AccountSnapshot>>;

    fn post_account(
        &self, address: &AddressWithSpace,
    ) -> DbResult<Option<AccountSnapshot>>;

    fn pre_storage(
        &self, address: &AddressWithSpace, key: &H256,
    ) -> DbResult<U256>;

    fn post_storage(
        &self, address: &AddressWithSpace, key: &H256,
    ) -> DbResult<U256>;
}

/// Final transaction state exposed to observers before the transaction cache
/// is committed into the epoch-level cache.
pub struct TxEndContext<'a> {
    pub state: &'a dyn TxStateView,
}

/// Transaction lifecycle hooks around execution state mutations.
#[autoimpl(for<T: trait + ?Sized> &mut T)]
pub trait TxTracer {
    /// Invoked before the executor mutates transaction state, including the
    /// sender nonce and gas balance.
    fn tx_start(&mut self, _context: &TxStartContext<'_>) {}

    /// Invoked after execution cleanup and refunds but before observer results
    /// are drained.
    fn tx_end(&mut self, _context: &TxEndContext<'_>) -> DbResult<()> { Ok(()) }
}

#[impl_for_tuples(4)]
impl TxTracer for Tuple {
    fn tx_start(&mut self, context: &TxStartContext<'_>) {
        for_tuples!( #( Tuple.tx_start(context); )* );
    }

    fn tx_end(&mut self, context: &TxEndContext<'_>) -> DbResult<()> {
        for_tuples!( #( Tuple.tx_end(context)?; )* );
        Ok(())
    }
}

impl<T: TxTracer> TxTracer for Option<T> {
    fn tx_start(&mut self, context: &TxStartContext<'_>) {
        if let Some(tracer) = self {
            tracer.tx_start(context);
        }
    }

    fn tx_end(&mut self, context: &TxEndContext<'_>) -> DbResult<()> {
        if let Some(tracer) = self {
            tracer.tx_end(context)?;
        }
        Ok(())
    }
}
