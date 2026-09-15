use std::path::PathBuf;

use anyhow::{bail, Result};

const MARKER: &str = "docker-compose.yml";

pub fn find_root() -> Result<PathBuf> {
    if let Ok(explicit) = std::env::var("STELLAR_IBC_ROOT") {
        let path = PathBuf::from(explicit);
        if path.join(MARKER).exists() {
            return Ok(path);
        }
        bail!("STELLAR_IBC_ROOT is set but {MARKER} not found under it");
    }

    let mut dir = std::env::current_dir()?;
    loop {
        if dir.join(MARKER).exists() {
            return Ok(dir);
        }
        if !dir.pop() {
            break;
        }
    }

    bail!("could not locate the ipsa-backend stack root (looked for {MARKER} upward from the current directory; set STELLAR_IBC_ROOT to the ipsa-backend checkout)");
}
