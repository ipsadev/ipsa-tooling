use std::path::Path;

use crate::{config::Config, logger, probe, tools};

pub fn stellar_balances(cfg: &Config, root: &Path, address: &str, denom: Option<&str>) {
    logger::banner(&format!("balances — stellar {address}"));

    let transfer = cfg.deployment.transfer_app.as_str();

    if transfer.is_empty() {
        logger::warn("TRANSFER_CONTRACT_ADDRESS unset — run `ipsa start` first");

        return;
    }

    let denom = denom.filter(|d| !d.is_empty()).unwrap_or("XLM");

    logger::detail(&format!("using denom — {denom}"));

    let args = [
        "contract",
        "invoke",
        "--id",
        cfg.deployment.transfer_app.as_str(),
        "--source",
        cfg.stellar.signing_key.as_str(),
        "--rpc-url",
        cfg.stellar.rpc_url.as_str(),
        "--network-passphrase",
        cfg.stellar.network_passphrase.as_str(),
        "--",
        "balance_of",
        "--who",
        address,
        "--denom",
        denom,
    ];

    let result = tools::stellar::capture_quiet(root, &args);

    match result {
        Ok(out) => logger::ok(&format!(
            "balance: {} {denom}",
            out.trim().trim_matches('"')
        )),
        Err(_) => logger::warn(&format!("could not get balance for address {address}")),
    }
}

pub async fn cosmos_balances(
    cfg: &Config,
    http: &reqwest::Client,
    address: &str,
    denom: Option<&str>,
) {
    logger::banner(&format!("balances — cosmos {address}"));

    let url = format!(
        "{}/cosmos/bank/v1beta1/balances/{address}",
        cfg.cosmos.rest_url
    );

    match probe::get_json(http, &url).await {
        Some(value) => match value["balances"].as_array() {
            Some(coins) if !coins.is_empty() => {
                for coin in coins {
                    let coin_denom = coin["denom"].as_str().unwrap_or("?");
                    let amount = coin["amount"].as_str().unwrap_or("?");

                    let is_filtered = {
                        if let Some(coin) = denom {
                            coin == coin_denom
                        } else {
                            false
                        }
                    };

                    if is_filtered {
                        logger::detail(&format!("{amount} {coin_denom}"));

                        break;
                    } else {
                        logger::detail(&format!("{amount} {coin_denom}"));
                    }
                }
            }
            _ => logger::detail("(no balances)"),
        },
        _ => logger::warn(&format!("could not query {url}")),
    }
}
