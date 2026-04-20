// Typed error enums for each RPC provider.
//
// Workers match on these to decide which failure_code to write to the database.
// Each enum classifies errors by recovery action, not by wire format.
//
// The classify_* functions at the bottom of each section translate from the
// provider's native error types into our enums.

use serde::{Deserialize, Serialize};
use std::fmt;

// ---------------------------------------------------------------------------
// Mina Daemon (GraphQL over HTTP via reqwest + graphql_client)
// ---------------------------------------------------------------------------

/// Errors from Mina daemon GraphQL queries and proof assembly.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum MinaDaemonError {
    /// Network-level failure: connection refused, timeout, DNS, HTTP non-200.
    RpcUnreachable(String),

    /// The GraphQL response contained an `errors` array.
    /// The daemon accepted the request but the query itself failed
    /// (e.g. block not found, internal server error).
    GraphQLError(String),

    /// The state hash returned by the daemon does not match the expected value.
    /// Happens when the daemon's frontier has moved between queries.
    StateHashMismatch(String),

    /// Local Pickles verification rejected the assembled proof.
    /// The proof is structurally valid but does not verify -- likely a
    /// bug in proof assembly or a daemon returning inconsistent data.
    LocalVerificationFailed(String),

    /// The requested block height is beyond what the daemon can serve.
    /// The daemon only keeps MINA_DAEMON_MAX_QUERYABLE_BLOCKS (290) blocks.
    BlockTooOld(String),

    /// The daemon's transition frontier does not contain a block at the requested
    /// height. This can mean the block has fallen off the ~290-block frontier
    /// (too old), the daemon has just restarted and has not yet built a full
    /// frontier, or the height has not been reached yet (future block). The daemon
    /// cannot tell us which case it is -- only that it cannot answer.
    BlockNotInFrontier(String),

    /// The account or account membership proof was not found for the
    /// given public key / token ID / state hash combination.
    AccountNotFound(String),

    /// The response was structurally unexpected: missing fields, wrong types,
    /// failed deserialization of daemon data.
    MalformedResponse(String),

    /// We constructed a bad request: invalid URL, invalid headers, redirect
    /// loop, or the server returned 4xx (our fault, not the network's).
    /// Never transient -- indicates a code or configuration bug.
    BadRequest(String),

    /// Our own serialization of assembled proof data failed (bincode, JSON),
    /// or conversion of daemon data to another format (e.g. Ethereum ABI)
    /// failed. The daemon response was fine -- our code failed to process it.
    /// Never transient -- indicates a code bug in data structures.
    SerializationError(String),
}

impl fmt::Display for MinaDaemonError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::RpcUnreachable(msg) => write!(f, "rpc unreachable: {msg}"),
            Self::GraphQLError(msg) => write!(f, "graphql error: {msg}"),
            Self::StateHashMismatch(msg) => write!(f, "state hash mismatch: {msg}"),
            Self::LocalVerificationFailed(msg) => write!(f, "local verification failed: {msg}"),
            Self::BlockTooOld(msg) => write!(f, "block too old: {msg}"),
            Self::BlockNotInFrontier(msg) => write!(f, "block not in frontier: {msg}"),
            Self::AccountNotFound(msg) => write!(f, "account not found: {msg}"),
            Self::MalformedResponse(msg) => write!(f, "malformed response: {msg}"),
            Self::BadRequest(msg) => write!(f, "bad request: {msg}"),
            Self::SerializationError(msg) => write!(f, "serialization error: {msg}"),
        }
    }
}

impl std::error::Error for MinaDaemonError {}

// Classification from reqwest errors (used by graphql_client::reqwest::post_graphql).
//
// reqwest::Error exposes:
//   .is_connect()   -- TCP/DNS failure
//   .is_timeout()   -- read/connect timeout
//   .is_request()   -- request construction failure
//   .is_status()    -- HTTP non-2xx (call .status() for the code)
//   .is_decode()    -- response body decode failure
//
// All of these are RpcUnreachable except decode errors which are MalformedResponse.

impl From<reqwest::Error> for MinaDaemonError {
    fn from(e: reqwest::Error) -> Self {
        if e.is_decode() {
            Self::MalformedResponse(e.to_string())
        } else if e.is_request() {
            // Request construction failure: invalid URL, invalid headers,
            // redirect loop. Our fault, not the network's.
            Self::BadRequest(e.to_string())
        } else if e.is_status() {
            // HTTP non-2xx. 4xx = our fault (bad request), 5xx = server issue.
            if let Some(status) = e.status() {
                if status.is_client_error() {
                    return Self::BadRequest(e.to_string());
                }
            }
            Self::RpcUnreachable(e.to_string())
        } else {
            Self::RpcUnreachable(e.to_string())
        }
    }
}

// Bridge impl: allows workers that still use `crate::error::Error` to accept
// MinaDaemonError via `?`. Remove once workers match on typed errors directly.
impl From<MinaDaemonError> for crate::error::Error {
    fn from(e: MinaDaemonError) -> Self {
        crate::error::Error(e.to_string())
    }
}

// ---------------------------------------------------------------------------
// Mina Archive Node (GraphQL over HTTP via reqwest + graphql_client)
// ---------------------------------------------------------------------------

/// Errors from Mina archive node GraphQL queries.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum MinaArchiveError {
    /// Network-level failure: connection refused, timeout, DNS, HTTP non-200.
    RpcUnreachable(String),

    /// The GraphQL response contained an `errors` array.
    GraphQLError(String),

    /// The response was structurally unexpected.
    MalformedResponse(String),

    /// We constructed a bad request: invalid URL, invalid headers, redirect
    /// loop, or the server returned 4xx (our fault, not the network's).
    /// Never transient -- indicates a code or configuration bug.
    BadRequest(String),
}

impl fmt::Display for MinaArchiveError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::RpcUnreachable(msg) => write!(f, "rpc unreachable: {msg}"),
            Self::GraphQLError(msg) => write!(f, "graphql error: {msg}"),
            Self::MalformedResponse(msg) => write!(f, "malformed response: {msg}"),
            Self::BadRequest(msg) => write!(f, "bad request: {msg}"),
        }
    }
}

impl std::error::Error for MinaArchiveError {}

impl From<reqwest::Error> for MinaArchiveError {
    fn from(e: reqwest::Error) -> Self {
        if e.is_decode() {
            Self::MalformedResponse(e.to_string())
        } else if e.is_request() {
            Self::BadRequest(e.to_string())
        } else if e.is_status() {
            if let Some(status) = e.status() {
                if status.is_client_error() {
                    return Self::BadRequest(e.to_string());
                }
            }
            Self::RpcUnreachable(e.to_string())
        } else {
            Self::RpcUnreachable(e.to_string())
        }
    }
}

impl From<MinaArchiveError> for crate::error::Error {
    fn from(e: MinaArchiveError) -> Self {
        crate::error::Error(e.to_string())
    }
}

// ---------------------------------------------------------------------------
// Aligned (SDK over WebSocket via tokio-tungstenite)
// ---------------------------------------------------------------------------

/// Errors from Aligned batcher submission and on-chain verification checks.
///
/// The aligned SDK (ethers 2.0, pinned at rev 11d1801) exposes two public
/// error enums:
///   - `aligned_sdk::common::errors::SubmitError` (35 variants)
///   - `aligned_sdk::common::errors::VerificationError` (4 variants)
///
/// We collapse these into a small set grouped by recovery action.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum AlignedError {
    /// Network-level failure: WebSocket connection refused/dropped, Ethereum
    /// provider unreachable, batcher did not respond, nonce fetch failed.
    /// Transient -- retry with backoff.
    RpcUnreachable(String),

    /// Batcher was reachable but rejected the submission for a transient
    /// account-state reason: stale nonce, fee too low, insufficient balance,
    /// queue full, proof replaced by a newer submission, queue flushed.
    /// Transient -- retry with fresh nonce/fee estimate.
    BatcherRejected(String),

    /// Batcher received the proof and attempted to batch it, but the batch
    /// did not land: on-chain submission failed, verification timed out,
    /// or the event stream watching for verification broke.
    /// Transient -- retry (proof is still valid).
    BatchFailed(String),

    /// Our CBOR-encoded proof payload exceeds the batcher's 4 MiB limit.
    /// Not transient -- the proof must be regenerated at a different anchor.
    PayloadTooLarge(String),

    /// The batcher or verifier explicitly rejected the proof as invalid.
    /// Not transient -- the proof must be regenerated.
    ProofRejected(String),

    /// SDK-level configuration error: invalid chain ID, invalid address,
    /// unsupported proving system, missing parameters, not a contract.
    /// Not transient -- requires human intervention to fix config.
    ConfigurationError(String),

    /// Wallet or signer error: invalid signature, wallet signer failure.
    /// Not transient -- indicates a wallet setup or key issue.
    WalletError(String),

    /// Data error: empty verification data, serialization failure, hex
    /// decoding failure. Our code produced bad data for the SDK.
    /// Not transient -- indicates a code bug in data preparation.
    DataError(String),

    /// Local filesystem IO failure: the SDK could not read a file.
    /// Not transient -- indicates a deployment or permissions issue.
    IoError(String),

    /// An Ethereum call to the AlignedLayerServiceManager contract failed.
    /// Used when reading batch state (e.g. `batchesState`) for verification
    /// disambiguation. The inner `EthError` carries the full typed failure.
    BatcherContractCallFailed(EthError),

    /// The SDK returned an error we cannot classify from its type alone.
    /// The worker must match on the inner string for known SDK messages
    /// before falling back to logging for triage.
    Unclassified(String),
}

impl fmt::Display for AlignedError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::RpcUnreachable(msg) => write!(f, "rpc unreachable: {msg}"),
            Self::BatcherRejected(msg) => write!(f, "batcher rejected: {msg}"),
            Self::BatchFailed(msg) => write!(f, "batch failed: {msg}"),
            Self::PayloadTooLarge(msg) => write!(f, "payload too large: {msg}"),
            Self::ProofRejected(msg) => write!(f, "proof rejected: {msg}"),
            Self::ConfigurationError(msg) => write!(f, "configuration error: {msg}"),
            Self::WalletError(msg) => write!(f, "wallet error: {msg}"),
            Self::DataError(msg) => write!(f, "data error: {msg}"),
            Self::IoError(msg) => write!(f, "io error: {msg}"),
            Self::BatcherContractCallFailed(e) => write!(f, "batcher contract call failed: {e}"),
            Self::Unclassified(msg) => write!(f, "unclassified: {msg}"),
        }
    }
}

impl std::error::Error for AlignedError {}

/// Classifies an aligned SDK `SubmitError` into our `AlignedError`.
///
/// Uses the aligned SDK's public typed enum directly -- no string matching.
/// The SDK is pinned at rev 11d1801 so these variants are stable for us.
pub fn classify_submit_error(
    e: aligned_sdk::common::errors::SubmitError,
) -> AlignedError {
    use aligned_sdk::common::errors::SubmitError;

    match e {
        // -- Network / transient --
        SubmitError::WebSocketConnectionError(ws_err) => {
            AlignedError::RpcUnreachable(format!("websocket: {ws_err}"))
        }
        SubmitError::WebSocketClosedUnexpectedlyError(frame) => {
            AlignedError::RpcUnreachable(format!("websocket closed: {frame}"))
        }
        SubmitError::EthereumProviderError(msg) => {
            // Same split as classify_verification_error: URL parse errors are
            // configuration, actual RPC failures are network.
            if msg.contains("url") || msg.contains("Url") || msg.contains("parse") {
                AlignedError::ConfigurationError(format!("invalid rpc url: {msg}"))
            } else {
                AlignedError::RpcUnreachable(format!("ethereum provider: {msg}"))
            }
        }
        SubmitError::NoResponseFromBatcher => {
            AlignedError::RpcUnreachable("no response from batcher".into())
        }
        SubmitError::GetNonceError(msg) => {
            AlignedError::RpcUnreachable(format!("get nonce: {msg}"))
        }

        // -- Batch accepted but did not land on-chain --
        SubmitError::BatchVerificationTimeout { timeout_seconds } => {
            AlignedError::BatchFailed(format!("verification timeout: {timeout_seconds}s"))
        }
        SubmitError::BatchSubmissionFailed(msg) => {
            AlignedError::BatchFailed(format!("on-chain submission failed: {msg}"))
        }
        SubmitError::BatchVerifiedEventStreamError(msg) => {
            AlignedError::BatchFailed(format!("event stream broke: {msg}"))
        }

        // -- Local filesystem / IO --
        SubmitError::IoError(path, io_err) => {
            AlignedError::IoError(format!("{}: {io_err}", path.display()))
        }

        // -- Proof too large --
        SubmitError::ProofTooLarge => {
            AlignedError::PayloadTooLarge("proof exceeds batcher size limit".into())
        }

        // -- Proof rejected --
        SubmitError::InvalidProof(reason) => {
            AlignedError::ProofRejected(format!("invalid proof: {reason:?}"))
        }
        SubmitError::AddToBatchError => {
            AlignedError::ProofRejected("add to batch rejected".into())
        }
        SubmitError::InvalidProofInclusionData => {
            AlignedError::ProofRejected("invalid proof inclusion data".into())
        }
        SubmitError::ProofQueueFlushed => {
            AlignedError::BatchFailed("proof queue flushed".into())
        }

        // -- Batcher reachable but rejected (transient account-state) --
        SubmitError::InvalidNonce => {
            AlignedError::BatcherRejected("invalid nonce".into())
        }
        SubmitError::InvalidMaxFee => {
            AlignedError::BatcherRejected("invalid max fee".into())
        }
        SubmitError::InsufficientBalance(addr) => {
            AlignedError::BatcherRejected(format!("insufficient balance for {addr:?}"))
        }
        SubmitError::BatchQueueLimitExceededError => {
            AlignedError::BatcherRejected("batch queue limit exceeded".into())
        }
        SubmitError::ProofReplaced => {
            AlignedError::BatcherRejected("proof replaced by newer submission".into())
        }
        SubmitError::UserFundsUnlocked => {
            AlignedError::BatcherRejected("user funds unlocked during submission".into())
        }
        SubmitError::UnexpectedBatcherResponse(msg) => {
            AlignedError::BatcherRejected(format!("unexpected batcher response: {msg}"))
        }
        SubmitError::InvalidReplacementMessage => {
            AlignedError::BatcherRejected("invalid replacement message".into())
        }

        // -- Configuration (human intervention) --
        SubmitError::InvalidChainId => {
            AlignedError::ConfigurationError("invalid chain ID".into())
        }
        SubmitError::UnsupportedProvingSystem(msg) => {
            AlignedError::ConfigurationError(format!("unsupported proving system: {msg}"))
        }
        SubmitError::InvalidEthereumAddress(msg) => {
            AlignedError::ConfigurationError(format!("invalid ethereum address: {msg}"))
        }
        SubmitError::MissingRequiredParameter(msg) => {
            AlignedError::ConfigurationError(format!("missing parameter: {msg}"))
        }
        SubmitError::InvalidPaymentServiceAddress(expected, actual) => {
            AlignedError::ConfigurationError(format!(
                "payment service address mismatch: expected {expected:?}, got {actual:?}"
            ))
        }
        SubmitError::InvalidSignature => {
            AlignedError::WalletError("invalid wallet signature".into())
        }
        SubmitError::EmptyVerificationDataCommitments => {
            AlignedError::DataError("empty verification data commitments".into())
        }
        SubmitError::EmptyVerificationDataList => {
            AlignedError::DataError("empty verification data list".into())
        }

        // -- Encoding / serialization --
        SubmitError::SerializationError(ser_err) => {
            AlignedError::DataError(format!("serialization: {ser_err}"))
        }
        SubmitError::HexDecodingError(msg) => {
            AlignedError::DataError(format!("hex decoding: {msg}"))
        }
        SubmitError::WalletSignerError(msg) => {
            AlignedError::WalletError(format!("wallet signer: {msg}"))
        }

        // -- GenericError: the SDK stuffs several distinct situations here --
        // Match on the known fixed strings from the SDK source (rev 11d1801).
        SubmitError::GenericError(msg) => classify_generic_submit_error(msg),

        // Non-exhaustive guard: if the SDK adds variants at a future pin,
        // log and triage -- do not silently treat as transient.
        #[allow(unreachable_patterns)]
        other => AlignedError::Unclassified(format!("unclassified submit: {other}")),
    }
}

/// Classifies the SDK's `GenericError(String)` by matching on known fixed
/// messages from the SDK source (rev 11d1801).
///
/// The SDK uses GenericError as a catch-all for several distinct situations:
///   - "Trying to submit too many proofs at once" → BatcherRejected
///   - "Server is busy processing requests, please retry" → BatcherRejected
///   - "Connection was closed before receive() processed all sent messages " → RpcUnreachable
///   - "No response from the batcher" → RpcUnreachable
///   - Batcher error message (arbitrary string from protocol) → BatcherRejected
///   - WebSocket close error → RpcUnreachable
///   - Wrapped inner submit_multiple error → Unclassified (already classified upstream)
fn classify_generic_submit_error(msg: String) -> AlignedError {
    // Fixed messages from the SDK source. Matched with contains() because
    // the SDK may prepend/append context in future revisions.
    if msg.contains("too many proofs") {
        return AlignedError::BatcherRejected(format!("too many proofs: {msg}"));
    }
    if msg.contains("Server is busy") || msg.contains("server is busy") {
        return AlignedError::BatcherRejected(format!("server busy: {msg}"));
    }
    if msg.contains("Connection was closed") || msg.contains("connection was closed") {
        return AlignedError::RpcUnreachable(format!("connection closed: {msg}"));
    }
    if msg.contains("No response from the batcher") {
        return AlignedError::RpcUnreachable(format!("no response: {msg}"));
    }

    // Remaining GenericError instances are either:
    //   - A batcher protocol error message (arbitrary string)
    //   - A wrapped inner error from submit_multiple
    // We cannot reliably distinguish these, so mark as unclassified for
    // the worker to log and triage.
    AlignedError::Unclassified(format!("generic: {msg}"))
}

/// Classifies an aligned SDK `VerificationError` into our `AlignedError`.
pub fn classify_verification_error(
    e: aligned_sdk::common::errors::VerificationError,
) -> AlignedError {
    use aligned_sdk::common::errors::VerificationError;

    match e {
        VerificationError::EthereumProviderError(msg) => {
            // The SDK uses this for two things:
            //   1. URL parse error when creating the provider → configuration
            //   2. get_code RPC call failure → network
            // Distinguish via the error message.
            if msg.contains("url") || msg.contains("Url") || msg.contains("parse") {
                AlignedError::ConfigurationError(format!("invalid rpc url: {msg}"))
            } else {
                AlignedError::RpcUnreachable(format!("ethereum provider: {msg}"))
            }
        }
        VerificationError::EthereumCallError(msg) => {
            // Fires from exactly one callsite: service_manager.verify_batch_inclusion().
            // This is an ethers ContractError from an eth_call. If the contract
            // reverted, the proof is not verified on-chain (yet or ever).
            // If the call failed for network reasons, we can't reach the node.
            //
            // The ethers ContractError.to_string() for reverts typically starts
            // with "Revert" or "execution reverted". Network errors don't.
            let lower = msg.to_lowercase();
            if lower.contains("revert") || lower.contains("execution reverted") {
                AlignedError::ProofRejected(format!("batch inclusion check reverted: {msg}"))
            } else {
                AlignedError::RpcUnreachable(format!("verification call failed: {msg}"))
            }
        }
        VerificationError::EthereumNotAContract(addr) => {
            AlignedError::ConfigurationError(format!("not a contract: {addr:?}"))
        }
        VerificationError::HexDecodingError(msg) => {
            AlignedError::DataError(format!("hex decoding: {msg}"))
        }

        #[allow(unreachable_patterns)]
        other => AlignedError::Unclassified(format!("unclassified verification: {other}")),
    }
}

// ---------------------------------------------------------------------------
// Ethereum (alloy 1.x)
// ---------------------------------------------------------------------------

/// Every revert our contracts can produce.
///
/// Derived from MinaStateSettlementExample.sol, NoriTokenBridge.sol,
/// MinaAccountValidationExample.sol. The `alloy::sol!` block in eth.rs
/// generates types that implement `SolError`; `ErrorPayload::as_decoded_error`
/// matches selectors and decodes arguments automatically. This enum is the
/// bridge-side mirror used by workers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ContractRevert {
    // --- MinaStateSettlementExample custom errors ---

    /// error MinaProvingSystemIdIsNotValid(bytes32)
    ProvingSystemIdInvalid,

    /// error MinaNetworkIsWrong()
    NetworkMismatch,

    /// error NewStateIsNotValid()
    NewStateInvalid,

    /// error TipStateIsWrong(bytes32, bytes32)
    TipStateWrong,

    /// error AccountIsNotValid(bytes32)
    AccountIsNotValid,

    // --- MinaAccountValidationExample custom errors ---

    /// error MinaAccountProvingSystemIdIsNotValid(bytes32)
    AccountProvingSystemIdInvalid,

    // --- NoriTokenBridge custom errors ---

    /// error AlignedContractsNotConfigured()
    ContractsNotConfigured,

    /// error ZeroAddress()
    ZeroAddress,

    /// error NotBridgeOperator()
    NotBridgeOperator,

    // --- NoriTokenBridge require() messages ---

    /// require "Invalid Ledger"
    InvalidLedger,

    /// require "Invalid Zkapp Account"
    InvalidAccount,

    /// require "Burn so far is greater than the amount to burn"
    BurnAlreadyProcessed,

    /// require "To unlock amount is greater than the amount to unlock"
    UnlockAmountExceeded,

    /// require "You must send some Ether to lock"
    ZeroLockAmount,

    /// require "Must be multiple of smallest bridge unit"
    NotBridgeUnit,

    /// require "Total locked exceeds maximum allowed"
    MaxMagnitudeExceeded,

    /// require "This Mina account is already linked to a different ETH address"
    AccountAlreadyLinked,

    /// require "No ETH to withdraw"
    NothingToWithdraw,

    /// Revert data present but does not match any known selector or require
    /// message. Carries the raw hex for debugging.
    Unknown(String),
}

impl fmt::Display for ContractRevert {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ProvingSystemIdInvalid => write!(f, "MinaProvingSystemIdIsNotValid"),
            Self::NetworkMismatch => write!(f, "MinaNetworkIsWrong"),
            Self::NewStateInvalid => write!(f, "NewStateIsNotValid"),
            Self::TipStateWrong => write!(f, "TipStateIsWrong"),
            Self::AccountIsNotValid => write!(f, "AccountIsNotValid"),
            Self::AccountProvingSystemIdInvalid => {
                write!(f, "MinaAccountProvingSystemIdIsNotValid")
            }
            Self::ContractsNotConfigured => write!(f, "AlignedContractsNotConfigured"),
            Self::ZeroAddress => write!(f, "ZeroAddress"),
            Self::NotBridgeOperator => write!(f, "NotBridgeOperator"),
            Self::InvalidLedger => write!(f, "Invalid Ledger"),
            Self::InvalidAccount => write!(f, "Invalid Zkapp Account"),
            Self::BurnAlreadyProcessed => write!(f, "BurnAlreadyProcessed"),
            Self::UnlockAmountExceeded => write!(f, "UnlockAmountExceeded"),
            Self::ZeroLockAmount => write!(f, "ZeroLockAmount"),
            Self::NotBridgeUnit => write!(f, "NotBridgeUnit"),
            Self::MaxMagnitudeExceeded => write!(f, "MaxMagnitudeExceeded"),
            Self::AccountAlreadyLinked => write!(f, "AccountAlreadyLinked"),
            Self::NothingToWithdraw => write!(f, "NothingToWithdraw"),
            Self::Unknown(hex) => write!(f, "Unknown({hex})"),
        }
    }
}

/// Errors from Ethereum JSON-RPC and contract interactions via alloy.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum EthError {
    /// Transport-level failure: connection refused, timeout, DNS, TLS,
    /// HTTP 502/503, rate limited.
    /// Transient -- retry with backoff.
    RpcUnreachable(String),

    /// Transaction reverted with a decoded contract revert reason.
    Reverted(ContractRevert),

    /// Nonce conflict: nonce too low or replacement transaction underpriced.
    /// Transient -- retry (nonce manager should resolve).
    NonceCollision(String),

    /// EVM execution consumed all gas, or the Ethereum node rejected the
    /// transaction because gas required exceeds allowance / intrinsic gas
    /// too low.
    OutOfGas(String),

    /// Wallet balance insufficient for gas * gas_price + value.
    /// Reported by the Ethereum node via -32000 "insufficient funds".
    InsufficientFunds(String),

    /// Our gas safety policy rejected the transaction before sending.
    /// Either the network gas price exceeds MAX_GAS_PRICE_GWEI or the
    /// estimated gas exceeds MAX_GAS_LIMIT_VALUE.
    /// Transient -- retry later when gas conditions improve.
    GasSafetyLimit(String),

    /// Node returned a JSON-RPC error response that is not a known -32000
    /// subcategory and not a revert. Examples: null response, method not
    /// found (-32601), invalid params (-32602), internal error (-32603).
    /// Node is reachable but unhappy with our request.
    UnexpectedRpcResponse(String),

    /// Ethereum ABI encoding/decoding failure via alloy. Wrong types passed
    /// to a contract call or stale ABI JSON. Never transient -- indicates
    /// a code bug in contract bindings that requires a fix and redeploy.
    AbiError(String),

    /// Our own data serialization/deserialization failed. bincode, JSON, or
    /// other internal formats used to prepare or parse contract call data.
    /// Never transient -- indicates a code bug in our data structures.
    SerializationError(String),
}

impl fmt::Display for EthError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::RpcUnreachable(msg) => write!(f, "rpc unreachable: {msg}"),
            Self::Reverted(revert) => write!(f, "reverted: {revert}"),
            Self::NonceCollision(msg) => write!(f, "nonce collision: {msg}"),
            Self::OutOfGas(msg) => write!(f, "out of gas: {msg}"),
            Self::InsufficientFunds(msg) => write!(f, "insufficient funds: {msg}"),
            Self::GasSafetyLimit(msg) => write!(f, "gas safety limit: {msg}"),
            Self::UnexpectedRpcResponse(msg) => write!(f, "unexpected rpc response: {msg}"),
            Self::AbiError(msg) => write!(f, "abi error: {msg}"),
            Self::SerializationError(msg) => write!(f, "serialization error: {msg}"),
        }
    }
}

impl std::error::Error for EthError {}

impl EthError {
    /// Returns `true` for errors that are transient and worth retrying with backoff.
    /// Returns `false` for permanent failures where retrying would waste gas or indicate
    /// a code/config bug.
    pub fn is_retryable(&self) -> bool {
        matches!(
            self,
            EthError::RpcUnreachable(_)
                | EthError::NonceCollision(_)
                | EthError::UnexpectedRpcResponse(_)
                | EthError::GasSafetyLimit(_)
        )
    }
}

// AccountIsNotValid is defined in MinaStateSettlementExample.sol but absent from
// the ABI JSON (used internally via cross-contract call). Define it here so
// alloy::sol! generates a type with the correct selector for revert decoding.
alloy::sol! {
    error AccountIsNotValid(bytes32 accountIdHash);
}

// The classify functions below use alloy's typed error hierarchy.
//
// alloy_contract::Error
//   -> TransportError (= RpcError<TransportErrorKind>)
//        -> ErrorResp(ErrorPayload { code, message, data })
//        -> Transport(TransportErrorKind)
//   -> PendingTransactionError
//        -> TransportError(TransportError)
//        -> TxWatcher(WatchTxError::Timeout)
//   -> AbiError
//   -> ZeroData
//
// For contract reverts, alloy::sol! generates types implementing SolError.
// ErrorPayload::as_decoded_error::<T>() checks the selector and decodes.
// ErrorPayload::as_decoded_interface_error::<T>() does the same for a
// SolInterface (union of multiple SolError types).
//
// For -32000 server errors (nonce/funds/gas), the Ethereum JSON-RPC spec
// uses a single error code. The distinction is in ErrorPayload.message --
// the structured JSON-RPC message field, not a stringified Rust error.

/// Classifies an `alloy_contract::Error` into our `EthError`.
pub fn classify_contract_call_error(e: alloy_contract::Error) -> EthError {
    match e {
        alloy_contract::Error::TransportError(transport_err) => {
            classify_transport_error(transport_err)
        }
        alloy_contract::Error::PendingTransactionError(pending_err) => {
            classify_pending_tx_error(pending_err)
        }
        alloy_contract::Error::ZeroData(fn_name, abi_err) => {
            // Contract returned no data -- likely a revert with no reason.
            EthError::Reverted(ContractRevert::Unknown(format!(
                "zero data from {fn_name}: {abi_err}"
            )))
        }
        alloy_contract::Error::AbiError(abi_err) => {
            // Encoding/decoding bug: wrong types or stale ABI. Never transient.
            EthError::AbiError(format!("{abi_err}"))
        }
        other => EthError::UnexpectedRpcResponse(other.to_string()),
    }
}

/// Classifies an alloy `TransportError` (= `RpcError<TransportErrorKind>`).
pub fn classify_transport_error(
    e: alloy::transports::TransportError,
) -> EthError {
    use alloy::transports::RpcError;

    match e {
        RpcError::ErrorResp(payload) => classify_error_payload(payload),
        RpcError::Transport(kind) => {
            // All transport-level errors (connection refused, timeout, HTTP 502,
            // TLS failure, backend gone) are RpcUnreachable.
            EthError::RpcUnreachable(kind.to_string())
        }
        RpcError::NullResp => EthError::UnexpectedRpcResponse("null response".into()),
        other => EthError::UnexpectedRpcResponse(other.to_string()),
    }
}

/// Classifies a JSON-RPC `ErrorPayload`.
///
/// Checks for contract reverts first (via revert data in the payload),
/// then rate limiting, then -32000 server errors for nonce/funds/gas.
pub fn classify_error_payload(
    payload: alloy::rpc::json_rpc::ErrorPayload,
) -> EthError {
    // First: check for contract revert data in the payload.
    // This covers eth_call simulation failures and eth_estimateGas reverts
    // where the node includes revert data in the error response.
    if let Some(revert) = classify_revert_from_payload(&payload) {
        return EthError::Reverted(revert);
    }

    // Second: rate limiting.
    if payload.is_retry_err() {
        return EthError::RpcUnreachable(format!("rate limited: {}", payload.message));
    }

    // Third: -32000 server errors. The Ethereum JSON-RPC spec uses -32000
    // as a catch-all for execution-layer rejections. The node distinguishes
    // them only via the message field. This is matching on the structured
    // ErrorPayload.message -- the JSON-RPC error.message from the node
    // response -- not on e.to_string().
    if payload.code == -32000 {
        let msg = payload.message.to_lowercase();
        if msg.contains("nonce too low") || msg.contains("replacement transaction underpriced") {
            return EthError::NonceCollision(payload.message.into_owned());
        }
        if msg.contains("insufficient funds") {
            return EthError::InsufficientFunds(payload.message.into_owned());
        }
        if msg.contains("gas required exceeds allowance")
            || msg.contains("intrinsic gas too low")
        {
            return EthError::OutOfGas(payload.message.into_owned());
        }
    }

    EthError::UnexpectedRpcResponse(format!("JSON-RPC error {}: {}", payload.code, payload.message))
}

/// Attempts to decode contract revert data from an `ErrorPayload`.
///
/// Uses `ErrorPayload::as_decoded_error` with the `alloy::sol!` generated
/// types from eth.rs. Each sol! error type implements `SolError` with
/// automatic selector matching and ABI decoding.
///
/// For `require(condition, "message")` reverts, uses the built-in
/// `alloy_sol_types::sol_data::Revert` type (selector 0x08c379a0).
fn classify_revert_from_payload(
    payload: &alloy::rpc::json_rpc::ErrorPayload,
) -> Option<ContractRevert> {
    // The sol! macro in eth.rs generates these types. We reference them
    // by their Solidity names. Each has a SELECTOR const and abi_decode.
    //
    // alloy::sol! types used here are generated in core/src/eth.rs from
    // the contract ABI JSON files. ErrorPayload::as_decoded_error checks
    // the selector in the revert data and decodes if it matches.

    use crate::eth_2::{
        MinaStateSettlementExample, MinaAccountValidationExample, NoriTokenBridge,
    };

    // Custom errors from MinaStateSettlementExample
    if payload
        .as_decoded_error::<MinaStateSettlementExample::MinaProvingSystemIdIsNotValid>()
        .is_some()
    {
        return Some(ContractRevert::ProvingSystemIdInvalid);
    }
    if payload
        .as_decoded_error::<MinaStateSettlementExample::MinaNetworkIsWrong>()
        .is_some()
    {
        return Some(ContractRevert::NetworkMismatch);
    }
    if payload
        .as_decoded_error::<MinaStateSettlementExample::NewStateIsNotValid>()
        .is_some()
    {
        return Some(ContractRevert::NewStateInvalid);
    }
    if payload
        .as_decoded_error::<MinaStateSettlementExample::TipStateIsWrong>()
        .is_some()
    {
        return Some(ContractRevert::TipStateWrong);
    }
    // AccountIsNotValid is defined in the Solidity source but not in the ABI
    // JSON (it is only used internally when NoriTokenBridge calls into
    // MinaStateSettlement). We define it inline so sol! generates the type.
    if payload
        .as_decoded_error::<AccountIsNotValid>()
        .is_some()
    {
        return Some(ContractRevert::AccountIsNotValid);
    }

    // Custom errors from MinaAccountValidationExample
    if payload
        .as_decoded_error::<MinaAccountValidationExample::MinaAccountProvingSystemIdIsNotValid>()
        .is_some()
    {
        return Some(ContractRevert::AccountProvingSystemIdInvalid);
    }

    // Custom errors from NoriTokenBridge
    if payload
        .as_decoded_error::<NoriTokenBridge::AlignedContractsNotConfigured>()
        .is_some()
    {
        return Some(ContractRevert::ContractsNotConfigured);
    }
    if payload
        .as_decoded_error::<NoriTokenBridge::ZeroAddress>()
        .is_some()
    {
        return Some(ContractRevert::ZeroAddress);
    }
    if payload
        .as_decoded_error::<NoriTokenBridge::NotBridgeOperator>()
        .is_some()
    {
        return Some(ContractRevert::NotBridgeOperator);
    }

    // require(condition, "message") -- standard Revert(string)
    if let Some(revert) = payload.as_decoded_error::<alloy_sol_types::Revert>() {
        return Some(match revert.reason.as_str() {
            "Invalid Ledger" => ContractRevert::InvalidLedger,
            "Invalid Zkapp Account" => ContractRevert::InvalidAccount,
            "Burn so far is greater than the amount to burn" => {
                ContractRevert::BurnAlreadyProcessed
            }
            "To unlock amount is greater than the amount to unlock" => {
                ContractRevert::UnlockAmountExceeded
            }
            "You must send some Ether to lock" => ContractRevert::ZeroLockAmount,
            "Must be multiple of smallest bridge unit" => ContractRevert::NotBridgeUnit,
            "Total locked exceeds maximum allowed" => ContractRevert::MaxMagnitudeExceeded,
            "This Mina account is already linked to a different ETH address" => {
                ContractRevert::AccountAlreadyLinked
            }
            "No ETH to withdraw" => ContractRevert::NothingToWithdraw,
            other => ContractRevert::Unknown(other.to_string()),
        });
    }

    // Revert data present but not decodable as any known type
    payload
        .as_revert_data()
        .map(|bytes| ContractRevert::Unknown(alloy::hex::encode(&bytes)))
}

/// Classifies a `PendingTransactionError` (returned when awaiting tx confirmation).
fn classify_pending_tx_error(
    e: alloy::providers::PendingTransactionError,
) -> EthError {
    use alloy::providers::PendingTransactionError;

    match e {
        PendingTransactionError::TransportError(transport_err) => {
            classify_transport_error(transport_err)
        }
        PendingTransactionError::TxWatcher(watch_err) => {
            // WatchTxError has one variant: Timeout. get_receipt().await uses
            // the tx watcher internally. Legacy code (eth.rs, nori.rs) hits
            // this path. New code (eth_2.rs) polls with get_tx_receipt instead.
            // For legacy callers this is a genuine confirmation timeout, but
            // since they will be replaced, map to UnexpectedRpcResponse to
            // surface it rather than silently retry.
            EthError::UnexpectedRpcResponse(format!("tx watcher timeout: {watch_err}"))
        }
        other => EthError::UnexpectedRpcResponse(other.to_string()),
    }
}

/// Checks a failed transaction receipt for out-of-gas.
///
/// Call this when `receipt.status() == false`. Revert data is not available
/// on the receipt itself -- contract reverts should be caught by eth_call
/// simulation before sending.
pub fn classify_receipt_failure(gas_used: u64, gas_limit: u64) -> EthError {
    if gas_used == gas_limit {
        EthError::OutOfGas(format!("execution consumed all gas ({gas_limit})"))
    } else {
        // Receipt failed but not out-of-gas. Revert reason should have been
        // caught by eth_call simulation before send.
        EthError::Reverted(ContractRevert::Unknown(
            "receipt failed, no revert data (simulate with eth_call before send)".into(),
        ))
    }
}
