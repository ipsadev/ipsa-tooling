use std::path::Path;

use anyhow::{bail, Result};

use crate::{
    config::Config, logger, run, services::cosmos::config::COMPOSE_SERVICE, shared, tools,
};

const COSMOS_HOME: &str = "/root/.simapp";
const COSMOS_FUND_AMOUNT: &str = "1000000000stake";

const KEYRING_FLAGS: [&str; 4] = ["--keyring-backend", "test", "--home", COSMOS_HOME];

pub fn show(cfg: &Config) {
    logger::banner("accounts (dedicated sender + receiver per chain)");

    logger::step("Stellar (Soroban) — origin of a stellar→cosmos transfer");
    account_line("sender", &cfg.accounts.stellar_sender_address);
    account_line("receiver", &cfg.accounts.stellar_receiver_address);
    account_line("deployer / admin", &cfg.deployment.deployer_address);
    logger::detail(&format!(
        "relayer signer key: {} (held by ipsa-api, not the gateway)",
        cfg.stellar.key_name
    ));

    logger::step("Cosmos (ibc-go v11 simd) — destination");
    account_line("sender", &cfg.accounts.cosmos_sender_address);
    account_line("receiver", &cfg.accounts.cosmos_receiver_address);
    logger::detail(&format!("relayer signer key: {}", cfg.cosmos.key_name));

    logger::hint(
        "sender & receiver are dedicated accounts, distinct from the deployer/relayer keys",
    );
}

fn account_line(label: &str, address: &str) {
    if address.is_empty() {
        logger::warn(&format!("{label:<18} (not provisioned — run `ipsa start`)"));
    } else {
        logger::ok(&format!("{label:<18} {address}"));
    }
}

pub fn provision(cfg: &Config, root: &Path, force: bool) -> Result<()> {
    logger::banner("accounts (dedicated sender + receiver per chain)");

    provision_stellar(cfg, root, force)?;
    provision_cosmos(cfg, root, force)?;

    logger::ok("accounts ready");
    logger::hint("addresses written to .env (STELLAR_/COSMOS_ SENDER + RECEIVER)");

    Ok(())
}

fn provision_stellar(cfg: &Config, root: &Path, force: bool) -> Result<()> {
    if !run::has("stellar") {
        bail!("stellar CLI not found in PATH — needed to generate sender/receiver identities");
    }

    let sender_id = cfg.accounts.stellar_sender_identity.as_str();
    let receiver_id = cfg.accounts.stellar_receiver_identity.as_str();

    let (sender_addr, sender_secret) = ensure_stellar_identity(cfg, root, sender_id, force)?;
    let (receiver_addr, receiver_secret) = ensure_stellar_identity(cfg, root, receiver_id, force)?;

    logger::ok(&format!("stellar sender   {sender_id} → {sender_addr}"));
    logger::ok(&format!("stellar receiver {receiver_id} → {receiver_addr}"));

    shared::env_upsert(
        crate::config::tooling_env().as_path(),
        &[
            ("STELLAR_SENDER_IDENTITY", sender_id),
            ("STELLAR_SENDER_ADDRESS", sender_addr.as_str()),
            ("STELLAR_SENDER_KEY", sender_secret.as_str()),
            ("STELLAR_RECEIVER_IDENTITY", receiver_id),
            ("STELLAR_RECEIVER_ADDRESS", receiver_addr.as_str()),
            ("STELLAR_RECEIVER_KEY", receiver_secret.as_str()),
        ],
    )?;

    Ok(())
}

fn ensure_stellar_identity(
    cfg: &Config,
    root: &Path,
    name: &str,
    force: bool,
) -> Result<(String, String)> {
    let existing = stellar_address(root, name);

    if let Some(addr) = &existing {
        if !force {
            logger::detail(&format!("stellar identity {name} already exists → reusing"));
            fund_stellar_identity(cfg, root, name);
            let secret = stellar_secret(root, name).unwrap_or_default();

            return Ok((addr.clone(), secret));
        }
    }

    logger::step(&format!("generating + funding stellar identity {name}"));

    let mut args = vec![
        "keys",
        "generate",
        name,
        "--rpc-url",
        cfg.stellar.rpc_url.as_str(),
        "--network-passphrase",
        cfg.stellar.network_passphrase.as_str(),
        "--fund",
    ];

    if existing.is_some() {
        args.push("--overwrite");
    }

    tools::stellar::command(root, &args)?;

    let addr = stellar_address(root, name)
        .ok_or_else(|| anyhow::anyhow!("could not resolve address for stellar identity {name}"))?;
    let secret = stellar_secret(root, name).unwrap_or_default();

    Ok((addr, secret))
}

fn fund_stellar_identity(cfg: &Config, root: &Path, name: &str) {
    let funded = tools::stellar::capture_all(
        root,
        &[
            "keys",
            "fund",
            name,
            "--rpc-url",
            cfg.stellar.rpc_url.as_str(),
            "--network-passphrase",
            cfg.stellar.network_passphrase.as_str(),
        ],
    );

    if funded.is_err() {
        logger::detail(&format!(
            "{name} already funded (or friendbot declined) — continuing"
        ));
    }
}

fn stellar_address(root: &Path, name: &str) -> Option<String> {
    tools::stellar::capture_quiet(root, &["keys", "public-key", name])
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

fn stellar_secret(root: &Path, name: &str) -> Option<String> {
    tools::stellar::capture_quiet(root, &["keys", "secret", name])
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

fn provision_cosmos(cfg: &Config, root: &Path, force: bool) -> Result<()> {
    if !cfg.cosmos.is_local() {
        logger::detail("cosmos is not the local devnet — skipping keyring provisioning");

        return Ok(());
    }

    let sender_name = cfg.accounts.cosmos_sender_key_name.as_str();
    let receiver_name = cfg.accounts.cosmos_receiver_key_name.as_str();

    let (sender_addr, sender_created) = ensure_cosmos_key(root, sender_name, force)?;
    let (receiver_addr, _receiver_created) = ensure_cosmos_key(root, receiver_name, force)?;

    if sender_created {
        fund_cosmos(cfg, root, &sender_addr)?;
    }

    logger::ok(&format!("cosmos sender   {sender_name} → {sender_addr}"));
    logger::ok(&format!(
        "cosmos receiver {receiver_name} → {receiver_addr} (unfunded — only receives the voucher)"
    ));

    shared::env_upsert(
        crate::config::tooling_env().as_path(),
        &[
            ("COSMOS_SENDER_KEY_NAME", sender_name),
            ("COSMOS_SENDER_ADDRESS", sender_addr.as_str()),
            ("COSMOS_RECEIVER_KEY_NAME", receiver_name),
            ("COSMOS_RECEIVER_ADDRESS", receiver_addr.as_str()),
        ],
    )?;

    Ok(())
}

fn ensure_cosmos_key(root: &Path, name: &str, force: bool) -> Result<(String, bool)> {
    let existing = cosmos_key_address(root, name);

    if let Some(addr) = existing {
        if !force {
            logger::detail(&format!("cosmos key {name} already exists → reusing"));

            return Ok((addr, false));
        }

        simd(root, &["keys", "delete", name, "--yes"])?;
    }

    logger::step(&format!("adding cosmos key {name}"));
    simd(root, &["keys", "add", name])?;

    let addr = cosmos_key_address(root, name)
        .ok_or_else(|| anyhow::anyhow!("could not resolve address for cosmos key {name}"))?;

    Ok((addr, true))
}

fn cosmos_key_address(root: &Path, name: &str) -> Option<String> {
    simd(root, &["keys", "show", name, "-a"])
        .ok()
        .map(|out| {
            out.lines()
                .map(str::trim)
                .find(|line| line.starts_with("cosmos"))
                .unwrap_or("")
                .to_string()
        })
        .filter(|s| !s.is_empty())
}

fn fund_cosmos(cfg: &Config, root: &Path, address: &str) -> Result<()> {
    let chain_id = cfg.cosmos.chain_id.as_str();
    let from = cfg.cosmos.key_name.as_str();

    logger::step(&format!(
        "funding {address} with {COSMOS_FUND_AMOUNT} from {from}"
    ));

    simd(
        root,
        &[
            "tx",
            "bank",
            "send",
            from,
            address,
            COSMOS_FUND_AMOUNT,
            "--chain-id",
            chain_id,
            "--gas",
            "auto",
            "--gas-adjustment",
            "1.5",
            "--gas-prices",
            "0.025stake",
            "--broadcast-mode",
            "sync",
            "--yes",
        ],
    )?;

    Ok(())
}

fn simd(root: &Path, args: &[&str]) -> Result<String> {
    let mut full = vec!["compose", "exec", "-T", COMPOSE_SERVICE, "simd"];
    full.extend_from_slice(args);
    full.extend_from_slice(&KEYRING_FLAGS);

    tools::docker::capture_all(root, &full)
}
