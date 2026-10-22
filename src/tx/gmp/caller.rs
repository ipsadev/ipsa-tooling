use std::path::Path;

use anyhow::{bail, Context, Result};
use serde_json::Value;

use super::evm;
use crate::{
    config::{self, Config},
    logger, shared,
    tx::contracts::{self, config::ContractsConfig, deploy_all, release},
};

const WASM: &str = "ipsa_gmp_caller.wasm";

#[derive(clap::Subcommand)]
pub enum CallerCmd {
    #[command(
        about = "Deploy ipsa-gmp-caller from the CONTRACTS_VERSION release and record GMP_CALLER_ADDRESS"
    )]
    Deploy,

    #[command(
        about = "Show what the caller recorded for a call: pending, acknowledged or timed out"
    )]
    Outcome(OutcomeArgs),
}

#[derive(clap::Args)]
pub struct OutcomeArgs {
    #[arg(long, help = "Sequence the call was sent with")]
    pub sequence: u64,
    #[arg(long, help = "Source client id; defaults to STELLAR_EVM_CLIENT_ID")]
    pub client_id: Option<String>,
    #[arg(
        long,
        help = "Solidity type to decode an Ethereum call's result as, e.g. uint256"
    )]
    pub returns: Option<String>,
}

pub fn address() -> Result<String> {
    let caller = config::get("GMP_CALLER_ADDRESS", "");

    if caller.is_empty() {
        bail!("GMP_CALLER_ADDRESS is not set — deploy the caller with `ipsa tx gmp caller deploy`");
    }

    Ok(caller)
}

pub fn run(cfg: &Config, root: &Path, cmd: CallerCmd) -> Result<()> {
    match cmd {
        CallerCmd::Deploy => deploy(cfg, root),
        CallerCmd::Outcome(args) => outcome(cfg, root, args),
    }
}

fn deploy(cfg: &Config, root: &Path) -> Result<()> {
    logger::banner("gmp caller deploy");

    let gmp = super::app(cfg)?;
    let cc = ContractsConfig::from(cfg);
    let owner = deploy_all::deployer_address(&cc)?;
    let wasm = release::fetch(root)?.join(WASM).display().to_string();

    logger::step(&format!(
        "deploying ipsa-gmp-caller (owner {owner}, gmp {gmp})"
    ));
    let caller = contracts::deploy(&cc, root, &wasm, &["--owner", &owner, "--gmp", &gmp])?;

    shared::env_upsert(
        &config::tooling_env(),
        &[("GMP_CALLER_ADDRESS", caller.as_str())],
    )?;

    logger::ok(&format!("ipsa-gmp-caller: {caller} (GMP_CALLER_ADDRESS)"));

    Ok(())
}

fn outcome(cfg: &Config, root: &Path, args: OutcomeArgs) -> Result<()> {
    logger::banner("gmp caller outcome");

    let caller = address()?;
    let client = args
        .client_id
        .unwrap_or_else(|| cfg.evm.stellar_client_id.clone());
    let sequence = args.sequence.to_string();

    let out = contracts::read(
        &ContractsConfig::from(cfg),
        root,
        &caller,
        &[
            "outcome",
            "--source_client_id",
            &client,
            "--sequence",
            &sequence,
        ],
        &cfg.stellar.signing_key,
    )?;

    let recorded: Value =
        serde_json::from_str(out.trim()).unwrap_or(Value::String(out.trim().to_string()));

    match &recorded {
        Value::Null => logger::warn(&format!(
            "nothing recorded for {client} sequence {sequence}"
        )),
        Value::String(state) => logger::ok(&format!("{client} sequence {sequence}: {state}")),
        Value::Object(map) if map.contains_key("TimedOut") => logger::ok(&format!(
            "{client} sequence {sequence}: timed out, nothing ran on the destination"
        )),
        Value::Object(map) => {
            let acknowledged = map
                .get("Acknowledged")
                .context("unexpected outcome shape")?;
            let success = acknowledged
                .get("success")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let raw = acknowledged
                .get("acknowledgement")
                .and_then(Value::as_str)
                .unwrap_or_default();

            if !success {
                logger::ok(&format!(
                    "{client} sequence {sequence}: failed — the destination returned the universal error acknowledgement"
                ));
                return Ok(());
            }

            logger::ok(&format!("{client} sequence {sequence}: succeeded"));

            let bytes = hex::decode(raw).context("the acknowledgement is not hex")?;

            match evm::acknowledgement_result(&bytes) {
                Ok(result) => {
                    logger::detail(&format!("result 0x{}", hex::encode(&result)));

                    if let Some(kind) = args.returns {
                        logger::ok(&format!(
                            "returned {}",
                            evm::decode_returns(&kind, &result)?
                        ));
                    }
                }
                Err(_) => logger::detail(&format!("acknowledgement 0x{raw}")),
            }
        }
        other => logger::detail(&other.to_string()),
    }

    println!("{recorded}");

    Ok(())
}
