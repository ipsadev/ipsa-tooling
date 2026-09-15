pub mod proto {
    pub mod stellar {
        #![allow(dead_code, clippy::doc_overindented_list_items)]

        tonic::include_proto!("stellar.gateway.v1");
    }

    pub mod proofapi {
        #![allow(dead_code, clippy::doc_overindented_list_items)]

        tonic::include_proto!("proofapi");
    }
}

mod trust_root;

use std::collections::HashMap;

use anyhow::{anyhow, Context, Result};
use proto::{
    proofapi::{proof_api_service_client::ProofApiServiceClient, CreateClientRequest},
    stellar::{
        stellar_gateway_query_client::StellarGatewayQueryClient, QueryStellarClientParamsRequest,
        QueryStellarClientParamsResponse, QuorumConfig,
    },
};
use tonic::transport::Channel;
pub use trust_root::verify_quorum_fingerprints;

pub struct Gateway {
    endpoint: String,
}

impl Gateway {
    pub fn new(address: &str) -> Self {
        Self {
            endpoint: with_scheme(address),
        }
    }

    async fn channel(&self) -> Result<Channel> {
        Channel::from_shared(self.endpoint.clone())
            .with_context(|| format!("{} is not a usable gateway address", self.endpoint))?
            .connect()
            .await
            .with_context(|| {
                format!(
                    "could not reach the gateway at {} — start it with `ipsa services up --gateway`",
                    self.endpoint
                )
            })
    }

    pub async fn client_params(&self, height: u64) -> Result<QueryStellarClientParamsResponse> {
        let mut client = StellarGatewayQueryClient::new(self.channel().await?);

        let response = client
            .query_stellar_client_params(QueryStellarClientParamsRequest { height })
            .await
            .map_err(|status| anyhow!("gateway QueryStellarClientParams failed: {status}"))?;

        Ok(response.into_inner())
    }

    pub async fn create_tendermint_client_on_stellar(
        &self,
        src_chain: &str,
        dst_chain: &str,
        parameters: HashMap<String, String>,
    ) -> Result<Vec<u8>> {
        let mut client = ProofApiServiceClient::new(self.channel().await?);

        let response = client
            .create_client(CreateClientRequest {
                src_chain: src_chain.to_string(),
                dst_chain: dst_chain.to_string(),
                parameters,
            })
            .await
            .map_err(|status| anyhow!("gateway CreateClient failed: {status}"))?;

        Ok(response.into_inner().tx)
    }

    pub async fn create_stellar_client_on_cosmos(
        &self,
        src_chain: &str,
        dst_chain: &str,
        parameters: HashMap<String, String>,
    ) -> Result<Vec<u8>> {
        let mut client = ProofApiServiceClient::new(self.channel().await?);

        let response = client
            .create_client(CreateClientRequest {
                src_chain: src_chain.to_string(),
                dst_chain: dst_chain.to_string(),
                parameters,
            })
            .await
            .map_err(|status| anyhow!("gateway CreateClient failed: {status}"))?;

        Ok(response.into_inner().tx)
    }
}

pub fn quorum_configs_json(configs: &[QuorumConfig]) -> String {
    let entries: Vec<serde_json::Value> = configs
        .iter()
        .map(|config| {
            serde_json::json!({
                "quorum_set_xdr": hex::encode(&config.quorum_set_xdr),
                "valid_from": config.valid_from,
            })
        })
        .collect();

    serde_json::Value::Array(entries).to_string()
}

fn with_scheme(address: &str) -> String {
    let trimmed = address.trim().trim_end_matches('/');

    if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
        return trimmed.to_string();
    }

    format!("http://{trimmed}")
}

#[cfg(test)]
mod tests {
    use super::with_scheme;

    #[test]
    fn a_bare_host_port_gets_a_scheme() {
        assert_eq!(with_scheme("127.0.0.1:50052"), "http://127.0.0.1:50052");
        assert_eq!(with_scheme(" gateway:50052/ "), "http://gateway:50052");
    }

    #[test]
    fn an_explicit_scheme_is_left_alone() {
        assert_eq!(with_scheme("https://g:50052"), "https://g:50052");
    }
}
