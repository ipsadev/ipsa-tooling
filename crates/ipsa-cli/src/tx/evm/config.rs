use anyhow::{bail, Result};

use crate::config::get;

pub struct EvmCorridorConfig {
    pub rpc_url: String,
    pub private_key: String,
    pub ics26_router: String,
    pub ics20_transfer: String,
    pub evm_client_id: String,
    pub stellar_router: String,
    pub stellar_transfer: String,
    pub stellar_client_id: String,
    pub stellar_client_type: String,
    pub stellar_sender_identity: String,
    pub stellar_relayer_identity: String,
    pub proof_api_url: String,
    pub stellar_chain_id: String,
    pub evm_chain_id: String,
}

impl EvmCorridorConfig {
    pub fn from_env() -> Self {
        let stellar_router = get("ROUTER_CONTRACT_ADDRESS", "");
        let stellar_transfer = get("TRANSFER_CONTRACT_ADDRESS", "");
        let stellar_chain_id = get("STELLAR_CHAIN_ID", "stellar-testnet");

        Self {
            rpc_url: get("EVM_RPC_URL", ""),
            private_key: get("EVM_PRIVATE_KEY", ""),
            ics26_router: get("EVM_ICS26_ROUTER", ""),
            ics20_transfer: get("EVM_ICS20_TRANSFER", ""),
            evm_client_id: get("EVM_STELLAR_CLIENT_ID", "stellar-testnet-1"),
            stellar_router: get("STELLAR_EVM_ROUTER", &stellar_router),
            stellar_transfer: get("STELLAR_EVM_TRANSFER", &stellar_transfer),
            stellar_client_id: get("STELLAR_EVM_CLIENT_ID", "sepolia-0"),
            stellar_client_type: get("STELLAR_EVM_CLIENT_TYPE", "sepolia"),
            stellar_sender_identity: get("EVM_STELLAR_SENDER_IDENTITY", ""),
            stellar_relayer_identity: get("STELLAR_EVM_RELAYER_IDENTITY", ""),
            proof_api_url: get("PROOF_API_URL", "http://127.0.0.1:3100"),
            stellar_chain_id: get("STELLAR_EVM_CHAIN_ID", &stellar_chain_id),
            evm_chain_id: get("EVM_CHAIN_ID", "11155111"),
        }
    }
}

pub fn required<'a>(value: &'a str, variable: &str) -> Result<&'a str> {
    if value.is_empty() {
        bail!("{variable} is not set in .env — the Stellar ↔ EVM corridor needs it");
    }

    Ok(value)
}
