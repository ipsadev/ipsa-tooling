use std::path::Path;

use anyhow::{bail, Result};

use crate::{
    config::Config,
    tx::{self, TransferTx, TxCmd},
};

pub async fn run(cfg: &Config, root: &Path, http: &reqwest::Client) -> Result<()> {
    super::relayer_routes::ensure(cfg, root)?;

    let stellar_sender = cfg.accounts.stellar_sender_address.clone();
    if stellar_sender.is_empty() {
        bail!("STELLAR_SENDER_ADDRESS is unset — run `ipsa start` first");
    }

    let cosmos_receiver = cfg.accounts.cosmos_receiver_address.clone();
    if cosmos_receiver.is_empty() {
        bail!("COSMOS_RECEIVER_ADDRESS is unset — run `ipsa start` first");
    }

    transfer(
        cfg,
        root,
        http,
        "stellar → cosmos (XLM)",
        stellar_sender,
        cosmos_receiver,
        "XLM",
    )
    .await?;

    let cosmos_sender = cfg.accounts.cosmos_sender_address.clone();
    if cosmos_sender.is_empty() {
        bail!("COSMOS_SENDER_ADDRESS is unset — run `ipsa start` first");
    }

    let stellar_receiver = cfg.accounts.stellar_receiver_address.clone();
    if stellar_receiver.is_empty() {
        bail!("STELLAR_RECEIVER_ADDRESS is unset — run `ipsa start` first");
    }

    transfer(
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

async fn transfer(
    cfg: &Config,
    root: &Path,
    http: &reqwest::Client,
    label: &str,
    from: String,
    to: String,
    denom: &str,
) -> Result<()> {
    crate::logger::step(&format!("transfer — {label}"));

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
    .await?;

    crate::logger::ok(&format!("delivered and acknowledged — {label}"));

    Ok(())
}
