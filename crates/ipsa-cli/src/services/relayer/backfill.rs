use std::path::Path;

use anyhow::{anyhow, Context, Result};
use serde_json::Value;

use crate::{config::Config, logger, probe};

const PAGE: u64 = 500;

#[derive(Default)]
struct Plan {
    statements: Vec<String>,
    transfers: usize,
    contract_calls: usize,
    without_amount: usize,
}

pub async fn backfill(
    cfg: &Config,
    http: &reqwest::Client,
    api_url: Option<&str>,
    out: &Path,
) -> Result<()> {
    logger::banner("relayer backfill");

    let base = api_url
        .unwrap_or(&cfg.stellar.api_url)
        .trim_end_matches('/')
        .to_string();

    let rows = read_transfers(http, &base).await?;

    logger::detail(&format!("{} transfer(s) read from {base}", rows.len()));

    let plan = plan(&rows);

    std::fs::write(out, script(&base, &plan.statements))
        .with_context(|| format!("writing {}", out.display()))?;

    logger::detail(&format!(
        "{} with an amount, {} contract call(s), {} without an amount the api could read",
        plan.transfers, plan.contract_calls, plan.without_amount
    ));
    logger::ok(&format!("wrote {}", out.display()));
    logger::hint(
        "each statement only fills empty columns, so running the script twice changes nothing",
    );

    Ok(())
}

async fn read_transfers(http: &reqwest::Client, base: &str) -> Result<Vec<Value>> {
    let mut rows = Vec::new();
    let mut offset = 0;

    loop {
        let url = format!("{base}/relay/transfers?limit={PAGE}&offset={offset}");

        let value = probe::get_json(http, &url)
            .await
            .ok_or_else(|| anyhow!("could not read {url}"))?;

        let page = value
            .get("transfers")
            .and_then(Value::as_array)
            .ok_or_else(|| anyhow!("unexpected response shape from {url}"))?;

        let total = value.get("total").and_then(Value::as_u64).unwrap_or(0);

        rows.extend(page.iter().cloned());
        offset += PAGE;

        if page.is_empty() || offset >= total {
            return Ok(rows);
        }
    }
}

fn plan(rows: &[Value]) -> Plan {
    let mut plan = Plan::default();

    for row in rows {
        let Some(id) = row.get("id").and_then(Value::as_i64) else {
            continue;
        };

        match row.get("application").and_then(Value::as_str) {
            Some("ics27") => {
                plan.statements.push(format!(
                    "UPDATE ibcv2_transfers SET updated_at = now(), application = 'ics27' \
                     WHERE id = {id} AND application IS NULL;"
                ));
                plan.contract_calls += 1;
            }
            Some("ics20") => match transfer_statement(id, row) {
                Some(statement) => {
                    plan.statements.push(statement);
                    plan.transfers += 1;
                }
                None => plan.without_amount += 1,
            },
            _ => plan.without_amount += 1,
        }
    }

    plan
}

fn transfer_statement(id: i64, row: &Value) -> Option<String> {
    let amount = text(row, "amount").filter(|amount| amount.chars().all(|c| c.is_ascii_digit()))?;
    let denom = text(row, "denom")?;

    Some(format!(
        "UPDATE ibcv2_transfers SET updated_at = now(), \
         application = coalesce(application, 'ics20'), \
         sender = coalesce(sender, {}), \
         receiver = coalesce(receiver, {}), \
         transfer_amount = {amount}, \
         denom = coalesce(denom, {}) \
         WHERE id = {id} AND transfer_amount IS NULL;",
        literal(text(row, "sender")),
        literal(text(row, "receiver")),
        literal(Some(denom)),
    ))
}

fn text<'a>(row: &'a Value, key: &str) -> Option<&'a str> {
    row.get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
}

fn literal(value: Option<&str>) -> String {
    match value {
        Some(value) => format!("'{}'", value.replace('\'', "''")),
        None => "NULL".to_string(),
    }
}

fn script(base: &str, statements: &[String]) -> String {
    let mut lines = vec![
        format!("-- ipsa relayer backfill, read from {base}"),
        "-- fills application, sender, receiver, transfer_amount and denom where the relayer left them empty".to_string(),
        "BEGIN;".to_string(),
    ];

    lines.extend(statements.iter().cloned());
    lines.push("COMMIT;".to_string());
    lines.push(
        "SELECT count(*) AS transfers, count(transfer_amount) AS with_amount FROM ibcv2_transfers;"
            .to_string(),
    );

    lines.join("\n") + "\n"
}
