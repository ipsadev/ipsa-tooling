use crate::config::{get, ChainId};

pub struct StellarConfig {
    pub chain_id: ChainId,
    pub signing_key: String,
    pub rpc_url: String,
    pub network_passphrase: String,
    pub cli_identity: String,
    pub key_name: String,
    pub gateway_url: String,
    pub api_url: String,
    pub ibc_router: String,
    pub transfer_app: String,
    pub pinned_quorum_set_hashes: Vec<String>,
}

impl StellarConfig {
    pub fn from_env() -> Self {
        let api_port = get("STELLAR_API_PORT", "8101");
        let grpc_port = get("STELLAR_GATEWAY_GRPC_PORT", "50052");

        let chain_id = get("STELLAR_CHAIN_ID", "stellar-testnet");

        Self {
            chain_id: ChainId::Stellar(chain_id),
            signing_key: get("STELLAR_SIGNING_KEY", ""),
            rpc_url: get("STELLAR_RPC_URL", "https://soroban-testnet.stellar.org"),
            network_passphrase: get("NETWORK_PASSPHRASE", "Test SDF Network ; September 2015"),
            cli_identity: get("DEPLOYER_IDENTITY", "admin"),
            key_name: get("STELLAR_KEY_NAME", "stellar-relayer"),
            gateway_url: get("STELLAR_GATEWAY_URL", &format!("127.0.0.1:{grpc_port}")),
            api_url: get("STELLAR_API_URL", &format!("http://127.0.0.1:{api_port}")),
            ibc_router: get("STELLAR_ROUTER_CONTRACT", ""),
            transfer_app: get("STELLAR_TRANSFER_CONTRACT", ""),
            pinned_quorum_set_hashes: split_list(&get("PINNED_QUORUM_SET_HASHES", "")),
        }
    }
}

fn split_list(value: &str) -> Vec<String> {
    value
        .split(',')
        .map(str::trim)
        .filter(|entry| !entry.is_empty())
        .map(str::to_string)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::split_list;

    #[test]
    fn an_unset_list_is_empty_not_a_blank_entry() {
        assert!(split_list("").is_empty());
        assert!(split_list("  ,  ").is_empty());
    }

    #[test]
    fn entries_are_split_and_trimmed() {
        assert_eq!(
            split_list(" a , b "),
            vec!["a".to_string(), "b".to_string()]
        );
    }
}
