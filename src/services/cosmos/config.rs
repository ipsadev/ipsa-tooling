use crate::config::{get, ChainId};

pub const COMPOSE_SERVICE: &str = "cosmos";

const DEFAULT_DEVNET_CHAIN_ID: &str = "simd-1";
const DEFAULT_DEVNET_RPC_URL: &str = "http://127.0.0.1:26657";
const DEFAULT_DEVNET_REST_URL: &str = "http://127.0.0.1:1317";

const DEFAULT_TESTNET_CHAIN_ID: &str = "provider";
const DEFAULT_TESTNET_RPC_URL: &str = "https://rpc.provider-sentry-01.hub-testnet.polypore.xyz";
const DEFAULT_TESTNET_REST_URL: &str = "https://rest.provider-sentry-01.hub-testnet.polypore.xyz";
const DEFAULT_TESTNET_FAUCET_URL: &str = "https://faucet.polypore.xyz";

const DEFAULT_KEY_NAME: &str = "relayer";
const DEFAULT_ACCOUNT_PREFIX: &str = "cosmos";
const DEFAULT_GAS_DENOM: &str = "stake";
const DEFAULT_TESTNET_GAS_DENOM: &str = "uatom";

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum CosmosNetwork {
    Devnet,
    Testnet,
}

impl CosmosNetwork {
    pub fn is_local(self) -> bool {
        matches!(self, CosmosNetwork::Devnet)
    }
}

pub struct CosmosConfig {
    pub network: CosmosNetwork,
    pub chain_id: ChainId,
    pub rpc_url: String,
    pub rest_url: String,
    pub faucet_url: Option<String>,
    pub key_name: String,
    pub relayer_mnemonic: String,
    pub receiver_address: String,
    pub account_prefix: String,
    pub gas_denom: String,
    pub relayer_key_hex: String,
    pub validator_key_hex: String,
}

pub const DEFAULT_GAS_PRICE: &str = "0.025";

pub fn gas_price() -> f64 {
    get("COSMOS_GAS_PRICE", DEFAULT_GAS_PRICE)
        .parse()
        .unwrap_or(0.025)
}

pub fn fee_for(gas: u64) -> u128 {
    fee_at_price(gas, gas_price())
}

pub fn fee_at_price(gas: u64, price: f64) -> u128 {
    if !price.is_finite() || price <= 0.0 {
        return 0;
    }

    (gas as f64 * price).ceil() as u128
}

impl CosmosConfig {
    pub fn devnet() -> Self {
        Self {
            network: CosmosNetwork::Devnet,
            chain_id: ChainId::Cosmos(get("COSMOS_CHAIN_ID", DEFAULT_DEVNET_CHAIN_ID)),
            rpc_url: get("COSMOS_RPC_URL", DEFAULT_DEVNET_RPC_URL),
            rest_url: get("COSMOS_REST_URL", DEFAULT_DEVNET_REST_URL),
            faucet_url: None,
            key_name: get("COSMOS_KEY_NAME", DEFAULT_KEY_NAME),
            relayer_mnemonic: get("COSMOS_RELAYER_MNEMONIC", ""),
            receiver_address: get("COSMOS_RECEIVER_ADDRESS", ""),
            account_prefix: get("COSMOS_ACCOUNT_PREFIX", DEFAULT_ACCOUNT_PREFIX),
            gas_denom: get("COSMOS_GAS_DENOM", DEFAULT_GAS_DENOM),
            relayer_key_hex: get("COSMOS_RELAYER_PRIVATE_KEY", ""),
            validator_key_hex: get("COSMOS_VALIDATOR_PRIVATE_KEY", ""),
        }
    }

    pub fn testnet() -> Self {
        Self {
            network: CosmosNetwork::Testnet,
            chain_id: ChainId::Cosmos(get("COSMOS_TESTNET_CHAIN_ID", DEFAULT_TESTNET_CHAIN_ID)),
            rpc_url: get("COSMOS_TESTNET_RPC_URL", DEFAULT_TESTNET_RPC_URL),
            rest_url: get("COSMOS_TESTNET_REST_URL", DEFAULT_TESTNET_REST_URL),
            faucet_url: Some(get("COSMOS_TESTNET_FAUCET_URL", DEFAULT_TESTNET_FAUCET_URL)),
            key_name: get("COSMOS_KEY_NAME", DEFAULT_KEY_NAME),
            relayer_mnemonic: get("COSMOS_RELAYER_MNEMONIC", ""),
            receiver_address: get("COSMOS_TESTNET_RECEIVER_ADDRESS", ""),
            account_prefix: get("COSMOS_ACCOUNT_PREFIX", DEFAULT_ACCOUNT_PREFIX),
            gas_denom: get("COSMOS_TESTNET_GAS_DENOM", DEFAULT_TESTNET_GAS_DENOM),
            relayer_key_hex: get("COSMOS_RELAYER_PRIVATE_KEY", ""),
            validator_key_hex: get("COSMOS_VALIDATOR_PRIVATE_KEY", ""),
        }
    }

    pub fn is_local(&self) -> bool {
        self.network.is_local()
    }
}

#[cfg(test)]
mod tests {
    use super::{fee_at_price, DEFAULT_GAS_PRICE};

    #[test]
    fn the_default_price_covers_what_simd_demanded() {
        let price: f64 = DEFAULT_GAS_PRICE.parse().unwrap();

        assert_eq!(fee_at_price(2_000_000, price), 50_000);
        assert_eq!(fee_at_price(400_000, price), 10_000);
    }

    #[test]
    fn a_fee_is_rounded_up_so_it_is_never_short() {
        assert_eq!(fee_at_price(1, 0.025), 1);
        assert_eq!(fee_at_price(41, 0.025), 2);
    }

    #[test]
    fn a_zero_or_nonsense_price_asks_for_no_fee() {
        assert_eq!(fee_at_price(2_000_000, 0.0), 0);
        assert_eq!(fee_at_price(2_000_000, -1.0), 0);
        assert_eq!(fee_at_price(2_000_000, f64::NAN), 0);
    }
}
