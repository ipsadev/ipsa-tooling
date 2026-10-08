use std::path::Path;

use anyhow::{anyhow, bail, Result};

use crate::{
    config::Config,
    shared::{self, Chain},
};

pub mod clients;
pub mod contracts;
pub mod evm;
pub mod gmp;
pub mod query;
pub mod relay;
pub mod transfer;

use clients::config::ClientsConfig;
use contracts::config::ContractsConfig;
use query::QueryArgs;
use relay::RelayArgs;
use transfer::TransferParams;

#[derive(clap::Subcommand)]
pub enum TxCmd {
    #[command(about = "Client lifecycle: create clients, register counterparties")]
    Clients {
        #[command(subcommand)]
        cmd: ClientsTx,
    },
    #[command(about = "Contracts: upload wasm, deploy on Stellar or store code on Cosmos")]
    Contracts {
        #[command(subcommand)]
        cmd: ContractsTx,
    },
    #[command(about = "Originate an ICS-20 transfer, routed by the from/to address")]
    Transfer(TransferTx),
    #[command(about = "ICS-027 general message passing: derive an account, send a call")]
    Gmp {
        #[command(subcommand)]
        cmd: gmp::GmpTx,
    },
    #[command(
        about = "Hand a source tx to the relayer (stellar ↔ cosmos or stellar ↔ evm) and follow its packets"
    )]
    Relay(RelayArgs),
    #[command(about = "Start a query transaction")]
    Query(QueryArgs),
}

#[derive(clap::Subcommand)]
pub enum ClientsTx {
    #[command(about = "Create the Cosmos and/or Stellar client (both when neither flag is given)")]
    Create(SideArgs),
    #[command(
        about = "Register the counterparty on Cosmos and/or Stellar (both when neither flag is given)"
    )]
    Counterparty(SideArgs),
}

#[derive(clap::Args)]
pub struct SideArgs {
    #[arg(
        long,
        help = "Act on the Stellar ↔ EVM zk corridor instead (create: both light clients; counterparty: the Stellar side)"
    )]
    pub evm: bool,
    #[arg(long, help = "Act on the Cosmos side")]
    pub cosmos: bool,
    #[arg(long, help = "Act on the Stellar side")]
    pub stellar: bool,
    #[arg(
        long,
        help = "Force-recreate even if the id is already set (create only)"
    )]
    pub force: bool,
}

#[derive(clap::Subcommand)]
pub enum ContractsTx {
    #[command(about = "Build + upload (install) every Soroban contract wasm to the network")]
    Upload,
    #[command(
        about = "Deploy soroban contracts (--stellar) or store the contract code (--cosmos)"
    )]
    Deploy(DeployArgs),
    #[command(about = "Invoke a function on a deployed soroban contract (fn + args after `--`)")]
    Invoke(InvokeArgs),
    #[command(
        about = "Register the assets from ipsa/config/assets.json on the deployed transfer contract"
    )]
    RegisterAssets,
}

#[derive(clap::Args)]
pub struct InvokeArgs {
    #[arg(long, help = "Deployed contract id")]
    pub id: String,
    #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
    pub call: Vec<String>,
}

#[derive(clap::Args)]
pub struct DeployArgs {
    #[arg(long, help = "Deploy soroban contracts on Stellar (the default)")]
    pub stellar: bool,
    #[arg(long, help = "Store the contract code on Cosmos (08-wasm)")]
    pub cosmos: bool,
    #[arg(
        long,
        requires = "cosmos",
        help = "With --cosmos: write the store-code request for the public Cosmos testnet (an authz exec file and a gov proposal) instead of storing on the local devnet"
    )]
    pub testnet: bool,
    #[arg(
        long,
        value_name = "KEY",
        requires = "testnet",
        help = "With --testnet: submit the gov proposal from this gaiad key"
    )]
    pub from: Option<String>,
    #[arg(
        long,
        value_name = "NAME",
        help = "Wasm file name without the .wasm extension; omit to act on all contracts"
    )]
    pub contract: Option<String>,
    #[arg(
        long,
        help = "Redeploy even if ROUTER_CONTRACT_ADDRESS is set (stellar, all)"
    )]
    pub force: bool,
    #[arg(long, help = "Also deploy the attestation light client (stellar, all)")]
    pub attestation: bool,

    #[arg(
        long,
        help = "Devnet only: also deploy and register the mock light client, which accepts every proof"
    )]
    pub mock: bool,
}

#[derive(clap::Args)]
pub struct TransferTx {
    #[arg(long, help = "Source address (cosmos or stellar — chain inferred)")]
    pub from: String,
    #[arg(
        long,
        help = "Destination address (cosmos or stellar — chain inferred)"
    )]
    pub to: String,
    #[arg(long, help = "Amount to transfer")]
    pub amount: i128,
    #[arg(long, default_value = "XLM", help = "Token denom to transfer")]
    pub denom: String,
    #[arg(long, default_value_t = 600, help = "Timeout in seconds from now")]
    pub timeout_secs: u64,
    #[arg(long, help = "Skip minting the amount to the sender first")]
    pub no_mint: bool,
    #[arg(
        long,
        help = "Hand the resulting tx to the relayer straight away (nothing is relayed otherwise)"
    )]
    pub relay: bool,
    #[arg(
        long,
        help = "With --relay, block until every packet completes or fails"
    )]
    pub wait: bool,
    #[arg(
        long,
        help = "With --wait, exit non-zero if the packets have not settled by the timeout"
    )]
    pub strict: bool,
    #[arg(
        long,
        help = "Give up waiting after this many seconds (--wait); omit to derive it from the watcher"
    )]
    pub relay_timeout_secs: Option<u64>,
}

enum Sides {
    Cosmos,
    Stellar,
    Both,
}

fn sides(cosmos: bool, stellar: bool) -> Sides {
    match (cosmos, stellar) {
        (true, false) => Sides::Cosmos,
        (false, true) => Sides::Stellar,
        _ => Sides::Both,
    }
}

pub async fn run(cfg: &Config, root: &Path, http: &reqwest::Client, cmd: TxCmd) -> Result<()> {
    match cmd {
        TxCmd::Clients { cmd } => clients_tx(cfg, root, http, cmd).await,
        TxCmd::Contracts { cmd } => contracts_tx(cfg, root, http, cmd).await,
        TxCmd::Transfer(args) => transfer_tx(cfg, root, http, args).await,
        TxCmd::Gmp { cmd } => gmp::run(cfg, root, cmd),
        TxCmd::Relay(args) => relay::run(cfg, http, args).await,
        TxCmd::Query(args) => query_tx(cfg, root, http, args).await,
    }
}

async fn clients_tx(
    cfg: &Config,
    root: &Path,
    http: &reqwest::Client,
    cmd: ClientsTx,
) -> Result<()> {
    let cc = ClientsConfig::from(cfg);

    match &cmd {
        ClientsTx::Create(args) if args.evm => return evm::clients::create(cfg, root).await,
        ClientsTx::Counterparty(args) if args.evm => return evm::clients::counterparty(cfg, root),
        _ => {}
    }

    match cmd {
        ClientsTx::Create(args) => match sides(args.cosmos, args.stellar) {
            Sides::Cosmos => clients::cosmos::run(cfg, &cc, root, http, args.force)
                .await
                .map(|_| ()),
            Sides::Stellar => clients::stellar::run(cfg, &cc, http, args.force)
                .await
                .map(|_| ()),
            Sides::Both => {
                clients::cosmos::run(cfg, &cc, root, http, args.force).await?;
                clients::stellar::run(cfg, &cc, http, args.force).await?;

                Ok(())
            }
        },
        ClientsTx::Counterparty(args) => match sides(args.cosmos, args.stellar) {
            Sides::Cosmos => clients::counterparty::run(cfg, &cc, root, http, "cosmos").await,
            Sides::Stellar => clients::counterparty::run(cfg, &cc, root, http, "stellar").await,
            Sides::Both => {
                clients::counterparty::run(cfg, &cc, root, http, "cosmos").await?;
                clients::counterparty::run(cfg, &cc, root, http, "stellar").await?;

                Ok(())
            }
        },
    }
}

async fn contracts_tx(
    cfg: &Config,
    root: &Path,
    http: &reqwest::Client,
    cmd: ContractsTx,
) -> Result<()> {
    let cc = ContractsConfig::from(cfg);

    match cmd {
        ContractsTx::Upload => contracts::upload_all(&cc, root),
        ContractsTx::Deploy(args) => {
            if args.cosmos {
                return contracts::wasm::upload(root, http, args.testnet, args.from.as_deref())
                    .await;
            }

            match args.contract {
                Some(name) => contracts::deploy_one(&cc, root, &name).map(|_| ()),
                _ => contracts::deploy_all::run(&cc, root, args.force, args.attestation, args.mock)
                    .map(|_| ()),
            }
        }
        ContractsTx::Invoke(args) => {
            let call: Vec<&str> = args.call.iter().map(String::as_str).collect();

            contracts::invoke(&cc, root, &args.id, &call)
        }
        ContractsTx::RegisterAssets => {
            if cfg.deployment.transfer_app.is_empty() {
                bail!("TRANSFER_CONTRACT_ADDRESS is not set in .env — deploy the contracts with `ipsa tx contracts deploy --stellar`, which writes it");
            }

            transfer::assets::register_all(&cc, root, &cfg.deployment.transfer_app)
        }
    }
}

async fn transfer_tx(
    cfg: &Config,
    root: &Path,
    http: &reqwest::Client,
    args: TransferTx,
) -> Result<()> {
    let from = shared::chain_of(&args.from)
        .ok_or_else(|| anyhow!("could not classify --from address {:?}", args.from))?;
    let to = shared::chain_of(&args.to)
        .ok_or_else(|| anyhow!("could not classify --to address {:?}", args.to))?;

    let relay = args.relay;
    let wait = args.wait;
    let strict = args.strict;
    let relay_timeout_secs = args.relay_timeout_secs;

    let params = TransferParams {
        denom: args.denom,
        amount: args.amount,
        receiver: args.to,
        sender: args.from,
        memo: String::new(),
        timeout_secs: args.timeout_secs,
        mint: !args.no_mint,
    };

    let hash = match (from, to) {
        (Chain::Stellar, Chain::Evm) => evm::transfer::stellar_to_evm(cfg, root, &params)?,
        (Chain::Evm, Chain::Stellar) => evm::transfer::evm_to_stellar(cfg, &params).await?,
        (Chain::Stellar, Chain::Cosmos) => transfer::stellar_to_cosmos(cfg, root, &params)?,
        (Chain::Cosmos, Chain::Stellar) => transfer::cosmos_to_stellar(cfg, root, &params)?,
        _ => bail!(
            "transfer must cross chains (stellar↔cosmos or stellar↔evm): got {} → {}",
            from.as_str(),
            to.as_str()
        ),
    };

    if !relay {
        return Ok(());
    }

    let Some(hash) = hash else {
        bail!("--relay was given but no transaction hash was reported — relay it by hand once you have the hash");
    };

    relay::after_transfer(
        cfg,
        http,
        (from, to),
        &hash,
        wait,
        strict,
        relay_timeout_secs,
    )
    .await
}

async fn query_tx(
    cfg: &Config,
    root: &Path,
    http: &reqwest::Client,
    args: QueryArgs,
) -> Result<()> {
    query::run(cfg, root, http, args).await
}
