use crate::config::{ClientTypes, Config};

pub struct ContractsConfig {
    pub rpc_url: String,
    pub network_passphrase: String,
    pub signing_key: String,
    pub deployer_address: String,
    pub ibc_router: String,
    pub transfer_port: String,
    pub mock_client_type: String,
    pub attestation_client_type: String,
    pub tendermint_client_type: String,
    pub relayer_config: String,
    pub stellar_chain_id: String,
}

impl From<&Config> for ContractsConfig {
    fn from(cfg: &Config) -> Self {
        Self {
            rpc_url: cfg.stellar.rpc_url.clone(),
            network_passphrase: cfg.stellar.network_passphrase.clone(),
            signing_key: cfg.stellar.signing_key.clone(),
            deployer_address: cfg.deployment.deployer_address.clone(),
            ibc_router: cfg.deployment.ibc_router.clone(),
            transfer_port: cfg.deployment.transfer_port.clone(),
            mock_client_type: cfg.deployment.mock_client_type.clone(),
            attestation_client_type: cfg.deployment.attestation_client_type.clone(),
            tendermint_client_type: cfg.deployment.tendermint_client_type.clone(),
            relayer_config: cfg.relayer.config.clone(),
            stellar_chain_id: cfg.stellar.chain_id.as_str().to_string(),
        }
    }
}

impl ContractsConfig {
    pub fn net_flags(&self) -> Vec<String> {
        vec![
            "--rpc-url".to_string(),
            self.rpc_url.clone(),
            "--network-passphrase".to_string(),
            self.network_passphrase.clone(),
        ]
    }

    pub fn client_type(&self, kind: ClientTypes) -> &str {
        match kind {
            ClientTypes::Mock => &self.mock_client_type,
            ClientTypes::Attestation => &self.attestation_client_type,
            ClientTypes::Tendermint => &self.tendermint_client_type,
        }
    }
}
