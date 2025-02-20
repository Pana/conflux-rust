#!/usr/bin/env python3

from eth_utils import decode_hex
from conflux.rpc import RpcClient
from conflux.transactions import CONTRACT_DEFAULT_GAS, charged_of_huge_gas
from conflux.utils import encode_hex, priv_to_addr, parse_as_int, bytes_to_int
from test_framework.block_gen_thread import BlockGenThread
from test_framework.blocktools import create_transaction, encode_hex_0x
from test_framework.test_framework import ConfluxTestFramework
from test_framework.mininode import *
from test_framework.util import *
from web3 import Web3

class WithdrawDepositTest(ConfluxTestFramework):
    def set_test_params(self):
        self.num_nodes = 1

    def setup_network(self):
        self.setup_nodes()
        sync_blocks(self.nodes)
    
    def get_block_number(self, client, tx_hash):
        receipt = client.get_transaction_receipt(tx_hash)
        epoch_number = int(receipt['epochNumber'], 16)
        assert epoch_number is not None
        block_hash = receipt['blockHash']
        blocks = []
        for epoch in range(epoch_number + 1):
            blocks.extend(client.block_hashes_by_epoch(client.EPOCH_NUM(epoch)))
        for (i, block) in enumerate(blocks):
            if block == block_hash:
                return i + 1
        return None


    def run_test(self):
        file_dir = os.path.dirname(os.path.realpath(__file__))
        file_path = os.path.join(file_dir, "..", "internal_contract", "metadata", "Staking.json")
        staking_contract_dict = json.loads(open(os.path.join(file_path), "r").read())
        staking_contract = get_contract_instance(contract_dict=staking_contract_dict)

        start_p2p_connection(self.nodes)

        self.log.info("Initializing contract")
        genesis_key = default_config["GENESIS_PRI_KEY"]
        genesis_addr = priv_to_addr(genesis_key)
        nonce = 0
        gas_price = 1
        gas = CONTRACT_DEFAULT_GAS
        block_gen_thread = BlockGenThread(self.nodes, self.log)
        block_gen_thread.start()
        self.tx_conf = {"from":Web3.to_checksum_address(encode_hex_0x(genesis_addr)), "nonce":int_to_hex(nonce), "gas":int_to_hex(gas), "gasPrice":int_to_hex(gas_price), "chainId":0}

        total_num_blocks = 2 * 60 * 60 * 24 * 365
        accumulate_interest_rate = [2 ** 80 * total_num_blocks]
        for _ in range(1000):
            accumulate_interest_rate.append(accumulate_interest_rate[-1] * (
                40000 + 1000000 * total_num_blocks) // (total_num_blocks * 1000000))

        # Setup balance for node 0
        node = self.nodes[0]
        client = RpcClient(node)
        (addr, priv_key) = client.rand_account()
        self.log.info("addr=%s priv_key=%s", addr, priv_key)
        tx = client.new_tx(value=5 * 10 ** 18, receiver=addr)
        client.send_tx(tx)
        self.wait_for_tx([tx])
        assert_equal(client.get_balance(addr), 5 * 10 ** 18)
        assert_equal(client.get_staking_balance(addr), 0)

        self.tx_conf["to"] = Web3.to_checksum_address("0888000000000000000000000000000000000002")
        # deposit 10**18
        tx_data = decode_hex(staking_contract.functions.deposit(10 ** 18).build_transaction(self.tx_conf)["data"])
        tx = client.new_tx(value=0, sender=addr, receiver=self.tx_conf["to"], gas=gas, data=tx_data, priv_key=priv_key)
        client.send_tx(tx)
        self.wait_for_tx([tx])
        deposit_time = self.get_block_number(client, tx.hash_hex())
        assert_equal(client.get_staking_balance(addr), 10 ** 18)
        assert_equal(client.get_balance(addr), 4 * 10 ** 18 - charged_of_huge_gas(gas))

        # withdraw 5 * 10**17
        balance = client.get_balance(addr)
        capital = 5 * 10 ** 17
        tx_data = decode_hex(staking_contract.functions.withdraw(capital).build_transaction(self.tx_conf)["data"])
        tx = client.new_tx(value=0, sender=addr, receiver=self.tx_conf["to"], gas=gas, data=tx_data, priv_key=priv_key)
        client.send_tx(tx)
        self.wait_for_tx([tx])
        withdraw_time = self.get_block_number(client, tx.hash_hex())
        # interest = capital * accumulate_interest_rate[withdraw_time] // accumulate_interest_rate[deposit_time] - capital
        interest = 0
        assert_equal(client.get_staking_balance(addr), 10 ** 18 - capital)
        assert_equal(client.get_balance(addr), balance + capital + interest - charged_of_huge_gas(gas))

        # lock 4 * 10 ** 17 until block number 100000
        balance = client.get_balance(addr)
        tx_data = decode_hex(staking_contract.functions.voteLock(4 * 10 ** 17, 100000).build_transaction(self.tx_conf)["data"])
        tx = client.new_tx(value=0, sender=addr, receiver=self.tx_conf["to"], gas=gas, data=tx_data, priv_key=priv_key)
        client.send_tx(tx)
        self.wait_for_tx([tx])
        assert_equal(client.get_balance(addr), balance - charged_of_huge_gas(gas))
        assert_equal(client.get_staking_balance(addr), 5 * 10 ** 17)

        # withdraw 5 * 10**17 and it should fail
        balance = client.get_balance(addr)
        capital = 5 * 10 ** 17
        tx_data = decode_hex(staking_contract.functions.withdraw(capital).build_transaction(self.tx_conf)["data"])
        tx = client.new_tx(value=0, sender=addr, receiver=self.tx_conf["to"], gas=gas, data=tx_data, priv_key=priv_key)
        client.send_tx(tx)
        self.wait_for_tx([tx])
        assert_equal(client.get_balance(addr), balance - gas)
        assert_equal(client.get_staking_balance(addr), 5 * 10 ** 17)

        # withdraw 10**17 + 1 and it should fail
        balance = client.get_balance(addr)
        tx_data = decode_hex(staking_contract.functions.withdraw(10 ** 17 + 1).build_transaction(self.tx_conf)["data"])
        tx = client.new_tx(value=0, sender=addr, receiver=self.tx_conf["to"], gas=gas, data=tx_data, priv_key=priv_key)
        client.send_tx(tx)
        self.wait_for_tx([tx])
        assert_equal(client.get_balance(addr), balance - gas)
        assert_equal(client.get_staking_balance(addr), 5 * 10 ** 17)

        # withdraw 10**17 and it should succeed
        balance = client.get_balance(addr)
        capital = 10 ** 17
        tx_data = decode_hex(staking_contract.functions.withdraw(capital).build_transaction(self.tx_conf)["data"])
        tx = client.new_tx(value=0, sender=addr, receiver=self.tx_conf["to"], gas=gas, data=tx_data, priv_key=priv_key)
        client.send_tx(tx)
        self.wait_for_tx([tx])
        withdraw_time = self.get_block_number(client, tx.hash_hex())
        # interest = capital * accumulate_interest_rate[withdraw_time] // accumulate_interest_rate[deposit_time] - capital
        interest = 0
        assert_equal(client.get_balance(addr), balance + capital + interest - charged_of_huge_gas(gas))
        assert_equal(client.get_staking_balance(addr), 5 * 10 ** 17 - capital)

        vote_list = client.get_vote_list(addr)
        assert_equal(len(vote_list), 1)
        assert_equal(vote_list[0]['unlockBlockNumber'], "0x186a0")
        assert_equal(vote_list[0]['amount'], "0x58d15e176280000")

        deposit_list = client.get_deposit_list(addr)
        assert_equal(len(deposit_list), 1)
        assert_equal(deposit_list[0]['amount'], "0x58d15e176280000")

        block_gen_thread.stop()
        block_gen_thread.join()
        sync_blocks(self.nodes)
        self.log.info("Pass")

if __name__ == "__main__":
    WithdrawDepositTest().main()
