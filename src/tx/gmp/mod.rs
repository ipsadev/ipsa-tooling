use std::{
    path::Path,
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::{bail, Result};

use crate::{
    config::Config,
    logger,
    tx::contracts::{self, config::ContractsConfig},
};

#[derive(clap::Subcommand)]
pub enum GmpTx {
    #[command(
        about = "Show the account a (client, sender, salt) triplet derives to, before it exists"
    )]
    Account(AccountArgs),

    #[command(about = "Send a general message to a registered gmpport on the counterparty")]
    Call(CallArgs),
}

#[derive(clap::Args)]
pub struct AccountArgs {
    #[arg(long, help = "Client id on this chain, e.g. 07-tendermint-0")]
    pub client_id: String,
    #[arg(long, help = "Sender identifier on the source chain")]
    pub sender: String,
    #[arg(long, default_value = "", help = "Hex salt distinguishing accounts")]
    pub salt: String,
}

#[derive(clap::Args)]
pub struct CallArgs {
    #[arg(long, help = "Stellar account originating the call")]
    pub from: String,
    #[arg(long, help = "Source client id, e.g. 07-tendermint-0")]
    pub client_id: String,
    #[arg(long, help = "Destination-specific receiver identifier")]
    pub receiver: String,
    #[arg(long, default_value = "", help = "Hex salt distinguishing accounts")]
    pub salt: String,
    #[arg(
        long,
        help = "Hex call payload, interpreted by the destination runtime"
    )]
    pub payload: String,
    #[arg(
        long,
        default_value_t = 600,
        help = "Seconds until the packet times out"
    )]
    pub timeout_secs: u64,
    #[arg(long, default_value = "", help = "Optional metadata for middleware")]
    pub memo: String,
    #[arg(
        long,
        default_value = "application/json",
        help = "Packet encoding: application/json for interop, application/x-soroban-xdr for larger payloads"
    )]
    pub encoding: String,
}

pub fn run(cfg: &Config, root: &Path, cmd: GmpTx) -> Result<()> {
    match cmd {
        GmpTx::Account(args) => account(cfg, root, args),
        GmpTx::Call(args) => call(cfg, root, args),
    }
}

fn app(cfg: &Config) -> Result<String> {
    let app = cfg.deployment.gmp_app.clone();

    if app.is_empty() {
        bail!("GMP_CONTRACT_ADDRESS is not set in .env — deploy with contracts-cd in ipsa-contracts, then set the address it reports");
    }

    Ok(app)
}

fn account(cfg: &Config, root: &Path, args: AccountArgs) -> Result<()> {
    logger::banner("gmp account (derived, read only)");

    let app = app(cfg)?;
    let salt = normalise_salt(&args.salt)?;

    logger::detail(
        "the address is a pure function of (client_id, sender, salt), so it can be handed out \
         before anything is deployed",
    );

    let out = contracts::invoke_as(
        &ContractsConfig::from(cfg),
        root,
        &app,
        &[
            "account_address",
            "--client_id",
            &args.client_id,
            "--sender",
            &args.sender,
            "--salt",
            &salt,
        ],
        &cfg.stellar.signing_key,
    )?;

    logger::ok(out.trim());

    Ok(())
}

fn call(cfg: &Config, root: &Path, args: CallArgs) -> Result<()> {
    logger::banner("gmp call (stellar → counterparty)");

    let app = app(cfg)?;
    let salt = normalise_salt(&args.salt)?;
    let payload = normalise_hex(&args.payload, "payload")?;

    if payload.is_empty() {
        bail!("--payload is required; it is the call the destination will execute");
    }

    let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();
    let timeout = (now + args.timeout_secs).to_string();

    logger::step(&format!(
        "send_call client={} receiver={} timeout={timeout}",
        args.client_id, args.receiver
    ));

    let out = contracts::invoke_as(
        &ContractsConfig::from(cfg),
        root,
        &app,
        &[
            "send_call",
            "--sender",
            &args.from,
            "--source_client_id",
            &args.client_id,
            "--receiver",
            &args.receiver,
            "--salt",
            &salt,
            "--payload",
            &payload,
            "--timeout_timestamp",
            &timeout,
            "--memo",
            &args.memo,
            "--encoding",
            &args.encoding,
        ],
        &cfg.stellar.signing_key,
    )?;

    logger::ok(&format!("sequence {}", out.trim()));
    logger::hint("relay it: ipsa tx relay --tx <hash> --stellar --wait");

    Ok(())
}

fn normalise_salt(raw: &str) -> Result<String> {
    normalise_hex(raw, "salt")
}

fn normalise_hex(raw: &str, what: &str) -> Result<String> {
    let trimmed = raw.trim().trim_start_matches("0x");

    if trimmed.is_empty() {
        return Ok(String::new());
    }

    if hex::decode(trimmed).is_err() {
        bail!("--{what} must be hex, got {raw:?}");
    }

    Ok(trimmed.to_string())
}
