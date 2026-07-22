def test_trace_simple_cfx_transfer(ew3, evm_accounts):
    account = evm_accounts[0]
    to_address = ew3.eth.account.create().address
    tx_hash = ew3.eth.send_transaction({
        "from": account.address,
        "to": to_address,
        "value": ew3.to_wei(1, "ether"),
    })
    ew3.eth.wait_for_transaction_receipt(tx_hash)
    assert ew3.eth.get_balance(to_address) == ew3.to_wei(1, "ether")

    tx_trace = ew3.manager.request_blocking('debug_traceTransaction', [tx_hash])
    assert tx_trace['failed'] == False
    assert tx_trace['gas'] == 21000
    assert tx_trace['returnValue'] == '0x'
    assert len(tx_trace['structLogs']) == 0

def test_trace_deploy_contract(ew3, erc20_contract):
    tx_hash = erc20_contract["deploy_hash"]
    tx_trace = ew3.manager.request_blocking('debug_traceTransaction', [tx_hash])
    
    oplog_len = len(tx_trace["structLogs"])
    assert tx_trace['failed'] == False
    assert tx_trace['gas'] > 21000
    assert oplog_len > 0
    # key check
    keys = ["pc", "op", "gas", "gasCost", "depth", "stack"]
    for key in keys:
        assert key in tx_trace['structLogs'][0]

    assert tx_trace["structLogs"][oplog_len-1]["op"] == "RETURN"

def test_transfer_trace(ew3, erc20_token_transfer):
    transfer_hash = erc20_token_transfer["tx_hash"]
    transfer_trace = ew3.manager.request_blocking('debug_traceTransaction', [transfer_hash])

    assert transfer_trace["failed"] == False
    oplog_len = len(transfer_trace["structLogs"])
    assert oplog_len > 0
    assert transfer_trace["structLogs"][oplog_len-1]["op"] == "RETURN"

def test_noop_trace(ew3, erc20_token_transfer):
    transfer_hash = erc20_token_transfer["tx_hash"]
    noop_trace = ew3.manager.request_blocking('debug_traceTransaction', [transfer_hash, {"tracer": "noopTracer"}])
    assert noop_trace == {}

def test_four_byte_trace(ew3, erc20_token_transfer):
    transfer_hash = erc20_token_transfer["tx_hash"]
    four_byte_trace = ew3.manager.request_blocking('debug_traceTransaction', [transfer_hash, {"tracer": "4byteTracer"}])
    assert four_byte_trace == {'0xa9059cbb-64': 1}

def test_call_trace(ew3, erc20_token_transfer):
    transfer_hash = erc20_token_transfer["tx_hash"]
    call_trace = ew3.manager.request_blocking('debug_traceTransaction', [transfer_hash, {"tracer": "callTracer"}])
    assert call_trace["from"] == "0x0e768d12395c8abfdedf7b1aeb0dd1d27d5e2a7f"
    # assert call_trace["to"] == "0xe2182fba747b5706a516d6cf6bf62d6117ef86ea"
    assert call_trace["type"] == 'CALL'
    assert call_trace["value"] == "0x0"
    assert call_trace["output"] == "0x0000000000000000000000000000000000000000000000000000000000000001"

def test_prestate_trace_default_mode(ew3, erc20_token_transfer):
    transfer_hash = erc20_token_transfer["tx_hash"]
    receipt = erc20_token_transfer["receipt"]
    sender = receipt["from"].lower()
    contract = receipt["to"].lower()

    trace = ew3.manager.request_blocking('debug_traceTransaction', [transfer_hash, {"tracer": "prestateTracer"}])

    # sender is included with its pre-transaction basic fields
    assert sender in trace
    assert int(trace[sender]["balance"], 16) > 0
    assert trace[sender]["nonce"] > 0

    # the ERC20 contract is included with code and the accessed balance slots
    assert contract in trace
    assert trace[contract]["code"].startswith("0x6080")
    assert len(trace[contract]["storage"]) > 0

def test_prestate_trace_diff_mode(ew3, erc20_token_transfer):
    transfer_hash = erc20_token_transfer["tx_hash"]
    receipt = erc20_token_transfer["receipt"]
    sender = receipt["from"].lower()
    contract = receipt["to"].lower()

    trace = ew3.manager.request_blocking('debug_traceTransaction', [transfer_hash, {
        "tracer": "prestateTracer",
        "tracerConfig": {"diffMode": True},
    }])
    pre, post = trace["pre"], trace["post"]

    # sender pays gas and bumps its nonce
    assert pre[sender]["nonce"] + 1 == post[sender]["nonce"]
    assert int(pre[sender]["balance"], 16) > int(post[sender]["balance"], 16)

    # token balances of the transfer parties changed; unchanged fields
    # (e.g. the contract code) are not repeated in the diff
    assert len(post[contract]["storage"]) > 0
    assert "code" not in post[contract]
    for slot, value in pre[contract]["storage"].items():
        assert post[contract]["storage"].get(slot) != value

def test_prestate_trace_create_diff_mode(ew3, erc20_contract):
    deploy_hash = erc20_contract["deploy_hash"]
    receipt = ew3.eth.get_transaction_receipt(deploy_hash)
    contract = receipt["contractAddress"].lower()

    trace = ew3.manager.request_blocking('debug_traceTransaction', [deploy_hash, {
        "tracer": "prestateTracer",
        "tracerConfig": {"diffMode": True},
    }])

    # created accounts are excluded from pre and carry the code in post
    assert contract not in trace["pre"]
    assert contract in trace["post"]
    assert trace["post"][contract]["code"].startswith("0x6080")

def test_prestate_trace_disable_code_and_storage(ew3, erc20_token_transfer):
    transfer_hash = erc20_token_transfer["tx_hash"]

    trace = ew3.manager.request_blocking('debug_traceTransaction', [transfer_hash, {
        "tracer": "prestateTracer",
        "tracerConfig": {"disableCode": True, "disableStorage": True},
    }])

    for account_state in trace.values():
        assert "code" not in account_state
        assert "storage" not in account_state

def test_prestate_trace_block(ew3, erc20_token_transfer):
    receipt = erc20_token_transfer["receipt"]
    block_number = receipt["blockNumber"]

    traces = ew3.manager.request_blocking('debug_traceBlockByNumber', [hex(block_number), {"tracer": "prestateTracer"}])

    assert len(traces) > 0
    transfer_trace = next(
        t for t in traces
        if t["txHash"] == erc20_token_transfer["tx_hash"].to_0x_hex()
    )
    assert receipt["from"].lower() in transfer_trace["result"]

def test_prestate_trace_keeps_storage_access_from_reverted_execution(ew3, evm_accounts):
    sender = evm_accounts[0].address

    # Runtime: PUSH1 0; SLOAD; PUSH1 0; PUSH1 0; REVERT.
    # The transaction reverts after reading slot zero, but geth's prestate
    # tracer still reports accesses made by reverted execution frames.
    runtime = "60005460006000fd"
    init_code = "6008600c60003960086000f3" + runtime
    deploy_hash = ew3.eth.send_transaction({
        "from": sender,
        "data": "0x" + init_code,
        "gas": 200_000,
    })
    deploy_receipt = ew3.eth.wait_for_transaction_receipt(deploy_hash)
    contract_address = deploy_receipt["contractAddress"]
    contract = contract_address.lower()

    call_hash = ew3.eth.send_transaction({
        "from": sender,
        "to": contract_address,
        "gas": 100_000,
    })
    receipt = ew3.eth.wait_for_transaction_receipt(call_hash)
    assert receipt["status"] == 0

    trace = ew3.manager.request_blocking("debug_traceTransaction", [
        call_hash,
        {"tracer": "prestateTracer"},
    ])

    zero_slot = "0x" + "00" * 32
    assert contract in trace
    assert trace[contract]["storage"][zero_slot] == zero_slot

def test_opcode_trace_with_config(ew3, erc20_token_transfer):
    tx_hash = erc20_token_transfer["tx_hash"]
    trace = ew3.manager.request_blocking('debug_traceTransaction', [tx_hash, {
        "enableMemory": True,
        "disableStack": False,
        "disableStorage": False,
        "enableReturnData": True
    }])

    oplog_len = len(trace["structLogs"])
    assert trace["failed"] == False
    assert oplog_len == 304

    # limit parameter test
    limited_trace = ew3.manager.request_blocking('debug_traceTransaction', [tx_hash, {
        "enableMemory": True,
        "disableStack": False,
        "disableStorage": False,
        "enableReturnData": True,
        "limit": 10
    }])
    assert len(limited_trace["structLogs"]) == 10

    no_stack_storage_trace = ew3.manager.request_blocking('debug_traceTransaction', [tx_hash, {
        "enableMemory": True,
        "disableStack": True,
        "disableStorage": True,
        "enableReturnData": True
    }])

    disable_all_trace = ew3.manager.request_blocking('debug_traceTransaction', [tx_hash, {
        "enableMemory": False,
        "disableStack": True,
        "disableStorage": True,
        "enableReturnData": False
    }])

    for i, oplog in enumerate(trace["structLogs"]):
        oplog = trace["structLogs"][i]
        
        if "memory" in oplog:
            assert "memory" not in disable_all_trace["structLogs"][i]

        if "returnData" in oplog:
            assert "returnData" not in disable_all_trace["structLogs"][i]
        
        if "stack" in oplog:
            assert "stack" not in no_stack_storage_trace["structLogs"][i]
        
        if "storage" in oplog:
            assert "storage" not in no_stack_storage_trace["structLogs"][i]
