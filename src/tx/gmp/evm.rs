use alloy::{
    dyn_abi::{DynSolType, DynSolValue, JsonAbiExt, Specifier},
    json_abi::Function,
    network::TransactionBuilder,
    primitives::{Address, Bytes},
    providers::{Provider, ProviderBuilder},
    rpc::types::TransactionRequest,
    sol_types::SolCall,
};
use anyhow::{bail, Context, Result};

use crate::{config::Config, tx::evm::config::required};

alloy::sol! {
    struct AccountIdentifier {
        string clientId;
        string sender;
        bytes salt;
    }

    function getOrComputeAccountAddress(AccountIdentifier accountId) external view returns (address);
}

pub fn calldata(signature: &str, args: &[String]) -> Result<Vec<u8>> {
    let function = Function::parse(signature).with_context(|| {
        format!("--evm-call {signature:?} is not a solidity function signature")
    })?;

    if function.inputs.len() != args.len() {
        bail!(
            "{} takes {} argument(s) but --evm-args gave {}",
            function.signature(),
            function.inputs.len(),
            args.len()
        );
    }

    let values = function
        .inputs
        .iter()
        .zip(args)
        .map(|(param, arg)| {
            let kind: DynSolType = param
                .resolve()
                .with_context(|| format!("resolving the type of {}", param.name))?;

            kind.coerce_str(arg)
                .with_context(|| format!("{arg:?} is not a {kind}"))
        })
        .collect::<Result<Vec<DynSolValue>>>()?;

    function
        .abi_encode_input(&values)
        .context("encoding the call")
}

pub fn acknowledgement_result(acknowledgement: &[u8]) -> Result<Vec<u8>> {
    let decoded = DynSolType::Tuple(vec![DynSolType::Bytes])
        .abi_decode(acknowledgement)
        .context("the acknowledgement is not an abi-encoded GMPAcknowledgement")?;

    let bytes = find_bytes(&decoded).context("the acknowledgement holds no result bytes")?;

    Ok(bytes)
}

pub fn decode_returns(kind: &str, data: &[u8]) -> Result<String> {
    let parsed: DynSolType = kind
        .parse()
        .with_context(|| format!("--returns {kind:?} is not a solidity type"))?;

    let value = parsed
        .abi_decode(data)
        .with_context(|| format!("the result is not a {kind}"))?;

    Ok(format!("{value:?}"))
}

pub async fn account_address(
    cfg: &Config,
    gmp: &str,
    client_id: &str,
    sender: &str,
    salt: &[u8],
) -> Result<Address> {
    let rpc_url = required(&cfg.evm.rpc_url, "EVM_RPC_URL")?;
    let provider = ProviderBuilder::new()
        .connect(rpc_url)
        .await
        .with_context(|| format!("connecting to {rpc_url}"))?;

    let gmp: Address = gmp.parse().context("EVM_ICS27_GMP is not an EVM address")?;
    let request = TransactionRequest::default().with_to(gmp).with_input(
        getOrComputeAccountAddressCall {
            accountId: AccountIdentifier {
                clientId: client_id.to_string(),
                sender: sender.to_string(),
                salt: Bytes::copy_from_slice(salt),
            },
        }
        .abi_encode(),
    );

    let returned = provider
        .call(request)
        .await
        .context("getOrComputeAccountAddress on ICS27GMP")?;

    getOrComputeAccountAddressCall::abi_decode_returns(&returned)
        .context("decoding the account address")
}

fn find_bytes(value: &DynSolValue) -> Option<Vec<u8>> {
    match value {
        DynSolValue::Bytes(bytes) => Some(bytes.clone()),
        DynSolValue::Tuple(values) => values.iter().find_map(find_bytes),
        _ => None,
    }
}
