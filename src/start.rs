use std::path::Path;

use anyhow::{bail, Result};

use crate::{
    config::Config,
    logger, probe, run,
    services::{self, relayer, Service},
    tools,
    tx::contracts::config::ContractsConfig,
};

const WAIT_TIMEOUT_SECS: u64 = 300;

pub async fn run(
    cfg: &Config,
    root: &Path,
    http: &reqwest::Client,
    skip_images: bool,
    skip_accounts: bool,
    skip_relayer: bool,
    deploy_contracts: bool,
) -> Result<()> {
    logger::banner("start");

    if !run::has("docker") {
        bail!("docker not found in PATH — required to bring up the stack");
    }

    if skip_images {
        logger::detail("skip image pull");
    } else {
        logger::step("Step 0: pulling images (cosmos, watcher, api, gateway, postgres, relayer)");
        tools::docker::compose(
            root,
            &[
                "pull",
                "--ignore-pull-failures",
                "cosmos",
                "stellar-watcher",
                "ipsa-api",
                "ipsa-gateway",
                "postgres",
                "relayer",
            ],
        )?;
    }

    logger::step("Step 1: ensuring cosmos and the stellar-core watcher are up");
    logger::detail(
        "the watcher starts first because it catches up in the background; until it does, the api serves SCP evidence from the history archives",
    );

    services::up(root, &[Service::Cosmos, Service::Watcher])?;

    logger::step("Step 2: ensuring api + gateway are up");
    if deploy_contracts {
        logger::detail("force redeploy — deferring api + gateway startup to the contract step");
    } else if probe::http_ok(http, &cfg.api_health_url()).await {
        logger::ok("api already reachable");
    } else {
        tools::docker::compose(root, &["up", "-d", "ipsa-api", "ipsa-gateway"])?;

        if !probe::wait_http(http, &cfg.api_health_url(), WAIT_TIMEOUT_SECS).await {
            bail!(
                "api not reachable within {WAIT_TIMEOUT_SECS}s (docker compose logs api gateway)"
            );
        }

        logger::ok("api + gateway reachable");
    }

    logger::step("Step 3: deploying Soroban contracts");
    let deployed = crate::tx::contracts::deploy_all::run(
        &ContractsConfig::from(cfg),
        root,
        deploy_contracts,
        false,
        false,
    )?;

    if deployed {
        if deploy_contracts {
            logger::step("force redeploy — rebuilding + pushing api + gateway images");
            services::push(cfg, root, &[Service::Api, Service::Gateway])?;
        }

        logger::step("recreating api + gateway to pick up ROUTER_CONTRACT_ADDRESS");
        tools::docker::compose(
            root,
            &["up", "-d", "--force-recreate", "ipsa-api", "ipsa-gateway"],
        )?;

        if !probe::wait_http(http, &cfg.api_health_url(), WAIT_TIMEOUT_SECS).await {
            bail!(
                "api not reachable within {WAIT_TIMEOUT_SECS}s (docker compose logs api gateway)"
            );
        }

        logger::ok("api + gateway reachable");
    } else {
        logger::detail("contracts already deployed — skipping api + gateway recreate");
    }

    if skip_accounts {
        logger::detail("skip sender + receiver account provisioning");
    } else {
        logger::step("Step 4: provisioning sender + receiver accounts");
        crate::accounts::provision(cfg, root, false)?;
    }

    if skip_relayer {
        logger::detail("skip ibcv2 relayer");
    } else {
        logger::step("Step 5: writing relayer keys + starting postgres and the relayer");
        relayer::keys::import(cfg, root)?;
        services::up(root, &[Service::Postgres, Service::Relayer])?;
    }

    logger::ok("start complete");
    logger::hint("check: ipsa check   then: ipsa tx clients create --cosmos");

    Ok(())
}
