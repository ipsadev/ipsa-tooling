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

fn with_field(text: &str, chain_id: &str, key: &str, value: &str) -> Result<String> {
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

fn with_route(
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

#[cfg(test)]
mod tests {
    use super::with_route;

    const CONFIG: &str = "chains:\n  simd-1:\n    type: 'cosmos'\n    ibcv2:\n      counterparty_chains:\n        08-wasm-2: 'stellar-testnet'\n\n  stellar-testnet:\n    type: 'stellar'\n    ibcv2:\n      counterparty_chains:\n        07-tendermint-6: 'simd-1'\n";

    #[test]
    fn a_new_client_id_replaces_the_route_to_the_same_chain() {
        let out = with_route(CONFIG, "simd-1", "08-wasm-30", "stellar-testnet").unwrap();

        assert!(out.contains("        08-wasm-30: 'stellar-testnet'"));
        assert!(!out.contains("08-wasm-2:"));
    }

    #[test]
    fn the_other_chains_table_is_left_alone() {
        let out = with_route(CONFIG, "simd-1", "08-wasm-30", "stellar-testnet").unwrap();

        assert!(out.contains("        07-tendermint-6: 'simd-1'"));
    }

    #[test]
    fn writing_the_same_route_twice_changes_nothing() {
        let once = with_route(CONFIG, "simd-1", "08-wasm-30", "stellar-testnet").unwrap();
        let twice = with_route(&once, "simd-1", "08-wasm-30", "stellar-testnet").unwrap();

        assert_eq!(once, twice);
    }

    #[test]
    fn a_table_with_no_route_to_that_chain_gains_one() {
        let config = "chains:\n  simd-1:\n    ibcv2:\n      counterparty_chains:\n";
        let out = with_route(config, "simd-1", "08-wasm-1", "stellar-testnet").unwrap();

        assert!(out.contains("        08-wasm-1: 'stellar-testnet'"));
    }

    #[test]
    fn a_chain_block_is_found_by_its_chain_id_when_its_key_differs() {
        let config = "chains:\n  stellar-testnet:\n    chain_id: 'stellar-testnet'\n    ibcv2:\n      counterparty_chains: {}\n  sepolia:\n    chain_id: '11155111'\n    ibcv2:\n      counterparty_chains: {}\n";
        let out = with_route(config, "11155111", "stellar-testnet-2", "stellar-testnet").unwrap();

        assert!(out.contains("  sepolia:\n    chain_id: '11155111'\n    ibcv2:\n      counterparty_chains:\n        stellar-testnet-2: 'stellar-testnet'"));
        assert!(out.contains("  stellar-testnet:\n    chain_id: 'stellar-testnet'\n    ibcv2:\n      counterparty_chains: {}"));
    }

    #[test]
    fn an_unknown_chain_is_an_error_not_a_silent_no_op() {
        assert!(with_route(CONFIG, "osmosis-1", "08-wasm-1", "stellar-testnet").is_err());
    }

    #[test]
    fn an_empty_inline_table_is_expanded_to_hold_the_route() {
        let config = "chains:\n  simd-1:\n    ibcv2:\n      counterparty_chains: {}\n";

        let out = with_route(config, "simd-1", "08-wasm-0", "stellar-testnet").unwrap();

        assert_eq!(
            out,
            "chains:\n  simd-1:\n    ibcv2:\n      counterparty_chains:\n        08-wasm-0: 'stellar-testnet'\n"
        );
    }

    #[test]
    fn a_chain_without_the_table_is_an_error() {
        let config = "chains:\n  simd-1:\n    type: 'cosmos'\n";

        assert!(with_route(config, "simd-1", "08-wasm-1", "stellar-testnet").is_err());
    }

    #[test]
    fn a_contract_id_is_replaced_in_place() {
        let config =
            "chains:\n  stellar-testnet:\n    stellar:\n      router_contract_id: 'COLD'\n";
        let out =
            super::with_field(config, "stellar-testnet", "router_contract_id", "CNEW").unwrap();

        assert!(out.contains("      router_contract_id: 'CNEW'"));
        assert!(!out.contains("COLD"));
    }

    #[test]
    fn a_missing_field_is_an_error_not_an_invented_one() {
        let config = "chains:\n  stellar-testnet:\n    stellar:\n";

        assert!(super::with_field(config, "stellar-testnet", "router_contract_id", "C").is_err());
    }

    #[test]
    fn the_trailing_newline_survives() {
        let out = with_route(CONFIG, "simd-1", "08-wasm-30", "stellar-testnet").unwrap();

        assert!(out.ends_with('\n'));
    }
}
