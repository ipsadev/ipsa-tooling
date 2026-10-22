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
