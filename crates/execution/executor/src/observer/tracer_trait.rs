use super::{
    CallTracer, CheckpointTracer, InternalTransferTracer, OpcodeTracer,
    SetAuthTracer, StorageTracer, TxTracer,
};

pub trait TracerTrait:
    CheckpointTracer
    + CallTracer
    + InternalTransferTracer
    + StorageTracer
    + OpcodeTracer
    + SetAuthTracer
    + TxTracer
{
}

impl<
        T: CheckpointTracer
            + CallTracer
            + InternalTransferTracer
            + OpcodeTracer
            + StorageTracer
            + SetAuthTracer
            + TxTracer,
    > TracerTrait for T
{
}
