use super::error::Error;
use crate::mina_archive::{detect_nori_burn, BurnEvent};
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
}
