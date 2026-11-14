use std::{collections::HashMap, path::Path};

use anyhow::{anyhow, bail, Result};
use base64::Engine;

use crate::{
    config::Config,
    gateway::Gateway,
    logger, probe, tools,
    tx::{
        clients::{config::ClientsConfig, CreateSpec},
        contracts::config::ContractsConfig,
    },
};

const CLIENT_TYPE: &str = "07-tendermint";

pub async fn run(
    cfg: &Config,
    cc: &ClientsConfig,
    root: &Path,
    http: &reqwest::Client,
    force: bool,
) -> Result<String> {
    logger::banner("clients cosmos via the proof api (Cosmos client on Stellar, 07-tendermint)");

    if !force {
        if let Some(existing) = &cc.cosmos_client {
            logger::warn(&format!(
                "{} already set ({}). Use --force to create another.",
                cfg.cosmos.client_id_variable(),
                existing.as_str()
            ));

            return Ok(existing.as_str().to_string());
        }
    }

    if cfg.deployment.ibc_router.is_empty() {
        bail!("ROUTER_CONTRACT_ADDRESS is not set in .env — deploy the contracts with `ipsa tx contracts deploy --stellar`, which writes it");
    }

    logger::step("gateway CreateClient (cosmos → stellar)");
    logger::detail(
        "the proof api builds the tendermint genesis and prepares the router invocation; the \
         relayer key signs it",
    );

    let tx = Gateway::new(&cfg.stellar.gateway_url)
        .create_tendermint_client_on_stellar(
            cfg.cosmos.chain_id.as_str(),
            cfg.stellar.chain_id.as_str(),
            HashMap::new(),
        )
        .await?;

    if tx.is_empty() {
        bail!("the proof api returned an empty transaction for the tendermint client");
    }

    let unsigned = base64::engine::general_purpose::STANDARD.encode(&tx);
    let contracts = ContractsConfig::from(cfg);

    logger::step("signing the prepared transaction");
    let signed = sign(&contracts, root, &unsigned, &cfg.stellar.signing_key)?;

    logger::step("submitting to stellar");
    send(&contracts, root, &signed)?;

    let client_id = newest_client(http, cc)
        .await?
        .ok_or_else(|| anyhow!("the router lists no {CLIENT_TYPE} client after creation"))?;

    logger::ok(&format!("created: {client_id}"));

    super::record(
        cc,
        &CreateSpec {
            host_chain: cfg.stellar.chain_id.as_str(),
            reference_chain: cfg.cosmos.chain_id.as_str(),
            result_env_var: cfg.cosmos.client_id_variable(),
        },
        &client_id,
    )?;

    Ok(client_id)
}

fn sign(cfg: &ContractsConfig, root: &Path, envelope: &str, key: &str) -> Result<String> {
    let mut args = vec![
        "tx".to_string(),
        "sign".to_string(),
        envelope.to_string(),
        "--sign-with-key".to_string(),
        key.to_string(),
    ];
    args.extend(cfg.net_flags());

    let refs: Vec<&str> = args.iter().map(String::as_str).collect();

    let output = tools::stellar::capture(root, &refs)?;

    output
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .next_back()
        .map(str::to_string)
        .ok_or_else(|| anyhow!("stellar tx sign returned no envelope"))
}

fn send(cfg: &ContractsConfig, root: &Path, envelope: &str) -> Result<String> {
    let mut args = vec!["tx".to_string(), "send".to_string(), envelope.to_string()];
    args.extend(cfg.net_flags());

    let refs: Vec<&str> = args.iter().map(String::as_str).collect();

    tools::stellar::capture_all(root, &refs)
}

async fn newest_client(http: &reqwest::Client, cc: &ClientsConfig) -> Result<Option<String>> {
    let value = probe::get_json(http, &cc.clients_url())
        .await
        .ok_or_else(|| anyhow!("could not read {}", cc.clients_url()))?;

    let mut ids = value
        .get("clients")
        .and_then(|c| c.as_array())
        .ok_or_else(|| anyhow!("unexpected response shape from {}", cc.clients_url()))?
        .iter()
        .filter(|entry| entry.get("client_type").and_then(|t| t.as_str()) == Some(CLIENT_TYPE))
        .filter_map(|entry| entry.get("client_ids").and_then(|v| v.as_array()))
        .flatten()
        .filter_map(|v| v.as_str());

    Ok(ids.next_back().map(str::to_string))
}
