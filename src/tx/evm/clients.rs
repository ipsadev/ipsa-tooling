use std::{collections::HashMap, path::Path};

use alloy::{
    network::TransactionBuilder,
    primitives::{Address, Bytes},
    providers::{Provider, ProviderBuilder},
    rpc::types::TransactionRequest,
    signers::local::PrivateKeySigner,
    sol_types::SolCall,
};
use anyhow::{anyhow, bail, Context, Result};
use serde_json::Value;

use crate::{
    config::Config,
    gateway::proto::proofapi::CreateClientRequest,
    logger,
    services::relayer::wiring,
    shared, tools,
    tx::{
        contracts::{self, config::ContractsConfig},
        evm::{config::required, relay::proof_api, soroban},
    },
};

alloy::sol! {
    struct CounterpartyInfo {
        string clientId;
        bytes[] merklePrefix;
    }

    function addClient(string clientId, CounterpartyInfo counterpartyInfo, address client) external returns (string);

    function getClient(string clientId) external view returns (address);

    function getCounterparty(string clientId) external view returns (CounterpartyInfo);
}

const MAX_CLIENT_ID_PROBES: u32 = 100;

pub async fn create(cfg: &Config, root: &Path) -> Result<()> {
    logger::banner("tx clients create (the Stellar ↔ EVM zk light clients)");

    create_pair(cfg, root, None).await?;

    logger::hint("register the Stellar side's counterparty: ipsa tx clients counterparty --evm");

    Ok(())
}

pub async fn ensure(cfg: &Config, root: &Path) -> Result<bool> {
    let corridor = &cfg.evm;

    logger::banner("evm clients (the Stellar ↔ EVM zk light clients)");

    if paired(cfg, root).await? {
        logger::ok(&format!(
            "{} on Stellar and {} on the EVM router already name each other",
            corridor.stellar_client_id, corridor.evm_client_id
        ));

        return Ok(false);
    }

    let reusable = unpaired_stellar_client(cfg, root)?;
    let (stellar_client, evm_client) = create_pair(cfg, root, reusable).await?;

    register_on_stellar(cfg, root, &stellar_client, &evm_client)?;

    Ok(true)
}

async fn create_pair(
    cfg: &Config,
    root: &Path,
    reusable: Option<String>,
) -> Result<(String, String)> {
    let evm_client = free_evm_client_id(cfg).await?;

    let stellar_client = match reusable {
        Some(existing) => {
            logger::ok(&format!(
                "{existing} already exists on the Stellar router with no counterparty — reusing it"
            ));

            existing
        }
        None => {
            let created = create_on_stellar(cfg, root).await?;

            shared::env_upsert(
                &crate::config::tooling_env(),
                &[("STELLAR_EVM_CLIENT_ID", created.as_str())],
            )?;

            created
        }
    };

    create_on_evm(cfg, &stellar_client, &evm_client).await?;
    record(cfg, &stellar_client, &evm_client)?;

    Ok((stellar_client, evm_client))
}

fn unpaired_stellar_client(cfg: &Config, root: &Path) -> Result<Option<String>> {
    let corridor = &cfg.evm;

    if corridor.stellar_client_id.is_empty() {
        return Ok(None);
    }

    let router = required(&corridor.stellar_router, "STELLAR_EVM_ROUTER")?;
    let contracts_config = ContractsConfig::from(cfg);
    let read = |call: &[&str]| -> Result<Value> {
        let output = contracts::read(
            &contracts_config,
            root,
            router,
            call,
            &cfg.stellar.cli_identity,
        )?;

        Ok(serde_json::from_str(output.trim()).unwrap_or(Value::Null))
    };

    let light_client = read(&[
        "light_client_address",
        "--client_id",
        &corridor.stellar_client_id,
    ])?;
    let client_type = read(&[
        "client_type_address",
        "--client_type",
        &corridor.stellar_client_type,
    ])?;

    if light_client.is_null() || light_client != client_type {
        return Ok(None);
    }

    let counterparty = read(&["counterparty", "--client_id", &corridor.stellar_client_id])?;

    Ok(counterparty
        .is_null()
        .then(|| corridor.stellar_client_id.clone()))
}

async fn paired(cfg: &Config, root: &Path) -> Result<bool> {
    let corridor = &cfg.evm;

    if corridor.stellar_client_id.is_empty() || corridor.evm_client_id.is_empty() {
        return Ok(false);
    }

    let router = required(&corridor.stellar_router, "STELLAR_EVM_ROUTER")?;
    let output = contracts::read(
        &ContractsConfig::from(cfg),
        root,
        router,
        &["counterparty", "--client_id", &corridor.stellar_client_id],
        &cfg.stellar.cli_identity,
    )?;
    let named: Value = serde_json::from_str(output.trim()).unwrap_or(Value::Null);

    if named.get("client_id").and_then(Value::as_str) != Some(corridor.evm_client_id.as_str()) {
        return Ok(false);
    }

    let counterparty = evm_counterparty(cfg, &corridor.evm_client_id).await?;

    Ok(counterparty.as_deref() == Some(corridor.stellar_client_id.as_str()))
}

async fn evm_counterparty(cfg: &Config, client_id: &str) -> Result<Option<String>> {
    let corridor = &cfg.evm;
    let provider = read_provider(cfg).await?;
    let router: Address = required(&corridor.ics26_router, "EVM_ICS26_ROUTER")?
        .parse()
        .context("EVM_ICS26_ROUTER is not an EVM address")?;
    let request = TransactionRequest::default().with_to(router).with_input(
        getCounterpartyCall {
            clientId: client_id.to_string(),
        }
        .abi_encode(),
    );

    match provider.call(request).await {
        Ok(output) => {
            let info = getCounterpartyCall::abi_decode_returns(&output)
                .context("decoding getCounterparty")?;

            Ok(Some(info.clientId))
        }
        Err(error) if reverted(&error.to_string()) => Ok(None),
        Err(error) => Err(anyhow!("getCounterparty({client_id}): {error}")),
    }
}

async fn evm_client_exists(cfg: &Config, client_id: &str) -> Result<bool> {
    let corridor = &cfg.evm;
    let provider = read_provider(cfg).await?;
    let router: Address = required(&corridor.ics26_router, "EVM_ICS26_ROUTER")?
        .parse()
        .context("EVM_ICS26_ROUTER is not an EVM address")?;
    let request = TransactionRequest::default().with_to(router).with_input(
        getClientCall {
            clientId: client_id.to_string(),
        }
        .abi_encode(),
    );

    match provider.call(request).await {
        Ok(_) => Ok(true),
        Err(error) if reverted(&error.to_string()) => Ok(false),
        Err(error) => Err(anyhow!("getClient({client_id}): {error}")),
    }
}

async fn free_evm_client_id(cfg: &Config) -> Result<String> {
    let configured = cfg.evm.evm_client_id.as_str();

    if !evm_client_exists(cfg, configured).await? {
        return Ok(configured.to_string());
    }

    let base = configured
        .trim_end_matches(|c: char| c.is_ascii_digit())
        .trim_end_matches('-');

    for index in 0..MAX_CLIENT_ID_PROBES {
        let candidate = format!("{base}-{index}");

        if !evm_client_exists(cfg, &candidate).await? {
            logger::detail(&format!(
                "{configured} is taken on the EVM router — using {candidate}"
            ));

            return Ok(candidate);
        }
    }

    bail!("no free client id on the EVM router among {base}-0 … {base}-{MAX_CLIENT_ID_PROBES}")
}

fn reverted(message: &str) -> bool {
    message.contains("revert")
}

async fn read_provider(cfg: &Config) -> Result<impl Provider> {
    let rpc_url = required(&cfg.evm.rpc_url, "EVM_RPC_URL")?;

    ProviderBuilder::new()
        .connect(rpc_url)
        .await
        .with_context(|| format!("connecting to {rpc_url}"))
}

fn record(cfg: &Config, stellar_client: &str, evm_client: &str) -> Result<()> {
    let corridor = &cfg.evm;

    shared::env_upsert(
        &crate::config::tooling_env(),
        &[
            ("STELLAR_EVM_CLIENT_ID", stellar_client),
            ("EVM_STELLAR_CLIENT_ID", evm_client),
        ],
    )?;
    logger::detail(&format!(
        "STELLAR_EVM_CLIENT_ID={stellar_client}  EVM_STELLAR_CLIENT_ID={evm_client}"
    ));

    let path = Path::new(&cfg.relayer.config);

    for (host, client, counterparty) in [
        (
            &corridor.stellar_chain_id,
            stellar_client,
            &corridor.evm_chain_id,
        ),
        (
            &corridor.evm_chain_id,
            evm_client,
            &corridor.stellar_chain_id,
        ),
    ] {
        match wiring::set_route(path, host, client, counterparty) {
            Ok(true) => logger::detail(&format!(
                "{}: {host} routes {client} to {counterparty}",
                path.display()
            )),
            Ok(false) => logger::detail(&format!(
                "{} already routes {client} to {counterparty}",
                path.display()
            )),
            Err(error) => logger::warn(&format!(
                "could not route {client} in {}: {error:#} — the relayer will not relay it until it does",
                path.display()
            )),
        }
    }

    Ok(())
}

async fn create_on_stellar(cfg: &Config, root: &Path) -> Result<String> {
    let corridor = &cfg.evm;
    let router = required(&corridor.stellar_router, "STELLAR_EVM_ROUTER")?;
    let router_id = soroban::contract_id(router)?;
    let identity = relayer_identity(cfg);

    logger::step(&format!(
        "CreateClient {} → {} (client type {})",
        corridor.evm_chain_id, corridor.stellar_chain_id, corridor.stellar_client_type
    ));

    let response = proof_api(cfg)
        .await?
        .create_client(CreateClientRequest {
            src_chain: corridor.evm_chain_id.clone(),
            dst_chain: corridor.stellar_chain_id.clone(),
            parameters: HashMap::from([(
                "client_type".to_string(),
                corridor.stellar_client_type.clone(),
            )]),
        })
        .await
        .map_err(|status| anyhow!("CreateClient failed: {}", status.message()))?
        .into_inner();

    if response.address != router {
        bail!(
            "the proof API answered for {}, but STELLAR_EVM_ROUTER is {router}",
            response.address
        );
    }

    let functions = soroban::decode(&response.tx)?;
    let [function] = functions.as_slice() else {
        bail!(
            "the proof API returned {} calls for a client creation, expected one",
            functions.len()
        );
    };
    let (target, name) = soroban::target_and_function(function)?;

    if target != router_id || name != "create_client" {
        bail!("the proof API returned `{name}` on another contract, not `create_client` on STELLAR_EVM_ROUTER");
    }

    let contracts_config = ContractsConfig::from(cfg);
    let hash = soroban::submit(root, &contracts_config, identity, function.clone())?;
    let client_id = returned_string(root, &contracts_config, &hash)?;

    logger::ok(&format!(
        "the Soroban router created {client_id} (tx {hash})"
    ));

    Ok(client_id)
}

async fn create_on_evm(cfg: &Config, stellar_client: &str, evm_client: &str) -> Result<()> {
    let corridor = &cfg.evm;
    let rpc_url = required(&corridor.rpc_url, "EVM_RPC_URL")?;
    let router: Address = required(&corridor.ics26_router, "EVM_ICS26_ROUTER")?
        .parse()
        .context("EVM_ICS26_ROUTER is not an EVM address")?;
    let signer: PrivateKeySigner = required(&corridor.private_key, "EVM_PRIVATE_KEY")?
        .trim_start_matches("0x")
        .parse()
        .map_err(|error| anyhow!("EVM_PRIVATE_KEY is not a private key: {error}"))?;
    let sender = signer.address();

    logger::step(&format!(
        "CreateClient {} → {} (SP1ICSStellar, genesis at the latest archived ledger)",
        corridor.stellar_chain_id, corridor.evm_chain_id
    ));

    let creation = proof_api(cfg)
        .await?
        .create_client(CreateClientRequest {
            src_chain: corridor.stellar_chain_id.clone(),
            dst_chain: corridor.evm_chain_id.clone(),
            parameters: HashMap::new(),
        })
        .await
        .map_err(|status| anyhow!("CreateClient failed: {}", status.message()))?
        .into_inner();

    let provider = ProviderBuilder::new()
        .wallet(signer)
        .connect(rpc_url)
        .await
        .with_context(|| format!("connecting to {rpc_url}"))?;

    let deployment = TransactionRequest::default()
        .with_from(sender)
        .with_deploy_code(Bytes::from(creation.tx));

    provider
        .call(deployment.clone())
        .await
        .map_err(|error| anyhow!("the client deployment would revert: {error}"))?;

    let deployment = super::priced(&provider, deployment).await?;
    let receipt = provider
        .send_transaction(deployment)
        .await
        .context("sending the client deployment")?
        .get_receipt()
        .await
        .context("waiting for the client deployment")?;

    let client = receipt
        .contract_address
        .filter(|_| receipt.status())
        .ok_or_else(|| anyhow!("the client deployment {} failed", receipt.transaction_hash))?;

    logger::ok(&format!(
        "deployed SP1ICSStellar at {client} (tx {})",
        receipt.transaction_hash
    ));
    logger::step(&format!(
        "addClient {evm_client} → {client}, counterparty {stellar_client}"
    ));

    let registration = TransactionRequest::default()
        .with_from(sender)
        .with_to(router)
        .with_input(
            addClientCall {
                clientId: evm_client.to_string(),
                counterpartyInfo: CounterpartyInfo {
                    clientId: stellar_client.to_string(),
                    merklePrefix: vec![Bytes::new()],
                },
                client,
            }
            .abi_encode(),
        );

    provider
        .call(registration.clone())
        .await
        .map_err(|error| anyhow!("addClient would revert: {error}"))?;

    let registration = super::priced(&provider, registration).await?;
    let receipt = provider
        .send_transaction(registration)
        .await
        .context("sending addClient")?
        .get_receipt()
        .await
        .context("waiting for addClient")?;

    if !receipt.status() {
        bail!("addClient {} reverted", receipt.transaction_hash);
    }

    logger::ok(&format!(
        "{evm_client} is registered on the EVM router (tx {})",
        receipt.transaction_hash
    ));

    Ok(())
}

pub fn counterparty(cfg: &Config, root: &Path) -> Result<()> {
    let corridor = &cfg.evm;

    logger::banner("tx clients counterparty (the Stellar ↔ EVM corridor, Stellar side)");

    register_on_stellar(
        cfg,
        root,
        &corridor.stellar_client_id,
        &corridor.evm_client_id,
    )
}

fn register_on_stellar(
    cfg: &Config,
    root: &Path,
    stellar_client: &str,
    evm_client: &str,
) -> Result<()> {
    let router = required(&cfg.evm.stellar_router, "STELLAR_EVM_ROUTER")?;

    logger::step(&format!(
        "register_counterparty {stellar_client} → {evm_client} (empty prefix)"
    ));

    contracts::invoke_as(
        &ContractsConfig::from(cfg),
        root,
        router,
        &[
            "register_counterparty",
            "--client_id",
            stellar_client,
            "--counterparty_client_id",
            evm_client,
            "--counterparty_commitment_prefix",
            "[\"\"]",
        ],
        &cfg.stellar.cli_identity,
    )?;

    logger::ok("the Soroban router names its counterparty");

    Ok(())
}

fn relayer_identity(cfg: &Config) -> &str {
    if cfg.evm.stellar_relayer_identity.is_empty() {
        cfg.stellar.cli_identity.as_str()
    } else {
        cfg.evm.stellar_relayer_identity.as_str()
    }
}

fn returned_string(root: &Path, contracts_config: &ContractsConfig, hash: &str) -> Result<String> {
    let mut arguments = vec![
        "tx".to_string(),
        "fetch".to_string(),
        "meta".to_string(),
        "--hash".to_string(),
        hash.to_string(),
        "--output".to_string(),
        "json".to_string(),
    ];

    arguments.extend(contracts_config.net_flags());

    let references: Vec<&str> = arguments.iter().map(String::as_str).collect();
    let output = tools::stellar::capture_all(root, &references)?;
    let meta: Value =
        serde_json::from_str(output.trim()).context("the transaction meta is not JSON")?;

    meta.as_object()
        .and_then(|versions| versions.values().next())
        .and_then(|body| body.pointer("/soroban_meta/return_value/string"))
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| anyhow!("transaction {hash} returned no client id"))
}
