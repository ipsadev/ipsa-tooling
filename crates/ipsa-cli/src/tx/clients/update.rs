use std::{path::Path, time::Duration};

use anyhow::{bail, Result};

use crate::{
    config::{get, Config},
    gateway::Gateway,
    logger,
    tx::{contracts::config::ContractsConfig, evm::soroban},
};

pub async fn run(
    cfg: &Config,
    root: &Path,
    client: Option<String>,
    every: Option<u64>,
) -> Result<()> {
    let client_id = match client {
        Some(client_id) => client_id,
        None => get(cfg.cosmos.client_id_variable(), ""),
    };

    if client_id.is_empty() {
        bail!(
            "no client to update: pass --client or set {}",
            cfg.cosmos.client_id_variable()
        );
    }

    let Some(seconds) = every else {
        return update(cfg, root, &client_id).await;
    };

    logger::banner(&format!("keeping {client_id} updated every {seconds}s"));

    loop {
        if let Err(error) = update(cfg, root, &client_id).await {
            logger::warn(&format!("{error:#}"));
        }

        tokio::time::sleep(Duration::from_secs(seconds)).await;
    }
}

async fn update(cfg: &Config, root: &Path, client_id: &str) -> Result<()> {
    logger::banner(&format!(
        "update {client_id} on Stellar to the latest Cosmos height"
    ));

    let contracts_config = ContractsConfig::from(cfg);
    let router = contracts_config.ibc_router.clone();

    if router.is_empty() {
        bail!("ROUTER_CONTRACT_ADDRESS is not set");
    }

    let gateway = Gateway::new(&cfg.stellar.gateway_url);
    let (tx, address) = gateway
        .update_client(
            cfg.cosmos.chain_id.as_str(),
            &contracts_config.stellar_chain_id,
            client_id,
        )
        .await?;

    if address != router {
        bail!(
            "the gateway built the update for {address}, but ROUTER_CONTRACT_ADDRESS is {router}"
        );
    }

    let functions = soroban::decode(&tx)?;
    let router_id = soroban::contract_id(&router)?;

    for function in functions {
        let (target, name) = soroban::target_and_function(&function)?;

        if target != router_id || name != "update_client" {
            bail!("the gateway returned `{name}` on another contract, not `update_client` on ROUTER_CONTRACT_ADDRESS");
        }

        let hash = soroban::submit(root, &contracts_config, &cfg.stellar.cli_identity, function)?;

        logger::ok(&format!("{client_id} updated (tx {hash})"));
    }

    Ok(())
}
