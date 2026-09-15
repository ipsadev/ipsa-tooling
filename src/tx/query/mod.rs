use core::unimplemented;
use std::path::Path;

use anyhow::{bail, Result};

use crate::{
    config::Config,
    shared::{self, Chain},
};

mod balances;
mod clients;

#[derive(clap::Args)]
pub struct QueryArgs {
    #[arg(long, help = "Query client states")]
    pub clients: Option<bool>,
    #[arg(long, help = "Scope to the Stellar network")]
    pub stellar: bool,
    #[arg(long, help = "Scope to the Cosmos network")]
    pub cosmos: bool,
    #[arg(long, value_name = "ID", help = "Restrict to a single client id")]
    pub client_id: Option<String>,
    #[arg(long, value_name = "address", help = "Query an address balances")]
    pub address: Option<String>,
    #[arg(long, value_name = "denom", help = "Query by denom")]
    pub denom: Option<String>,
}

fn get_chain(cosmos: bool, stellar: bool) -> Chain {
    match (cosmos, stellar) {
        (true, false) => Chain::Cosmos,
        (false, true) => Chain::Stellar,
        _ => Chain::All,
    }
}

pub async fn run(cfg: &Config, root: &Path, http: &reqwest::Client, args: QueryArgs) -> Result<()> {
    let chain = get_chain(args.cosmos, args.stellar);

    if args.clients.is_some() {
        get_clients(cfg, http, chain, args).await?;
    } else if args.address.is_some() {
        balance_of(cfg, root, http, args).await?;
    }

    Ok(())
}

pub async fn get_clients(
    cfg: &Config,
    http: &reqwest::Client,
    chain: Chain,
    args: QueryArgs,
) -> Result<()> {
    let id = args.client_id.as_deref();

    match chain {
        Chain::Cosmos => {
            clients::cosmos_clients(cfg, http, id).await;
        }
        Chain::Stellar => {
            clients::stellar_clients(cfg, http, id).await;
        }
        Chain::Cardano => {
            unimplemented!()
        }
        Chain::All => {
            clients::stellar_clients(cfg, http, id).await;
            clients::cosmos_clients(cfg, http, id).await;
        }
    }

    Ok(())
}

async fn balance_of(
    cfg: &Config,
    root: &Path,
    http: &reqwest::Client,
    args: QueryArgs,
) -> Result<()> {
    let denom = args.denom.as_deref();

    if let Some(address) = args.address {
        if let Some(chain) = shared::chain_of(&address) {
            match chain {
                Chain::Cosmos => {
                    balances::cosmos_balances(cfg, http, &address, denom).await;
                }
                Chain::Stellar => {
                    balances::stellar_balances(cfg, root, &address, denom);
                }
                Chain::Cardano => {
                    unimplemented!()
                }
                Chain::All => {
                    balances::stellar_balances(cfg, root, &address, denom);
                    balances::cosmos_balances(cfg, http, &address, denom).await;
                }
            }
        }
    } else {
        bail!("address is required")
    }

    Ok(())
}
