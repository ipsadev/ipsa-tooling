use crate::config::{ClientId, Config};

pub struct ClientsConfig {
    pub ibc_router: String,
    pub cosmos_rpc_url: String,
    pub api_url: String,
    pub relayer_config: String,
    pub cosmos_client: Option<ClientId>,
    pub stellar_client: Option<ClientId>,
}

impl From<&Config> for ClientsConfig {
    fn from(cfg: &Config) -> Self {
        Self {
            ibc_router: cfg.deployment.ibc_router.clone(),
            cosmos_rpc_url: cfg.cosmos.rpc_url.clone(),
            api_url: cfg.stellar.api_url.clone(),
            relayer_config: cfg.relayer.config.clone(),
            cosmos_client: cfg.deployment.cosmos_client(),
            stellar_client: cfg.deployment.stellar_client(),
        }
    }
}

impl ClientsConfig {
    pub fn clients_url(&self) -> String {
        format!(
            "{}/stellar/clients?contract_id={}",
            self.api_url, self.ibc_router
        )
    }

    pub fn cosmos_status_url(&self) -> String {
        format!("{}/status", self.cosmos_rpc_url)
    }

    pub fn api_health_url(&self) -> String {
        format!("{}/health", self.api_url)
    }
}
