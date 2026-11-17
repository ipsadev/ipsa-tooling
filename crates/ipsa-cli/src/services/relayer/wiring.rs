use std::path::Path;

use anyhow::{anyhow, bail, Context, Result};

pub fn set_field(path: &Path, chain_id: &str, key: &str, value: &str) -> Result<bool> {
    let text =
        std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;

    let updated = with_field(&text, chain_id, key, value)?;

    if updated == text {
        return Ok(false);
    }

    std::fs::write(path, &updated).with_context(|| format!("writing {}", path.display()))?;

    Ok(true)
}

pub fn with_field(text: &str, chain_id: &str, key: &str, value: &str) -> Result<String> {
    let mut lines: Vec<String> = text.lines().map(str::to_string).collect();

    let chain = chain_block(&lines, chain_id).ok_or_else(|| {
        anyhow!("no `{chain_id}:` chain block — the relayer has no config for that chain")
    })?;

    let chain_end = block_end(&lines, chain);
    let needle = format!("{key}:");

    for i in chain + 1..chain_end {
        if lines[i].trim_start().starts_with(&needle) && !lines[i].trim_start().starts_with('#') {
            lines[i] = format!("{}{key}: '{value}'", " ".repeat(indent(&lines[i])));

            return Ok(join(&lines, text));
        }
    }

    bail!("the `{chain_id}:` block has no `{key}` to set")
}

pub fn set_route(
    path: &Path,
    chain_id: &str,
    client_id: &str,
    counterparty_chain: &str,
) -> Result<bool> {
    let text =
        std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;

    let updated = with_route(&text, chain_id, client_id, counterparty_chain)?;

    if updated == text {
        return Ok(false);
    }

    std::fs::write(path, &updated).with_context(|| format!("writing {}", path.display()))?;

    Ok(true)
}

pub fn with_route(
    text: &str,
    chain_id: &str,
    client_id: &str,
    counterparty_chain: &str,
) -> Result<String> {
    let mut lines: Vec<String> = text.lines().map(str::to_string).collect();

    let chain = chain_block(&lines, chain_id).ok_or_else(|| {
        anyhow!("no `{chain_id}:` chain block — the relayer has no config for that chain")
    })?;

    let chain_end = block_end(&lines, chain);

    if let Some(empty) = empty_table(&lines, chain + 1, chain_end, "counterparty_chains") {
        lines[empty] = format!("{}counterparty_chains:", " ".repeat(indent(&lines[empty])));
    }

    let table =
        block_key(&lines, chain + 1, chain_end, "counterparty_chains").ok_or_else(|| {
            anyhow!(
            "the `{chain_id}:` block has no `counterparty_chains:` table to route {client_id} with"
        )
        })?;

    let table_end = block_end(&lines, table);
    let entry_indent = indent(&lines[table]) + 2;
    let rendered = format!(
        "{}{client_id}: '{counterparty_chain}'",
        " ".repeat(entry_indent)
    );

    for i in table + 1..table_end {
        if routes_to(&lines[i], counterparty_chain) {
            lines[i] = rendered;

            return Ok(join(&lines, text));
        }
    }

    lines.insert(table + 1, rendered);

    Ok(join(&lines, text))
}

fn chain_block(lines: &[String], chain_id: &str) -> Option<usize> {
    if let Some(found) = block_key(lines, 0, lines.len(), chain_id) {
        return Some(found);
    }

    let chains = block_key(lines, 0, lines.len(), "chains")?;
    let chains_end = block_end(lines, chains);
    let child = indent(&lines[chains]) + 2;

    (chains + 1..chains_end)
        .filter(|&i| indent(&lines[i]) == child && lines[i].trim().ends_with(':'))
        .find(|&i| {
            let end = block_end(lines, i);

            lines[i + 1..end].iter().any(|line| {
                line.trim()
                    .strip_prefix("chain_id:")
                    .map(|value| value.trim().trim_matches(|c| c == '\'' || c == '"') == chain_id)
                    .unwrap_or(false)
            })
        })
}

fn block_key(lines: &[String], from: usize, to: usize, key: &str) -> Option<usize> {
    let needle = format!("{key}:");

    (from..to).find(|&i| lines[i].trim() == needle && !lines[i].trim_start().starts_with('#'))
}

fn empty_table(lines: &[String], from: usize, to: usize, key: &str) -> Option<usize> {
    let needle = format!("{key}:");

    (from..to).find(|&i| {
        let trimmed = lines[i].trim();

        trimmed.starts_with(&needle)
            && trimmed[needle.len()..].trim() == "{}"
            && !trimmed.starts_with('#')
    })
}

fn block_end(lines: &[String], start: usize) -> usize {
    let parent = indent(&lines[start]);

    for (offset, line) in lines.iter().enumerate().skip(start + 1) {
        if line.trim().is_empty() {
            continue;
        }
        if indent(line) <= parent {
            return offset;
        }
    }

    lines.len()
}

fn routes_to(line: &str, counterparty_chain: &str) -> bool {
    let trimmed = line.trim();

    if trimmed.is_empty() || trimmed.starts_with('#') {
        return false;
    }

    let Some((_, value)) = trimmed.split_once(':') else {
        return false;
    };

    value.trim().trim_matches(|c| c == '\'' || c == '"') == counterparty_chain
}

fn indent(line: &str) -> usize {
    line.len() - line.trim_start().len()
}

fn join(lines: &[String], original: &str) -> String {
    let mut out = lines.join("\n");

    if original.ends_with('\n') {
        out.push('\n');
    }

    out
}
