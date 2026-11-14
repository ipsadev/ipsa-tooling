use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};

const MARKER: &str = "docker-compose.yml";

const RELAYER_CONFIG: &str = "relayer.yml";

const RELAYER_TEMPLATE: &str = "relayer.example.yml";

pub fn stack_dir() -> PathBuf {
    let explicit = std::env::var("IPSA_STACK_DIR").unwrap_or_default();

    if !explicit.trim().is_empty() {
        return PathBuf::from(explicit.trim());
    }

    Path::new(env!("CARGO_MANIFEST_DIR")).join("stack")
}

pub fn find_root() -> Result<PathBuf> {
    let root = stack_dir();

    if !root.join(MARKER).exists() {
        bail!(
            "no {MARKER} in {} — the stack ships in ipsa-tooling/stack; set IPSA_STACK_DIR only to use a copy of it",
            root.display()
        );
    }

    ensure_relayer_config(&root)?;

    Ok(root)
}

fn ensure_relayer_config(root: &Path) -> Result<()> {
    let config = root.join(RELAYER_CONFIG);

    if config.exists() {
        return Ok(());
    }

    let template = root.join(RELAYER_TEMPLATE);

    std::fs::copy(&template, &config)
        .with_context(|| format!("creating {} from {}", config.display(), template.display()))?;

    Ok(())
}
