use std::{
    path::Path,
    sync::atomic::{AtomicBool, Ordering},
};

use anyhow::{bail, Context, Result};

use crate::{config::Config, logger};

static RESTARTED: AtomicBool = AtomicBool::new(false);

pub fn ensure(cfg: &Config, root: &Path) -> Result<()> {
    let path = Path::new(&cfg.relayer.config);

    let text =
        std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;

    let router = required(&cfg.deployment.ibc_router, "ROUTER_CONTRACT_ADDRESS")?;

    if !text.contains(router) {
        bail!(
            "{} does not name the deployed router {router}. The relayer filters router events by \
             contract id, so a stale one does not error — it reports finding no packets in a \
             transaction that plainly has them. Set router_contract_id in {}, then: \
             ipsa services restart --relayer",
            path.display(),
            path.display()
        );
    }

    for (id, name) in [
        (
            &cfg.deployment.cosmos_client_id,
            cfg.cosmos.client_id_variable(),
        ),
        (
            &cfg.deployment.stellar_client_id,
            cfg.cosmos.stellar_client_id_variable(),
        ),
    ] {
        let id = required(id, name)?;

        if !names_client(&text, id) {
            bail!(
                "{} has no counterparty_chains entry for {id} ({name}). The relayer maps a \
                 packet's source client to its destination chain with that table, and drops \
                 packets it cannot map — silently, with the relay request still reporting \
                 success. Add {id} to {}, then: ipsa services restart --relayer",
                path.display(),
                path.display()
            );
        }
    }

    logger::ok("relayer.yml names the deployed router and routes both client ids");

    // The relayer parses that file once, at startup. Creating clients earlier in
    // the run rewrites it under a process that has already read the old table.
    if !RESTARTED.swap(true, Ordering::SeqCst) {
        crate::services::restart(root, &[crate::services::Service::Relayer])?;
    }

    Ok(())
}

fn names_client(config: &str, id: &str) -> bool {
    config
        .lines()
        .map(str::trim)
        .any(|line| line.starts_with(&format!("{id}:")))
}

fn required<'a>(value: &'a str, name: &str) -> Result<&'a str> {
    let value = value.trim();

    if value.is_empty() {
        bail!("{name} is unset — run `ipsa start` first");
    }

    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::names_client;

    const CONFIG: &str = "      counterparty_chains:\n        08-wasm-2: 'stellar-testnet'\n";

    #[test]
    fn a_listed_client_is_routable() {
        assert!(names_client(CONFIG, "08-wasm-2"));
    }

    #[test]
    fn an_unlisted_client_is_not() {
        assert!(!names_client(CONFIG, "08-wasm-17"));
    }

    // 08-wasm-2 is a prefix of 08-wasm-20, and routing one is not routing the other.
    #[test]
    fn a_prefix_of_a_listed_client_does_not_count() {
        assert!(!names_client(CONFIG, "08-wasm-20"));
        assert!(!names_client("        08-wasm-20: 'x'", "08-wasm-2"));
    }
}
