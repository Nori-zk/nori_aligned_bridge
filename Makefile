.PHONY: submit_mainnet_state submit_devnet_state submit_account gen_contract_abi deploy_example_bridge_contracts

submit_mainnet_state:
	@cargo run --manifest-path core/Cargo.toml --release -- submit-state

submit_devnet_state:
	@cargo run --manifest-path core/Cargo.toml --release -- submit-state --devnet

submit_account:
	@cargo run --manifest-path core/Cargo.toml --release -- submit-account $(PUBLIC_KEY) $(TOKEN_ID) $(STATE_HASH)

gen_contract_abis:
	forge build --root contract/
	forge build --root example/eth_contract
	cp contract/out/MinaStateSettlementExample.sol/MinaStateSettlementExample.json core/abi/MinaStateSettlementExample.json
	cp contract/out/MinaAccountValidationExample.sol/MinaAccountValidationExample.json core/abi/MinaAccountValidationExample.json
	cp contract/out/NoriTokenBridge.sol/NoriTokenBridge.json core/abi/NoriTokenBridge.json
	cp contract/out/NoriTokenBridge.sol/NoriTokenBridge.json example/app/abi/NoriTokenBridge.json
	cp example/eth_contract/out/SudokuValidity.sol/SudokuValidity.json example/app/abi/SudokuValidity.json

deploy_example_bridge_contracts:
	@cargo run --manifest-path contract_deployer/Cargo.toml --release

deploy_example_app_contracts:
	@cargo run --manifest-path example/app/Cargo.toml --release -- deploy-contract

execute_example:
	cd example/mina_zkapp; \
	npm run build; \
	node build/src/run.js
	cargo run --manifest-path example/app/Cargo.toml --release -- validate-solution

execute_example_unlock_nori_token:
	@cargo run --manifest-path example/app/Cargo.toml --release -- unlock-nori-token

execute_example_transfer:
	@cargo run --manifest-path example/app/Cargo.toml --release -- transfer --private-key ${PRIVATE_KEY} --to ${TO} --amount ${AMOUNT}