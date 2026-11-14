use std::{
    path::Path,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use anyhow::{anyhow, bail, Result};
use cosmrs::Any;
use prost::Message;

use crate::{
    config::Config,
    logger,
    services::cosmos::{
        config::{fee_for, COMPOSE_SERVICE},
        tx::{address_from_key, CosmosSigner},
    },
    tools,
    tx::contracts::{self, config::ContractsConfig},
};

pub mod assets;

const COSMOS_HOME: &str = "/root/.simapp";

const MSG_TRANSFER_TYPE_URL: &str = "/ibc.applications.transfer.v1.MsgTransfer";

const TRANSFER_GAS_LIMIT: u64 = 300_000;

const TRANSFER_WAIT: Duration = Duration::from_secs(60);

#[derive(Clone, PartialEq, Message)]
struct Coin {
    #[prost(string, tag = "1")]
    denom: String,
    #[prost(string, tag = "2")]
    amount: String,
}

#[derive(Clone, PartialEq, Message)]
struct MsgTransfer {
    #[prost(string, tag = "1")]
    source_port: String,
    #[prost(string, tag = "2")]
    source_channel: String,
    #[prost(message, optional, tag = "3")]
    token: Option<Coin>,
    #[prost(string, tag = "4")]
    sender: String,
    #[prost(string, tag = "5")]
    receiver: String,
    #[prost(uint64, tag = "7")]
    timeout_timestamp: u64,
    #[prost(string, tag = "8")]
    memo: String,
}

pub struct TransferParams {
    pub denom: String,
    pub amount: i128,
    pub receiver: String,
    pub sender: String,
    pub memo: String,
    pub timeout_secs: u64,
    pub mint: bool,
}

pub fn stellar_to_cosmos(
    cfg: &Config,
    root: &Path,
    args: &TransferParams,
) -> Result<Option<String>> {
    logger::banner("transfer stellar (Stellar → Cosmos ICS-20)");

    let transfer_app = cfg.deployment.transfer_app.as_str();
    let source_client = cfg.deployment.cosmos_client_id.as_str();

    let sender = if !args.sender.is_empty() {
        args.sender.as_str()
    } else if !cfg.accounts.stellar_sender_address.is_empty() {
        cfg.accounts.stellar_sender_address.as_str()
    } else {
        cfg.deployment.deployer_address.as_str()
    };

    let sender_identity = if !cfg.accounts.stellar_sender_identity.is_empty() {
        cfg.accounts.stellar_sender_identity.as_str()
    } else {
        cfg.stellar.cli_identity.as_str()
    };

    if transfer_app.is_empty() {
        bail!("TRANSFER_CONTRACT_ADDRESS is not set — run `ipsa start` first");
    }

    if source_client.is_empty() {
        bail!(
            "{} is not set — run `ipsa tx clients create --cosmos` first",
            cfg.cosmos.client_id_variable()
        );
    }

    if sender.is_empty() {
        bail!("no Stellar sender — run `ipsa start` to provision STELLAR_SENDER_ADDRESS (or set DEPLOYER_ADDRESS)");
    }

    let receiver = if !args.receiver.is_empty() {
        args.receiver.clone()
    } else if !cfg.cosmos.receiver_address.trim().is_empty() {
        let addr = cfg.cosmos.receiver_address.trim().to_string();
        logger::detail(&format!("using COSMOS_RECEIVER_ADDRESS → {addr}"));
        addr
    } else {
        cosmos_relayer_address(cfg, root)?
    };

    let cc = ContractsConfig::from(cfg);
    let amount = args.amount.to_string();
    let timeout = transfer_timeout(args.timeout_secs)?.to_string();
    let memo_json = format!("\"{}\"", args.memo);

    let entry = assets::find(&args.denom)?.ok_or_else(|| {
        anyhow!(
            "denom '{}' is not registered — add it to ipsa/config/assets.json and run `ipsa tx contracts deploy --force` to register it",
            args.denom
        )
    })?;

    if entry.voucher {
        if args.mint {
            logger::step(&format!("mint {} {} to {sender}", args.amount, args.denom));

            contracts::invoke(
                &cc,
                root,
                transfer_app,
                &[
                    "mint",
                    "--to",
                    sender,
                    "--denom",
                    &args.denom,
                    "--amount",
                    &amount,
                ],
            )?;
        }
    } else {
        logger::detail(&format!(
            "{} is a local asset — escrowing the sender's existing balance (no mint)",
            args.denom
        ));
    }

    let denom = assets::registered_denom(&cc, root, transfer_app, &entry)?;

    if denom != args.denom {
        logger::detail(&format!(
            "the transfer app registers {} as {denom}",
            args.denom
        ));
    }

    logger::step(&format!(
        "initiate_transfer {} {} → {receiver} (client {source_client}, timeout {timeout})",
        args.amount, args.denom
    ));

    let out = contracts::invoke_as(
        &cc,
        root,
        transfer_app,
        &[
            "initiate_transfer",
            "--sender",
            sender,
            "--source_client_id",
            source_client,
            "--denom",
            &denom,
            "--amount",
            &amount,
            "--receiver",
            &receiver,
            "--timeout_timestamp",
            &timeout,
            "--memo",
            &memo_json,
        ],
        sender_identity,
    )?;

    let hash = contracts::tx_hash(&out);

    match &hash {
        Some(hash) => {
            logger::ok(&format!("transfer initiated — tx {hash}"));
            println!("{hash}");
        }
        None => logger::warn(
            "transfer initiated, but the stellar CLI printed no transaction hash — \
             find it on the explorer to relay it by hand",
        ),
    }

    relay_hint(hash.as_deref(), "--stellar");

    Ok(hash)
}

pub fn cosmos_to_stellar(
    cfg: &Config,
    root: &Path,
    args: &TransferParams,
) -> Result<Option<String>> {
    logger::banner("transfer cosmos (Cosmos → Stellar ICS-20)");

    let source_client = cfg.deployment.stellar_client_id.as_str();
    let port = cfg.deployment.transfer_port.as_str();
    let chain_id = cfg.cosmos.chain_id.as_str();
    let gas_denom = cfg.cosmos.gas_denom.as_str();

    if source_client.is_empty() {
        bail!(
            "{} is not set — run `ipsa tx clients create --stellar` first",
            cfg.cosmos.stellar_client_id_variable()
        );
    }

    let from = if !args.sender.is_empty() {
        args.sender.as_str()
    } else if !cfg.accounts.cosmos_sender_key_name.is_empty() {
        cfg.accounts.cosmos_sender_key_name.as_str()
    } else {
        cfg.cosmos.key_name.as_str()
    };

    if from.is_empty() {
        bail!("no Cosmos sender — set COSMOS_SENDER_KEY_NAME (or pass --from)");
    }

    let receiver = stellar_receiver(cfg, args)?;

    let coin = format!("{}{}", args.amount, args.denom);
    let timeout = transfer_timeout(args.timeout_secs)?.to_string();
    let gas_prices = format!("0.025{gas_denom}");

    logger::step(&format!(
        "ibc-transfer {coin} → {receiver} (client {source_client}, from {from}, timeout {}s)",
        args.timeout_secs
    ));

    let out = submit(
        root,
        &[
            "tx",
            "ibc-transfer",
            "transfer",
            port,
            source_client,
            &receiver,
            &coin,
            "--from",
            from,
            "--chain-id",
            chain_id,
            "--gas",
            "auto",
            "--gas-adjustment",
            "1.5",
            "--gas-prices",
            &gas_prices,
            "--packet-timeout-timestamp",
            &timeout,
            "--absolute-timeouts",
            "--memo",
            args.memo.as_str(),
            "--broadcast-mode",
            "sync",
            "--yes",
        ],
    )?;

    let hash = tx_hash(&out);

    match &hash {
        Some(hash) => {
            logger::ok(&format!("transfer initiated — tx {hash}"));
            println!("{hash}");
        }
        None => logger::warn(
            "transfer initiated, but simd printed no txhash — \
             query the tx to relay it by hand",
        ),
    }

    relay_hint(hash.as_deref(), "--cosmos");

    Ok(hash)
}

pub async fn cosmos_to_stellar_signed(
    cfg: &Config,
    http: &reqwest::Client,
    args: &TransferParams,
) -> Result<Option<String>> {
    let chain_id = cfg.cosmos.chain_id.as_str();

    logger::banner(&format!(
        "transfer cosmos (Cosmos → Stellar ICS-20 on {chain_id})"
    ));

    let source_client = cfg.deployment.stellar_client_id.as_str();

    if source_client.is_empty() {
        bail!(
            "{} is not set — run `ipsa tx clients create --stellar` first",
            cfg.cosmos.stellar_client_id_variable()
        );
    }

    let signer = CosmosSigner::from_config(&cfg.cosmos, http.clone())?;
    let sender = signer.relayer_address()?.to_string();

    if !args.sender.is_empty() && args.sender != sender {
        bail!(
            "--from {} is not {sender}, the address of COSMOS_RELAYER_PRIVATE_KEY, which signs every transfer on {chain_id}",
            args.sender
        );
    }

    let receiver = stellar_receiver(cfg, args)?;
    let timeout = transfer_timeout(args.timeout_secs)?;

    let msg = MsgTransfer {
        source_port: cfg.deployment.transfer_port.clone(),
        source_channel: source_client.to_string(),
        token: Some(Coin {
            denom: args.denom.clone(),
            amount: args.amount.to_string(),
        }),
        sender,
        receiver: receiver.clone(),
        timeout_timestamp: timeout,
        memo: args.memo.clone(),
    };

    logger::step(&format!(
        "MsgTransfer {}{} → {receiver} (client {source_client}, timeout {}s)",
        args.amount, args.denom, args.timeout_secs
    ));

    let landed = signer
        .broadcast_as_relayer(
            Any {
                type_url: MSG_TRANSFER_TYPE_URL.to_string(),
                value: msg.encode_to_vec(),
            },
            "ibc-transfer",
            TRANSFER_GAS_LIMIT,
            fee_for(TRANSFER_GAS_LIMIT),
            TRANSFER_WAIT,
        )
        .await?;

    let hash = landed
        .pointer("/tx_response/txhash")
        .and_then(|v| v.as_str())
        .map(str::to_string);

    match &hash {
        Some(hash) => {
            logger::ok(&format!("transfer initiated — tx {hash}"));
            println!("{hash}");
        }
        None => logger::warn("transfer landed, but the node returned no txhash"),
    }

    relay_hint(hash.as_deref(), "--cosmos");

    Ok(hash)
}

fn stellar_receiver(cfg: &Config, args: &TransferParams) -> Result<String> {
    if !args.receiver.is_empty() {
        return Ok(args.receiver.clone());
    }

    if !cfg.accounts.stellar_receiver_address.trim().is_empty() {
        let addr = cfg.accounts.stellar_receiver_address.trim().to_string();
        logger::detail(&format!("using STELLAR_RECEIVER_ADDRESS → {addr}"));

        return Ok(addr);
    }

    if !cfg.accounts.stellar_sender_address.is_empty() {
        return Ok(cfg.accounts.stellar_sender_address.clone());
    }

    bail!("no Stellar receiver — set STELLAR_RECEIVER_ADDRESS (or pass --to)")
}

fn relay_hint(hash: Option<&str>, flag: &str) {
    match hash {
        Some(hash) => logger::hint(&format!(
            "relay it: ipsa tx relay --tx {hash} {flag} --wait"
        )),
        None => logger::hint(&format!(
            "nothing is relayed until you run: ipsa tx relay --tx <hash> {flag}"
        )),
    }
}

fn transfer_timeout(secs: u64) -> Result<u64> {
    let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();

    Ok(now + secs)
}

fn submit(root: &Path, args: &[&str]) -> Result<String> {
    let mut argv = vec!["compose", "exec", "-T", COMPOSE_SERVICE, "simd"];
    argv.extend_from_slice(args);
    argv.extend_from_slice(&["--keyring-backend", "test", "--home", COSMOS_HOME]);

    tools::docker::capture_all(root, &argv)
}

fn tx_hash(out: &str) -> Option<String> {
    out.lines()
        .map(str::trim)
        .find_map(|line| line.strip_prefix("txhash:"))
        .map(|hash| hash.trim().to_string())
        .filter(|hash| !hash.is_empty())
}

fn cosmos_relayer_address(cfg: &Config, root: &Path) -> Result<String> {
    if !cfg.cosmos.is_local() {
        logger::detail("no --receiver given, using the address of COSMOS_RELAYER_PRIVATE_KEY");

        return address_from_key(&cfg.cosmos.relayer_key_hex, &cfg.cosmos.account_prefix);
    }

    let key_name = cfg.cosmos.key_name.as_str();

    logger::detail(&format!(
        "no --receiver given, deriving the cosmos `{key_name}` address"
    ));

    let out = tools::docker::capture_all(
        root,
        &[
            "compose",
            "exec",
            "-T",
            COMPOSE_SERVICE,
            "simd",
            "keys",
            "show",
            key_name,
            "-a",
            "--keyring-backend",
            "test",
            "--home",
            "/root/.simapp",
        ],
    )?;

    let address = out
        .lines()
        .map(str::trim)
        .find(|line| line.starts_with("cosmos"))
        .map(str::to_string);

    match address {
        Some(addr) => Ok(addr),
        None => {
            bail!("could not derive the cosmos `{key_name}` address — pass --receiver explicitly")
        }
    }
}
