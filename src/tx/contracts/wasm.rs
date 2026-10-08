use std::{
    path::{Path, PathBuf},
    time::Duration,
};

use anyhow::{bail, Context, Result};
use base64::Engine;
use sha2::{Digest, Sha256};

use crate::{logger, run, services::cosmos::tx::CosmosSigner, tools};

const DEPOSIT_AMOUNT: u64 = 10_000_000;
const STORE_GAS_LIMIT: u64 = 150_000_000;
const STORE_FEE_AMOUNT: u64 = 4_500_000;
const VOTE_OPTION: u64 = 1;
const VOTE_GAS_LIMIT: u64 = 200_000;
const VOTE_FEE_AMOUNT: u64 = 10_000;
const TX_WAIT_TIMEOUT_SECS: u64 = 60;
const VOTING_PERIOD_SECS: u64 = 25;
const VERIFY_RETRIES: u32 = 15;
const VERIFY_INTERVAL_SECS: u64 = 2;
const FUND_AMOUNT: u64 = 100_000_000;
const FUND_GAS_LIMIT: u64 = 200_000;
const FUND_FEE_AMOUNT: u64 = 10_000;

const FETCH_SCRIPT: &str = include_str!("../../../scripts/fetch-wasm.sh");
const GOV_AUTHORITY: &str = "cosmos10d07y265gmmuvt4z0w9aw880jnsr700j6zn9kn";
const TESTNET_DEPOSIT_UATOM: u64 = 50_000_000;

const METADATA_FILE: &str = "provider-store-code-metadata.json";

pub async fn upload(
    root: &Path,
    http: &reqwest::Client,
    testnet: bool,
    from: Option<&str>,
) -> Result<()> {
    if testnet {
        return prepare_testnet_proposal(root, from);
    }

    logger::banner("contracts upload-wasm (light client -> Cosmos 08-wasm)");

    let (bytes, local_sha) = build_wasm(root)?;

    let cosmos = crate::services::cosmos::config::CosmosConfig::devnet();
    let signer = CosmosSigner::from_config(&cosmos, http.clone())?;

    if !signer.node_info_ok().await {
        logger::warn(&format!(
            "cosmos REST not reachable at {} — start it with: ipsa cosmos start",
            cosmos.rest_url
        ));

        return Ok(());
    }

    if checksum_registered(&signer, local_sha.as_str()).await {
        logger::ok(&format!(
            "light-client wasm already uploaded (checksum {local_sha}), skipping store-code proposal"
        ));
    } else {
        let relayer = signer.relayer_address()?.to_string();
        logger::step(&format!("funding relayer {relayer}"));
        if signer
            .fund_account(
                &relayer,
                FUND_AMOUNT as u128,
                FUND_GAS_LIMIT,
                FUND_FEE_AMOUNT as u128,
                true,
            )
            .await?
        {
            logger::ok("relayer funded");
        } else {
            logger::detail("relayer already has an account, skipping funding");
        }

        logger::step("submitting store-code proposal");
        let proposal_id = signer
            .submit_store_code(
                bytes,
                "upload-light-client".to_string(),
                "Registers stellar_light_client.wasm for the 08-wasm client type".to_string(),
                DEPOSIT_AMOUNT as u128,
                STORE_GAS_LIMIT,
                STORE_FEE_AMOUNT as u128,
                Duration::from_secs(TX_WAIT_TIMEOUT_SECS),
            )
            .await?;
        logger::ok(&format!("proposal id: {proposal_id}"));

        logger::step(&format!("voting YES on proposal {proposal_id}"));
        signer
            .vote(
                proposal_id,
                VOTE_OPTION as i32,
                VOTE_GAS_LIMIT,
                VOTE_FEE_AMOUNT as u128,
            )
            .await?;

        logger::detail(&format!(
            "waiting {VOTING_PERIOD_SECS}s for the voting period"
        ));
        tokio::time::sleep(Duration::from_secs(VOTING_PERIOD_SECS)).await;

        logger::step("verifying checksum on-chain");
        if !checksum_registered(&signer, local_sha.as_str()).await {
            bail!("local sha256 {local_sha} did not appear in on-chain checksums (proposal may not have passed)");
        }
        logger::ok(&format!("wasm registered with checksum {local_sha}"));
    }

    crate::shared::env_upsert(
        &crate::config::tooling_env(),
        &[("LIGHT_CLIENT_WASM_CHECKSUM", local_sha.as_str())],
    )?;
    logger::detail(&format!("LIGHT_CLIENT_WASM_CHECKSUM={local_sha}"));

    Ok(())
}

pub fn build(root: &Path) -> Result<()> {
    logger::banner("contracts build --cosmos (light-client wasm)");

    build_wasm(root).map(|_| ())
}

fn build_wasm(root: &Path) -> Result<(Vec<u8>, String)> {
    let local = crate::config::get("LIGHT_CLIENT_WASM_DIR", "");

    let dist = if local.trim().is_empty() {
        fetch_release(root)?
    } else {
        logger::step(&format!(
            "light client from LIGHT_CLIENT_WASM_DIR={}",
            local.trim()
        ));

        PathBuf::from(local.trim())
    };

    verify_checksums(&dist)?;

    let wasm_file = dist.join("stellar_light_client.wasm");
    let gz_file = dist.join("stellar_light_client.wasm.gz");

    for path in [&wasm_file, &gz_file] {
        if !path.exists() {
            bail!("expected release artifact not found at {}", path.display());
        }
    }

    let plain =
        std::fs::read(&wasm_file).with_context(|| format!("reading {}", wasm_file.display()))?;
    let gz = std::fs::read(&gz_file).with_context(|| format!("reading {}", gz_file.display()))?;

    let sha = hex::encode(Sha256::digest(&plain));
    logger::detail(&format!(
        "{} bytes uncompressed, {} bytes gzipped, on-chain sha256={sha}",
        plain.len(),
        gz.len()
    ));

    Ok((gz, sha))
}

struct Release {
    repository: String,
    tag: String,
}

impl Release {
    fn from_env() -> Result<Self> {
        Ok(Self {
            repository: crate::config::required("LIGHT_CLIENT_REPOSITORY")?,
            tag: crate::config::required("LIGHT_CLIENT_VERSION")?,
        })
    }

    fn name(&self) -> &str {
        self.repository
            .rsplit('/')
            .next()
            .unwrap_or(&self.repository)
    }
}

fn fetch_release(root: &Path) -> Result<PathBuf> {
    let dist = root.join("dist");
    let dist_text = dist.display().to_string();
    let release = Release::from_env()?;

    logger::step(&format!(
        "fetch-wasm ({} {} release, checksum verified)",
        release.repository, release.tag
    ));

    std::fs::create_dir_all(&dist).with_context(|| format!("creating {dist_text}"))?;

    let script = dist.join("fetch-wasm.sh");

    std::fs::write(&script, FETCH_SCRIPT)
        .with_context(|| format!("writing {}", script.display()))?;

    let script_text = script.display().to_string();

    run::command(
        root,
        "bash",
        &[
            script_text.as_str(),
            dist_text.as_str(),
            release.tag.as_str(),
            release.repository.as_str(),
        ],
    )?;

    Ok(dist)
}

fn verify_checksums(dist: &Path) -> Result<()> {
    let listing = dist.join("checksums.txt");
    let checksums = std::fs::read_to_string(&listing)
        .with_context(|| format!("reading {}", listing.display()))?;

    for line in checksums.lines().filter(|line| !line.trim().is_empty()) {
        let Some((expected, name)) = line.split_once(char::is_whitespace) else {
            bail!("{} has a malformed line: {line}", listing.display());
        };

        let file = dist.join(name.trim().trim_start_matches('*'));
        let bytes = std::fs::read(&file).with_context(|| format!("reading {}", file.display()))?;
        let actual = hex::encode(Sha256::digest(&bytes));

        if actual != expected {
            bail!(
                "{} does not match {}: expected {expected}, got {actual}",
                file.display(),
                listing.display()
            );
        }
    }

    Ok(())
}

fn proposal_summary(checksum: &str, release: &Release) -> String {
    let repository = &release.repository;
    let tag = &release.tag;
    let name = release.name();

    format!(
        "Stores the wasm byte code for a Stellar light client, so that clients of type 08-wasm \
tracking Stellar can be created on this chain. Storing code does not create a client and grants no \
permissions.\n\n\
Stellar runs the Stellar Consensus Protocol, which has no global validator set and no fixed \
threshold: each node declares the quorum slices it trusts. The client accepts a Stellar ledger only \
when signed EXTERNALIZE statements for it satisfy the quorum configuration pinned in the client \
state, checked in eight steps: envelope signatures, statement validation, quorum-set \
authentication, the quorum check, value agreement, ledger binding, next-slot binding and \
result-set binding. Packet membership is then proven against the root the router publishes in that \
ledger.\n\n\
Artifact: https://github.com/{repository}/releases/tag/{tag}\n\
sha256 (uncompressed, as recorded on chain): {checksum}\n\n\
Reproduce it:\n\
  git clone --branch {tag} https://github.com/{repository}\n\
  cd {name}\n\
  ./scripts/release-build.sh dist\n\
  cat dist/checksums.txt\n\n\
The build runs in a pinned container so the result does not depend on the rebuilding machine. The \
contract implements the full 08-wasm sudo surface including migrate_client_store, so a client \
created from this code can be migrated or recovered rather than abandoned.\n\n\
This is a testnet request. The code has not yet had a third-party audit.\n\n\
Runbook: https://github.com/{repository}/blob/{tag}/README.md"
    )
}

fn unsigned_tx(message: serde_json::Value) -> serde_json::Value {
    serde_json::json!({
        "body": {
            "messages": [message],
            "memo": "",
            "timeout_height": "0",
            "extension_options": [],
            "non_critical_extension_options": []
        },
        "auth_info": {
            "signer_infos": [],
            "fee": { "amount": [], "gas_limit": "0", "payer": "", "granter": "" }
        },
        "signatures": []
    })
}

fn prepare_testnet_proposal(root: &Path, from: Option<&str>) -> Result<()> {
    logger::banner("contracts upload-wasm --testnet (08-wasm store-code gov proposal)");

    let testnet = crate::services::cosmos::config::CosmosConfig::testnet();
    let (bytes, sha) = build_wasm(root)?;
    let wasm_b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);

    let authority = crate::config::get("COSMOS_TESTNET_GOV_AUTHORITY", GOV_AUTHORITY);
    let store_msg = serde_json::json!({
        "@type": "/ibc.lightclients.wasm.v1.MsgStoreCode",
        "signer": authority,
        "wasm_byte_code": wasm_b64,
    });

    let release = Release::from_env()?;
    let title = "Store the Stellar (SCP) light client wasm";
    let summary = proposal_summary(&sha, &release);
    let release_url = format!(
        "https://github.com/{}/releases/tag/{}",
        release.repository, release.tag
    );
    let metadata_url = crate::config::get(
        "COSMOS_PROPOSAL_METADATA",
        &format!(
            "https://github.com/{}/releases/download/{}/{METADATA_FILE}",
            release.repository, release.tag
        ),
    );

    let metadata = serde_json::json!({
        "title": title,
        "authors": ["ipsa"],
        "summary": summary,
        "details": format!("Release: {release_url}\nsha256 of the uncompressed wasm, as recorded on chain: {sha}"),
        "proposal_forum_url": crate::config::get("COSMOS_PROPOSAL_FORUM_URL", &release_url),
        "vote_option_context": "Yes stores the code. Storing code creates no client and grants no permission.",
    });

    let metadata_path = root.join(METADATA_FILE);
    std::fs::write(&metadata_path, serde_json::to_string_pretty(&metadata)?)
        .with_context(|| format!("writing {}", metadata_path.display()))?;
    logger::ok(&format!(
        "wrote {} — publish it at {metadata_url}",
        metadata_path.display()
    ));

    let proposal = serde_json::json!({
        "messages": [store_msg.clone()],
        "metadata": metadata_url,
        "title": title,
        "summary": summary,
        "deposit": format!("{TESTNET_DEPOSIT_UATOM}uatom"),
    });

    let authz_path = root.join("provider-store-code.json");
    std::fs::write(
        &authz_path,
        serde_json::to_string_pretty(&unsigned_tx(store_msg))?,
    )
    .with_context(|| format!("writing {}", authz_path.display()))?;
    logger::ok(&format!(
        "wrote {} for an authz holder: gaiad tx authz exec {} --from <grantee>",
        authz_path.display(),
        authz_path.display()
    ));

    let proposal_path = root.join("provider-store-code-proposal.json");
    std::fs::write(&proposal_path, serde_json::to_string_pretty(&proposal)?)
        .with_context(|| format!("writing {}", proposal_path.display()))?;
    logger::ok(&format!("wrote {}", proposal_path.display()));

    let rpc = testnet.rpc_url.as_str();
    let chain_id = testnet.chain_id.as_str();
    let keyring = crate::config::get("COSMOS_TESTNET_KEYRING_BACKEND", "test");
    let path = proposal_path.display().to_string();

    match resolve_testnet_key(root, from, &keyring)? {
        Some(key) => {
            logger::step(&format!(
                "gaiad tx gov submit-proposal (from={key}, node={rpc}, chain-id={chain_id})"
            ));
            tools::gaiad::command(
                root,
                &[
                    "tx",
                    "gov",
                    "submit-proposal",
                    path.as_str(),
                    "--from",
                    key.as_str(),
                    "--keyring-backend",
                    keyring.as_str(),
                    "--node",
                    rpc,
                    "--chain-id",
                    chain_id,
                    "--gas",
                    "auto",
                    "--gas-adjustment",
                    "1.5",
                    "--gas-prices",
                    "0.025uatom",
                    "-y",
                ],
            )?;
            logger::ok("proposal submitted");
        }
        None => {
            logger::step("submit (set COSMOS_TESTNET_MNEMONIC, or pass --from <gaiad-key>, to run this for you):");
            println!("  gaiad tx gov submit-proposal {path} \\");
            println!("    --from <key> --keyring-backend {keyring} --node {rpc} --chain-id {chain_id} \\");
            println!("    --gas auto --gas-adjustment 1.5 --gas-prices 0.025uatom -y");
        }
    }

    println!();
    logger::step("it must pass governance, then confirm + wire it up:");
    println!("  gaiad query ibc-wasm checksums --node {rpc}");
    logger::step(&format!(
        "set it in .env:  LIGHT_CLIENT_WASM_CHECKSUM={sha}"
    ));

    if let Some(faucet) = &testnet.faucet_url {
        logger::hint(&format!("fund your key first via the faucet: {faucet}"));
    }
    logger::warn(
        "provider is ibc-go v10 / wasmvm 2.x — confirm the wasm + 08-wasm ABI before spending on the proposal",
    );

    Ok(())
}

fn resolve_testnet_key(root: &Path, from: Option<&str>, keyring: &str) -> Result<Option<String>> {
    if let Some(key) = from {
        if !run::has("gaiad") {
            bail!("gaiad not found in PATH — install Gaia v27 to submit");
        }

        return Ok(Some(key.to_string()));
    }

    let mnemonic = crate::config::get("COSMOS_TESTNET_MNEMONIC", "");
    if mnemonic.trim().is_empty() {
        return Ok(None);
    }

    if !run::has("gaiad") {
        bail!("gaiad not found in PATH — install Gaia v27 to use the env mnemonic");
    }

    let key_name = crate::config::get("COSMOS_TESTNET_KEY_NAME", "ipsa-cosmos");
    ensure_gaiad_key(root, &key_name, mnemonic.trim(), keyring)?;

    Ok(Some(key_name))
}

fn ensure_gaiad_key(root: &Path, name: &str, mnemonic: &str, keyring: &str) -> Result<()> {
    if tools::gaiad::capture_quiet(
        root,
        &["keys", "show", name, "-a", "--keyring-backend", keyring],
    )
    .is_ok()
    {
        logger::detail(&format!("gaiad key '{name}' already in the keyring"));

        return Ok(());
    }

    logger::step(&format!(
        "importing mnemonic into the gaiad keyring as '{name}' (keyring-backend {keyring})"
    ));
    tools::gaiad::piped(
        root,
        &[
            "keys",
            "add",
            name,
            "--recover",
            "--keyring-backend",
            keyring,
        ],
        &format!("{mnemonic}\n"),
    )?;

    Ok(())
}

async fn checksum_registered(signer: &CosmosSigner, local_sha: &str) -> bool {
    for attempt in 1..=VERIFY_RETRIES {
        if let Ok(checksums) = signer.checksums().await {
            if checksums.iter().any(|c| c.eq_ignore_ascii_case(local_sha)) {
                return true;
            }
        }

        logger::detail(&format!(
            "attempt {attempt}/{VERIFY_RETRIES}: not yet on-chain, retrying in {VERIFY_INTERVAL_SECS}s"
        ));
        tokio::time::sleep(Duration::from_secs(VERIFY_INTERVAL_SECS)).await;
    }

    false
}
