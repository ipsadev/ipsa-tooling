use std::path::Path;

use anyhow::Result;

use crate::tx::contracts;

#[derive(clap::Args)]
pub struct BuildArgs {
    #[arg(long, help = "Build the Soroban contracts (the default)")]
    pub soroban: bool,
    #[arg(long, help = "Build the CosmWasm light-client wasm")]
    pub cosmos: bool,
}

pub fn run(root: &Path, args: BuildArgs) -> Result<()> {
    if args.cosmos {
        contracts::wasm::build(root)?;
    }

    if args.soroban || !args.cosmos {
        contracts::build::run(root)?;
    }

    Ok(())
}
