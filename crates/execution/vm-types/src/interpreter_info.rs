use cfx_types::{Address, U256};

pub trait InterpreterInfo {
    fn gas_remainning(&self) -> U256;

    fn program_counter(&self) -> u64;

    fn current_opcode(&self) -> u8;

    fn opcode(&self, pc: u64) -> Option<u8>;

    fn mem(&self) -> &Vec<u8>;

    fn stack(&self) -> &Vec<U256>;

    /// Returns the current EVM return-data buffer.
    ///
    /// This is the buffer exposed to `RETURNDATASIZE` and
    /// `RETURNDATACOPY`, not the final output of the current call frame.
    fn return_data(&self) -> &[u8];

    fn return_stack(&self) -> &Vec<usize>;

    fn contract_address(&self) -> Address;
}
