use std::{
    path::Path,
    time::{Duration, Instant},
};

use alloy::{
    network::TransactionBuilder,
    primitives::{Address, Bytes},
    providers::{Provider, ProviderBuilder},
    rpc::types::TransactionRequest,
    signers::local::PrivateKeySigner,
};
use anyhow::{anyhow, bail, Context, Result};
use tonic::transport::Endpoint;

use crate::{
    config::Config,
    gateway::proto::proofapi::{proof_api_service_client::ProofApiServiceClient, RelayByTxRequest},
    logger,
    tx::{
        contracts::config::ContractsConfig,
        evm::{config::required, soroban},
    },
};

const PROOF_API_DEADLINE: Duration = Duration::from_secs(1_800);

pub struct Relayed {
    pub transaction_hash: String,
    pub block: u64,
    pub gas_used: u64,
}

pub async fn stellar_to_evm(cfg: &Config, stellar_transaction: &str) -> Result<Option<Relayed>> {
    let corridor = &cfg.evm;
    let hash = stellar_transaction
        .trim()
        .trim_start_matches("0x")
        .to_lowercase();

    if hash.len() != 64 || !hash.chars().all(|character| character.is_ascii_hexdigit()) {
        bail!("{stellar_transaction} is not a Stellar transaction hash");
    }

    logger::banner("tx relay (stellar → evm, through the proof API)");

    let direction = Direction {
        source_chain: &corridor.stellar_chain_id,
        destination_chain: &corridor.evm_chain_id,
        source_client: &corridor.stellar_client_id,
        destination_client: &corridor.evm_client_id,
    };

    let Some((calldata, address)) = relay_by_tx(cfg, &direction, &hash).await? else {
        dry_run_passed();

        return Ok(None);
    };

    let address: Address = address
        .parse()
        .with_context(|| format!("the proof API returned {address:?}, not an EVM address"))?;
    let calldata = Bytes::from(calldata);

    let expected_router: Address = required(&corridor.ics26_router, "EVM_ICS26_ROUTER")?
        .parse()
        .context("EVM_ICS26_ROUTER is not an EVM address")?;

    if address != expected_router {
        bail!(
            "the proof API asked for a transaction to {address}, but EVM_ICS26_ROUTER is {expected_router} — refusing to send it"
        );
    }

    let relayed = submit(
        required(&corridor.rpc_url, "EVM_RPC_URL")?,
        required(&corridor.private_key, "EVM_PRIVATE_KEY")?,
        address,
        calldata,
    )
    .await?;

    logger::ok(&format!(
        "relayed — EVM tx {} in block {}, {} gas",
        relayed.transaction_hash, relayed.block, relayed.gas_used
    ));
    println!("{}", relayed.transaction_hash);

    Ok(Some(relayed))
}

pub(crate) async fn proof_api(
    cfg: &Config,
) -> Result<ProofApiServiceClient<tonic::transport::Channel>> {
    let corridor = &cfg.evm;

    let endpoint = Endpoint::from_shared(corridor.proof_api_url.clone())
        .with_context(|| {
            format!(
                "{} is not a usable proof API address",
                corridor.proof_api_url
            )
        })?
        .timeout(PROOF_API_DEADLINE);

    let channel = endpoint.connect().await.with_context(|| {
        format!(
            "could not reach the proof API at {} — start stellar-proof-api with the corridor's configuration",
            corridor.proof_api_url
        )
    })?;

    Ok(ProofApiServiceClient::new(channel))
}

struct Direction<'a> {
    source_chain: &'a str,
    destination_chain: &'a str,
    source_client: &'a str,
    destination_client: &'a str,
}

async fn relay_by_tx(
    cfg: &Config,
    direction: &Direction<'_>,
    identifier: &str,
) -> Result<Option<(Vec<u8>, String)>> {
    let client = proof_api(cfg).await?;

    logger::step(&format!(
        "RelayByTx {identifier} ({} → {}, clients {} → {})",
        direction.source_chain,
        direction.destination_chain,
        direction.source_client,
        direction.destination_client
    ));
    logger::detail(
        "the proof API waits for the source chain to settle — a history-archive checkpoint or \
         Ethereum finality — then proves on the Succinct network: expect minutes, not seconds",
    );

    let started = Instant::now();

    let response = client
        .clone()
        .relay_by_tx(RelayByTxRequest {
            src_chain: direction.source_chain.to_string(),
            dst_chain: direction.destination_chain.to_string(),
            source_tx_ids: vec![identifier.as_bytes().to_vec()],
            timeout_tx_ids: Vec::new(),
            src_client_id: direction.source_client.to_string(),
            dst_client_id: direction.destination_client.to_string(),
            src_packet_sequences: Vec::new(),
            dst_packet_sequences: Vec::new(),
        })
        .await
        .map_err(|status| {
            logger::hint(
                "the proof API reports only a generic status to callers; the reason is in its own log",
            );

            anyhow!("RelayByTx failed: {}", status.message())
        })?
        .into_inner();

    logger::detail(&format!(
        "the proof API answered in {}s",
        started.elapsed().as_secs()
    ));

    if response.tx.is_empty() {
        return Ok(None);
    }

    Ok(Some((response.tx, response.address)))
}

fn dry_run_passed() {
    logger::ok(
        "dry run passed — the proof API is in execute mode: the packet was found, the source chain \
         settled, every proof input built and every guest executed, but nothing was proven",
    );
    logger::hint("set \"prover\": \"network\" in the proof API's configuration to relay for real");
}

pub async fn evm_to_stellar(
    cfg: &Config,
    root: &Path,
    evm_transaction: &str,
) -> Result<Vec<String>> {
    let corridor = &cfg.evm;
    let router = required(&corridor.stellar_router, "STELLAR_EVM_ROUTER")?;
    let router_id = soroban::contract_id(router)?;
    let identity = if corridor.stellar_relayer_identity.is_empty() {
        cfg.stellar.cli_identity.as_str()
    } else {
        corridor.stellar_relayer_identity.as_str()
    };

    logger::banner("tx relay (evm → stellar, through the proof API)");

    let direction = Direction {
        source_chain: &corridor.evm_chain_id,
        destination_chain: &corridor.stellar_chain_id,
        source_client: &corridor.evm_client_id,
        destination_client: &corridor.stellar_client_id,
    };

    let Some((encoded, address)) = relay_by_tx(cfg, &direction, evm_transaction.trim()).await?
    else {
        dry_run_passed();

        return Ok(Vec::new());
    };

    if address != router {
        bail!("the proof API answered for {address}, but STELLAR_EVM_ROUTER is {router} — refusing to submit");
    }

    let functions = soroban::decode(&encoded)?;

    for function in &functions {
        let (target, name) = soroban::target_and_function(function)?;

        if target != router_id {
            bail!("the proof API returned a call to a contract other than STELLAR_EVM_ROUTER — refusing to submit");
        }

        if !matches!(
            name.as_str(),
            "update_client" | "recv_packet" | "acknowledge_packet"
        ) {
            bail!("the proof API returned a call to `{name}`, which a relay never needs — refusing to submit");
        }
    }

    let contracts_config = ContractsConfig::from(cfg);
    let mut hashes = Vec::new();

    for function in functions {
        let (_, name) = soroban::target_and_function(&function)?;

        logger::step(&format!("{name} on {router}, signed by {identity}"));

        hashes.push(soroban::submit(
            root,
            &contracts_config,
            identity,
            function,
        )?);
    }

    logger::ok(&format!("relayed — {} Stellar transactions", hashes.len()));

    for hash in &hashes {
        println!("{hash}");
    }

    Ok(hashes)
}

async fn submit(rpc_url: &str, private_key: &str, to: Address, calldata: Bytes) -> Result<Relayed> {
    let signer: PrivateKeySigner = private_key
        .trim_start_matches("0x")
        .parse()
        .context("EVM_PRIVATE_KEY is not a private key")?;
    let sender = signer.address();

    let provider = ProviderBuilder::new()
        .wallet(signer)
        .connect(rpc_url)
        .await
        .with_context(|| format!("connecting to {rpc_url}"))?;

    let request = TransactionRequest::default()
        .with_from(sender)
        .with_to(to)
        .with_input(calldata);

    logger::step(&format!(
        "simulate the relay transaction to {to} from {sender}"
    ));

    provider
        .call(request.clone())
        .await
        .map_err(|error| anyhow!("the relay transaction would revert: {error}"))?;

    logger::step("send it");

    let request = super::priced(&provider, request).await?;
    let pending = provider
        .send_transaction(request)
        .await
        .context("sending the relay transaction")?;

    logger::detail(&format!(
        "sent {} — waiting for the receipt",
        pending.tx_hash()
    ));

    let receipt = pending
        .get_receipt()
        .await
        .context("waiting for the relay transaction's receipt")?;

    if !receipt.status() {
        bail!(
            "the relay transaction {} reverted",
            receipt.transaction_hash
        );
    }

    Ok(Relayed {
        transaction_hash: receipt.transaction_hash.to_string(),
        block: receipt.block_number.unwrap_or_default(),
        gas_used: receipt.gas_used,
    })
}
