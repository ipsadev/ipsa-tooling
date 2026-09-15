use std::path::Path;

use anyhow::{anyhow, bail, Result};

use crate::{
    config::Config,
    logger, probe,
    tx::clients::{self, config::ClientsConfig},
};

pub async fn run(cfg: &Config, root: &Path, http: &reqwest::Client) -> Result<()> {
    if cfg.deployment.cosmos_client_id.is_empty() || cfg.deployment.stellar_client_id.is_empty() {
        bail!("client ids are empty — run the ics02-clients flow first");
    }

    let cc = ClientsConfig::from(cfg);

    clients::counterparty::run(cfg, &cc, root, http, "stellar").await?;
    clients::counterparty::run(cfg, &cc, root, http, "cosmos").await?;

    let listed = registered_clients(&cc, http).await?;
    if listed.is_empty() {
        bail!("the Stellar router lists no clients after counterparty registration");
    }

    logger::ok(&format!(
        "counterparties paired — Stellar router lists {}",
        listed.join(", ")
    ));

    Ok(())
}

async fn registered_clients(cc: &ClientsConfig, http: &reqwest::Client) -> Result<Vec<String>> {
    let value = probe::get_json(http, &cc.clients_url())
        .await
        .ok_or_else(|| anyhow!("could not read {}", cc.clients_url()))?;

    let clients = value
        .get("clients")
        .and_then(|c| c.as_array())
        .ok_or_else(|| anyhow!("unexpected response shape from {}", cc.clients_url()))?;

    let ids = clients
        .iter()
        .filter_map(|c| c.get("client_ids").and_then(|v| v.as_array()))
        .flatten()
        .filter_map(|v| v.as_str())
        .map(str::to_string)
        .collect();

    Ok(ids)
}
