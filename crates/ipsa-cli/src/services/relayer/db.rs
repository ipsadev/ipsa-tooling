use std::path::Path;

use anyhow::{anyhow, Result};
use serde_json::Value;

use crate::{
    config::{self, Config},
    logger, probe, tools,
};

const DEFAULT_LIMIT: u32 = 20;

pub fn reset(root: &Path) -> Result<()> {
    logger::step("resetting the relayer database so packet sequences can restart at 1");

    tools::docker::compose(root, &["up", "-d", "--wait", "postgres"])?;

    let user = config::get("POSTGRES_USER", "relayer");
    let database = config::get("POSTGRES_DB", "relayer");

    tools::docker::compose(
        root,
        &[
            "exec",
            "-T",
            "postgres",
            "psql",
            "--username",
            &user,
            "--dbname",
            &database,
            "--set",
            "ON_ERROR_STOP=1",
            "--command",
            "drop schema public cascade; create schema public;",
        ],
    )
}

pub async fn transfers(
    cfg: &Config,
    http: &reqwest::Client,
    limit: Option<u32>,
    status: Option<&str>,
) -> Result<()> {
    logger::banner("relayer transfers");

    let mut url = format!(
        "{}/relay/transfers?limit={}",
        cfg.stellar.api_url.trim_end_matches('/'),
        limit.unwrap_or(DEFAULT_LIMIT)
    );

    if let Some(status) = status {
        url.push_str(&format!("&status={status}"));
    }

    let value = probe::get_json(http, &url)
        .await
        .ok_or_else(|| anyhow!("could not read {url}"))?;

    let rows = value
        .get("transfers")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow!("unexpected response shape from {url}"))?;

    if rows.is_empty() {
        logger::detail("the relayer is tracking no packets");
        return Ok(());
    }

    for row in rows {
        logger::detail(&describe(row));
    }

    logger::ok(&format!("{} packet(s)", rows.len()));

    Ok(())
}

pub async fn summary(cfg: &Config, http: &reqwest::Client) -> Result<()> {
    logger::banner("relayer summary");

    let url = format!(
        "{}/relay/transfers/summary",
        cfg.stellar.api_url.trim_end_matches('/')
    );

    let value = probe::get_json(http, &url)
        .await
        .ok_or_else(|| anyhow!("could not read {url}"))?;

    let counts = value
        .get("by_status")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow!("unexpected response shape from {url}"))?;

    if counts.is_empty() {
        logger::detail("the relayer is tracking no packets");
        return Ok(());
    }

    for entry in counts {
        logger::detail(&format!(
            "{:<34} {}",
            text(entry, "status"),
            entry.get("count").and_then(Value::as_i64).unwrap_or(0)
        ));
    }

    logger::ok(&format!(
        "{} tracked",
        value.get("total").and_then(Value::as_i64).unwrap_or(0)
    ));

    Ok(())
}

fn describe(row: &Value) -> String {
    let stage = [
        "timeout_tx_hash",
        "ack_tx_hash",
        "write_ack_tx_hash",
        "recv_tx_hash",
    ]
    .iter()
    .find(|key| row.get(**key).and_then(Value::as_str).is_some())
    .map(|key| key.trim_end_matches("_tx_hash"))
    .unwrap_or("sent");

    format!(
        "seq {:<4} {} -> {}  {:<34} {:>6}s  last: {}",
        row.get("sequence").and_then(Value::as_i64).unwrap_or(0),
        text(row, "source_client_id"),
        text(row, "destination_client_id"),
        text(row, "status"),
        row.get("age_seconds").and_then(Value::as_i64).unwrap_or(0),
        stage,
    )
}

fn text(value: &Value, key: &str) -> String {
    value
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or("-")
        .to_string()
}
