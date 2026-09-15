use std::path::Path;

use anyhow::{bail, Result};

use crate::{config::Config, logger};

mod clients;
mod counterparty;
mod query;
mod relay;
mod relayer_routes;
mod transfer;

#[derive(clap::Args)]
pub struct TestArgs {
    #[arg(
        long,
        value_enum,
        help = "ICS flow to test (omit to run every flow in dependency order)"
    )]
    pub ics: Option<Ics>,
}

#[derive(clap::ValueEnum, Clone, Copy, PartialEq, Eq)]
pub enum Ics {
    #[value(name = "ics02-clients")]
    Ics02Clients,
    #[value(name = "ics02-counterparty")]
    Ics02Counterparty,
    #[value(name = "ics20-transfer")]
    Ics20Transfer,
    #[value(name = "ics20-relay")]
    Ics20Relay,
    #[value(name = "ics02-query")]
    Ics02Query,
}

impl Ics {
    fn label(self) -> &'static str {
        match self {
            Self::Ics02Clients => "ics02-clients",
            Self::Ics02Counterparty => "ics02-counterparty",
            Self::Ics20Transfer => "ics20-transfer",
            Self::Ics20Relay => "ics20-relay",
            Self::Ics02Query => "ics02-query",
        }
    }

    async fn exec(self, cfg: &Config, root: &Path, http: &reqwest::Client) -> Result<()> {
        match self {
            Self::Ics02Clients => clients::run(cfg, root, http).await,
            Self::Ics02Counterparty => counterparty::run(cfg, root, http).await,
            Self::Ics20Transfer => transfer::run(cfg, root, http).await,
            Self::Ics20Relay => relay::run(cfg, root, http).await,
            Self::Ics02Query => query::run(cfg, root, http).await,
        }
    }
}

const ALL: [Ics; 5] = [
    Ics::Ics02Clients,
    Ics::Ics02Counterparty,
    Ics::Ics20Transfer,
    Ics::Ics20Relay,
    Ics::Ics02Query,
];

pub async fn run(root: &Path, http: &reqwest::Client, args: TestArgs) -> Result<()> {
    match args.ics {
        Some(ics) => one(ics, root, http).await,
        None => every(root, http).await,
    }
}

async fn one(ics: Ics, root: &Path, http: &reqwest::Client) -> Result<()> {
    logger::banner(&format!("ipsa test — ics {}", ics.label()));

    let cfg = Config::load(root);

    ics.exec(&cfg, root, http).await?;

    logger::ok(&format!("ics {} passed", ics.label()));

    Ok(())
}

async fn every(root: &Path, http: &reqwest::Client) -> Result<()> {
    logger::banner("ipsa test — all ics flows");

    let mut failures = Vec::new();

    for ics in ALL {
        logger::step(&format!("ics {}", ics.label()));

        let cfg = Config::load(root);

        match ics.exec(&cfg, root, http).await {
            Ok(()) => logger::ok(&format!("ics {} passed", ics.label())),
            Err(err) => {
                logger::warn(&format!("ics {} failed: {err:#}", ics.label()));
                failures.push(ics.label());
            }
        }
    }

    if !failures.is_empty() {
        bail!("ics flows failed: {}", failures.join(", "));
    }

    logger::ok("all ics flows passed");

    Ok(())
}
