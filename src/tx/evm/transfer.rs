use std::{
    path::Path,
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::{anyhow, bail, Result};

use crate::{
    config::Config,
    logger, tools,
    tx::{
        contracts::{self, config::ContractsConfig},
        evm::config::required,
        transfer::TransferParams,
    },
};

const MINIMUM_TIMEOUT_SECONDS: u64 = 21_600;

pub fn stellar_to_evm(
    cfg: &Config,
    root: &Path,
    params: &TransferParams,
) -> Result<Option<String>> {
    logger::banner("transfer stellar (Stellar → EVM ICS-20, through the zk light clients)");

    let corridor = &cfg.evm;
    let transfer_app = required(&corridor.stellar_transfer, "STELLAR_EVM_TRANSFER")?;
    let source_client = corridor.stellar_client_id.as_str();
    let contracts_config = ContractsConfig::from(cfg);

    let identity = if corridor.stellar_sender_identity.is_empty() {
        cfg.stellar.cli_identity.as_str()
    } else {
        corridor.stellar_sender_identity.as_str()
    };

    let identity_address = tools::stellar::capture_all(root, &["keys", "address", identity])?
        .trim()
        .to_string();

    if identity_address != params.sender {
        bail!(
            "--from {} is not the address of the signing identity `{identity}` ({identity_address}) — \
             set EVM_STELLAR_SENDER_IDENTITY to the identity that owns {}",
            params.sender,
            params.sender
        );
    }

    let denom = match params.denom.as_str() {
        "XLM" | "xlm" | "native" => native_asset_contract(root, &contracts_config)?,
        other => other.to_string(),
    };

    let timeout_seconds = params.timeout_secs.max(MINIMUM_TIMEOUT_SECONDS);

    if timeout_seconds != params.timeout_secs {
        logger::detail(&format!(
            "timeout raised from {}s to {timeout_seconds}s: the packet waits for a history-archive \
             checkpoint and two zk proofs before it can be received",
            params.timeout_secs
        ));
    }

    let timeout = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs() + timeout_seconds;
    let amount = params.amount.to_string();
    let memo = format!("\"{}\"", params.memo);

    logger::step(&format!(
        "initiate_transfer {amount} {denom} → {} (client {source_client}, signed by {identity})",
        params.receiver
    ));

    let output = contracts::invoke_as(
        &contracts_config,
        root,
        transfer_app,
        &[
            "initiate_transfer",
            "--sender",
            &params.sender,
            "--source_client_id",
            source_client,
            "--denom",
            &denom,
            "--amount",
            &amount,
            "--receiver",
            &params.receiver,
            "--timeout_timestamp",
            &timeout.to_string(),
            "--memo",
            &memo,
        ],
        identity,
    )?;

    let hash = contracts::tx_hash(&output);

    match &hash {
        Some(hash) => {
            logger::ok(&format!("transfer initiated — tx {hash}"));
            println!("{hash}");
            logger::hint(&format!("relay it: ipsa tx relay --tx {hash} --evm"));
        }
        None => logger::warn(
            "transfer initiated, but the stellar CLI printed no transaction hash — find it on the \
             explorer and run `ipsa tx relay --tx <hash> --evm`",
        ),
    }

    Ok(hash)
}

fn native_asset_contract(root: &Path, contracts_config: &ContractsConfig) -> Result<String> {
    let mut arguments = vec![
        "contract".to_string(),
        "id".to_string(),
        "asset".to_string(),
        "--asset".to_string(),
        "native".to_string(),
    ];

    arguments.extend(contracts_config.net_flags());

    let references: Vec<&str> = arguments.iter().map(String::as_str).collect();
    let output = tools::stellar::capture_all(root, &references)?;

    output
        .lines()
        .map(str::trim)
        .find(|line| line.len() == 56 && line.starts_with('C'))
        .map(str::to_string)
        .ok_or_else(|| anyhow!("the stellar CLI did not print the native asset contract id"))
}

alloy::sol! {
    function approve(address spender, uint256 amount) external returns (bool);

    struct SendTransferMsg {
        address denom;
        uint256 amount;
        string receiver;
        string sourceClient;
        string destPort;
        uint64 timeoutTimestamp;
        string memo;
    }

    function sendTransfer(SendTransferMsg msg_) external returns (uint64);
}

const MAXIMUM_TIMEOUT_SECONDS: u64 = 82_800;

pub async fn evm_to_stellar(cfg: &Config, params: &TransferParams) -> Result<Option<String>> {
    use alloy::{
        network::TransactionBuilder,
        primitives::{Address, U256},
        providers::{Provider, ProviderBuilder},
        rpc::types::TransactionRequest,
        signers::local::PrivateKeySigner,
        sol_types::SolCall,
    };

    logger::banner("transfer evm (EVM → Stellar ICS-20, through the zk light clients)");

    let corridor = &cfg.evm;
    let rpc_url = required(&corridor.rpc_url, "EVM_RPC_URL")?;
    let transfer_app: Address = required(&corridor.ics20_transfer, "EVM_ICS20_TRANSFER")?
        .parse()
        .map_err(|error| anyhow!("EVM_ICS20_TRANSFER is not an EVM address: {error}"))?;
    let signer: PrivateKeySigner = required(&corridor.private_key, "EVM_PRIVATE_KEY")?
        .trim_start_matches("0x")
        .parse()
        .map_err(|error| anyhow!("EVM_PRIVATE_KEY is not a private key: {error}"))?;
    let sender = signer.address();

    let from: Address = params
        .sender
        .parse()
        .map_err(|error| anyhow!("--from {} is not an EVM address: {error}", params.sender))?;

    if from != sender {
        bail!("--from {from} is not the address of EVM_PRIVATE_KEY ({sender})");
    }

    let denom: Address = params.denom.parse().map_err(|_| {
        anyhow!(
            "--denom {} is not an ERC-20 address; on the EVM side a denom is the token contract \
             (an IBCERC20 voucher's address sends it home)",
            params.denom
        )
    })?;

    let amount = U256::from(
        u128::try_from(params.amount).map_err(|_| anyhow!("--amount must be positive"))?,
    );
    let timeout_seconds = params
        .timeout_secs
        .clamp(MINIMUM_TIMEOUT_SECONDS, MAXIMUM_TIMEOUT_SECONDS);

    if timeout_seconds != params.timeout_secs {
        logger::detail(&format!(
            "timeout set to {timeout_seconds}s: at least six hours for Ethereum finality and two proofs, \
             at most the router's one-day ceiling"
        ));
    }

    let timeout = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs() + timeout_seconds;

    let provider = ProviderBuilder::new()
        .wallet(signer)
        .connect(rpc_url)
        .await
        .map_err(|error| anyhow!("connecting to {rpc_url}: {error}"))?;

    logger::step(&format!("approve {amount} of {denom} for {transfer_app}"));

    let approval = TransactionRequest::default()
        .with_from(sender)
        .with_to(denom)
        .with_input(
            approveCall {
                spender: transfer_app,
                amount,
            }
            .abi_encode(),
        );

    provider
        .call(approval.clone())
        .await
        .map_err(|error| anyhow!("the approval would revert: {error}"))?;
    let approval = super::priced(&provider, approval).await?;
    provider
        .send_transaction(approval)
        .await
        .map_err(|error| anyhow!("sending the approval: {error}"))?
        .get_receipt()
        .await
        .map_err(|error| anyhow!("waiting for the approval: {error}"))?;

    logger::step(&format!(
        "sendTransfer {amount} {denom} → {} (client {}, timeout {timeout})",
        params.receiver, corridor.evm_client_id
    ));

    let send = TransactionRequest::default()
        .with_from(sender)
        .with_to(transfer_app)
        .with_input(
            sendTransferCall {
                msg_: SendTransferMsg {
                    denom,
                    amount,
                    receiver: params.receiver.clone(),
                    sourceClient: corridor.evm_client_id.clone(),
                    destPort: "transfer".to_string(),
                    timeoutTimestamp: timeout,
                    memo: params.memo.clone(),
                },
            }
            .abi_encode(),
        );

    provider
        .call(send.clone())
        .await
        .map_err(|error| anyhow!("sendTransfer would revert: {error}"))?;

    let send = super::priced(&provider, send).await?;
    let receipt = provider
        .send_transaction(send)
        .await
        .map_err(|error| anyhow!("sending the transfer: {error}"))?
        .get_receipt()
        .await
        .map_err(|error| anyhow!("waiting for the transfer: {error}"))?;

    if !receipt.status() {
        bail!("the transfer {} reverted", receipt.transaction_hash);
    }

    let hash = receipt.transaction_hash.to_string();

    logger::ok(&format!("transfer initiated — tx {hash}"));
    println!("{hash}");
    logger::hint(&format!("relay it: ipsa tx relay --tx {hash}"));

    Ok(Some(hash))
}
