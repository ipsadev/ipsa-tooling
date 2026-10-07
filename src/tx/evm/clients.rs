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
    logger, tools,
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
}

pub async fn create(cfg: &Config, root: &Path) -> Result<()> {
    let corridor = &cfg.evm;

    logger::banner("tx clients create (the Stellar ↔ EVM zk light clients)");

    let stellar_client = create_on_stellar(cfg, root).await?;

    if stellar_client != corridor.stellar_client_id {
        logger::warn(&format!(
            "the Soroban router named the new client {stellar_client}, not STELLAR_EVM_CLIENT_ID ({}) — set it before relaying",
            corridor.stellar_client_id
        ));
    }

    create_on_evm(cfg, &stellar_client).await?;

    logger::hint("register the Stellar side's counterparty: ipsa tx clients counterparty --evm");

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

async fn create_on_evm(cfg: &Config, stellar_client: &str) -> Result<()> {
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
        "addClient {} → {client}, counterparty {stellar_client}",
        corridor.evm_client_id
    ));

    let registration = TransactionRequest::default()
        .with_from(sender)
        .with_to(router)
        .with_input(
            addClientCall {
                clientId: corridor.evm_client_id.clone(),
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
        "{} is registered on the EVM router (tx {})",
        corridor.evm_client_id, receipt.transaction_hash
    ));

    Ok(())
}

pub fn counterparty(cfg: &Config, root: &Path) -> Result<()> {
    let corridor = &cfg.evm;
    let router = required(&corridor.stellar_router, "STELLAR_EVM_ROUTER")?;

    logger::banner("tx clients counterparty (the Stellar ↔ EVM corridor, Stellar side)");
    logger::step(&format!(
        "register_counterparty {} → {} (empty prefix)",
        corridor.stellar_client_id, corridor.evm_client_id
    ));

    contracts::invoke_as(
        &ContractsConfig::from(cfg),
        root,
        router,
        &[
            "register_counterparty",
            "--client_id",
            &corridor.stellar_client_id,
            "--counterparty_client_id",
            &corridor.evm_client_id,
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
