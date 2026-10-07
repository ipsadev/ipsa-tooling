use std::path::Path;

use anyhow::Result;

use crate::tx::contracts;

#[derive(clap::Args)]
pub struct BuildArgs {
    #[arg(
        long,
        help = "Fetch and verify the released Soroban contracts (the default)"
    )]
    pub soroban: bool,
    #[arg(long, help = "Build the CosmWasm light-client wasm")]
    pub cosmos: bool,
}

pub fn run(root: &Path, args: BuildArgs) -> Result<()> {
    if args.cosmos {
        contracts::wasm::build(root)?;
    }

    if args.soroban || !args.cosmos {
        contracts::release::fetch(root)?;
    }

    Ok(())
}
