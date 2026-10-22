mod caller;
mod evm;

use std::{
    path::Path,
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::{bail, Result};

use crate::{
    config::{self, Config},
    logger, shared,
    shared::Chain,
    tx::{
        contracts::{self, config::ContractsConfig, deploy_all, release, upload},
        relay,
    },
};

const SOLIDITY_ABI: &str = "application/x-solidity-abi";

const JSON: &str = "application/json";

const EVM_DEFAULT_TIMEOUT_SECS: u64 = 6 * 60 * 60;

const EVM_MAX_TIMEOUT_SECS: u64 = 24 * 60 * 60;

#[derive(clap::Subcommand)]
pub enum GmpTx {
    #[command(
        about = "Show the account a (client, sender, salt) triplet derives to, before it exists"
    )]
    Account(AccountArgs),

    #[command(about = "Send a general message to a registered gmpport on the counterparty")]
    Call(Box<CallArgs>),

    #[command(
        about = "The ipsa-gmp-caller contract, which sends calls as itself and records their outcome"
    )]
    Caller {
        #[command(subcommand)]
        cmd: caller::CallerCmd,
    },

    #[command(
        about = "Deploy the GMP app from the CONTRACTS_VERSION release and move the router's gmpport to it"
    )]
    Replace,
}

#[derive(clap::Args)]
pub struct AccountArgs {
    #[arg(
        long,
        help = "Client id on the destination chain; defaults to EVM_STELLAR_CLIENT_ID with --evm"
    )]
    pub client_id: Option<String>,
    #[arg(long, help = "Sender identifier on the source chain")]
    pub sender: String,
    #[arg(long, default_value = "", help = "Hex salt distinguishing accounts")]
    pub salt: String,
    #[arg(
        long,
        help = "Ask ICS27GMP on the EVM chain (EVM_ICS27_GMP) instead of the Stellar GMP app"
    )]
    pub evm: bool,
}

#[derive(clap::Args)]
pub struct CallArgs {
    #[arg(
        long,
        help = "Stellar account originating the call; not needed with --via-caller"
    )]
    pub from: Option<String>,
    #[arg(
        long,
        help = "Source client id; defaults to STELLAR_EVM_CLIENT_ID for a 0x receiver"
    )]
    pub client_id: Option<String>,
    #[arg(
        long,
        help = "Destination contract: a 0x address on an EVM chain, or a destination-specific id"
    )]
    pub receiver: String,
    #[arg(long, default_value = "", help = "Hex salt distinguishing accounts")]
    pub salt: String,
    #[arg(
        long,
        help = "Hex call payload, interpreted by the destination runtime"
    )]
    pub payload: Option<String>,
    #[arg(
        long,
        help = "Solidity function to call on an EVM receiver, e.g. \"set(uint256)\"; builds the payload"
    )]
    pub evm_call: Option<String>,
    #[arg(long, num_args = 0.., help = "Arguments for --evm-call, in order")]
    pub evm_args: Vec<String>,
    #[arg(
        long,
        help = "Seconds until the packet times out; defaults to 600, or 6 hours for an EVM receiver"
    )]
    pub timeout_secs: Option<u64>,
    #[arg(long, default_value = "", help = "Optional metadata for middleware")]
    pub memo: String,
    #[arg(
        long,
        help = "Packet encoding; defaults to application/x-solidity-abi for a 0x receiver, else application/json"
    )]
    pub encoding: Option<String>,
    #[arg(
        long,
        help = "Send through ipsa-gmp-caller (GMP_CALLER_ADDRESS), so the outcome is recorded"
    )]
    pub via_caller: bool,
    #[arg(long, help = "Hand the transaction to the relayer straight away")]
    pub relay: bool,
    #[arg(long, help = "With --relay, wait until the packet completes or fails")]
    pub wait: bool,
    #[arg(
        long,
        help = "With --wait, exit non-zero if the packet has not settled"
    )]
    pub strict: bool,
}

pub async fn run(cfg: &Config, root: &Path, http: &reqwest::Client, cmd: GmpTx) -> Result<()> {
    match cmd {
        GmpTx::Account(args) => account(cfg, root, args).await,
        GmpTx::Call(args) => call(cfg, root, http, *args).await,
        GmpTx::Caller { cmd } => caller::run(cfg, root, cmd),
        GmpTx::Replace => replace(cfg, root),
    }
}

pub(crate) fn app(cfg: &Config) -> Result<String> {
    let app = cfg.deployment.gmp_app.clone();

    if app.is_empty() {
        bail!("GMP_CONTRACT_ADDRESS is not set in .env — deploy the contracts with `ipsa tx contracts deploy --stellar`, which writes it");
    }

    Ok(app)
}

async fn account(cfg: &Config, root: &Path, args: AccountArgs) -> Result<()> {
    let salt = normalise_hex(&args.salt, "salt")?;

    if args.evm {
        logger::banner("gmp account on the evm chain (derived, read only)");

        let gmp = config::get("EVM_ICS27_GMP", "");

        if gmp.is_empty() {
            bail!("EVM_ICS27_GMP is not set in .env");
        }

        let mut derive_cfg_client = cfg.evm.evm_client_id.clone();
        if let Some(client) = &args.client_id {
            derive_cfg_client = client.clone();
        }

        logger::detail(&format!(
            "ICS27GMP derives it from (client {derive_cfg_client}, sender {}, salt {salt:?}); calls from that sender run as it",
            args.sender
        ));

        let address = evm::account_address(
            cfg,
            &gmp,
            &derive_cfg_client,
            &args.sender,
            &hex::decode(&salt)?,
        )
        .await?;

        logger::ok(&address.to_string());
        println!("{address}");

        return Ok(());
    }

    logger::banner("gmp account (derived, read only)");

    let app = app(cfg)?;

    let Some(client_id) = args.client_id else {
        bail!("--client-id is required for a Stellar account");
    };

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
            &client_id,
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

async fn call(cfg: &Config, root: &Path, http: &reqwest::Client, args: CallArgs) -> Result<()> {
    let to_evm = args.receiver.trim().starts_with("0x");
    let destination = if to_evm { Chain::Evm } else { Chain::Cosmos };

    logger::banner(&format!(
        "gmp call (stellar → {})",
        if to_evm { "evm" } else { "counterparty" }
    ));

    let encoding = args
        .encoding
        .clone()
        .unwrap_or_else(|| if to_evm { SOLIDITY_ABI } else { JSON }.to_string());

    let client_id = match (&args.client_id, to_evm) {
        (Some(client), _) => client.clone(),
        (None, true) => cfg.evm.stellar_client_id.clone(),
        (None, false) => bail!("--client-id is required for a non-EVM receiver"),
    };

    let payload = match (&args.payload, &args.evm_call) {
        (Some(_), Some(_)) => bail!("give --payload or --evm-call, not both"),
        (Some(raw), None) => normalise_hex(raw, "payload")?,
        (None, Some(signature)) => hex::encode(evm::calldata(signature, &args.evm_args)?),
        (None, None) => bail!(
            "--payload or --evm-call is required; it is the call the destination will execute"
        ),
    };

    if payload.is_empty() {
        bail!("the payload is empty; it is the call the destination will execute");
    }

    let salt = normalise_hex(&args.salt, "salt")?;

    let timeout_secs = match (args.timeout_secs, to_evm) {
        (Some(seconds), true) => seconds.clamp(EVM_DEFAULT_TIMEOUT_SECS, EVM_MAX_TIMEOUT_SECS),
        (Some(seconds), false) => seconds,
        (None, true) => EVM_DEFAULT_TIMEOUT_SECS,
        (None, false) => 600,
    };

    let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();
    let timeout = (now + timeout_secs).to_string();

    let salt = format!("\"{salt}\"");
    let memo = format!("\"{}\"", args.memo);

    let call = [
        "--source_client_id",
        client_id.as_str(),
        "--receiver",
        args.receiver.as_str(),
        "--salt",
        salt.as_str(),
        "--payload",
        payload.as_str(),
        "--timeout_timestamp",
        timeout.as_str(),
        "--memo",
        memo.as_str(),
        "--encoding",
        encoding.as_str(),
    ];

    let (contract, sender_flags): (String, Vec<&str>) = if args.via_caller {
        (caller::address()?, vec!["send_call"])
    } else {
        let Some(from) = args.from.as_deref() else {
            bail!("--from is required unless --via-caller is given");
        };

        (app(cfg)?, vec!["send_call", "--sender", from])
    };

    let mut invocation = sender_flags;
    invocation.extend_from_slice(&call);

    logger::step(&format!(
        "send_call client={client_id} receiver={} encoding={encoding} timeout={timeout}{}",
        args.receiver,
        if args.via_caller {
            " (through the caller)"
        } else {
            ""
        }
    ));

    let output = contracts::invoke_as(
        &ContractsConfig::from(cfg),
        root,
        &contract,
        &invocation,
        &cfg.stellar.signing_key,
    )?;

    let sequence = output
        .lines()
        .rev()
        .map(|line| line.trim().trim_matches('"'))
        .find(|line| !line.is_empty() && line.chars().all(|c| c.is_ascii_digit()))
        .unwrap_or("unknown")
        .to_string();
    let hash = contracts::tx_hash(&output);

    logger::ok(&format!("sequence {sequence}"));

    let Some(hash) = hash else {
        logger::warn(
            "the stellar CLI printed no transaction hash — find it on the explorer to relay it",
        );
        return Ok(());
    };

    logger::ok(&format!("tx {hash}"));
    println!("{hash}");

    if args.via_caller {
        logger::hint(&format!(
            "outcome: ipsa tx gmp caller outcome --sequence {sequence} --client-id {client_id}"
        ));
    }

    if !args.relay {
        logger::hint(&format!(
            "relay it: ipsa tx relay --tx {hash}{}",
            if to_evm { " --evm" } else { " --stellar" }
        ));
        return Ok(());
    }

    relay::after_transfer(
        cfg,
        http,
        (Chain::Stellar, destination),
        &hash,
        args.wait,
        args.strict,
        None,
    )
    .await
}

fn replace(cfg: &Config, root: &Path) -> Result<()> {
    logger::banner("gmp replace (deploy the release's gmp app and move gmpport to it)");

    let cc = ContractsConfig::from(cfg);
    let router = cfg.deployment.ibc_router.clone();

    if router.is_empty() {
        bail!("ROUTER_CONTRACT_ADDRESS is not set in .env");
    }

    let directory = release::fetch(root)?;
    let owner = deploy_all::deployer_address(&cc)?;

    logger::step("deploying stellar_ibc_gmp");
    let gmp = contracts::deploy(
        &cc,
        root,
        &directory.join("stellar_ibc_gmp.wasm").display().to_string(),
        &["--owner", &owner, "--router", &router],
    )?;
    logger::ok(&format!("gmp app: {gmp}"));

    logger::step("uploading ipsa_gmp_account and setting it on the app");
    let account = upload::hash(
        &cc,
        root,
        &directory
            .join("ipsa_gmp_account.wasm")
            .display()
            .to_string(),
    )?;
    contracts::invoke(
        &cc,
        root,
        &gmp,
        &["set_account_wasm", "--wasm_hash", &account],
    )?;

    logger::step(&format!("moving gmpport on {router} to {gmp}"));
    contracts::invoke(
        &cc,
        root,
        &router,
        &[
            "reassign_port",
            "--port_id",
            "gmpport",
            "--app_address",
            &gmp,
        ],
    )?;

    shared::env_upsert(
        &config::tooling_env(),
        &[("GMP_CONTRACT_ADDRESS", gmp.as_str())],
    )?;

    logger::ok(&format!(
        "gmpport now routes to {gmp} (GMP_CONTRACT_ADDRESS)"
    ));
    logger::hint("a caller is bound to its gmp app: redeploy it with `ipsa tx gmp caller deploy`");

    Ok(())
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
