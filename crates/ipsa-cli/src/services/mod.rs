use std::path::Path;

use anyhow::{bail, Result};

use crate::{
    config::{get, Config},
    logger, tools,
};

pub mod cosmos;
pub mod gateway;
pub mod proof_api;
pub mod relayer;
pub mod stellar;
pub mod verifier;

#[derive(clap::Subcommand)]
pub enum ServicesCmd {
    #[command(about = "Pull the service image(s) — tag from .env; all when no flag")]
    Pull(ServiceArgs),
    #[command(about = "Pull + start the container(s); all when no flag")]
    Up(ServiceArgs),
    #[command(about = "Remove the existing container(s) then bring them up; all when no flag")]
    Restart(ServiceArgs),
    #[command(about = "Stop + remove the container(s); all when no flag")]
    Down(ServiceArgs),
    #[command(about = "Build the service image(s) — tag from .env; all when no flag")]
    Build(ServiceArgs),
    #[command(about = "Build + push the service image(s) — tag from .env; all when no flag")]
    Push(ServiceArgs),
}

#[derive(clap::Args)]
pub struct ServiceArgs {
    #[arg(long, help = "Act on the api service")]
    pub api: bool,
    #[arg(long, help = "Act on the gateway service")]
    pub gateway: bool,
    #[arg(long, help = "Act on the cosmos service")]
    pub cosmos: bool,
    #[arg(long, help = "Act on the relayer service (ibcv2 stack)")]
    pub relayer: bool,
    #[arg(long, help = "Act on the postgres service (ibcv2 stack)")]
    pub postgres: bool,
    #[arg(
        long,
        help = "Act on the stellar-core watcher (optional SCP accelerator)"
    )]
    pub watcher: bool,
    #[arg(long, help = "Act on the scp-verifier service")]
    pub verifier: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Service {
    Api,
    Gateway,
    Cosmos,
    Relayer,
    Postgres,
    Watcher,
    Verifier,
}

impl Service {
    fn compose(self) -> &'static str {
        match self {
            Self::Api => "ipsa-api",
            Self::Gateway => "ipsa-gateway",
            Self::Cosmos => "cosmos",
            Self::Relayer => "relayer",
            Self::Postgres => "postgres",
            Self::Watcher => "stellar-watcher",
            Self::Verifier => "scp-verifier",
        }
    }

    fn buildable(self) -> bool {
        matches!(self, Self::Relayer | Self::Verifier)
    }

    fn image(self, cfg: &Config) -> Result<String> {
        let (image, variable) = match self {
            Self::Api => (&cfg.api, "API_IMAGE"),
            Self::Gateway => (&cfg.gateway.image, "GATEWAY_IMAGE"),
            Self::Relayer => (&cfg.relayer.image, "RELAYER_IMAGE"),
            Self::Verifier => (&cfg.verifier.image, "VERIFIER_IMAGE"),
            Self::Cosmos => {
                return Ok(get(
                    "COSMOS_CHAIN_IMAGE",
                    "ghcr.io/cosmos/ibc-go-wasm-simd:v11.0.0",
                ))
            }
            Self::Postgres => return Ok(get("POSTGRES_IMAGE", "postgres:18")),
            Self::Watcher => {
                return Ok(format!(
                    "{}:{}",
                    get("WATCHER_IMAGE", "stellar/stellar-core"),
                    get("WATCHER_TAG", "latest")
                ))
            }
        };

        if image.image.is_empty() {
            bail!(
                "{variable} is not set — add the image name to {}",
                crate::config::tooling_env().display()
            );
        }

        Ok(image.reference())
    }
}

const ALL: [Service; 7] = [
    Service::Api,
    Service::Gateway,
    Service::Cosmos,
    Service::Relayer,
    Service::Postgres,
    Service::Watcher,
    Service::Verifier,
];

fn selected(args: &ServiceArgs) -> Vec<Service> {
    let mut picked = Vec::new();

    if args.api {
        picked.push(Service::Api);
    }

    if args.gateway {
        picked.push(Service::Gateway);
    }

    if args.cosmos {
        picked.push(Service::Cosmos);
    }

    if args.relayer {
        picked.push(Service::Relayer);
    }

    if args.postgres {
        picked.push(Service::Postgres);
    }

    if args.watcher {
        picked.push(Service::Watcher);
    }

    if args.verifier {
        picked.push(Service::Verifier);
    }

    if picked.is_empty() {
        ALL.to_vec()
    } else {
        picked
    }
}

fn names(services: &[Service]) -> Vec<&'static str> {
    services.iter().map(|service| service.compose()).collect()
}

pub fn run(cfg: &Config, root: &Path, cmd: ServicesCmd) -> Result<()> {
    match cmd {
        ServicesCmd::Pull(args) => pull(root, &selected(&args)),
        ServicesCmd::Up(args) => up(root, &selected(&args)),
        ServicesCmd::Restart(args) => restart(root, &selected(&args)),
        ServicesCmd::Down(args) => down(root, &selected(&args)),
        ServicesCmd::Build(args) => build_or_push(cfg, root, &args, false),
        ServicesCmd::Push(args) => build_or_push(cfg, root, &args, true),
    }
}

fn pull(root: &Path, services: &[Service]) -> Result<()> {
    logger::banner(&format!("services pull ({})", names(services).join(", ")));

    let mut argv = vec!["pull", "--ignore-pull-failures"];
    argv.extend(names(services));

    tools::docker::compose(root, &argv)
}

pub fn up(root: &Path, services: &[Service]) -> Result<()> {
    pull(root, services)?;

    logger::banner(&format!("services up ({})", names(services).join(", ")));

    let mut argv = vec!["up", "-d"];
    argv.extend(names(services));

    tools::docker::compose(root, &argv)
}

fn down(root: &Path, services: &[Service]) -> Result<()> {
    logger::banner(&format!("services down ({})", names(services).join(", ")));

    let mut argv = vec!["rm", "-s", "-f"];
    argv.extend(names(services));

    tools::docker::compose(root, &argv)
}

pub fn restart(root: &Path, services: &[Service]) -> Result<()> {
    logger::banner(&format!(
        "services restart ({})",
        names(services).join(", ")
    ));

    down(root, services)?;
    up(root, services)
}

pub fn push(cfg: &Config, root: &Path, services: &[Service]) -> Result<()> {
    for service in services
        .iter()
        .copied()
        .filter(|service| service.buildable())
    {
        build_one(cfg, root, service, true)?;
    }

    Ok(())
}

fn build_or_push(cfg: &Config, root: &Path, args: &ServiceArgs, push: bool) -> Result<()> {
    if args.cosmos {
        bail!("cosmos uses an upstream image — there is nothing to build or push");
    }

    if args.postgres {
        bail!("postgres uses an upstream image — there is nothing to build or push");
    }

    if args.watcher {
        bail!("the watcher runs upstream stellar-core — there is nothing to build or push");
    }

    for service in selected(args)
        .into_iter()
        .filter(|service| service.buildable())
    {
        build_one(cfg, root, service, push)?;
    }

    Ok(())
}

fn build_one(cfg: &Config, root: &Path, service: Service, push: bool) -> Result<()> {
    let verb = if push { "push" } else { "build" };
    logger::banner(&format!("services {verb} {}", service.compose()));
    logger::detail(&format!("image: {}", service.image(cfg)?));

    let (dockerfile, context) = match service {
        Service::Relayer => {
            let repo = relayer_repo(cfg, root)?;

            (format!("{repo}/Dockerfile.relayer"), repo)
        }
        Service::Verifier => {
            let repo = verifier_repo(cfg, root)?;

            (format!("{repo}/Dockerfile"), repo)
        }
        Service::Api
        | Service::Gateway
        | Service::Cosmos
        | Service::Postgres
        | Service::Watcher => return Ok(()),
    };

    build_image(cfg, root, service, &dockerfile, &context, push)
}

fn build_image(
    cfg: &Config,
    root: &Path,
    service: Service,
    dockerfile: &str,
    context: &str,
    push: bool,
) -> Result<()> {
    let image = service.image(cfg)?;

    logger::step("docker build (host arch)");
    tools::docker::command(root, &["build", "-t", &image, "-f", dockerfile, context])?;

    if !push {
        return Ok(());
    }

    docker_login(root)?;

    logger::step("docker push");
    tools::docker::command(root, &["push", &image])
}

fn checkout_repo(
    root: &Path,
    label: &str,
    repo_url: &str,
    branch: &str,
    dir: &str,
) -> Result<String> {
    let checkout = root.join("target").join(dir);
    let repo = checkout.display().to_string();

    if checkout.join(".git").is_dir() {
        logger::step(&format!("updating {label} in {repo}"));
        tools::git::command(&checkout, &["remote", "set-url", "origin", repo_url])?;
    } else {
        logger::step(&format!("cloning {label} from {repo_url}"));
        tools::git::command(root, &["clone", repo_url, &repo])?;
    }

    logger::step(&format!("checking out {branch}"));
    tools::git::command(&checkout, &["fetch", "origin", branch])?;
    tools::git::command(&checkout, &["checkout", branch])?;
    tools::git::command(&checkout, &["reset", "--hard", &format!("origin/{branch}")])?;

    Ok(repo)
}

fn relayer_repo(cfg: &Config, root: &Path) -> Result<String> {
    let relayer = &cfg.relayer;

    if relayer.repo_url.trim().is_empty() {
        let context = relayer.context.clone();

        if context.trim().is_empty() {
            bail!("neither RELAYER_REPO_URL nor RELAYER_CONTEXT is set — set one to build the relayer");
        }

        logger::detail(&format!("RELAYER_REPO_URL unset — building from {context}"));

        return Ok(context);
    }

    checkout_repo(
        root,
        "relayer",
        &relayer.repo_url,
        &relayer.branch,
        "relayer",
    )
}

fn verifier_repo(cfg: &Config, root: &Path) -> Result<String> {
    let verifier = &cfg.verifier;

    if verifier.repo_url.trim().is_empty() {
        bail!(
            "VERIFIER_REPO_URL is not set — add the verifier's git url to {} to build it",
            crate::config::tooling_env().display()
        );
    }

    checkout_repo(
        root,
        "verifier",
        &verifier.repo_url,
        &verifier.branch,
        "verifier",
    )
}

fn docker_login(root: &Path) -> Result<()> {
    let user = get("DOCKER_USERNAME", "");
    let token = get("DOCKER_TOKEN", "");

    if user.is_empty() || token.is_empty() {
        logger::detail("DOCKER_USERNAME / DOCKER_TOKEN unset — assuming an existing docker login");

        return Ok(());
    }

    logger::step("docker login");

    tools::docker::piped(root, &["login", "-u", &user, "--password-stdin"], &token)
}
