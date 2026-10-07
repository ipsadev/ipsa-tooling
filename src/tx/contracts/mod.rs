pub mod config;
pub mod deploy_all;
pub mod release;
pub mod upload;
pub mod wasm;

use std::path::Path;

use anyhow::{bail, Result};

use crate::{logger, tools, tx::contracts::config::ContractsConfig};

pub(crate) const SOROBAN_WASMS: [&str; 5] = [
    "stellar_mock_light_client",
    "stellar_ibc_router",
    "stellar_ibc_transfer",
    "stellar_tendermint_light_client",
    "stellar_attestation_light_client",
];

fn wasm_path(directory: &Path, name: &str) -> String {
    directory.join(format!("{name}.wasm")).display().to_string()
}

pub fn upload_all(cfg: &ContractsConfig, root: &Path) -> Result<()> {
    logger::banner("tx contracts upload (install all soroban wasms)");

    let directory = release::fetch(root)?;

    for name in SOROBAN_WASMS {
        logger::step(&format!("uploading {name}"));
        upload::run(cfg, root, &wasm_path(&directory, name))?;
    }

    Ok(())
}

pub fn deploy_one(cfg: &ContractsConfig, root: &Path, name: &str) -> Result<String> {
    logger::banner(&format!("tx contracts deploy --contract {name}"));

    if !SOROBAN_WASMS.contains(&name) {
        bail!(
            "unknown contract '{name}' — expected one of: {}",
            SOROBAN_WASMS.join(", ")
        );
    }

    let directory = release::fetch(root)?;
    let deployer = deploy_all::deployer_address(cfg)?;
    let wasm = wasm_path(&directory, name);

    let ctor: Vec<String> = match name {
        "stellar_ibc_router" => vec!["--owner".into(), deployer],
        "stellar_ibc_transfer" => {
            if cfg.ibc_router.is_empty() {
                bail!("ROUTER_CONTRACT_ADDRESS is unset — deploy the router first or run `tx contracts deploy --stellar`");
            }

            vec![
                "--router".into(),
                cfg.ibc_router.clone(),
                "--owner".into(),
                deployer,
            ]
        }
        _ => vec![],
    };

    let refs: Vec<&str> = ctor.iter().map(String::as_str).collect();
    let id = deploy(cfg, root, &wasm, &refs)?;

    logger::ok(&format!("{name}: {id}"));
    println!("{id}");

    Ok(id)
}

pub(crate) fn last_line(out: &str) -> String {
    out.lines()
        .rev()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or("")
        .to_string()
}

fn stellar_base(cfg: &ContractsConfig, sub: &str) -> Vec<String> {
    stellar_base_as(cfg, sub, &cfg.signing_key)
}

pub(crate) fn latest_ledger(cfg: &ContractsConfig, root: &Path) -> Result<u32> {
    let mut args = vec!["ledger".to_string(), "latest".to_string()];
    args.extend(cfg.net_flags());

    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    let output = tools::stellar::capture_quiet(root, &refs)?;

    parse_sequence(&output)
        .ok_or_else(|| anyhow::anyhow!("no `Sequence:` line in `stellar ledger latest` output"))
}

fn parse_sequence(output: &str) -> Option<u32> {
    output
        .lines()
        .find_map(|line| line.trim().strip_prefix("Sequence:"))
        .and_then(|value| value.trim().parse().ok())
}

fn stellar_base_as(cfg: &ContractsConfig, sub: &str, source: &str) -> Vec<String> {
    let mut args = vec![
        "contract".to_string(),
        sub.to_string(),
        "--source".to_string(),
        source.to_string(),
    ];
    args.extend(cfg.net_flags());

    args
}

pub(crate) fn deploy(
    cfg: &ContractsConfig,
    root: &Path,
    wasm: &str,
    ctor: &[&str],
) -> Result<String> {
    let mut args = stellar_base(cfg, "deploy");
    args.push("--wasm".to_string());
    args.push(wasm.to_string());

    if !ctor.is_empty() {
        args.push("--".to_string());
        args.extend(ctor.iter().map(|s| s.to_string()));
    }

    let refs: Vec<&str> = args.iter().map(String::as_str).collect();

    Ok(last_line(&tools::stellar::capture(root, &refs)?))
}

pub(crate) fn invoke(cfg: &ContractsConfig, root: &Path, id: &str, call: &[&str]) -> Result<()> {
    invoke_as(cfg, root, id, call, &cfg.signing_key).map(|_| ())
}

pub(crate) fn invoke_as(
    cfg: &ContractsConfig,
    root: &Path,
    id: &str,
    call: &[&str],
    source: &str,
) -> Result<String> {
    let mut args = stellar_base_as(cfg, "invoke", source);
    args.push("--id".to_string());
    args.push(id.to_string());
    args.push("--".to_string());
    args.extend(call.iter().map(|s| s.to_string()));

    let refs: Vec<&str> = args.iter().map(String::as_str).collect();

    invoke_with_seq_retry(root, &refs)
}

fn invoke_with_seq_retry(root: &Path, refs: &[&str]) -> Result<String> {
    const MAX_ATTEMPTS: u32 = 6;
    const DELAY_SECS: u64 = 4;

    let mut attempt = 1;

    loop {
        match tools::stellar::capture_all(root, refs) {
            Ok(output) => return Ok(output),
            Err(error) => {
                let stale_sequence = format!("{error:#}").contains("TxBadSeq");

                if stale_sequence && attempt < MAX_ATTEMPTS {
                    logger::detail(&format!(
                        "TxBadSeq — stale stellar sequence, retrying ({attempt}/{MAX_ATTEMPTS}) in {DELAY_SECS}s"
                    ));
                    std::thread::sleep(std::time::Duration::from_secs(DELAY_SECS));
                    attempt += 1;

                    continue;
                }

                return Err(error);
            }
        }
    }
}

pub(crate) fn tx_hash(output: &str) -> Option<String> {
    const MARKERS: [&str; 2] = ["Signing transaction:", "Transaction hash is"];

    output.lines().rev().find_map(|line| {
        MARKERS.iter().find_map(|marker| {
            let rest = line.split(marker).nth(1)?;
            let token = rest.split_whitespace().next()?;

            is_tx_hash(token).then(|| token.to_string())
        })
    })
}

fn is_tx_hash(token: &str) -> bool {
    token.len() == 64 && token.chars().all(|c| c.is_ascii_hexdigit())
}

#[cfg(test)]
mod tests {
    use super::tx_hash;

    const HASH: &str = "3f1c2b8a9d4e5f60718293a4b5c6d7e8f90a1b2c3d4e5f60718293a4b5c6d7e8";
    const OTHER: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

    #[test]
    fn reads_the_signing_transaction_line() {
        let out = format!("ℹ️ Signing transaction: {HASH}\nsuccess\n");

        assert_eq!(tx_hash(&out).as_deref(), Some(HASH));
    }

    #[test]
    fn reads_the_transaction_hash_line() {
        let out = format!("Transaction hash is {HASH}\n🔗 https://stellar.expert/tx/{HASH}\n");

        assert_eq!(tx_hash(&out).as_deref(), Some(HASH));
    }

    #[test]
    fn takes_the_last_hash_when_a_restore_tx_ran_first() {
        let out = format!(
            "ℹ️ Signing transaction: {OTHER}\nℹ️ Transaction hash is {OTHER}\nℹ️ Signing transaction: {HASH}\nℹ️ Transaction hash is {HASH}\n"
        );

        assert_eq!(tx_hash(&out).as_deref(), Some(HASH));
    }

    #[test]
    fn returns_none_when_nothing_was_submitted() {
        assert!(
            tx_hash("Simulation identified as read-only. Send by rerunning with --send=yes.")
                .is_none()
        );
        assert!(tx_hash("").is_none());
        assert!(tx_hash("Signing transaction: not-a-hash").is_none());
        assert!(tx_hash(&format!("Signing transaction: {}", &HASH[..40])).is_none());
    }
}

#[cfg(test)]
mod ledger_tests {
    use super::parse_sequence;

    #[test]
    fn the_sequence_line_is_read() {
        let out = "Sequence: 4358291\nProtocol Version: 27\nHash: bba34545\n";

        assert_eq!(parse_sequence(out), Some(4_358_291));
    }

    #[test]
    fn output_without_a_sequence_yields_nothing() {
        assert_eq!(parse_sequence("Protocol Version: 27\n"), None);
    }
}
