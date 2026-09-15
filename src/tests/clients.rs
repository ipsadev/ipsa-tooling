use std::path::Path;

use anyhow::{bail, Result};

use crate::{config::Config, logger, tx::clients::config::ClientsConfig};

pub async fn run(cfg: &Config, root: &Path, http: &reqwest::Client) -> Result<()> {
    let cc = ClientsConfig::from(cfg);

    let cosmos_client = crate::tx::clients::cosmos::run(cfg, &cc, root, http, true).await?;
    let stellar_client = crate::tx::clients::stellar::run(cfg, &cc, root, http, true).await?;

    expect_prefix(&cosmos_client, "07-tendermint")?;
    expect_prefix(&stellar_client, "08-wasm")?;

    logger::ok(&format!(
        "created cosmos={cosmos_client} stellar={stellar_client}"
    ));

    Ok(())
}

fn expect_prefix(id: &str, prefix: &str) -> Result<()> {
    if !id.starts_with(prefix) {
        bail!("expected a {prefix}-N client id, got {id:?}");
    }

    Ok(())
}
