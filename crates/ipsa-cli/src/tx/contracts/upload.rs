use std::path::Path;

use anyhow::Result;

use super::last_line;
use crate::{logger, tools, tx::contracts::config::ContractsConfig};

pub fn run(cfg: &ContractsConfig, root: &Path, wasm: &str) -> Result<()> {
    logger::banner("contracts upload");
    logger::step(&format!("stellar contract upload --wasm {wasm}"));

    let digest = hash(cfg, root, wasm)?;

    logger::ok(&format!("wasm hash: {digest}"));
    println!("{digest}");

    Ok(())
}

pub fn hash(cfg: &ContractsConfig, root: &Path, wasm: &str) -> Result<String> {
    let mut args: Vec<String> = vec![
        "contract".into(),
        "upload".into(),
        "--source".into(),
        cfg.signing_key.clone(),
    ];
    args.extend(cfg.net_flags());
    args.push("--wasm".into());
    args.push(wasm.to_string());

    let refs: Vec<&str> = args.iter().map(String::as_str).collect();

    Ok(last_line(&tools::stellar::capture(root, &refs)?))
}
