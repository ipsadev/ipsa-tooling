pub mod clients;
pub mod config;
pub mod relay;
pub mod soroban;
pub mod transfer;

use alloy::{
    eips::BlockNumberOrTag,
    network::{BlockResponse, TransactionBuilder},
    providers::Provider,
    rpc::types::TransactionRequest,
};
use anyhow::{anyhow, Context, Result};

pub(crate) async fn priced<P: Provider>(
    provider: &P,
    request: TransactionRequest,
) -> Result<TransactionRequest> {
    let tip = provider
        .get_max_priority_fee_per_gas()
        .await
        .context("reading eth_maxPriorityFeePerGas")?;
    let block = provider
        .get_block_by_number(BlockNumberOrTag::Latest)
        .await
        .context("reading the latest block")?
        .ok_or_else(|| anyhow!("the node returned no latest block"))?;
    let base = u128::from(block.header().base_fee_per_gas.unwrap_or_default());

    Ok(request
        .with_max_priority_fee_per_gas(tip)
        .with_max_fee_per_gas(base * 2 + tip))
}
