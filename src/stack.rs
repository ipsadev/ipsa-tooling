use std::path::Path;

use anyhow::Result;

use crate::{logger, tools};

pub fn up(root: &Path, cosmos_only: bool, stellar_only: bool) -> Result<()> {
    logger::banner("up — docker compose");

    let services: Vec<&str> = if cosmos_only {
        vec!["cosmos"]
    } else if stellar_only {
        vec!["stellar-watcher", "ipsa-api", "ipsa-gateway"]
    } else {
        vec!["cosmos", "stellar-watcher", "ipsa-api", "ipsa-gateway"]
    };

    logger::step(&format!("starting: {}", services.join(", ")));

    let mut args = vec!["up", "-d"];
    args.extend_from_slice(&services);
    tools::docker::compose(root, &args)?;

    logger::ok("services started (detached)");
    logger::hint("check readiness with: ipsa check");

    Ok(())
}

pub fn down(root: &Path, volumes: bool) -> Result<()> {
    logger::banner("down — docker compose");

    let mut args = vec!["down"];

    if volumes {
        args.push("--volumes");
    }

    tools::docker::compose(root, &args)?;
    logger::ok("stack stopped");

    Ok(())
}
