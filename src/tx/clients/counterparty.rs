use std::{path::Path, time::Duration};

use anyhow::{bail, Result};
use cosmrs::Any;
use prost::Message;

use crate::{
    config::Config,
    logger,
    services::cosmos::tx::CosmosSigner,
    tx::{
        clients::config::ClientsConfig,
        contracts::{self, config::ContractsConfig},
    },
};

const TYPE_URL: &str = "/ibc.core.client.v2.MsgRegisterCounterparty";

const STELLAR_COMMITMENT_PREFIX: &[u8] = b"ibc";

const GAS_LIMIT: u64 = 400_000;

const WAIT: Duration = Duration::from_secs(60);

#[derive(Clone, PartialEq, Eq, Message)]
pub struct MsgRegisterCounterparty {
    #[prost(string, tag = "1")]
    pub client_id: String,
    #[prost(bytes = "vec", repeated, tag = "2")]
    pub counterparty_commitment_prefix: Vec<Vec<u8>>,
    #[prost(string, tag = "3")]
    pub counterparty_client_id: String,
    #[prost(string, tag = "4")]
    pub signer: String,
}

pub async fn run(
    cfg: &Config,
    cc: &ClientsConfig,
    root: &Path,
    http: &reqwest::Client,
    side: &str,
) -> Result<()> {
    let cosmos_client = cc.cosmos_client.as_ref().map(|c| c.as_str()).unwrap_or("");
    let stellar_client = cc.stellar_client.as_ref().map(|c| c.as_str()).unwrap_or("");

    if cosmos_client.is_empty() || stellar_client.is_empty() {
        bail!(
            "both {} and {} must be set — create the clients first",
            cfg.cosmos.client_id_variable(),
            cfg.cosmos.stellar_client_id_variable()
        );
    }

    match side {
        "stellar" => stellar(cfg, root, cosmos_client, stellar_client),
        _ => cosmos(cfg, http, stellar_client, cosmos_client).await,
    }
}

fn stellar(cfg: &Config, root: &Path, client_id: &str, counterparty: &str) -> Result<()> {
    logger::banner("clients counterparty stellar (register the Cosmos client as counterparty)");

    let router = cfg.deployment.ibc_router.clone();

    if router.is_empty() {
        bail!("ROUTER_CONTRACT_ADDRESS is not set in .env — deploy the contracts with `ipsa tx contracts deploy --stellar`, which writes it");
    }

    let prefix = format!("[\"{}\"]", hex::encode(STELLAR_COMMITMENT_PREFIX));

    logger::step(&format!(
        "router.register_counterparty client_id={client_id} counterparty={counterparty}"
    ));

    contracts::invoke(
        &ContractsConfig::from(cfg),
        root,
        &router,
        &[
            "register_counterparty",
            "--client_id",
            client_id,
            "--counterparty_client_id",
            counterparty,
            "--counterparty_commitment_prefix",
            &prefix,
        ],
    )?;

    logger::ok("counterparty registered on Stellar");

    Ok(())
}

async fn cosmos(
    cfg: &Config,
    http: &reqwest::Client,
    client_id: &str,
    counterparty: &str,
) -> Result<()> {
    logger::banner("clients counterparty cosmos (register the Stellar client as counterparty)");

    let signer = CosmosSigner::from_config(&cfg.cosmos, http.clone())?;
    let sender = signer.relayer_address()?.to_string();

    let msg = MsgRegisterCounterparty {
        client_id: client_id.to_string(),
        counterparty_commitment_prefix: vec![Vec::new()],
        counterparty_client_id: counterparty.to_string(),
        signer: sender,
    };

    logger::step(&format!(
        "MsgRegisterCounterparty client_id={client_id} counterparty={counterparty}"
    ));

    let landed = signer
        .broadcast_as_relayer(
            Any {
                type_url: TYPE_URL.to_string(),
                value: msg.encode_to_vec(),
            },
            "register-counterparty",
            GAS_LIMIT,
            crate::services::cosmos::config::fee_for(GAS_LIMIT),
            WAIT,
        )
        .await?;

    let hash = landed
        .pointer("/tx_response/txhash")
        .and_then(|v| v.as_str())
        .unwrap_or("");

    logger::ok(&format!("counterparty registered on Cosmos ({hash})"));

    Ok(())
}
