use std::path::Path;

use anyhow::{Context, Result};
use serde_json::{json, Map, Value};

use crate::{
    config::{get, Config},
    logger,
};

pub fn import(cfg: &Config, root: &Path) -> Result<()> {
    logger::banner("relayer keys-import (chain id -> private key)");

    let mut keys = Map::new();

    let cosmos_chain = cfg.cosmos.chain_id.as_str();
    let cosmos_key = get("COSMOS_RELAYER_PRIVATE_KEY", "");

    if cosmos_key.is_empty() {
        logger::warn(&format!(
            "COSMOS_RELAYER_PRIVATE_KEY unset — skipping {cosmos_chain}"
        ));
    } else {
        keys.insert(
            cosmos_chain.to_string(),
            json!({ "private_key": cosmos_key }),
        );
        logger::ok(&format!("{cosmos_chain}: relayer key written"));
    }

    let stellar_chain = cfg.stellar.chain_id.as_str().to_string();
    let stellar_key = get("STELLAR_SIGNING_KEY", "");

    if stellar_key.is_empty() {
        logger::warn(&format!(
            "STELLAR_SIGNING_KEY unset — skipping {stellar_chain}"
        ));
    } else {
        keys.insert(stellar_chain.clone(), json!({ "private_key": stellar_key }));
        logger::ok(&format!("{stellar_chain}: signing key written"));
    }

    if !cosmos_key.is_empty() {
        match crate::services::cosmos::tx::address_from_key(&cosmos_key, &cfg.cosmos.account_prefix)
        {
            Ok(address) => {
                crate::shared::env_upsert(
                    &root.join(".env"),
                    &[("COSMOS_RELAYER_ADDRESS", address.as_str())],
                )?;
                logger::ok(&format!("COSMOS_RELAYER_ADDRESS={address}"));
                logger::hint("the gateway stamps it into every Cosmos message it builds: ipsa services restart --gateway");
            }
            Err(error) => logger::warn(&format!(
                "could not derive COSMOS_RELAYER_ADDRESS: {error:#} — the gateway will build \
                 messages with an empty signer and ibc-go will reject them"
            )),
        }
    }

    let path = Path::new(&cfg.relayer.keys);
    let body = serde_json::to_string_pretty(&Value::Object(keys))?;

    std::fs::write(path, format!("{body}\n"))
        .with_context(|| format!("writing {}", path.display()))?;

    logger::ok(&format!("wrote {}", path.display()));

    Ok(())
}
