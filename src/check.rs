use std::path::Path;

use anyhow::Result;

use crate::{accounts, config::Config, logger, probe, run, shared};

pub async fn run(root: &Path, cfg: &Config, http: &reqwest::Client) -> Result<()> {
    logger::banner("check — prerequisites & configuration");

    logger::step("toolchain");
    shared::check(
        "docker",
        run::has("docker"),
        "required to run the chain + services",
    );
    shared::check(
        "stellar",
        run::has("stellar"),
        "Soroban CLI, used to build/deploy contracts",
    );
    shared::check(
        "cargo",
        run::has("cargo"),
        "builds the wasm light client + this CLI",
    );

    logger::step("repository");
    logger::ok(&format!("repo root: {}", root.display()));

    let env_file = root.join(".env");

    if env_file.exists() {
        logger::ok(".env present");
    } else {
        logger::fail(".env missing — copy .env.example and fill it in");
    }

    logger::step("configuration");
    shared::flag(
        "STELLAR_SIGNING_KEY",
        !cfg.stellar.signing_key.is_empty(),
        "needed to deploy + sign on Stellar",
    );
    shared::flag(
        "ROUTER_CONTRACT_ADDRESS",
        !cfg.stellar.ibc_router.is_empty(),
        "router address (from contracts-cd, set in .env)",
    );
    shared::flag(
        "TRANSFER_CONTRACT_ADDRESS",
        !cfg.stellar.transfer_app.is_empty(),
        "transfer-app address",
    );
    shared::flag(
        "relayer.yml",
        Path::new(&cfg.relayer.config).exists(),
        "mounted into the ibc-relayer container",
    );

    logger::step("services");

    let cosmos = probe::http_ok(http, &format!("{}/cosmos", cfg.cosmos.rest_url)).await;
    logger::status_line(cfg.cosmos.chain_id.as_str(), cosmos, &cfg.cosmos.rest_url);

    let api = probe::http_ok(http, &format!("{}/health", cfg.stellar.api_url)).await;
    logger::status_line("stellar-api", api, &cfg.stellar.api_url);

    let gateway = probe::tcp_ok(&cfg.stellar.gateway_url);
    logger::status_line("gateway-grpc", gateway, &cfg.stellar.gateway_url);

    let relayer = probe::tcp_ok(&cfg.relayer.api_addr);
    logger::status_line("ibc-relayer", relayer, &cfg.relayer.api_addr);

    if !cosmos || !api {
        logger::hint("bring the stack up with: ipsa up");
    }

    if !relayer {
        logger::hint("the ibcv2 stack is optional until the migration lands");
    }

    logger::step("accounts");

    accounts::show(cfg);

    Ok(())
}
