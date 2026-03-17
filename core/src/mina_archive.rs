use graphql_client::{reqwest::post_graphql, GraphQLQuery};
use mina_curves::pasta::Fp;

use crate::pubkey::{MinaCompressedPubKey, MinaPubKeyBase58};
use crate::rpcs::errors::MinaArchiveError;

#[derive(GraphQLQuery)]
#[graphql(
    schema_path = "graphql/archive/schema.json",
    query_path = "graphql/archive/address_events_query.graphql"
)]
struct AddressEventsQuery;

#[derive(GraphQLQuery)]
#[graphql(
    schema_path = "graphql/archive/schema.json",
    query_path = "graphql/archive/canonical_block_query.graphql"
)]
struct CanonicalBlockQuery;

/// A raw zkApp event as returned by the archive node, with block context.
///
/// o1js encodes every event emitted by a multi-event contract as [event_type, ...fields],
/// where event_type is the alphabetical index of the event name among all names declared
/// in `readonly events = { ... }`.
pub struct ZkAppEvent {
    pub block_height: u64,
    pub state_hash: String,
    /// data[0]: alphabetical index of the event type.
    pub event_type: String,
    /// data[1..]: payload field elements.
    pub fields: Vec<String>,
}

/// Parsed payload of a FungibleToken Burn event.
///
/// Corresponds to `BurnEvent { from: PublicKey, amount: UInt64 }` in o1js.
/// The raw x-coordinate and is-odd parity fields are parsed and encoded into
/// `from` at construction time; callers receive a ready-to-use address.
pub struct BurnEventPayload {
    /// Base58Check-encoded Mina public key of the burning account (o1js field: `from`).
    /// Reconstructed from data[1] (x-coordinate) and data[2] (is-odd parity).
    pub from: MinaPubKeyBase58,
    /// data[3]: amount burned (UInt64 as decimal string).
    pub amount: String,
}

/// A fully parsed Burn event with block context.
pub struct BurnEvent {
    pub block_height: u64,
    pub state_hash: String,
    pub payload: BurnEventPayload,
}

/// Fetches all zkApp events emitted at `contract_addr` from `from_height` onwards.
pub async fn detect_zk_app_events(
    rpc_url: &str,
    contract_addr: &str,
    from_height: u64,
) -> Result<Vec<ZkAppEvent>, MinaArchiveError> {
    let client = reqwest::Client::new();
    let variables = address_events_query::Variables {
        address: contract_addr.to_owned(),
        from: from_height as i64,
    };
    let response = post_graphql::<AddressEventsQuery, _>(&client, rpc_url, variables).await?;
    if let Some(errors) = response.errors {
        if !errors.is_empty() {
            let msgs: Vec<String> = errors.iter().map(|e| e.message.clone()).collect();
            return Err(MinaArchiveError::GraphQLError(msgs.join("; ")));
        }
    }
    let data = response.data.ok_or_else(|| MinaArchiveError::MalformedResponse("missing response data".into()))?;
    let mut events = Vec::new();
    for event_output in data.events.into_iter().flatten() {
        let block_info = event_output
            .block_info
            .ok_or_else(|| MinaArchiveError::MalformedResponse("Missing blockInfo in event output".into()))?;
        let block_height = block_info.height as u64;
        let state_hash = block_info.state_hash;
        for event_data in event_output.event_data.into_iter().flatten().flatten() {
            let mut data: Vec<String> = event_data.data.into_iter().flatten().collect();
            if data.is_empty() {
                continue;
            }
            let event_type = data.remove(0);
            events.push(ZkAppEvent {
                block_height,
                state_hash: state_hash.clone(),
                event_type,
                fields: data,
            });
        }
    }
    Ok(events)
}

/// Queries the state hash of the canonical block at `height` from the archive node.
///
/// Returns `Some(state_hash)` if a canonical block exists at that height, or `None` if the
/// archive node returns no result (no canonical block recorded — treat as non-canonical).
/// `canonical: true` is required in the filter: the archive stores both canonical and orphaned
/// blocks at the same height, and the `Block` return type does not include a `canonical` field,
/// so without the filter there is no way to identify the canonical block from the response.
pub async fn query_canonical_block_at_height(
    rpc_url: &str,
    height: u64,
) -> Result<Option<String>, MinaArchiveError> {
    let client = reqwest::Client::new();
    let variables = canonical_block_query::Variables {
        height_gte: height as i64,
        height_lt: (height + 1) as i64,
    };
    let response = post_graphql::<CanonicalBlockQuery, _>(&client, rpc_url, variables).await?;
    if let Some(errors) = response.errors {
        if !errors.is_empty() {
            let msgs: Vec<String> = errors.iter().map(|e| e.message.clone()).collect();
            return Err(MinaArchiveError::GraphQLError(msgs.join("; ")));
        }
    }
    let blocks = response
        .data
        .ok_or_else(|| MinaArchiveError::MalformedResponse("missing response data".into()))?
        .blocks;
    Ok(blocks.into_iter().flatten().next().map(|b| b.state_hash))
}

/// Alphabetical index of "Burn" among the FungibleToken contract's event names:
/// { BalanceChange=0, Burn=1, Mint=2, Pause=3, SetAdmin=4 }.
const BURN_EVENT_TYPE: &str = "1";

/// Fetches and parses Burn events emitted at `contract_addr` from `from_height` onwards.
///
/// Calls [`detect_zk_app_events`], filters for events with type tag `1` (Burn), and parses
/// the payload fields into a [`BurnEvent`].
pub async fn detect_nori_burn(
    rpc_url: &str,
    contract_addr: &str,
    from_height: u64,
) -> Result<Vec<BurnEvent>, MinaArchiveError> {
    let raw = detect_zk_app_events(rpc_url, contract_addr, from_height).await?;
    let mut burns = Vec::new();
    for event in raw {
        if event.event_type != BURN_EVENT_TYPE {
            continue;
        }
        if event.fields.len() < 3 {
            return Err(MinaArchiveError::MalformedResponse(format!(
                "Burn event at block {} has {} payload fields, expected 3",
                event.block_height,
                event.fields.len()
            )));
        }
        let pub_x = event.fields[0]
            .parse::<Fp>()
            .map_err(|_| MinaArchiveError::MalformedResponse(format!("Burn event at block {}: invalid x-coordinate: {}", event.block_height, event.fields[0])))?;
        let pub_x_is_odd = match event.fields[1].as_str() {
            "0" => false,
            "1" => true,
            other => return Err(MinaArchiveError::MalformedResponse(format!("Burn event at block {}: invalid is_odd value: {other}", event.block_height))),
        };
        burns.push(BurnEvent {
            block_height: event.block_height,
            state_hash: event.state_hash,
            payload: BurnEventPayload {
                from: MinaCompressedPubKey { x: pub_x, is_odd: pub_x_is_odd }.into(),
                amount: event.fields[2].clone(),
            },
        });
    }
    Ok(burns)
}
