pub mod config;
pub mod cosmos;
pub mod counterparty;
pub mod stellar;

use std::path::Path;

use anyhow::Result;

use crate::{logger, shared, tx::clients::config::ClientsConfig};

pub(crate) struct CreateSpec<'a> {
    pub host_chain: &'a str,
    pub reference_chain: &'a str,
    pub result_env_var: &'a str,
}

// A new client id has to reach both files the stack reads it from: .env, which
// every ipsa command loads, and relayer.yml, whose counterparty_chains table is
// how the relayer maps a packet's source client to its destination chain.
pub(crate) fn record(
    cfg: &ClientsConfig,
    root: &Path,
    spec: &CreateSpec<'_>,
    client_id: &str,
) -> Result<()> {
    shared::env_upsert(&root.join(".env"), &[(spec.result_env_var, client_id)])?;
    logger::detail(&format!("{}={client_id}", spec.result_env_var));

    let path = Path::new(&cfg.relayer_config);

    match crate::services::relayer::wiring::set_route(
        path,
        spec.host_chain,
        client_id,
        spec.reference_chain,
    ) {
        Ok(true) => {
            logger::detail(&format!(
                "{}: {} counterparty_chains {client_id} -> {}",
                path.display(),
                spec.host_chain,
                spec.reference_chain
            ));
            logger::hint("the relayer reads that file at startup: ipsa services restart --relayer");
        }
        Ok(false) => logger::detail(&format!("{} already routed {client_id}", path.display())),
        Err(error) => logger::warn(&format!(
            "could not route {client_id} in {}: {error:#} — the ibc-relayer will drop its packets \
             until that table names it",
            path.display()
        )),
    }

    Ok(())
}
