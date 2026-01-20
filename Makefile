submit_mainnet_state:
	@cargo run --manifest-path core/Cargo.toml --release -- submit-state

submit_devnet_state:
	@cargo run --manifest-path core/Cargo.toml --release -- submit-state --devnet

submit_account:
	@cargo run --manifest-path core/Cargo.toml --release -- submit-account $(PUBLIC_KEY) $(TOKEN_ID) $(STATE_HASH)

gen_contract_abis:
	forge build --root contract/
	cp contract/out/MinaStateSettlementExample.sol/MinaStateSettlementExample.json core/abi/MinaStateSettlementExample.json
	cp contract/out/MinaAccountValidationExample.sol/MinaAccountValidationExample.json core/abi/MinaAccountValidationExample.json
	cp contract/out/NoriTokenBridge.sol/NoriTokenBridge.json core/abi/NoriTokenBridge.json

deploy_all_bridge_contracts:
	@cargo run --manifest-path contract_deployer/Cargo.toml --release -- deploy-all-contracts ${NORI_TOKEN_BRIDGE_INITIAL_BALANCE}

deploy_nori_token_bridge_contract:
	@cargo run --manifest-path contract_deployer/Cargo.toml --release -- deploy-nori-bridge ${NORI_TOKEN_BRIDGE_INITIAL_BALANCE}

unlock_nori_token:
	@cargo run --manifest-path core/Cargo.toml --release -- unlock-nori-token --to-unlock-amount $(TO_UNLOCK_AMOUNT) $(PUBLIC_KEY) $(TOKEN_ID)