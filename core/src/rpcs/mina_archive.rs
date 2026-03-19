use crate::error::Error;
use crate::mina_archive::{detect_nori_burn, query_canonical_block_at_height, BurnEvent};
use reqwest::Url;
use std::env;
use super::errors::MinaArchiveError;

pub struct MinaArchiveRPC {
    rpc_url: Url,
}

impl MinaArchiveRPC {
    pub fn from_env() -> Result<Self, Error> {
        let rpc_url = env::var("MINA_ARCHIVE_RPC_URL")
            .map_err(|e| Error(format!("MINA_ARCHIVE_RPC_URL: {e}")))?
            .trim()
            .parse::<Url>()
            .map_err(|e| Error(format!("invalid MINA_ARCHIVE_RPC_URL: {e}")))?;
        Ok(Self { rpc_url })
    }

    pub async fn detect_nori_burn(
        &self,
        contract_addr: &str,
        from_height: u64,
    ) -> Result<Vec<BurnEvent>, MinaArchiveError> {
        detect_nori_burn(self.rpc_url.as_str(), contract_addr, from_height)
            .await
    }

    /// Returns the state hash of the canonical block at `height`, or `None` if the archive
    /// node has no canonical block recorded at that height.
    #[deprecated(note = "The archive node only assigns canonical status to blocks that have \
        fallen off the daemon's 290-block transition frontier (MINA_DAEMON_MAX_QUERYABLE_BLOCKS). \
        The daemon can only generate proofs for blocks within that same 290-block window. This \
        creates a deadlock: by the time the archive marks a block canonical, the daemon can no \
        longer prove it. Burns classified via this fallback will be stuck unprovable.")]
    pub async fn query_canonical_block_at_height(
        &self,
        height: u64,
    ) -> Result<Option<String>, MinaArchiveError> {
        query_canonical_block_at_height(self.rpc_url.as_str(), height)
            .await
    }
}
