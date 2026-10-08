use std::path::Path;

use anyhow::{bail, Context, Result};

use crate::{
    logger, tools,
    tx::contracts::{self, config::ContractsConfig},
};

const CONFIG_PATH: &str = "config/assets.json";

const DEFAULT_ASSETS: &str = r#"[{ "denom": "XLM", "asset": "native", "kind": "local" }]"#;

pub struct AssetEntry {
    pub denom: String,
    pub asset: String,
    pub voucher: bool,
}

pub fn load() -> Result<Vec<AssetEntry>> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(CONFIG_PATH);
    let raw = std::fs::read_to_string(&path).unwrap_or_else(|error| {
        logger::warn(&format!(
            "no asset config at {} ({error}); falling back to {DEFAULT_ASSETS}",
            path.display()
        ));
        DEFAULT_ASSETS.to_string()
    });

    parse(&raw)
}

fn parse(raw: &str) -> Result<Vec<AssetEntry>> {
    let value: serde_json::Value = serde_json::from_str(raw).context("parsing assets.json")?;
    let items = value
        .as_array()
        .context("assets.json must be a JSON array of asset entries")?;

    let mut entries = Vec::new();
    for item in items {
        let denom = field(item, "denom")?;
        let asset = field(item, "asset")?;
        let kind = item.get("kind").and_then(|v| v.as_str()).unwrap_or("local");
        let voucher = match kind {
            "voucher" => true,
            "local" => false,
            other => {
                bail!("asset '{denom}': unknown kind '{other}' (expected 'local' or 'voucher')")
            }
        };

        entries.push(AssetEntry {
            denom,
            asset,
            voucher,
        });
    }

    Ok(entries)
}

fn field(item: &serde_json::Value, key: &str) -> Result<String> {
    item.get(key)
        .and_then(|v| v.as_str())
        .map(str::to_string)
        .with_context(|| format!("asset entry missing string field '{key}'"))
}

pub fn find(denom: &str) -> Result<Option<AssetEntry>> {
    Ok(load()?.into_iter().find(|e| e.denom == denom))
}

pub fn register_all(cfg: &ContractsConfig, root: &Path, transfer_app: &str) -> Result<()> {
    let entries = load()?;
    if entries.is_empty() {
        logger::detail("no assets configured in assets.json — skipping registration");

        return Ok(());
    }

    for entry in &entries {
        let kind = if entry.voucher { "voucher" } else { "local" };
        logger::step(&format!(
            "register asset {} ({}, {kind})",
            entry.denom, entry.asset
        ));

        let sac = resolve_sac(cfg, root, &entry.asset)?;
        logger::detail(&format!("{} SAC → {sac}", entry.asset));

        contracts::invoke(
            cfg,
            root,
            transfer_app,
            &[
                "register_asset",
                "--denom",
                &entry.denom,
                "--address",
                &sac,
                "--voucher",
                if entry.voucher { "true" } else { "false" },
            ],
        )?;
    }

    Ok(())
}

pub fn registered_denom(
    cfg: &ContractsConfig,
    root: &Path,
    transfer_app: &str,
    entry: &AssetEntry,
) -> Result<String> {
    if entry.voucher {
        return Ok(entry.denom.clone());
    }

    let sac = resolve_sac(cfg, root, &entry.asset)?;
    let output = contracts::invoke_as(cfg, root, transfer_app, &["assets"], &cfg.signing_key)?;

    let listing = output
        .lines()
        .map(str::trim)
        .rfind(|line| line.starts_with('['))
        .with_context(|| format!("the transfer app {transfer_app} listed no assets"))?;

    let assets: Vec<serde_json::Value> = serde_json::from_str(listing)
        .with_context(|| format!("the transfer app's asset list is not JSON: {listing}"))?;

    let denom = assets
        .iter()
        .find(|asset| asset["address"].as_str() == Some(sac.as_str()))
        .and_then(|asset| asset["denom"].as_str())
        .with_context(|| {
            format!(
                "{} ({sac}) is not registered on the transfer app {transfer_app}",
                entry.denom
            )
        })?;

    Ok(denom.to_string())
}

pub fn resolve_sac(cfg: &ContractsConfig, root: &Path, asset: &str) -> Result<String> {
    let args = [
        "contract",
        "id",
        "asset",
        "--asset",
        asset,
        "--network-passphrase",
        &cfg.network_passphrase,
    ];

    let out = tools::stellar::capture(root, &args)?;
    let sac = out
        .lines()
        .map(str::trim)
        .rfind(|line| line.starts_with('C'))
        .with_context(|| format!("could not resolve a SAC address for asset '{asset}'"))?;

    Ok(sac.to_string())
}
