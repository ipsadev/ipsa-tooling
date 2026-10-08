mod accounts;
mod build;
mod check;
mod config;
mod gateway;
mod install;
mod logger;
mod probe;
mod repo;
mod run;
mod services;
mod shared;
mod stack;
mod start;
mod tests;
mod tools;
mod tx;

use std::time::Duration;

use anyhow::Result;
use clap::{Parser, Subcommand};
use config::Config;
use services::{proof_api::ProofApiCmd, relayer::RelayerCmd, ServicesCmd};
use tests::TestArgs;
use tx::TxCmd;

use crate::shared::{DownArgs, StartArgs, UpArgs};

#[derive(Parser)]
#[command(
    name = "ipsa",
    version,
    about = "Orchestrator for the Stellar<->Cosmos IBC v2 bridge",
    long_about = "A caribic-style orchestrator for the Stellar<->Cosmos bridge: ops \
(install/check/status/up/down/start), tx (clients/contracts/transfer writes), query + balances (reads), and the cosmos/gateway/api service groups. Drives docker, the stellar CLI, and the api directly — no shell scripts.",
    propagate_version = true
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    #[command(about = "Install the ipsa binary to the cargo bin dir")]
    Install,
    #[command(about = "Check prerequisites, configuration, and service health")]
    Check,
    #[command(
        about = "Build the contract wasms without deploying: soroban (--soroban, default) or the cosmwasm light client (--cosmos)"
    )]
    Build(build::BuildArgs),
    #[command(about = "Bring the stack up via docker compose (cosmos + api + gateway)")]
    Up(UpArgs),
    #[command(about = "Stop the stack via docker compose")]
    Down(DownArgs),
    #[command(
        about = "Full start: pull images, start chains, deploy contracts, upload wasm, import keys, start the relayer"
    )]
    Start(StartArgs),
    #[command(about = "Write operations: clients (create, counterparty), contracts, transfer")]
    Tx {
        #[command(subcommand)]
        cmd: TxCmd,
    },
    #[command(
        about = "Service lifecycle + images: pull/up/restart/down/build/push (api, gateway, cosmos, relayer, proof-api, postgres)"
    )]
    Services {
        #[command(subcommand)]
        cmd: ServicesCmd,
    },
    #[command(
        about = "stellar-proof-api on the host: start/stop/restart/status (PROOF_API_DIR, PROOF_API_URL)"
    )]
    ProofApi {
        #[command(subcommand)]
        cmd: ProofApiCmd,
    },
    #[command(about = "Relayer: write the signing key file")]
    Relayer {
        #[command(subcommand)]
        cmd: RelayerCmd,
    },
    Test(TestArgs),
}

#[tokio::main]
async fn main() -> Result<()> {
    logger::init();
    let cli = Cli::parse();
    config::load_tooling_env();
    let root = repo::find_root()?;
    let root = root.as_path();
    let cfg = Config::load(root);
    let http = reqwest::Client::builder()
        .timeout(Duration::from_secs(8))
        .build()?;

    match cli.command {
        Command::Install => install::run(root)?,
        Command::Check => check::run(root, &cfg, &http).await?,
        Command::Build(args) => build::run(root, args)?,
        Command::Up(args) => stack::up(root, args.cosmos, args.stellar)?,
        Command::Down(args) => stack::down(root, args.volumes)?,
        Command::Start(args) => start::run(&cfg, root, &http, &args).await?,

        Command::Tx { cmd } => tx::run(&cfg, root, &http, cmd).await?,

        Command::Services { cmd } => services::run(&cfg, root, cmd)?,

        Command::ProofApi { cmd } => services::proof_api::run(root, cmd)?,
        Command::Relayer { cmd } => match cmd {
            RelayerCmd::KeysImport => services::relayer::keys::import(&cfg)?,
            RelayerCmd::Transfers { limit, status } => {
                services::relayer::db::transfers(&cfg, &http, limit, status.as_deref()).await?
            }
            RelayerCmd::Summary => services::relayer::db::summary(&cfg, &http).await?,
        },

        Command::Test(args) => tests::run(root, &http, args).await?,
    }

    Ok(())
}
