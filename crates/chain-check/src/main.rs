use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::{Context, Result};
use clap::Parser;

use chain_check::crosscheck;

#[derive(Parser)]
#[command(
    name = "chain-check",
    about = "Verify a Stellar ledger with an independent implementation - separate XDR reader, ring instead of sha2 and ed25519-dalek"
)]
struct Cli {
    #[arg(long, default_value = "testnet")]
    network: String,
    #[arg(long, value_name = "SEQ")]
    ledger: u32,
    #[arg(
        long,
        value_name = "HEX",
        help = "Hex-encoded SCPQuorumSet you trust; the quorum is evaluated against it"
    )]
    trust_root: String,
    #[arg(long, value_name = "PATH", help = "Write a JSON test vector")]
    export: Option<PathBuf>,
}

fn run(cli: Cli) -> Result<()> {
    let net = crosscheck::network(&cli.network)?;
    let raw = hex::decode(cli.trust_root.trim()).context("--trust-root is not hex")?;
    let trust_root = crosscheck::parse_quorum_set(&raw).context("--trust-root")?;

    crosscheck::run(&net, cli.ledger, &trust_root, cli.export.as_deref())
}

fn main() -> ExitCode {
    match run(Cli::parse()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error:#}");

            ExitCode::FAILURE
        }
    }
}
