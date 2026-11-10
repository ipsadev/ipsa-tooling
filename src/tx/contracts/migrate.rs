use std::path::Path;

use anyhow::{bail, Result};

use crate::{
    config::{get, Config},
    logger, shared,
    tx::contracts::{config::ContractsConfig, deploy_one, invoke_as, read},
};

pub fn run(cfg: &Config, root: &Path, client: Option<String>) -> Result<()> {
    logger::banner("tx contracts migrate-client (move a Tendermint client to a new light client)");

    let contracts_config = ContractsConfig::from(cfg);
    let router = contracts_config.ibc_router.clone();
    let owner = cfg.stellar.cli_identity.clone();

    if router.is_empty() {
        bail!("ROUTER_CONTRACT_ADDRESS is not set");
    }

    let client_id = match client {
        Some(client_id) => client_id,
        None => get(cfg.cosmos.client_id_variable(), ""),
    };

    if client_id.is_empty() {
        bail!(
            "no client to migrate: pass --client or set {}",
            cfg.cosmos.client_id_variable()
        );
    }

    let view = |id: &str, call: &[&str]| -> Result<String> {
        Ok(unquote(&read(&contracts_config, root, id, call, &owner)?))
    };

    let current = view(
        &router,
        &["light_client_address", "--client_id", &client_id],
    )?;
    let height = view(&current, &["latest_height", "--client_id", &client_id])?;
    let client_state = view(&current, &["client_state", "--client_id", &client_id])?;
    let consensus_state = view(
        &current,
        &[
            "consensus_state",
            "--client_id",
            &client_id,
            "--height",
            &height,
        ],
    )?;

    logger::detail(&format!("{client_id} is on {current} at height {height}"));

    let replacement = deploy_one(&contracts_config, root, "stellar_tendermint_light_client")?;

    logger::step(&format!(
        "router migrate_client {client_id} -> {replacement}"
    ));
    invoke_as(
        &contracts_config,
        root,
        &router,
        &[
            "migrate_client",
            "--client_id",
            &client_id,
            "--light_client_address",
            &replacement,
            "--client_state",
            &client_state,
            "--consensus_state",
            &consensus_state,
            "--height",
            &height,
        ],
        &owner,
    )?;

    let moved = view(
        &router,
        &["light_client_address", "--client_id", &client_id],
    )?;

    if moved != replacement {
        bail!("the router still points {client_id} at {moved}, not {replacement}");
    }

    shared::env_upsert(
        &crate::config::tooling_env(),
        &[("TENDERMINT_LC_CONTRACT_ID", replacement.as_str())],
    )?;

    logger::ok(&format!(
        "{client_id} now runs on {replacement}, initialised at height {height}"
    ));

    Ok(())
}

fn unquote(value: &str) -> String {
    value.trim().trim_matches('"').to_string()
}
