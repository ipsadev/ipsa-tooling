use anyhow::{bail, Result};
use sha2::{Digest, Sha256};

use super::proto::stellar::QuorumConfig;

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

#[cfg(test)]
mod tests {
    use sha2::{Digest, Sha256};

    use super::{verify_quorum_fingerprints, QuorumConfig};

    fn config(xdr: &[u8]) -> QuorumConfig {
        QuorumConfig {
            quorum_set_xdr: xdr.to_vec(),
            valid_from: 0,
        }
    }

    fn pin(xdr: &[u8]) -> String {
        hex::encode::<[u8; 32]>(Sha256::digest(xdr).into())
    }

    #[test]
    fn a_pinned_quorum_set_passes() {
        assert!(verify_quorum_fingerprints(&[config(b"abc")], &[pin(b"abc")]).is_ok());
    }

    #[test]
    fn an_unpinned_quorum_set_is_refused() {
        assert!(verify_quorum_fingerprints(&[config(b"evil")], &[pin(b"abc")]).is_err());
    }

    #[test]
    fn no_pins_at_all_is_refused() {
        assert!(verify_quorum_fingerprints(&[config(b"abc")], &[]).is_err());
    }

    #[test]
    fn every_config_must_be_pinned_not_just_one() {
        let pins = vec![pin(b"abc")];

        assert!(verify_quorum_fingerprints(&[config(b"abc"), config(b"evil")], &pins).is_err());
    }

    #[test]
    fn a_malformed_pin_is_refused() {
        assert!(verify_quorum_fingerprints(&[config(b"abc")], &["zz".to_string()]).is_err());
        assert!(verify_quorum_fingerprints(&[config(b"abc")], &["ab".to_string()]).is_err());
    }
}
