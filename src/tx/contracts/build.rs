use std::path::Path;

use anyhow::Result;

use crate::{logger, tools};

pub fn run(root: &Path) -> Result<()> {
    logger::banner("contracts build");

    let contracts_dir = root.join("contracts/soroban");

    logger::step("stellar contract build --profile contract");
    tools::stellar::command(
        contracts_dir.as_path(),
        &["contract", "build", "--profile", "contract"],
    )?;

    logger::ok("built → contracts/target/wasm32v1-none/contract/");

    Ok(())
}
