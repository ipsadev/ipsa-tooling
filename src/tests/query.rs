use std::path::Path;

use anyhow::{bail, Result};

use crate::{
    config::Config,
    logger, probe,
    tx::{
        clients::config::ClientsConfig,
        query::{self, QueryArgs},
    },
};

pub async fn run(cfg: &Config, root: &Path, http: &reqwest::Client) -> Result<()> {
    let cc = ClientsConfig::from(cfg);

    if !probe::http_ok(http, &cc.api_health_url()).await {
        bail!("api health check failed at {}", cc.api_health_url());
    }

    logger::ok("api /health ok");

    if probe::get_json(http, &cc.clients_url()).await.is_none() {
        bail!("could not read {}", cc.clients_url());
    }

    logger::ok("api /stellar/clients ok");

    if !probe::http_ok(http, &cc.cosmos_status_url()).await {
        bail!("cosmos status check failed at {}", cc.cosmos_status_url());
    }

    logger::ok("cosmos /status ok");

    let client_states = format!("{}/ibc/core/client/v1/client_states", cfg.cosmos.rest_url);
    match probe::get_json(http, &client_states).await {
        Some(value) if value.get("client_states").is_some() => {
            logger::ok("cosmos client_states ok")
        }
        Some(_) => bail!("unexpected client_states response shape from {client_states}"),
        _ => bail!("could not read {client_states}"),
    }

    query::run(
        cfg,
        root,
        http,
        QueryArgs {
            clients: Some(true),
            stellar: false,
            cosmos: false,
            client_id: None,
            address: None,
            denom: None,
        },
    )
    .await
}
