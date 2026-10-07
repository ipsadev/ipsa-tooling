use std::{env, path::Path};

use crate::services::{
    cosmos::config::CosmosConfig, gateway::config::GatewayConfig, relayer::config::RelayerConfig,
    stellar::config::StellarConfig,
};

pub enum ChainId {
    Cosmos(String),
    #[allow(dead_code)]
    Cardano(String),
    Stellar(String),
}

impl ChainId {
    pub fn as_str(&self) -> &str {
        match self {
            Self::Cosmos(id) | Self::Cardano(id) | Self::Stellar(id) => id,
        }
    }
}

pub enum ClientId {
    Cosmos(String),
    #[allow(dead_code)]
    Cardano(String),
    Stellar(String),
}

impl ClientId {
    pub fn as_str(&self) -> &str {
        match self {
            Self::Cosmos(id) | Self::Cardano(id) | Self::Stellar(id) => id,
        }
    }
}

pub enum ClientTypes {
    Tendermint,
    Attestation,
    Mock,
}

impl ClientTypes {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Attestation => "attestation",
            Self::Tendermint => "07-tendermint",
            Self::Mock => "mock",
        }
    }

    pub fn attestation() -> &'static str {
        Self::Attestation.as_str()
    }

    pub fn mock() -> &'static str {
        Self::Mock.as_str()
    }

    pub fn tendermint() -> &'static str {
        Self::Tendermint.as_str()
    }
}

pub struct ImageRef {
    pub image: String,
    pub tag: String,
    pub registry: String,
}

impl ImageRef {
    pub fn reference(&self) -> String {
        if self.registry.is_empty() {
            return format!("{}:{}", self.image, self.tag);
        }

        format!("{}/{}:{}", self.registry, self.image, self.tag)
    }
}

pub struct DeploymentConfig {
    pub ibc_router: String,
    pub transfer_app: String,
    pub gmp_app: String,
    pub deployer_address: String,
    pub transfer_port: String,
    pub mock_client_type: String,
    pub attestation_client_type: String,
    pub tendermint_client_type: String,
    pub cosmos_client_id: String,
    pub stellar_client_id: String,
}

impl DeploymentConfig {
    pub fn cosmos_client(&self) -> Option<ClientId> {
        non_empty(&self.cosmos_client_id).map(ClientId::Cosmos)
    }

    pub fn stellar_client(&self) -> Option<ClientId> {
        non_empty(&self.stellar_client_id).map(ClientId::Stellar)
    }
}

pub struct AccountsConfig {
    pub stellar_sender_identity: String,
    pub stellar_sender_address: String,
    pub stellar_receiver_identity: String,
    pub stellar_receiver_address: String,
    pub cosmos_sender_key_name: String,
    pub cosmos_sender_address: String,
    pub cosmos_receiver_key_name: String,
    pub cosmos_receiver_address: String,
}

impl AccountsConfig {
    pub fn from_env() -> Self {
        Self {
            stellar_sender_identity: get("STELLAR_SENDER_IDENTITY", "stellar-sender"),
            stellar_sender_address: get("STELLAR_SENDER_ADDRESS", ""),
            stellar_receiver_identity: get("STELLAR_RECEIVER_IDENTITY", "stellar-receiver"),
            stellar_receiver_address: get("STELLAR_RECEIVER_ADDRESS", ""),
            cosmos_sender_key_name: get("COSMOS_SENDER_KEY_NAME", "cosmos-sender"),
            cosmos_sender_address: get("COSMOS_SENDER_ADDRESS", ""),
            cosmos_receiver_key_name: get("COSMOS_RECEIVER_KEY_NAME", "cosmos-receiver"),
            cosmos_receiver_address: get("COSMOS_RECEIVER_ADDRESS", ""),
        }
    }
}

pub struct Config {
    pub cosmos: CosmosConfig,
    pub stellar: StellarConfig,
    pub relayer: RelayerConfig,
    pub verifier: crate::services::verifier::VerifierConfig,
    pub api: ImageRef,
    pub gateway: GatewayConfig,
    pub deployment: DeploymentConfig,
    pub accounts: AccountsConfig,
    pub evm: crate::tx::evm::config::EvmCorridorConfig,
}

pub fn get(key: &str, default: &str) -> String {
    match env::var(key) {
        Ok(value) => value
            .trim()
            .trim_matches(|c| c == '"' || c == '\'')
            .to_string(),
        Err(_) => default.to_string(),
    }
}

impl Config {
    pub fn load(root: &Path) -> Self {
        let _ = dotenvy::from_path_override(root.join(".env"));

        Self {
            cosmos: CosmosConfig::devnet(),
            stellar: StellarConfig::from_env(),
            relayer: RelayerConfig::from_env(root),
            verifier: crate::services::verifier::VerifierConfig::from_env(),
            api: ImageRef {
                image: get("API_IMAGE", "amandagonsalvesx/ipsa-api"),
                tag: get("API_TAG", "latest"),
                registry: get("API_REGISTRY", ""),
            },
            gateway: GatewayConfig::from_env(),
            deployment: DeploymentConfig {
                ibc_router: get("ROUTER_CONTRACT_ADDRESS", ""),
                transfer_app: get("TRANSFER_CONTRACT_ADDRESS", ""),
                gmp_app: get("GMP_CONTRACT_ADDRESS", ""),
                deployer_address: get("DEPLOYER_ADDRESS", ""),
                transfer_port: get("TRANSFER_PORT", "transfer"),
                mock_client_type: get("MOCK_CLIENT_TYPE", ClientTypes::mock()),
                attestation_client_type: get("ATTESTATION_CLIENT_TYPE", ClientTypes::attestation()),
                tendermint_client_type: get("TENDERMINT_CLIENT_TYPE", ClientTypes::tendermint()),
                cosmos_client_id: get("COSMOS_CLIENT_ID", ""),
                stellar_client_id: get("STELLAR_CLIENT_ID", ""),
            },
            accounts: AccountsConfig::from_env(),
            evm: crate::tx::evm::config::EvmCorridorConfig::from_env(),
        }
    }

    pub fn api_health_url(&self) -> String {
        format!("{}/health", self.stellar.api_url)
    }
}

fn non_empty(value: &str) -> Option<String> {
    if value.is_empty() {
        return None;
    }

    Some(value.to_string())
}
