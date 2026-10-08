use std::path::{Path, PathBuf};

use anyhow::Result;

use crate::{logger, run};

pub fn run(root: &Path) -> Result<()> {
    logger::banner("install — cargo install ipsa");

    let crate_dir = Path::new(env!("CARGO_MANIFEST_DIR"));

    logger::step(&format!(
        "cargo install --path {} --force",
        crate_dir.display()
    ));
    run::command(
        root,
        "cargo",
        &[
            "install",
            "--path",
            crate_dir.to_str().unwrap_or("."),
            "--force",
        ],
    )?;

    let bin_dir = cargo_bin_dir();
    logger::ok(&format!("installed: {}", bin_dir.join("ipsa").display()));

    if on_path(bin_dir.as_path()) {
        logger::ok(&format!(
            "{} is on PATH — run: ipsa check",
            bin_dir.display()
        ));
    } else {
        logger::warn(&format!("{} is not on PATH", bin_dir.display()));
        logger::detail("add it to your shell profile (bash/zsh):");
        logger::detail(&format!("export PATH=\"{}:$PATH\"", bin_dir.display()));
    }

    Ok(())
}

fn cargo_bin_dir() -> PathBuf {
    if let Ok(home) = std::env::var("CARGO_HOME") {
        return PathBuf::from(home).join("bin");
    }

    if let Ok(home) = std::env::var("HOME") {
        return PathBuf::from(home).join(".cargo").join("bin");
    }

    PathBuf::from(".cargo/bin")
}

fn on_path(dir: &Path) -> bool {
    std::env::var_os("PATH")
        .map(|paths| std::env::split_paths(&paths).any(|entry| entry == dir))
        .unwrap_or(false)
}
