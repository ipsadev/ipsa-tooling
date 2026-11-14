use std::path::Path;

use anyhow::{bail, Result};

use super::{config::ContractsConfig, invoke, release, upload};
use crate::{config::Config, logger};

#[derive(clap::Args)]
pub struct UpgradeArgs {
    #[arg(long, help = "Upgrade the router (ROUTER_CONTRACT_ADDRESS)")]
    pub router: bool,
    #[arg(long, help = "Upgrade the transfer app (TRANSFER_CONTRACT_ADDRESS)")]
    pub transfer: bool,
    #[arg(
        long,
        help = "Upgrade the GMP app (GMP_CONTRACT_ADDRESS); apps from before v0.1.2 cannot be upgraded"
    )]
    pub gmp: bool,
    #[arg(
        long,
        conflicts_with_all = ["router", "transfer", "gmp"],
        help = "Migrate the Stellar light client on Cosmos to the LIGHT_CLIENT_VERSION release instead"
    )]
    pub cosmos: bool,
    #[arg(
        long,
        requires = "cosmos",
        help = "With --cosmos: write the migration for the public Cosmos testnet (authz exec files and a gov proposal)"
    )]
    pub testnet: bool,
    #[arg(
        long,
        value_name = "CLIENT_ID",
        requires = "testnet",
        help = "With --testnet: the 08-wasm client to migrate (default: COSMOS_TESTNET_STELLAR_CLIENT_ID)"
    )]
    pub client: Option<String>,
    #[arg(
        long,
        value_name = "KEY",
        requires = "testnet",
        help = "With --testnet: submit the gov proposal from this gaiad key"
    )]
    pub from: Option<String>,
}

pub fn run(cfg: &Config, root: &Path, args: UpgradeArgs) -> Result<()> {
    logger::banner("tx contracts upgrade (replace the code of deployed contracts in place)");

    let targets = [
        (
            args.router,
            "router",
            cfg.deployment.ibc_router.as_str(),
            "ROUTER_CONTRACT_ADDRESS",
            "stellar_ibc_router.wasm",
        ),
        (
            args.transfer,
            "transfer app",
            cfg.deployment.transfer_app.as_str(),
            "TRANSFER_CONTRACT_ADDRESS",
            "stellar_ibc_transfer.wasm",
        ),
        (
            args.gmp,
            "gmp app",
            cfg.deployment.gmp_app.as_str(),
            "GMP_CONTRACT_ADDRESS",
            "stellar_ibc_gmp.wasm",
        ),
    ];

    if !targets.iter().any(|(chosen, ..)| *chosen) {
        bail!("name at least one contract: --router, --transfer or --gmp");
    }

    let cc = ContractsConfig::from(cfg);
    let directory = release::fetch(root)?;

    for (chosen, name, address, variable, wasm) in targets {
        if !chosen {
            continue;
        }

        if address.is_empty() {
            bail!("{variable} is not set in .env");
        }

        let path = directory.join(wasm).display().to_string();

        logger::step(&format!("uploading {wasm}"));
        let hash = upload::hash(&cc, root, &path)?;
        logger::detail(&format!("wasm hash {hash}"));

        logger::step(&format!("upgrading the {name} {address}"));
        invoke(&cc, root, address, &["upgrade", "--wasm_hash", &hash])?;
        logger::ok(&format!("the {name} now runs {hash}"));
    }

    Ok(())
}
