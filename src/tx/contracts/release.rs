use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

use anyhow::{bail, Context, Result};
use sha2::{Digest, Sha256};

use crate::{logger, tools};

const REPOSITORY: &str = "ipsadev/ipsa-contracts";
const RELEASE_TAG: &str = "v0.1.0";
const CHECKSUMS: &str = "SHA256SUMS";

pub fn fetch(root: &Path) -> Result<PathBuf> {
    let tag = RELEASE_TAG;
    let directory = root.join("dist/contracts").join(tag);

    if verify(&directory).is_ok() {
        logger::detail(&format!(
            "{REPOSITORY}@{tag} already verified in {}",
            directory.display()
        ));

        return Ok(directory);
    }

    logger::step(&format!("downloading {REPOSITORY}@{tag}"));

    std::fs::create_dir_all(&directory)
        .with_context(|| format!("creating {}", directory.display()))?;

    let destination = directory.display().to_string();

    tools::gh::command(
        root,
        &[
            "release",
            "download",
            tag,
            "--repo",
            REPOSITORY,
            "--pattern",
            "*.wasm",
            "--pattern",
            CHECKSUMS,
            "--dir",
            &destination,
            "--clobber",
        ],
    )
    .with_context(|| format!("downloading the {tag} release of {REPOSITORY}"))?;

    verify(&directory)?;

    logger::ok(&format!(
        "{REPOSITORY}@{tag} verified in {}",
        directory.display()
    ));

    Ok(directory)
}

fn verify(directory: &Path) -> Result<()> {
    let listing = std::fs::read_to_string(directory.join(CHECKSUMS))
        .with_context(|| format!("reading {CHECKSUMS} in {}", directory.display()))?;
    let expected = parse_checksums(&listing);

    for name in super::SOROBAN_WASMS {
        let file = format!("{name}.wasm");

        if !expected.contains_key(&file) {
            bail!("{CHECKSUMS} does not list {file}");
        }
    }

    for (file, digest) in &expected {
        let bytes = std::fs::read(directory.join(file))
            .with_context(|| format!("reading {file} in {}", directory.display()))?;
        let actual = hex::encode(Sha256::digest(&bytes));

        if &actual != digest {
            bail!(
                "{file} has sha256 {actual}, but {CHECKSUMS} publishes {digest}; do not deploy it"
            );
        }
    }

    Ok(())
}

fn parse_checksums(listing: &str) -> BTreeMap<String, String> {
    listing
        .lines()
        .filter_map(|line| {
            let mut fields = line.split_whitespace();
            let digest = fields.next()?.to_ascii_lowercase();
            let file = fields.next()?.trim_start_matches('*').to_string();

            Some((file, digest))
        })
        .collect()
}
