use std::{collections::HashMap, time::Duration};

use anyhow::{anyhow, bail, Result};
use cosmrs::{proto::cosmos::tx::v1beta1::TxBody, Any};
use ibc_proto::ibc::{
    core::client::v1::MsgCreateClient, lightclients::wasm::v1::ClientState as WasmClientState,
};
use ipsa::gateway::stellar::StellarClientState;
use prost13::Message;
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::{
    config::Config,
    gateway::{quorum_configs_json, verify_quorum_fingerprints, Gateway},
    logger,
    services::cosmos::tx::CosmosSigner,
    tx::clients::{config::ClientsConfig, CreateSpec},
};

const ROOT_EVENT_TOPIC: &[u8] = b"ibc_root";

const GAS_LIMIT: u64 = 2_000_000;

const WAIT: Duration = Duration::from_secs(90);

pub async fn run(
    cfg: &Config,
    cc: &ClientsConfig,
    http: &reqwest::Client,
    force: bool,
) -> Result<String> {
    logger::banner("clients stellar via the gateway (Stellar client on Cosmos, 08-wasm)");

    if !force {
        if let Some(existing) = &cc.stellar_client {
            logger::warn(&format!(
                "{} already set ({}). Use --force to create another.",
                cfg.cosmos.stellar_client_id_variable(),
                existing.as_str()
            ));

            return Ok(existing.as_str().to_string());
        }
    }

    let checksum = wasm_checksum(cfg)?;
    let gateway = Gateway::new(&cfg.stellar.gateway_url);

    logger::step("querying the gateway for the stellar client params");
    let params = gateway.client_params(0).await?;

    logger::step(&format!(
        "checking {} quorum config(s) against the pinned fingerprints",
        params.quorum_configs.len()
    ));
    verify_quorum_fingerprints(
        &params.quorum_configs,
        &cfg.stellar.pinned_quorum_set_hashes,
    )?;
    logger::ok("quorum sets match the pinned fingerprints");

    let parameters = HashMap::from([
        (
            "quorum_configs".to_string(),
            quorum_configs_json(&params.quorum_configs),
        ),
        ("wasm_checksum".to_string(), checksum),
    ]);

    logger::step("gateway CreateClient (stellar → cosmos)");
    let tx = gateway
        .create_stellar_client_on_cosmos(
            cfg.stellar.chain_id.as_str(),
            cfg.cosmos.chain_id.as_str(),
            parameters,
        )
        .await?;

    logger::step("verifying the gateway's create-client trust-root fields");
    verify_stellar_client_state(
        &tx,
        cfg.deployment.ibc_router.trim(),
        cfg.stellar.chain_id.as_str(),
    )?;
    logger::ok("router, network and root-event topic match the operator's configuration");

    let signer = CosmosSigner::from_config(&cfg.cosmos, http.clone())?;

    logger::step("broadcasting MsgCreateClient on Cosmos");
    let landed = signer
        .broadcast_as_relayer(
            single_message(&tx)?,
            "create-stellar-client",
            GAS_LIMIT,
            crate::services::cosmos::config::fee_for(GAS_LIMIT),
            WAIT,
        )
        .await?;

    let client_id = client_id_from(&landed)
        .ok_or_else(|| anyhow!("no client_id in the create_client events of the landed tx"))?;

    logger::ok(&format!("created: {client_id}"));

    super::record(
        cc,
        &CreateSpec {
            host_chain: cfg.cosmos.chain_id.as_str(),
            reference_chain: cfg.stellar.chain_id.as_str(),
            result_env_var: cfg.cosmos.stellar_client_id_variable(),
        },
        &client_id,
    )?;

    Ok(client_id)
}

fn wasm_checksum(cfg: &Config) -> Result<String> {
    let configured = crate::config::get("LIGHT_CLIENT_WASM_CHECKSUM", "");

    if !configured.trim().is_empty() {
        return Ok(configured.trim().to_string());
    }

    bail!(
        "LIGHT_CLIENT_WASM_CHECKSUM is unset — upload the light client first with \
         `ipsa tx contracts deploy --cosmos`, which writes it to .env (chain {})",
        cfg.cosmos.chain_id.as_str()
    )
}

fn single_message(tx: &[u8]) -> Result<Any> {
    let body = TxBody::decode(tx)
        .map_err(|error| anyhow!("the gateway returned a tx body that will not decode: {error}"))?;

    if body.messages.len() != 1 {
        bail!(
            "expected one message in the gateway's create-client tx, got {}",
            body.messages.len()
        );
    }

    let message = &body.messages[0];

    Ok(Any {
        type_url: message.type_url.clone(),
        value: message.value.clone(),
    })
}

fn expected_network_id(chain_id: &str) -> [u8; 32] {
    let id = chain_id.to_ascii_lowercase();

    let passphrase = if id.contains("mainnet") || id.contains("public") || id.contains("pubnet") {
        "Public Global Stellar Network ; September 2015"
    } else {
        "Test SDF Network ; September 2015"
    };

    Sha256::digest(passphrase.as_bytes()).into()
}

fn verify_stellar_client_state(tx: &[u8], router: &str, chain_id: &str) -> Result<()> {
    let message = single_message(tx)?;

    let create = MsgCreateClient::decode(message.value.as_slice())
        .map_err(|error| anyhow!("the gateway's create-client message will not decode: {error}"))?;

    let client_state = create
        .client_state
        .ok_or_else(|| anyhow!("the gateway's create-client message carries no client state"))?;

    let wasm = WasmClientState::decode(client_state.value.as_slice()).map_err(|error| {
        anyhow!("the create-client client state is not a wasm client state: {error}")
    })?;

    let stellar =
        <StellarClientState as prost::Message>::decode(wasm.data.as_slice()).map_err(|error| {
            anyhow!("the wasm client state does not wrap a stellar client state: {error}")
        })?;

    let expected_router = stellar_strkey::Contract::from_string(router).map_err(|error| {
        anyhow!("ROUTER_CONTRACT_ADDRESS {router:?} is not a contract strkey: {error}")
    })?;

    if stellar.router_contract_id.as_slice() != expected_router.0.as_slice() {
        bail!(
            "refusing to create the client: the gateway's router_contract_id {} is not the \
             configured router {router}",
            hex::encode(&stellar.router_contract_id),
        );
    }

    if stellar.root_event_topic.as_slice() != ROOT_EVENT_TOPIC {
        bail!(
            "refusing to create the client: the gateway's root_event_topic {:?} is not \"ibc_root\"",
            String::from_utf8_lossy(&stellar.root_event_topic),
        );
    }

    let expected_network = expected_network_id(chain_id);

    if stellar.network_id.as_slice() != expected_network.as_slice() {
        bail!(
            "refusing to create the client: the gateway's network_id {} is not the expected \
             network for chain {chain_id}",
            hex::encode(&stellar.network_id),
        );
    }

    Ok(())
}

fn client_id_from(landed: &Value) -> Option<String> {
    let events = landed.pointer("/tx_response/events")?.as_array()?;

    for event in events {
        if event.get("type").and_then(Value::as_str) != Some("create_client") {
            continue;
        }

        let attributes = event.get("attributes")?.as_array()?;

        for attribute in attributes {
            if attribute.get("key").and_then(Value::as_str) == Some("client_id") {
                return attribute
                    .get("value")
                    .and_then(Value::as_str)
                    .map(str::to_string);
            }
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use ibc_proto::google::protobuf::Any as IbcAny;
    use serde_json::json;

    use super::*;

    fn create_client_tx(router: [u8; 32], topic: &[u8], network_id: [u8; 32]) -> Vec<u8> {
        let stellar = StellarClientState {
            chain_id: "stellar-testnet".to_string(),
            latest_height: None,
            frozen_height: None,
            quorum_configs: Vec::new(),
            proof_specs: Vec::new(),
            network_id: network_id.to_vec(),
            max_consensus_age: 0,
            router_contract_id: router.to_vec(),
            root_event_topic: topic.to_vec(),
        };

        let wasm = WasmClientState {
            data: prost::Message::encode_to_vec(&stellar),
            checksum: Vec::new(),
            latest_height: None,
        };

        let create = MsgCreateClient {
            client_state: Some(IbcAny {
                type_url: "/ibc.lightclients.wasm.v1.ClientState".to_string(),
                value: wasm.encode_to_vec(),
            }),
            consensus_state: None,
            signer: "relayer".to_string(),
        };

        let body = TxBody {
            messages: vec![Any {
                type_url: "/ibc.core.client.v1.MsgCreateClient".to_string(),
                value: create.encode_to_vec(),
            }],
            ..Default::default()
        };

        body.encode_to_vec()
    }

    #[test]
    fn a_matching_create_client_tx_is_accepted() {
        let router = [7u8; 32];
        let strkey = stellar_strkey::Contract(router).to_string();
        let network = expected_network_id("stellar-testnet");

        let tx = create_client_tx(router, b"ibc_root", network);

        verify_stellar_client_state(&tx, &strkey, "stellar-testnet").unwrap();
    }

    #[test]
    fn a_create_client_tx_for_another_router_is_refused() {
        let strkey = stellar_strkey::Contract([7u8; 32]).to_string();
        let network = expected_network_id("stellar-testnet");

        let tx = create_client_tx([9u8; 32], b"ibc_root", network);

        let error = verify_stellar_client_state(&tx, &strkey, "stellar-testnet").unwrap_err();

        assert!(error.to_string().contains("router_contract_id"));
    }

    #[test]
    fn a_create_client_tx_with_a_foreign_root_topic_is_refused() {
        let router = [7u8; 32];
        let strkey = stellar_strkey::Contract(router).to_string();
        let network = expected_network_id("stellar-testnet");

        let tx = create_client_tx(router, b"not_ibc_root", network);

        let error = verify_stellar_client_state(&tx, &strkey, "stellar-testnet").unwrap_err();

        assert!(error.to_string().contains("root_event_topic"));
    }

    #[test]
    fn the_client_id_comes_from_the_create_client_event() {
        let landed = json!({"tx_response": {"events": [
            {"type": "message", "attributes": [{"key": "client_id", "value": "wrong"}]},
            {"type": "create_client", "attributes": [
                {"key": "client_type", "value": "08-wasm"},
                {"key": "client_id", "value": "08-wasm-3"}
            ]}
        ]}});

        assert_eq!(client_id_from(&landed).unwrap(), "08-wasm-3");
    }

    #[test]
    fn a_tx_without_a_create_client_event_yields_nothing() {
        let landed = json!({"tx_response": {"events": [{"type": "message", "attributes": []}]}});

        assert!(client_id_from(&landed).is_none());
    }
}
