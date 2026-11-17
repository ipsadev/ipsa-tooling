use std::{
    path::Path,
    time::{Duration, Instant},
};

use anyhow::{anyhow, bail, Result};

use crate::{
    config::{get, Config},
    logger, probe, services,
    tx::{self, TransferTx, TxCmd},
};

const DEFAULT_PACKET_TIMEOUT_SECS: u64 = 600;

const RELAY_PATH_TIMEOUT_SECS: u64 = 120;

pub async fn run(cfg: &Config, root: &Path, http: &reqwest::Client) -> Result<()> {
    super::relayer_routes::ensure(cfg, root)?;

    wait_for_relay_path(cfg).await?;

    let stellar_sender = required(
        &cfg.accounts.stellar_sender_address,
        "STELLAR_SENDER_ADDRESS",
    )?;
    let cosmos_receiver = required(
        &cfg.accounts.cosmos_receiver_address,
        "COSMOS_RECEIVER_ADDRESS",
    )?;
    let cosmos_sender = required(&cfg.accounts.cosmos_sender_address, "COSMOS_SENDER_ADDRESS")?;
    let stellar_receiver = required(
        &cfg.accounts.stellar_receiver_address,
        "STELLAR_RECEIVER_ADDRESS",
    )?;

    relayed_transfer(
        cfg,
        root,
        http,
        "stellar → cosmos (XLM)",
        stellar_sender,
        cosmos_receiver,
        "XLM",
    )
    .await?;

    relayed_transfer(
        cfg,
        root,
        http,
        "cosmos → stellar (stake)",
        cosmos_sender,
        stellar_receiver,
        "stake",
    )
    .await
}

fn packet_timeout_secs() -> u64 {
    let raw = get("RELAY_TEST_TIMEOUT_SECS", "");

    if raw.trim().is_empty() {
        return DEFAULT_PACKET_TIMEOUT_SECS;
    }

    match raw.trim().parse() {
        Ok(secs) => secs,
        Err(_) => {
            logger::warn(&format!(
                "RELAY_TEST_TIMEOUT_SECS ({raw}) is not a number — using {DEFAULT_PACKET_TIMEOUT_SECS}s"
            ));

            DEFAULT_PACKET_TIMEOUT_SECS
        }
    }
}

async fn wait_for_relay_path(cfg: &Config) -> Result<()> {
    logger::step("waiting for the relay path: relayer api and proof-api reachable");

    let proof_api = services::proof_api::ProofApiConfig::from_env(false)?;
    let relayer_api = cfg.relayer.api_addr.clone();
    let proof_api_url = get("PROOF_API_URL", "http://127.0.0.1:3100");

    let deadline = Instant::now() + Duration::from_secs(RELAY_PATH_TIMEOUT_SECS);

    loop {
        let relayer_up = probe::tcp_ok(&relayer_api);
        let proof_api_up = services::proof_api::listening(&proof_api);

        if relayer_up && proof_api_up {
            logger::ok("relay path is up");

            return Ok(());
        }

        if Instant::now() >= deadline {
            bail!(
                "the relay path is not up after {RELAY_PATH_TIMEOUT_SECS}s: relayer api {relayer_api} {}, proof-api {proof_api_url} {}. Bring the stack up with `ipsa start`, start the proof-api with `ipsa proof-api start`, then retry",
                reachable(relayer_up),
                reachable(proof_api_up)
            );
        }

        tokio::time::sleep(Duration::from_secs(2)).await;
    }
}

fn reachable(up: bool) -> &'static str {
    if up {
        "reachable"
    } else {
        "unreachable"
    }
}

async fn relayed_transfer(
    cfg: &Config,
    root: &Path,
    http: &reqwest::Client,
    label: &str,
    from: String,
    to: String,
    denom: &str,
) -> Result<()> {
    logger::step(&format!("relayed transfer — {label}"));

    tx::run(
        cfg,
        root,
        http,
        TxCmd::Transfer(TransferTx {
            from,
            to,
            amount: 1000,
            denom: denom.to_string(),
            timeout_secs: packet_timeout_secs(),
            no_mint: false,
            relay: true,
            wait: true,
            strict: true,
            relay_timeout_secs: None,
        }),
    )
    .await
    .map_err(|error| anyhow!("{label}: {error:#}"))?;

    logger::ok(&format!("delivered and acknowledged — {label}"));

    Ok(())
}

fn required(value: &str, name: &str) -> Result<String> {
    if value.is_empty() {
        bail!("{name} is unset — run `ipsa start` first");
    }

    Ok(value.to_string())
}
