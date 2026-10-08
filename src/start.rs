use std::path::Path;

use anyhow::{bail, Result};

use crate::{
    config::Config,
    logger, probe, run,
    services::{self, relayer, Service},
    shared::StartArgs,
    tools,
    tx::{
        clients::{self, config::ClientsConfig},
        contracts::{self, config::ContractsConfig},
    },
};

const WAIT_TIMEOUT_SECS: u64 = 300;

pub async fn run(
    cfg: &Config,
    root: &Path,
    http: &reqwest::Client,
    args: &StartArgs,
) -> Result<()> {
    logger::banner("start");

    let deploy_contracts = args.deploy_contracts;

    if !run::has("docker") {
        bail!("docker not found in PATH — required to bring up the stack");
    }

    if args.skip_images {
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
    let deployed = contracts::deploy_all::run(
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

    let cfg = Config::load(root);

    if args.skip_accounts {
        logger::detail("skip sender + receiver account provisioning");
    } else {
        logger::step("Step 4: provisioning sender + receiver accounts");
        crate::accounts::provision(&cfg, root, false)?;
    }

    let created = if args.skip_clients {
        logger::detail("skip light-client wasm, clients and counterparties");

        false
    } else {
        pair_clients(root, http, deployed).await?
    };

    let cfg = Config::load(root);

    if args.skip_relayer {
        logger::detail("skip ibcv2 relayer");
    } else {
        logger::step("Step 8: writing relayer keys + starting postgres and the relayer");
        relayer::keys::import(&cfg)?;
        services::up(root, &[Service::Postgres])?;

        if deployed || created {
            logger::detail(
                "relayer.yml changed — recreating the relayer so it reads the new routes",
            );
            services::restart(root, &[Service::Relayer])?;
        } else {
            services::up(root, &[Service::Relayer])?;
        }
    }

    logger::ok("start complete");
    logger::hint("check: ipsa check   then: ipsa test --ics ics20-relay");

    Ok(())
}

async fn pair_clients(root: &Path, http: &reqwest::Client, force: bool) -> Result<bool> {
    logger::step("Step 5: storing the stellar light-client wasm on cosmos");
    contracts::wasm::upload(root, http, false, None).await?;

    let cfg = Config::load(root);
    let before = ClientsConfig::from(&cfg);

    logger::step("Step 6: creating the cosmos client on stellar and the stellar client on cosmos");
    if force {
        logger::detail("contracts were redeployed — creating clients on the new router");
    }

    clients::cosmos::run(&cfg, &before, root, http, force).await?;

    let cfg = Config::load(root);
    clients::stellar::run(&cfg, &ClientsConfig::from(&cfg), http, force).await?;

    let cfg = Config::load(root);
    let after = ClientsConfig::from(&cfg);

    let created = force || before.cosmos_client.is_none() || before.stellar_client.is_none();

    if !created {
        logger::detail("both clients already existed — counterparties are already registered");

        return Ok(false);
    }

    logger::step("Step 7: registering each client as the other's counterparty");
    clients::counterparty::run(&cfg, &after, root, http, "stellar").await?;
    clients::counterparty::run(&cfg, &after, root, http, "cosmos").await?;

    Ok(true)
}
