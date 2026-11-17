use anyhow::{anyhow, bail, Result};
use ipsa::gateway::stellar::QuorumConfig;
use sha2::{Digest, Sha256};
use stellar_xdr::{Limits, NodeId, PublicKey, ReadXdr, ScpQuorumSet};

use crate::logger;

pub fn verify_quorum_fingerprints(configs: &[QuorumConfig], pinned: &[String]) -> Result<()> {
    if pinned.is_empty() {
        bail!(
            "PINNED_QUORUM_SET_HASHES is empty — the quorum sets arrive over the gateway, which is \
             untrusted transport. Verify one independently (scp-verify --ledger <n>) and pin its \
             sha256 before creating a client"
        );
    }

    if configs.is_empty() {
        bail!("the gateway served no quorum configs — there is no trust root to check");
    }

    let allowed: Vec<[u8; 32]> = pinned
        .iter()
        .map(|entry| normalise(entry))
        .collect::<Result<_>>()?;

    for (index, config) in configs.iter().enumerate() {
        let digest: [u8; 32] = Sha256::digest(&config.quorum_set_xdr).into();

        if !allowed.contains(&digest) {
            bail!(
                "quorum config {index} (valid_from {}) hashes to {} which is not pinned — refusing \
                 to seed a light client with a validator set served over untrusted transport",
                config.valid_from,
                hex::encode(digest)
            );
        }
    }

    Ok(())
}

pub fn describe_trust_root(configs: &[QuorumConfig]) -> Result<()> {
    logger::step("decoded trust root (the quorum configuration seeded into the client):");

    for (index, config) in configs.iter().enumerate() {
        let digest: [u8; 32] = Sha256::digest(&config.quorum_set_xdr).into();

        logger::detail(&format!(
            "config {index}: valid_from {}, sha256 {}",
            config.valid_from,
            hex::encode(digest)
        ));

        let set =
            ScpQuorumSet::from_xdr(&config.quorum_set_xdr, Limits::none()).map_err(|error| {
                anyhow!("quorum config {index} does not decode as an ScpQuorumSet: {error}")
            })?;

        for line in describe_quorum_set(&set, 1) {
            logger::detail(&line);
        }
    }

    Ok(())
}

pub fn describe_quorum_set(set: &ScpQuorumSet, depth: usize) -> Vec<String> {
    let indent = "  ".repeat(depth);

    let mut lines = vec![format!(
        "{indent}threshold {} of {} validator(s) and {} inner set(s)",
        set.threshold,
        set.validators.len(),
        set.inner_sets.len()
    )];

    for validator in set.validators.iter() {
        lines.push(format!("{indent}  {}", node_strkey(validator)));
    }

    for (inner_index, inner) in set.inner_sets.iter().enumerate() {
        lines.push(format!("{indent}  inner set {inner_index}:"));
        lines.extend(describe_quorum_set(inner, depth + 2));
    }

    lines
}

fn node_strkey(node: &NodeId) -> String {
    match &node.0 {
        PublicKey::PublicKeyTypeEd25519(key) => stellar_strkey::ed25519::PublicKey(key.0)
            .to_string()
            .as_str()
            .to_owned(),
    }
}

fn normalise(entry: &str) -> Result<[u8; 32]> {
    let trimmed = entry.trim().trim_start_matches("0x");

    let decoded = match hex::decode(trimmed) {
        Ok(bytes) => bytes,
        Err(error) => bail!("pinned quorum set hash {entry:?} is not hex: {error}"),
    };

    if decoded.len() != 32 {
        bail!(
            "pinned quorum set hash {entry:?} is {} bytes, expected a 32 byte sha256",
            decoded.len()
        );
    }

    let mut out = [0u8; 32];
    out.copy_from_slice(&decoded);

    Ok(out)
}
