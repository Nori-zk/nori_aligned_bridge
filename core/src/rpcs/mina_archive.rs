use super::error::Error;
use crate::mina_archive::{detect_nori_burn, query_canonical_block_at_height, BurnEvent};
use reqwest::Url;
use std::env;

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
        from_height: u32,
    ) -> Result<Vec<BurnEvent>, Error> {
        detect_nori_burn(self.rpc_url.as_str(), contract_addr, from_height)
            .await
            .map_err(Error)
    }

    /// Returns the state hash of the canonical block at `height`, or `None` if the archive
    /// node has no canonical block recorded at that height.
    pub async fn query_canonical_block_at_height(
        &self,
        height: u32,
    ) -> Result<Option<String>, Error> {
        query_canonical_block_at_height(self.rpc_url.as_str(), height)
            .await
            .map_err(Error)
    }
}
