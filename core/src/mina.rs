use std::str::FromStr;

use alloy_sol_types::SolValue;
use hex;
use base64::prelude::*;
use futures::future::join_all;
use graphql_client::{reqwest::post_graphql, GraphQLQuery};
use kimchi::mina_curves::pasta::Fp;
use log::info;
use mina_state_verifier::verify_mina_state;
use mina_p2p_messages::{
    binprot::BinProtRead, hash::MinaHash, v2::{
        LedgerHash, MinaBaseAccountBinableArgStableV2 as MinaAccount, MinaBaseProofStableV2,
        MinaBaseZkappAccountStableV2, MinaStateProtocolStateValueStableV2, StateHash,
    }
};

use crate::{
    eth::get_bridge_tip_hash,
    proof::{
        account_proof::{MerkleNode, MinaAccountProof, MinaAccountPubInputs},
        state_proof::{MinaStateProof, MinaStatePubInputs},
    },
    sol::account::MinaAccountValidationExample,
    utils::constants::BRIDGE_TRANSITION_FRONTIER_LEN,
};

type StateHashAsDecimal = String;
type PrecomputedBlockProof = String;
type FieldElem = String;

#[derive(GraphQLQuery)]
#[graphql(
    schema_path = "graphql/mina_schema.json",
    query_path = "graphql/state_query.graphql"
)]
/// A query for a protocol state given some state hash (non-field).
struct StateQuery;

#[derive(GraphQLQuery)]
#[graphql(
    schema_path = "graphql/mina_schema.json",
    query_path = "graphql/best_chain_query.graphql"
)]
/// A query for the state hashes and proofs of the transition frontier.
struct BestChainQuery;

#[derive(GraphQLQuery)]
#[graphql(
    schema_path = "graphql/mina_schema.json",
    query_path = "graphql/account_query.graphql"
)]
/// A query for retrieving an a Mina account state at some block, along with its ledger hash and
/// merkle path.
struct AccountQuery;

type TokenId = String;
type PublicKey = String;

/// 将 zkApp 账户格式化为人类可读的字符串
fn format_zkapp_readable(zkapp: &MinaBaseZkappAccountStableV2) -> String {
    let mut output = String::new();
    
    // app_state - 8 个 field elements
    output.push_str("  app_state:\n");
    for (i, state) in zkapp.app_state.0.0.iter().enumerate() {
        output.push_str(&format!("    [{}]: 0x{}\n", i, hex::encode(state.as_ref())));
    }
    
    // verification_key
    output.push_str(&format!("  verification_key: {}\n", 
        if zkapp.verification_key.is_some() { "Some(...)" } else { "None" }));
    
    // zkapp_version
    output.push_str(&format!("  zkapp_version: {:?}\n", zkapp.zkapp_version));
    
    // action_state - 5 个 field elements
    output.push_str("  action_state:\n");
    for (i, state) in zkapp.action_state.iter().enumerate() {
        output.push_str(&format!("    [{}]: 0x{}\n", i, hex::encode(state.as_ref())));
    }
    
    // last_action_slot
    output.push_str(&format!("  last_action_slot: {:?}\n", zkapp.last_action_slot));
    
    // proved_state
    output.push_str(&format!("  proved_state: {}\n", zkapp.proved_state));
    
    // zkapp_uri
    let uri: Result<String, _> = (&zkapp.zkapp_uri).try_into();
    output.push_str(&format!("  zkapp_uri: {:?}\n", uri.unwrap_or_default()));
    
    output
}

/// Queries the Mina state from the Mina Node and returns the proof that the queried Mina state is the last finalized state
/// of the blockchain.
/// This proof along its public inputs are structured so that they can be sent to Aligned Layer to be verified.
/// This function also queries info from the Mina State Settlement Example Ethereum Contract to fetch one of the public
/// inputs.
///
/// The queried data consists of:
///
/// - Bridge tip state hash from the Mina State Settlement Example Ethereum Contract
/// - Mina candidate chain states from the Mina node
/// - Mina Bridge tip state from the Mina node
pub async fn get_mina_proof_of_state(
    rpc_url: &str,
    eth_rpc_url: &str,
    contract_addr: &str,
    is_state_proof_from_devnet: bool,
) -> Result<(MinaStateProof, MinaStatePubInputs), String> {
    let bridge_tip_state_hash = get_bridge_tip_hash(contract_addr, eth_rpc_url).await?.0;
    let (
        candidate_chain_states,
        candidate_chain_state_hashes,
        candidate_chain_ledger_hashes,
        candidate_tip_proof,
    ) = query_candidate_chain_0(rpc_url).await?;

    let candidate_tip_state_hash = candidate_chain_state_hashes
        .last()
        .ok_or("Missing candidate tip state hash".to_string())?;

    let bridge_tip_state = query_state(rpc_url, &bridge_tip_state_hash).await?;

    info!("Queried Mina candidate chain with tip {candidate_tip_state_hash} and its proof");

    let mina_state_proof = MinaStateProof {
        candidate_tip_proof,
        candidate_chain_states,
        bridge_tip_state,
    };
    let mina_state_pub_inputs = MinaStatePubInputs {
        is_state_proof_from_devnet,
        bridge_tip_state_hash,
        candidate_chain_state_hashes,
        candidate_chain_ledger_hashes,
    };

    let proof_bytes = bincode::serialize(&mina_state_proof)
        .map_err(|err| format!("Failed to serialize state proof: {err}"))?;
    let pub_input_bytes = bincode::serialize(&mina_state_pub_inputs)
        .map_err(|err| format!("Failed to serialize public inputs: {err}"))?;
    if !verify_mina_state(&proof_bytes, &pub_input_bytes) {
        return Err("Mina state proof verification failed".to_string());
    }
    info!("Mina state proof verification passed");

    Ok((
        mina_state_proof,
        mina_state_pub_inputs,
    ))
}

/// Queries the state of the account that corresponds to `public_key` from the Mina node and returns the proof that the
/// queried account is included in the ledger hash.
/// This proof along its public inputs are structured so that they can be sent to Aligned Layer to be verified.
///
/// The proof consists of:
///
/// - A Merkle root which maps to the ledger hash.
/// - A Merkle leaf which maps to the queried account.
/// - A Merkle path from the root to the leaf both mentioned above.
pub async fn get_mina_proof_of_account(
    public_key: &str,
    token_id: &str,
    state_hash: &str,
    rpc_url: &str,
) -> Result<(MinaAccountProof, MinaAccountPubInputs), String> {
    let (account, ledger_hash, merkle_path) =
        query_account(rpc_url, state_hash, public_key, token_id).await?;

    let encoded_account = MinaAccountValidationExample::Account::try_from(&account)?.abi_encode();

    info!(
        "Retrieved proof of account for ledger {}",
        LedgerHash::from_fp(ledger_hash)
    );

    Ok((
        MinaAccountProof {
            merkle_path,
            account,
        },
        MinaAccountPubInputs {
            ledger_hash,
            encoded_account,
        },
    ))
}

async fn query_state(
    rpc_url: &str,
    state_hash: &StateHash,
) -> Result<MinaStateProtocolStateValueStableV2, String> {
    let variables = state_query::Variables {
        state_hash: state_hash.to_string(),
    };
    info!("Querying state {}", variables.state_hash);
    let client = reqwest::Client::new();
    let proof = post_graphql::<StateQuery, _>(&client, rpc_url, variables)
        .await
        .map_err(|err| err.to_string())?
        .data
        .ok_or("Missing state query response data".to_string())
        .map(|response| response.protocol_state)
        .and_then(|base64| {
            BASE64_STANDARD
                .decode(base64)
                .map_err(|err| format!("Couldn't decode state from base64: {err}"))
        })
        .and_then(|binprot| {
            MinaStateProtocolStateValueStableV2::binprot_read(&mut binprot.as_slice())
                .map_err(|err| format!("Couldn't read state binprot: {err}"))
        })?;
    Ok(proof)
}

async fn query_candidate_chain(
    rpc_url: &str,
) -> Result<
    (
        Vec<MinaStateProtocolStateValueStableV2>,
        [StateHash; BRIDGE_TRANSITION_FRONTIER_LEN],
        [LedgerHash; BRIDGE_TRANSITION_FRONTIER_LEN],
        MinaBaseProofStableV2,
    ),
    String,
> {
    info!("Querying for candidate state");
    let client = reqwest::Client::new();
    let variables = best_chain_query::Variables {
        max_length: BRIDGE_TRANSITION_FRONTIER_LEN
            .try_into()
            .map_err(|_| "Transition frontier length conversion failure".to_string())?,
    };
    let response = post_graphql::<BestChainQuery, _>(&client, rpc_url, variables)
        .await
        .map_err(|err| err.to_string())?
        .data
        .ok_or("Missing candidate query response data".to_string())?;
    let best_chain = response
        .best_chain
        .ok_or("Missing best chain field".to_string())?;
    if best_chain.len() != BRIDGE_TRANSITION_FRONTIER_LEN {
        return Err(format!(
            "Not enough blocks ({}) were returned from query",
            best_chain.len()
        ));
    }
    let chain_state_hashes: [StateHash; BRIDGE_TRANSITION_FRONTIER_LEN] = best_chain
        .iter()
        .map(|state| state.state_hash.clone())
        .collect::<Vec<StateHash>>()
        .try_into()
        .map_err(|_| "Failed to convert chain state hashes vector into array".to_string())?;
    let chain_states = join_all(
        chain_state_hashes
            .iter()
            .map(|state_hash| query_state(rpc_url, state_hash)),
    )
    .await
    .into_iter()
    .collect::<Result<Vec<_>, _>>()?;

    // Derive ledger hashes from the full protocol states to ensure consistency
    // with on-chain verification logic.
    let chain_ledger_hashes: [LedgerHash; BRIDGE_TRANSITION_FRONTIER_LEN] = chain_states
        .iter()
        .map(|state| {
            state
                .body
                .blockchain_state
                .ledger_proof_statement
                .target
                .first_pass_ledger
                .clone()
        })
        .collect::<Vec<LedgerHash>>()
        .try_into()
        .map_err(|_| "Failed to convert chain ledger hashes vector into array".to_string())?;

    let tip = best_chain.last().ok_or("Missing best chain".to_string())?;
    let tip_state_proof = tip
        .protocol_state_proof
        .base64
        .clone()
        .ok_or("No tip state proof".to_string())
        .and_then(|base64| {
            BASE64_URL_SAFE
                .decode(base64)
                .map_err(|err| format!("Couldn't decode state proof from base64: {err}"))
        })
        .and_then(|binprot| {
            MinaBaseProofStableV2::binprot_read(&mut binprot.as_slice())
                .map_err(|err| format!("Couldn't read state proof binprot: {err}"))
        })?;

    info!("Queried state hashes: {chain_state_hashes:?}");
    info!("Queried ledger hashes: {chain_ledger_hashes:?}");

    Ok((
        chain_states,
        chain_state_hashes,
        chain_ledger_hashes,
        tip_state_proof,
    ))
}

/// Queries the Mina node with URL `rpc_url` for the root state hash of the transition frontier.
/// Returns the ledger hash structured so that it can be sent to the Mina State Settlement Ethereum Contract Example
/// constructor.
pub async fn query_root(rpc_url: &str, length: usize) -> Result<StateHash, String> {
    let client = reqwest::Client::new();
    let variables = best_chain_query::Variables {
        max_length: length as i64,
    };
    let response = post_graphql::<BestChainQuery, _>(&client, rpc_url, variables)
        .await
        .map_err(|err| err.to_string())?
        .data
        .ok_or("Missing root hash query response data".to_string())?;
    let best_chain = response
        .best_chain
        .ok_or("Missing best chain field".to_string())?;
    let root = best_chain.first().ok_or("No root state")?;
    Ok(root.state_hash.clone())
}

pub async fn query_account(
    rpc_url: &str,
    state_hash: &str,
    public_key: &str,
    token_id: &str
) -> Result<(MinaAccount, Fp, Vec<MerkleNode>), String> {
    info!(
        "Querying account[public_key:{public_key}, token_id:{token_id}], its merkle proof and ledger hash for state {state_hash}"
    );
    let client = reqwest::Client::new();
    
    let variables = account_query::Variables {
        state_hash: state_hash.to_owned(),
        account_infos: vec![account_query::AccountInput {
            public_key: public_key.to_owned(),
            token: Some(token_id.to_owned()),
        }],
    };


    let request_body = AccountQuery::build_query(variables);
    info!("Sending request to {}: {:?}", rpc_url, serde_json::to_string(&request_body).unwrap());

    let response = client
        .post(rpc_url)
        .json(&request_body)
        .send()
        .await
        .map_err(|e| e.to_string())?;

    let response_text = response.text().await.map_err(|e| e.to_string())?;
    info!("Raw response body: {}", response_text);

    /*
    let response_text = include_str!("../accoun_query_response.json");
    info!("Raw response body: {}", response_text);
    */
    let response: graphql_client::Response<account_query::ResponseData> =
        serde_json::from_str(&response_text).map_err(|e| format!("Failed to parse JSON: {}", e))?;

    let response = response
        .data
        .ok_or("Missing merkle query response data".to_string())?;

    let membership = response
        .encoded_snarked_ledger_account_membership
        .first()
        .ok_or("Failed to retrieve membership query field".to_string())?;

    // 解码 Base64 账户数据
    let account_bytes = BASE64_STANDARD
        .decode(&membership.account)
        .map_err(|err| format!("Failed to decode account from base64: {err}"))?;

    info!("Decoded account bytes length: {}", account_bytes.len());
    info!("Decoded account bytes (hex): {}", hex::encode(&account_bytes));

    let account = MinaAccount::binprot_read(&mut account_bytes.as_slice())
        .map_err(|err| format!("Failed to deserialize account binprot: {err}"))?;

    // 打印账户详细信息
    info!("=== Decoded MinaAccount ===");
    info!("Public Key: {:?}", account.public_key);
    info!("Token ID: {:?}", account.token_id);
    info!("Token Symbol: {:?}", account.token_symbol);
    info!("Balance: {:?}", account.balance);
    info!("Nonce: {:?}", account.nonce);
    info!("Delegate: {:?}", account.delegate);
    info!("Has zkApp: {}", account.zkapp.is_some());
    if let Some(ref zkapp) = account.zkapp {
        info!("=== zkApp Details (Human Readable) ===\n{}", format_zkapp_readable(zkapp));
        info!("=== End zkApp Details ===");
    }
    info!("=== End of MinaAccount ===");

    let ledger_hash = response
        .block
        .protocol_state
        .blockchain_state
        .snarked_ledger_hash
        .to_fp()
        .unwrap();

    let merkle_path = membership
        .merkle_path
        .iter()
        .map(|node| -> Result<MerkleNode, ()> {
            match (node.left.as_ref(), node.right.as_ref()) {
                (Some(fp_str), None) => Ok(MerkleNode::Left(Fp::from_str(fp_str)?)),
                (None, Some(fp_str)) => Ok(MerkleNode::Right(Fp::from_str(fp_str)?)),
                _ => unreachable!(),
            }
        })
        .collect::<Result<Vec<_>, ()>>()
        .map_err(|_| "Error deserializing merkle path nodes".to_string())?;

    Ok((account, ledger_hash, merkle_path))
}

async fn query_candidate_chain_0(
    rpc_url: &str,
) -> Result<
    (
        Vec<MinaStateProtocolStateValueStableV2>,
        [StateHash; BRIDGE_TRANSITION_FRONTIER_LEN],
        [LedgerHash; BRIDGE_TRANSITION_FRONTIER_LEN],
        MinaBaseProofStableV2,
    ),
    String,
> {
    info!("Querying for candidate state");
    let client = reqwest::Client::new();
    let variables = best_chain_query::Variables {
        max_length: BRIDGE_TRANSITION_FRONTIER_LEN
            .try_into()
            .map_err(|_| "Transition frontier length conversion failure".to_string())?,
    };
    let response = post_graphql::<BestChainQuery, _>(&client, rpc_url, variables)
        .await
        .map_err(|err| err.to_string())?
        .data
        .ok_or("Missing candidate query response data".to_string())?;
    let best_chain = response
        .best_chain
        .ok_or("Missing best chain field".to_string())?;
    if best_chain.len() != BRIDGE_TRANSITION_FRONTIER_LEN {
        return Err(format!(
            "Not enough blocks ({}) were returned from query",
            best_chain.len()
        ));
    }
    let chain_state_hashes: [StateHash; BRIDGE_TRANSITION_FRONTIER_LEN] = best_chain
        .iter()
        .map(|state| state.state_hash.clone())
        .collect::<Vec<StateHash>>()
        .try_into()
        .map_err(|_| "Failed to convert chain state hashes vector into array".to_string())?;
    let chain_ledger_hashes: [LedgerHash; BRIDGE_TRANSITION_FRONTIER_LEN] = best_chain
        .iter()
        .map(|state| {
            state
                .protocol_state
                .blockchain_state
                .snarked_ledger_hash
                .clone()
        })
        .collect::<Vec<LedgerHash>>()
        .try_into()
        .map_err(|_| "Failed to convert chain ledger hashes vector into array".to_string())?;
    info!("chain_ledger_hashes (snarked): {:?}", chain_ledger_hashes);

    let chain_states: Vec<MinaStateProtocolStateValueStableV2> = join_all(
        chain_state_hashes
            .iter()
            .map(|state_hash| query_state(rpc_url, state_hash)),
    )
    .await
    .into_iter()
    .collect::<Result<Vec<_>, _>>()?;

    let first_pass_ledgers: Vec<_> = chain_states
        .iter()
        .map(|state| {

            // when BRIDGE_TRANSITION_FRONTIER_LEN>16
            state
                .body
                .blockchain_state
                .ledger_proof_statement
                .target
                .first_pass_ledger
                .clone()

            /* original snippet when BRIDGE_TRANSITION_FRONTIER_LEN=16
            state
                .body
                .blockchain_state
                .snarked_ledger_hash
                .clone()
             */
            
        })
        .collect();
    info!("chain_ledger_hashes (first_pass_ledger in proof): {:?}", first_pass_ledgers);

    let tip = best_chain.last().ok_or("Missing best chain".to_string())?;
    let tip_state_proof = tip
        .protocol_state_proof
        .base64
        .clone()
        .ok_or("No tip state proof".to_string())
        .and_then(|base64| {
            BASE64_URL_SAFE
                .decode(base64)
                .map_err(|err| format!("Couldn't decode state proof from base64: {err}"))
        })
        .and_then(|binprot| {
            MinaBaseProofStableV2::binprot_read(&mut binprot.as_slice())
                .map_err(|err| format!("Couldn't read state proof binprot: {err}"))
        })?;

    info!("Queried state hashes: {chain_state_hashes:?}");
    info!("Queried ledger hashes: {chain_ledger_hashes:?}");

    Ok((
        chain_states,
        chain_state_hashes,
        chain_ledger_hashes,
        tip_state_proof,
    ))
}
