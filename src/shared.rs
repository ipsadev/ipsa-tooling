use std::path::Path;

use anyhow::{Context, Result};

use crate::logger;

#[derive(Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum Chain {
    Stellar,
    Cosmos,
    Evm,
    Cardano,
    All,
}

impl Chain {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Stellar => "stellar",
            Self::Cosmos => "cosmos",
            Self::Evm => "evm",
            Self::Cardano => "cardano",
            Self::All => "all",
        }
    }
}

pub fn chain_of(address: &str) -> Option<Chain> {
    let addr = address.trim();

    if addr.starts_with("cosmos1") {
        return Some(Chain::Cosmos);
    }

    let is_evm = addr.len() == 42
        && addr.starts_with("0x")
        && addr[2..]
            .chars()
            .all(|character| character.is_ascii_hexdigit());

    if is_evm {
        return Some(Chain::Evm);
    }

    let is_stellar = addr.len() == 56
        && (addr.starts_with('G') || addr.starts_with('C'))
        && addr
            .chars()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit());

    if is_stellar {
        return Some(Chain::Stellar);
    }

    None
}

pub fn env_upsert(path: &Path, updates: &[(&str, &str)]) -> Result<()> {
    let mut text = std::fs::read_to_string(path).unwrap_or_default();

    for (key, value) in updates {
        if value.is_empty() {
            continue;
        }

        let rendered = if value.contains(char::is_whitespace) {
            format!("\"{value}\"")
        } else {
            (*value).to_string()
        };

        let prefix = format!("{key}=");
        let mut lines: Vec<String> = text.lines().map(str::to_string).collect();
        let mut replaced = false;

        for line in lines.iter_mut() {
            if line.trim_start().starts_with(&prefix) {
                *line = format!("{key}={rendered}");
                replaced = true;
                break;
            }
        }

        if !replaced {
            lines.push(format!("{key}={rendered}"));
        }

        text = lines.join("\n");
        text.push('\n');
    }

    std::fs::write(path, text).with_context(|| format!("writing {}", path.display()))?;

    Ok(())
}

pub fn print_clients(value: &serde_json::Value, filter: Option<&str>) {
    let Some(clients) = value.get("clients").and_then(|c| c.as_array()) else {
        logger::warn("unexpected response shape from /stellar/clients");

        return;
    };

    let mut shown = 0;

    for client in clients {
        let client_type = client
            .get("client_type")
            .and_then(|v| v.as_str())
            .unwrap_or("?");

        let ids: Vec<&str> = client
            .get("client_ids")
            .and_then(|v| v.as_array())
            .map(|arr| arr.iter().filter_map(|v| v.as_str()).collect())
            .unwrap_or_default();

        let ids: Vec<&str> = match filter {
            Some(wanted) => ids.into_iter().filter(|id| *id == wanted).collect(),
            _ => ids,
        };

        if ids.is_empty() {
            continue;
        }

        shown += 1;
        logger::ok(&format!("{client_type}: {}", ids.join(", ")));
    }

    if shown == 0 {
        match filter {
            Some(wanted) => logger::detail(&format!("no client {wanted} on the stellar router")),
            _ => logger::detail("no clients created yet"),
        }
    }
}

pub fn check(name: &str, present: bool, note: &str) {
    if present {
        logger::ok(&format!("{name} found"));
    } else {
        logger::fail(&format!("{name} not found — {note}"));
    }
}

pub fn flag(name: &str, set: bool, note: &str) {
    if set {
        logger::ok(&format!("{name} set"));
    } else {
        logger::warn(&format!("{name} unset — {note}"));
    }
}

#[derive(clap::Args)]
pub struct UpArgs {
    #[arg(long, help = "Start only the Cosmos chain (cosmos)")]
    pub cosmos: bool,
    #[arg(long, help = "Start only the Stellar-side services (api + gateway)")]
    pub stellar: bool,
}

#[derive(clap::Args)]
pub struct DownArgs {
    #[arg(long, help = "Also remove named volumes (wipes chain + key state)")]
    pub volumes: bool,
}

#[derive(clap::Args)]
pub struct StartArgs {
    #[arg(long, help = "Skip pulling the docker images")]
    pub skip_images: bool,
    #[arg(long, help = "Skip importing the relayer keys")]
    pub skip_keys: bool,
    #[arg(long, help = "Skip provisioning the sender + receiver accounts")]
    pub skip_accounts: bool,
    #[arg(long, help = "Skip starting postgres + the ibcv2 relayer")]
    pub skip_relayer: bool,
    #[arg(
        long,
        help = "Skip storing the light-client wasm, creating the clients and registering counterparties"
    )]
    pub skip_clients: bool,
    #[arg(
        long,
        help = "Skip creating the Stellar ↔ EVM clients and registering their counterparties"
    )]
    pub skip_evm: bool,
    #[arg(
        long,
        help = "Redeploy contracts even if ROUTER_CONTRACT_ADDRESS is already set"
    )]
    pub deploy_contracts: bool,
}

#[cfg(test)]
mod tests {
    use super::{chain_of, Chain};

    #[test]
    fn classifies_cosmos_bech32() {
        let addr = "cosmos1qy352eufqy352eufqy352eufqy35qqqqqqqqqq";

        assert!(matches!(chain_of(addr), Some(Chain::Cosmos)));
    }

    #[test]
    fn classifies_stellar_account_and_contract() {
        let account = format!("G{}", "A".repeat(55));
        let contract = format!("C{}", "B".repeat(55));

        assert!(matches!(chain_of(&account), Some(Chain::Stellar)));
        assert!(matches!(chain_of(&contract), Some(Chain::Stellar)));
    }

    #[test]
    fn trims_whitespace_before_classifying() {
        let account = format!("  G{}  ", "A".repeat(55));

        assert!(matches!(chain_of(&account), Some(Chain::Stellar)));
    }

    #[test]
    fn rejects_unknown_addresses() {
        assert!(chain_of("").is_none());
        assert!(chain_of("0xabc123").is_none());
        assert!(chain_of(&format!("g{}", "a".repeat(55))).is_none());
        assert!(chain_of(&format!("G{}", "A".repeat(40))).is_none());
    }
}
