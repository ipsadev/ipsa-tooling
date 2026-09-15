use std::path::Path;

use anyhow::{anyhow, bail, Result};

use crate::{
    config::Config,
    logger,
    tx::{self, TransferTx, TxCmd},
};

pub async fn run(cfg: &Config, root: &Path, http: &reqwest::Client) -> Result<()> {
    super::relayer_routes::ensure(cfg, root)?;

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
            timeout_secs: 600,
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
