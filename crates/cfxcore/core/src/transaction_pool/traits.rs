use crate::{block_data_manager::BlockDataManager, consensus::BestInformation};
use cfx_executor::machine::Machine;
use cfx_types::{SpaceMap, H256, U256};
use primitives::SignedTransaction;
use std::sync::Arc;

pub trait PoolProviderTrait {
    fn machine(&self) -> Arc<Machine>;

    fn data_manager(&self) -> Arc<BlockDataManager>;
}

pub trait TransactionPoolTrait: PoolProviderTrait {
    fn get_transactions_can_be_pack<'a>(
        &self, num_txs: usize, block_gas_limit: U256, evm_gas_limit: U256,
        block_size_limit: usize, best_epoch_height: u64,
        best_block_number: u64,
    ) -> Vec<Arc<SignedTransaction>>;

    fn get_1559_transactions_can_be_pack<'a>(
        &self, num_txs: usize, block_gas_limit: U256,
        parent_base_price: SpaceMap<U256>, block_size_limit: usize,
        best_epoch_height: u64, best_block_number: u64,
    ) -> (Vec<Arc<SignedTransaction>>, SpaceMap<U256>);

    fn best_info_with_packed_transactions(
        &self, num_txs: usize, block_size_limit: usize,
        additional_transactions: Vec<Arc<SignedTransaction>>,
    ) -> (
        Arc<BestInformation>,
        U256,
        Vec<Arc<SignedTransaction>>,
        Option<SpaceMap<U256>>,
    );

    fn cal_1559_base_price<'a, I>(
        &self, parent_hash: &H256, block_gas_limit: U256, txs: I,
    ) -> Result<Option<SpaceMap<U256>>, String>
    where I: Iterator<Item = &'a SignedTransaction> + 'a;
}
