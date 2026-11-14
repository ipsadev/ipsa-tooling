use std::{io::Cursor, path::Path};

use anyhow::{anyhow, bail, Context, Result};
use stellar_xdr::{
    ContractId, Hash, HostFunction, InvokeHostFunctionOp, Limited, Limits, Memo, MuxedAccount,
    Operation, OperationBody, Preconditions, ReadXdr, ScAddress, SequenceNumber, Transaction,
    TransactionEnvelope, TransactionExt, TransactionV1Envelope, Uint256, WriteXdr,
};

use crate::{logger, tools, tx::contracts::config::ContractsConfig};

pub fn decode(encoded: &[u8]) -> Result<Vec<HostFunction>> {
    if encoded.len() < 4 {
        bail!(
            "the proof API returned {} bytes, too short for a host-function list",
            encoded.len()
        );
    }

    let count = u32::from_be_bytes(encoded[..4].try_into().expect("four bytes"));
    let mut reader = Limited::new(Cursor::new(&encoded[4..]), Limits::none());

    (0..count)
        .map(|index| {
            HostFunction::read_xdr(&mut reader)
                .map_err(|error| anyhow!("decoding host function {index}: {error}"))
        })
        .collect()
}

pub fn contract_id(strkey: &str) -> Result<[u8; 32]> {
    let contract = stellar_strkey::Contract::from_string(strkey)
        .map_err(|error| anyhow!("{strkey} is not a contract address: {error}"))?;

    Ok(contract.0)
}

pub fn target_and_function(function: &HostFunction) -> Result<([u8; 32], String)> {
    let HostFunction::InvokeContract(arguments) = function else {
        bail!("the proof API returned a host function that is not a contract call");
    };

    let ScAddress::Contract(ContractId(Hash(target))) = &arguments.contract_address else {
        bail!("the proof API returned a call to something that is not a contract");
    };

    Ok((*target, arguments.function_name.to_utf8_string_lossy()))
}

pub fn submit(
    root: &Path,
    contracts_config: &ContractsConfig,
    identity: &str,
    function: HostFunction,
) -> Result<String> {
    let address = tools::stellar::capture_all(root, &["keys", "address", identity])?
        .trim()
        .to_string();
    let public_key = stellar_strkey::ed25519::PublicKey::from_string(&address)
        .map_err(|error| anyhow!("{address} is not an account address: {error}"))?;

    let transaction = Transaction {
        source_account: MuxedAccount::Ed25519(Uint256(public_key.0)),
        fee: 100,
        seq_num: SequenceNumber(0),
        cond: Preconditions::None,
        memo: Memo::None,
        operations: vec![Operation {
            source_account: None,
            body: OperationBody::InvokeHostFunction(InvokeHostFunctionOp {
                host_function: function,
                auth: Default::default(),
            }),
        }]
        .try_into()
        .map_err(|_| anyhow!("one operation always fits"))?,
        ext: TransactionExt::V0,
    };

    let envelope = TransactionEnvelope::Tx(TransactionV1Envelope {
        tx: transaction,
        signatures: Default::default(),
    })
    .to_xdr_base64(Limits::none())
    .context("encoding the transaction envelope")?;

    let network = contracts_config.net_flags();

    let sequenced = step(
        root,
        &["tx", "update", "sequence-number", "next"],
        &network,
        &envelope,
    )?;
    let simulated = step(
        root,
        &["tx", "simulate", "--source-account", identity],
        &network,
        &sequenced,
    )?;
    let signed = step(
        root,
        &["tx", "sign", "--sign-with-key", identity],
        &network,
        &simulated,
    )?;
    let hash = step(root, &["tx", "hash"], &network, &signed)?;

    step(root, &["tx", "send"], &network, &signed)?;

    logger::detail(&format!("landed {hash}"));

    Ok(hash)
}

fn step(root: &Path, command: &[&str], network: &[String], input: &str) -> Result<String> {
    let arguments: Vec<&str> = command
        .iter()
        .copied()
        .chain(network.iter().map(String::as_str))
        .collect();

    tools::stellar::capture_with_input(root, &arguments, input)
}
