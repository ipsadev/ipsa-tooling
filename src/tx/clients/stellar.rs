use std::{collections::HashMap, time::Duration};

use anyhow::{anyhow, bail, Result};
use cosmrs::{proto::cosmos::tx::v1beta1::TxBody, Any};
use prost13::Message;
use serde_json::Value;

use crate::{
    config::Config,
    gateway::{quorum_configs_json, verify_quorum_fingerprints, Gateway},
    logger,
    services::cosmos::tx::CosmosSigner,
    tx::clients::{config::ClientsConfig, CreateSpec},
};

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
    use serde_json::json;

    use super::client_id_from;

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
